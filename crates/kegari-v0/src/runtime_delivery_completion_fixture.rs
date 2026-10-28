//! Synthetic upstream acceptance, real native-wire target proof and durable
//! runtime completion. This does not manufacture integrated execution evidence.
use super::*;

#[path = "runtime_owned_target_resume_fixture.rs"]
mod owned_resume;

struct TerminalFixture {
    built: crate::runtime_fixture::RuntimeFixture,
    data: Arc<std::sync::Mutex<wire::Fixture>>,
    lease: Lease,
    github: RuntimeSeedWire,
    native: Arc<RuntimeLineageWire>,
    merged: kegari_git::Revision,
    observation_barrier: Arc<tokio::sync::Mutex<()>>,
}

/// Run the complete terminal phase as one owned task. The earlier Git/lineage
/// phases keep their roots alive until this handle is joined, without nesting
/// their large fixture poll frames under every terminal provider observation.
pub(super) fn complete_and_recover(
    f: &ChainFixture<'_>,
    delivery: &Arc<RuntimeDelivery>,
    workspace: &WorkspaceDelivery,
    gate: &FreshGate,
    c: AttemptRecord,
    manifest: &evidence::ParentEvidence,
) -> tokio::task::JoinHandle<()> {
    let f = TerminalFixture {
        built: crate::runtime_fixture::RuntimeFixture {
            runtime: f.built.runtime.clone(),
            configuration: f.built.configuration.clone(),
            processes: f.built.processes.clone(),
        },
        data: f.data.clone(),
        lease: f.lease.clone(),
        github: f.github.clone(),
        native: f.native.clone(),
        merged: f.merged.clone(),
        observation_barrier: f.observation_barrier.clone(),
    };
    let delivery = delivery.clone();
    let workspace = workspace.clone();
    let gate = gate.clone();
    let manifest = manifest.clone();
    tokio::spawn(async move {
        tokio::time::timeout(
            std::time::Duration::from_secs(300),
            terminal_history(f, delivery, workspace, gate, c, manifest),
        )
        .await
        .expect("owned terminal fixture exceeded its finite phase bound");
    })
}

fn terminal_history(
    f: TerminalFixture,
    delivery: Arc<RuntimeDelivery>,
    workspace: WorkspaceDelivery,
    gate: FreshGate,
    mut c: AttemptRecord,
    manifest: evidence::ParentEvidence,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let delivery = &delivery;
        let workspace = &workspace;
        let gate = &gate;
        let manifest = &manifest;
        eprintln!("completion fixture: entered terminal C history scenario");
        let initial_history = delivery.release_attempts().unwrap();
        f.built.runtime.observations.refresh().await.unwrap();
        let mut permit = f
            .built
            .runtime
            .control
            .admit(
                &f.lease,
                c.attempt.plan.operation_id,
                gate.work_key().unwrap(),
                c.attempt.plan.digest().unwrap(),
                || (delivery.current_gate())(gate),
            )
            .unwrap();
        c.attempt.control_generation = permit.control_generation;
        c.attempt.owner_generation = permit.lease.owner_generation;
        c.attempt.hold_reason = None;
        c.local_retired = false;
        c.attempt
            .steps
            .values_mut()
            .for_each(|state| *state = StepState::NotStarted);
        c.attempt.steps.insert(
            "deploy".into(),
            StepState::SentUnknown {
                payload_digest: kegari_digest::bytes_digest(
                    b"synthetic accepted dispatch preparation",
                ),
            },
        );
        delivery.save_release(&c).unwrap();
        {
            let mut state = delivery.releases.state.lock().unwrap();
            state
                .attempts
                .insert(c.attempt.plan.operation_id, c.clone());
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
        // Same synthetic accepted C, two genuine refused broker attempts and
        // explicitly authorized fresh Control resume admissions. No Permit is read
        // from JSON; each returned permit is the actual admission result.
        for ordinal in 1..=2 {
            f.built.runtime.observations.refresh().await.unwrap();
            let failed = f
                .native
                .failed_target_release(TargetReleaseFixture {
                    coordinator: &workspace.coordinator,
                    observer: &workspace.observer,
                    gate,
                    permit: &permit,
                    plan: &c.attempt.plan,
                    vector_digest: &manifest.candidate_vector_digest().unwrap(),
                    attempt: ordinal,
                })
                .await;
            delivery
                .record_release_observation(c.attempt.plan.operation_id, &failed)
                .unwrap();
            let held = delivery.releases.state.lock().unwrap().attempts
                [&c.attempt.plan.operation_id]
                .clone();
            delivery
                .retire_release(c.attempt.plan.operation_id, &held)
                .unwrap();
            permit = authorize_resume(delivery, &f.lease, gate, c.attempt.plan.operation_id).await;
            let receipt = delivery
                .runtime
                .control
                .operation(permit.operation_id)
                .unwrap();
            delivery
                .update_release(c.attempt.plan.operation_id, |record, _| {
                    let prior = record
                        .attempt
                        .steps
                        .insert(
                            "deploy".into(),
                            StepState::SentUnknown {
                                payload_digest: kegari_digest::bytes_digest(
                                    b"synthetic accepted resumed step",
                                ),
                            },
                        )
                        .unwrap();
                    assert!(matches!(prior, StepState::Failed { .. }));
                    record
                        .attempt
                        .step_history
                        .entry("deploy".into())
                        .or_default()
                        .push(prior);
                    record
                        .attempt
                        .recovery_records
                        .push(domain::RecoveryRecord {
                            actor: "operator".into(),
                            action: "resume".into(),
                            reason: "explicit fixture retry after actual no-dispatch observation"
                                .into(),
                            baseline_digest: None,
                            observed_at: Utc::now(),
                        });
                    record.attempt.hold_reason = None;
                    record.attempt.owner_generation = permit.lease.owner_generation;
                    record.attempt.control_generation = permit.control_generation;
                    record.admission_operation = permit.operation_id;
                    record
                        .recovery_receipts
                        .insert(receipt.operation_id, receipt.payload_digest.clone());
                    record.local_retired = false;
                    record.admission_confirmed = true;
                    Ok(())
                })
                .unwrap();
        }
        delivery
            .update_release(c.attempt.plan.operation_id, |record, _| {
                record.attempt.steps.insert(
                    "verify".into(),
                    StepState::SentUnknown {
                        payload_digest: kegari_digest::bytes_digest(
                            b"synthetic accepted verify preparation",
                        ),
                    },
                );
                Ok(())
            })
            .unwrap();
        f.built.runtime.observations.refresh().await.unwrap();
        let observed = f
            .native
            .complete_target_release(TargetReleaseFixture {
                coordinator: &workspace.coordinator,
                observer: &workspace.observer,
                gate,
                permit: &permit,
                plan: &c.attempt.plan,
                vector_digest: &manifest.candidate_vector_digest().unwrap(),
                attempt: 3,
            })
            .await;
        assert_eq!(observed.len(), 2);
        for effect in &observed {
            delivery
                .record_release_observation(c.attempt.plan.operation_id, effect)
                .unwrap();
        }
        let mut later = c.attempt.plan.clone();
        later.operation_id = Uuid::new_v4();
        later.parent_id = Uuid::new_v4();
        later.spacing_seconds = 0;
        let after_spacing = Utc::now() + Duration::seconds(120);
        let projection = f.built.runtime.observations.refresh().await.unwrap();
        delivery.complete_release_chains(&projection).await.unwrap();
        assert!(
            delivery
                .releases
                .state
                .lock()
                .unwrap()
                .completions
                .is_empty()
        );
        assert!(
            delivery
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&later, after_spacing)
                .is_err()
        );
        Box::pin(owned_resume::assert_unretired(workspace)).await;
        let current =
            delivery.releases.state.lock().unwrap().attempts[&c.attempt.plan.operation_id].clone();
        delivery
            .retire_release(c.attempt.plan.operation_id, &current)
            .unwrap();
        Box::pin(owned_resume::assert_closed(
            &f, delivery, workspace, &permit,
        ))
        .await;
        let current =
            delivery.releases.state.lock().unwrap().attempts[&c.attempt.plan.operation_id].clone();
        assert!(current.local_retired && current.attempt.completed());
        assert_eq!(
            delivery
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&later, after_spacing)
                .unwrap_err()
                .code,
            "release_hold"
        );
        // Divergence at first mint cannot establish a coherent terminal inventory.
        let divergent = kegari_git::Revision {
            commit: "f".repeat(40),
            tree: "e".repeat(40),
        };
        f.github.set_production(&divergent);
        let divergence = delivery
            .complete_release_chains(&projection)
            .await
            .unwrap_err();
        assert!(delivery.release_history().unwrap().iter().any(
            |row| row.attempt.plan.operation_id == c.attempt.plan.operation_id
                && row.completion_error.is_some()
        ));
        delivery
            .releases
            .state
            .lock()
            .unwrap()
            .completion_errors
            .clear();
        // Exercise the production progression facade: maintenance must actually
        // revisit retained release effects, even while new admissions are off.
        crate::runtime_driver::RuntimeProgression::maintain(delivery.as_ref())
            .await
            .unwrap();
        assert_eq!(
            delivery.releases.state.lock().unwrap().completion_errors[&c.attempt.plan.operation_id]
                .code,
            divergence.code
        );
        let history = inspect_history(delivery, &issue(&c).unwrap()).await;
        assert!(
            history
                .iter()
                .any(|row| row["attempt"]["plan"]["operation_id"]
                    == json!(c.attempt.plan.operation_id)
                    && !row["diagnostic"].is_null())
        );
        assert!(history.iter().any(|row| row["active_hold"] == true));
        assert!(
            delivery
                .releases
                .state
                .lock()
                .unwrap()
                .completions
                .is_empty()
        );
        f.github.set_production(&f.merged);
        let completion_file = format!("release-completion-{}.json", c.attempt.plan.operation_id);
        // A present malformed record must not be replaced by recomputation.
        let tamper_window = f.observation_barrier.lock().await;
        delivery
            .records
            .write(&completion_file, &json!({"invalid":"interrupted record"}))
            .unwrap();
        let corrupt = delivery
            .complete_release_chains(&projection)
            .await
            .unwrap_err();
        assert_eq!(corrupt.code, "record_corrupt");
        let unavailable = delivery
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .eligible(&later, after_spacing)
            .unwrap_err();
        assert_eq!(unavailable.code, corrupt.code);
        assert_eq!(unavailable.message, corrupt.message);
        std::fs::remove_file(delivery.records.path().join(&completion_file)).unwrap();
        // Removing an untrusted inventory entry does not clear its installed
        // fatal status. A complete successful scan is required before new seals.
        assert_eq!(
            delivery
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&later, after_spacing)
                .unwrap_err()
                .code,
            "record_corrupt"
        );
        delivery.sync_released_completions().unwrap();
        assert_eq!(
            delivery
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&later, after_spacing)
                .unwrap_err()
                .code,
            "release_hold"
        );
        drop(tamper_window);
        let projection = f.built.runtime.observations.refresh().await.unwrap();
        delivery.complete_release_chains(&projection).await.unwrap();
        let proof = delivery.releases.state.lock().unwrap().completions
            [&c.attempt.plan.operation_id]
            .clone();
        assert_eq!(proof.validate().unwrap().len(), 2);
        let success = proof
            .validate_for_configuration(&workspace.configuration)
            .unwrap();
        assert_eq!(proof.work(), &c.work);
        assert_eq!(success.operation_id, c.attempt.plan.operation_id);
        let outcomes = f.built.runtime.dependency_outcomes().unwrap();
        assert_eq!(
            outcomes.parent_outcome(&issue(&c).unwrap(), &c.work.generation),
            Some(success.outcome_digest.as_str())
        );
        for ancestor in &initial_history {
            assert_ne!(ancestor.plan.generation, c.work.generation);
            assert!(
                outcomes
                    .parent_outcome(&issue(&c).unwrap(), &ancestor.plan.generation)
                    .is_none(),
                "a consumed ancestor hold is not a release of its failed generation"
            );
        }
        assert!(
            delivery
                .release_history()
                .unwrap()
                .iter()
                .all(|row| row.completion_error.is_none())
        );
        let history = inspect_history(delivery, &issue(&c).unwrap()).await;
        for ancestor in &initial_history {
            let row = history
                .iter()
                .find(|row| {
                    row["attempt"]["plan"]["operation_id"] == json!(ancestor.plan.operation_id)
                })
                .unwrap();
            assert_eq!(row["completed_by"], json!(c.attempt.plan.operation_id));
            assert_eq!(row["active_hold"], false);
            assert!(row["diagnostic"].is_null());
        }
        let queue = delivery.releases.state.lock().unwrap().queue.clone();
        assert_eq!(
            queue.eligible(&later, Utc::now()).unwrap_err().code,
            "release_spacing"
        );
        queue.eligible(&later, after_spacing).unwrap();
        later.dependencies = vec![domain::DependencyRequirement {
            parent_id: c.work.parent,
            generation: c.work.generation.clone(),
            outcome_digest: canonical_digest(1, &current.attempt.steps).unwrap(),
        }];
        queue.eligible(&later, after_spacing).unwrap();
        later.dependencies[0].outcome_digest =
            canonical_digest(1, &initial_history[0].steps).unwrap();
        assert_eq!(
            queue.eligible(&later, after_spacing).unwrap_err().code,
            "release_dependency"
        );
        later.dependencies.clear();
        for prior in &initial_history {
            let history = delivery.release_history().unwrap();
            let row = history
                .iter()
                .find(|row| row.attempt.plan.operation_id == prior.plan.operation_id)
                .unwrap();
            assert_eq!(row.attempt, *prior);
            assert!(!row.active_hold);
            assert_eq!(row.completed_by, Some(c.attempt.plan.operation_id));
        }
        let mut unrelated = initial_history[0].clone();
        unrelated.plan.operation_id = Uuid::new_v4();
        unrelated.assigned_successor = None;
        let mut other = queue.clone();
        other.restore_hold(&unrelated).unwrap();
        other.apply_completion(&proof).unwrap();
        assert_eq!(
            other.eligible(&later, after_spacing).unwrap_err().code,
            "release_hold"
        );
        // Every actual resume/authorization/completion and prior no-dispatch record
        // is retained. Deleting or replacing any reference invalidates both the
        // cached queue and a restarted recovery reader, without native reobservation.
        let completion: Signed<Value> = delivery.records.read(&completion_file).unwrap();
        let refs = completion.payload["receipts"].as_object().unwrap();
        let closed = delivery
            .runtime
            .control
            .closed_release_resume_receipts(permit.operation_id)
            .unwrap();
        assert_eq!(closed.len(), 9);
        for receipt in &closed {
            assert!(refs.contains_key(&receipt.operation_id.to_string()));
        }
        let receipt_root = f.built.runtime.registration.root().path().join("receipts");
        let receipt_dir = kegari_storage::PrivateDir::open(&receipt_root).unwrap();
        // Control owns its descriptor-relative receipt layout; locate only known
        // operation UUID leaves within this owned fixture registration.
        let tamper_window = f.observation_barrier.lock().await;
        for receipt in closed
            .iter()
            .filter(|r| r.operation_id != c.attempt.plan.operation_id)
        {
            let path = receipt_root.join(format!("{}.json", receipt.operation_id));
            let bytes = std::fs::read(&path).unwrap();
            std::fs::remove_file(&path).unwrap();
            assert!(queue.eligible(&later, after_spacing).is_err());
            receipt_dir
                .write_bytes(&format!("{}.json", receipt.operation_id), &bytes)
                .unwrap();
            let mut changed = receipt.clone();
            changed.actor = "substituted historical actor".into();
            receipt_dir
                .write(&format!("{}.json", receipt.operation_id), &changed)
                .unwrap();
            assert!(queue.eligible(&later, after_spacing).is_err());
            receipt_dir
                .write_bytes(&format!("{}.json", receipt.operation_id), &bytes)
                .unwrap();
            queue.eligible(&later, after_spacing).unwrap();
        }
        for id in refs.keys() {
            let id: Uuid = id.parse().unwrap();
            let actual = delivery.runtime.control.operation(id).unwrap();
            if actual.kind != "release_recovery_transition" {
                continue;
            }
            let name = format!("{id}.json");
            for changed in [
                {
                    let mut r = actual.clone();
                    r.observed_result.as_mut().unwrap()["attempt_digest"] =
                        json!(kegari_digest::bytes_digest(b"changed-history"));
                    r
                },
                {
                    let mut r = actual.clone();
                    r.observed_result.as_mut().unwrap()["resolved"] = json!(true);
                    r
                },
                {
                    let mut r = actual.clone();
                    r.observed_result.as_mut().unwrap()["abandoned"] = json!(true);
                    r
                },
                {
                    let mut r = actual.clone();
                    r.observed_result.as_mut().unwrap()["hold"] = json!("changed hold");
                    r
                },
                {
                    let mut r = actual.clone();
                    r.remote_ids.insert("unrelated".into(), "remote".into());
                    r
                },
            ] {
                receipt_dir.write(&name, &changed).unwrap();
                assert!(queue.eligible(&later, after_spacing).is_err());
                receipt_dir.write(&name, &actual).unwrap();
                queue.eligible(&later, after_spacing).unwrap();
            }
        }
        let mut files = completion.payload["effects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (false, r["leaf"].as_str().unwrap().to_owned()))
            .collect::<Vec<_>>();
        files.extend(
            completion.payload["target_proofs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| (true, r["leaf"].as_str().unwrap().to_owned())),
        );
        assert!(
            files
                .iter()
                .any(|(_, leaf)| leaf.starts_with("target-proof-"))
        );
        assert!(
            files
                .iter()
                .any(|(_, leaf)| leaf.starts_with("deployment-challenge-"))
        );
        assert!(
            !files
                .iter()
                .any(|(_, leaf)| leaf.starts_with("target-index-"))
        );
        for origin in completion.payload["origins"].as_object().unwrap().values() {
            files.push((false, origin["vector"]["leaf"].as_str().unwrap().into()));
        }
        assert!(
            files
                .iter()
                .any(|(_, leaf)| leaf.contains("no-dispatch") && leaf.ends_with("attempt-1.json"))
        );
        assert!(
            files
                .iter()
                .any(|(_, leaf)| leaf.contains("no-dispatch") && leaf.ends_with("attempt-2.json"))
        );
        files.sort();
        files.dedup();
        let workspace_dir = kegari_storage::PrivateDir::open(
            &delivery.records.path().join(c.workspace.to_string()),
        )
        .unwrap();
        let deployment_dir =
            kegari_storage::PrivateDir::open(&workspace_dir.path().join("deployment")).unwrap();
        for (target_reference, leaf) in files {
            let store = if target_reference {
                &deployment_dir
            } else {
                &workspace_dir
            };
            let path = store.path().join(&leaf);
            let bytes = std::fs::read(&path).unwrap();
            std::fs::remove_file(&path).unwrap();
            assert!(queue.eligible(&later, after_spacing).is_err());
            store.write_bytes(&leaf, &bytes).unwrap();
            queue.eligible(&later, after_spacing).unwrap();
        }
        drop(tamper_window);
        // Actual authenticated commands cannot rewrite either completed terminal
        // C or its consumed ancestors, even though domain abandon supports partials.
        for operation in [
            c.attempt.plan.operation_id,
            initial_history[0].plan.operation_id,
        ] {
            let before = delivery
                .records
                .read_bytes(&attempt_leaf(operation), 1024 * 1024)
                .unwrap();
            for action in ["abandon", "resolve", "supersede"] {
                let error = execute_recovery(delivery, &f.lease, gate, operation, action, false)
                    .await
                    .unwrap_err();
                assert_eq!(error.code, "release_completed");
                assert_eq!(
                    delivery
                        .records
                        .read_bytes(&attempt_leaf(operation), 1024 * 1024)
                        .unwrap(),
                    before
                );
                assert_eq!(
                    proof.digest(),
                    delivery.releases.state.lock().unwrap().completions
                        [&c.attempt.plan.operation_id]
                        .digest()
                );
                queue.eligible(&later, after_spacing).unwrap();
            }
        }
        // Later same-parent correction must not rediscover consumed A/B blockers.
        let next = correct_parent(&f.built, &f.data, &f.lease).await;
        let projection = f.built.runtime.observations.refresh().await.unwrap();
        assert!(
            delivery
                .known_predecessor_baseline(&projection, &next)
                .await
                .unwrap()
                .is_none()
        );
        f.github.set_production(&divergent);
        let restored = RuntimeDelivery::with_fixture(
            f.built.runtime.clone(),
            delivery.processes.clone(),
            delivery.git_path().unwrap(),
        )
        .unwrap();
        restored.restore_releases().unwrap();
        restored
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .eligible(&later, after_spacing)
            .unwrap();
        // Crash after immutable write but before cache commit: use the saved proof,
        // without asking later production D to equal C or renewing completion time.
        {
            let mut state = restored.releases.state.lock().unwrap();
            state.completions.clear();
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
        restored.complete_release_chains(&projection).await.unwrap();
        assert_eq!(
            restored.releases.state.lock().unwrap().completions[&c.attempt.plan.operation_id]
                .digest(),
            proof.digest()
        );
        restored
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .eligible(&later, after_spacing)
            .unwrap();
        // The historical later-production assertions above are complete. Keep
        // ordinary provider observations available for the independent read-only
        // missing-reference inspection checks below.
        f.github.set_production(&f.merged);
        // Prove the cold queue restores A/B holds before introducing an
        // unrelated overlapping hold that could otherwise mask this regression.
        let tamper_window = f.observation_barrier.lock().await;
        let signed: Signed<Value> = delivery.records.read(&completion_file).unwrap();
        std::fs::remove_file(delivery.records.path().join(&completion_file)).unwrap();
        assert!(queue.eligible(&later, after_spacing).is_err());
        let missing = RuntimeDelivery::with_fixture(
            f.built.runtime.clone(),
            delivery.processes.clone(),
            delivery.git_path().unwrap(),
        )
        .unwrap();
        missing.restore_releases().unwrap();
        assert_eq!(
            missing
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&later, after_spacing)
                .unwrap_err()
                .code,
            "record_missing"
        );
        delivery.records.write(&completion_file, &signed).unwrap();
        let intent_file = format!("release-successor-{}.json", c.attempt.plan.operation_id);
        let intent: Signed<Value> = delivery.records.read(&intent_file).unwrap();
        std::fs::remove_file(delivery.records.path().join(&intent_file)).unwrap();
        assert!(queue.eligible(&later, after_spacing).is_err());
        delivery.records.write(&intent_file, &intent).unwrap();
        queue.eligible(&later, after_spacing).unwrap();
        drop(tamper_window);
        // A separate retained plan has synthetic upstream input but an actual
        // admitted/retired Control owner and no dispatched effects. Its history
        // must remain visible when C's completion evidence becomes unavailable.
        let unrelated = retain_unrelated_history(delivery, &f.lease, &c);
        let target_leaf = completion.payload["target_proofs"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|reference| {
                reference["leaf"]
                    .as_str()
                    .filter(|leaf| leaf.starts_with("target-proof-"))
            })
            .unwrap();
        let target_bytes = deployment_dir.read_bytes(target_leaf, 1024 * 1024).unwrap();
        let tamper_window = f.observation_barrier.lock().await;
        std::fs::remove_file(deployment_dir.path().join(target_leaf)).unwrap();
        assert!(queue.eligible(&later, after_spacing).is_err());
        assert!(
            f.built
                .runtime
                .dependency_outcomes()
                .unwrap()
                .parent_outcome(&issue(&c).unwrap(), &c.work.generation)
                .is_none()
        );
        assert_diagnostic_inspection(
            delivery,
            &c,
            &initial_history,
            &unrelated,
            "cached target-proof deletion",
        )
        .await;
        let cold = RuntimeDelivery::with_fixture(
            f.built.runtime.clone(),
            delivery.processes.clone(),
            delivery.git_path().unwrap(),
        )
        .unwrap();
        cold.restore_releases().unwrap();
        assert_eq!(
            cold.releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&later, after_spacing)
                .unwrap_err()
                .code,
            "record_missing"
        );
        cold.sync_released_completions().unwrap();
        assert!(
            f.built
                .runtime
                .dependency_outcomes()
                .unwrap()
                .parent_outcome(&issue(&c).unwrap(), &c.work.generation)
                .is_none()
        );
        assert_diagnostic_inspection(
            &cold,
            &c,
            &initial_history,
            &unrelated,
            "cold target-proof deletion",
        )
        .await;
        deployment_dir
            .write_bytes(target_leaf, &target_bytes)
            .unwrap();
        queue.eligible(&later, after_spacing).unwrap();
        drop(tamper_window);
        // A signed attempt can still be presented diagnostically if its actual
        // owner reference disappears; scoped restoration preserves affected refusal.
        let owner_leaf = format!("{}.json", permit.operation_id);
        let owner_bytes = receipt_dir.read_bytes(&owner_leaf, 1024 * 1024).unwrap();
        let tamper_window = f.observation_barrier.lock().await;
        std::fs::remove_file(receipt_dir.path().join(&owner_leaf)).unwrap();
        assert!(queue.eligible(&later, after_spacing).is_err());
        assert_owner_history(delivery, &c, &unrelated);
        let owner_cold = RuntimeDelivery::with_fixture(
            f.built.runtime.clone(),
            delivery.processes.clone(),
            delivery.git_path().unwrap(),
        )
        .unwrap();
        owner_cold.restore_releases().unwrap();
        assert_eq!(
            owner_cold
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&later, after_spacing)
                .unwrap_err()
                .code,
            "record_missing"
        );
        assert_owner_history(&owner_cold, &c, &unrelated);
        receipt_dir.write_bytes(&owner_leaf, &owner_bytes).unwrap();
        queue.eligible(&later, after_spacing).unwrap();
        drop(tamper_window);
        f.github.set_production(&f.merged);
    })
}

fn retain_unrelated_history(
    delivery: &Arc<RuntimeDelivery>,
    lease: &Lease,
    source: &AttemptRecord,
) -> AttemptRecord {
    let mut plan = source.attempt.plan.clone();
    plan.operation_id = Uuid::new_v4();
    plan.parent_id = wire::id(9876);
    let mut work = source.work.clone();
    work.parent = plan.parent_id;
    let mut record = super::super::super::tests::record(plan, work);
    record
        .attempt
        .steps
        .values_mut()
        .for_each(|state| *state = StepState::NotStarted);
    record.attempt.hold_reason =
        Some("synthetic unrelated retained input; no external dispatch".into());
    let permit = delivery
        .runtime
        .control
        .admit(
            lease,
            record.attempt.plan.operation_id,
            issue(&record).unwrap().identity().unwrap(),
            record.attempt.plan.digest().unwrap(),
            || delivery.runtime.registration.verify_active_owner(lease),
        )
        .unwrap();
    delivery.runtime.control.fence(permit.operation_id).unwrap();
    delivery
        .runtime
        .control
        .retire_local(
            permit.operation_id,
            json!({"fixture":"no external dispatch"}),
        )
        .unwrap();
    record.attempt.owner_generation = permit.lease.owner_generation;
    record.attempt.control_generation = permit.control_generation;
    record.local_retired = true;
    delivery.save_release(&record).unwrap();
    {
        let mut state = delivery.releases.state.lock().unwrap();
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
    record
}

fn assert_owner_history(
    delivery: &RuntimeDelivery,
    terminal: &AttemptRecord,
    unrelated: &AttemptRecord,
) {
    let history = delivery.release_history().unwrap();
    assert!(
        history
            .iter()
            .all(|row| row.completed_by != Some(terminal.attempt.plan.operation_id))
    );
    let affected = history
        .iter()
        .find(|row| row.attempt.plan.operation_id == terminal.attempt.plan.operation_id)
        .unwrap();
    assert!(affected.active_hold);
    assert!(affected.completed_by.is_none());
    assert!(affected.completion_error.is_some());
    let other = history
        .iter()
        .find(|row| row.attempt.plan.operation_id == unrelated.attempt.plan.operation_id)
        .unwrap();
    assert_eq!(other.attempt, unrelated.attempt);
    assert!(other.completion_error.is_none());
}

async fn assert_diagnostic_inspection(
    delivery: &Arc<RuntimeDelivery>,
    terminal: &AttemptRecord,
    ancestors: &[ReleaseAttempt],
    unrelated: &AttemptRecord,
    stage: &str,
) {
    crate::runtime_driver::RuntimeProgression::reconcile(delivery.as_ref())
        .await
        .unwrap_or_else(|error| panic!("{stage}: scoped refresh failed globally: {error:?}"));
    let strict = delivery
        .releases
        .state
        .lock()
        .unwrap()
        .queue
        .eligible(&terminal.attempt.plan, Utc::now())
        .unwrap_err();
    assert_eq!(strict.code, "record_missing", "{stage}");
    assert_eq!(
        strict.details["completion_stage"], "target_reference",
        "{stage}"
    );
    let workflow = crate::runtime_workflow::RuntimeWorkflow::new(
        delivery.runtime.clone(),
        delivery.clone(),
        delivery.clone(),
    );
    let view = crate::runtime_host::WorkflowControl::inspect(&workflow)
        .await
        .unwrap_or_else(|error| panic!("{stage}: actual diagnostic inspection failed: {error:?}"));
    assert!(
        view.delivery.is_empty(),
        "{stage}: unavailable release prerequisites must not preserve stale product readiness"
    );
    let health = delivery.runtime.account_observations().unwrap();
    assert_eq!(
        health["delivery"][issue(terminal).unwrap().identity().unwrap()]["code"],
        "release_gates_unavailable",
        "{stage}"
    );
    let rows = inspection_rows(&view, terminal.workspace, &issue(terminal).unwrap());
    let current = rows
        .iter()
        .find(|row| {
            row["attempt"]["plan"]["operation_id"] == json!(terminal.attempt.plan.operation_id)
        })
        .unwrap();
    assert!(!current["diagnostic"].is_null());
    assert!(current["completed_by"].is_null());
    for ancestor in ancestors {
        let row = rows
            .iter()
            .find(|row| row["attempt"]["plan"]["operation_id"] == json!(ancestor.plan.operation_id))
            .unwrap();
        assert_eq!(row["active_hold"], true);
        assert!(row["completed_by"].is_null());
        assert!(!row["diagnostic"].is_null());
    }
    let rows = inspection_rows(&view, unrelated.workspace, &issue(unrelated).unwrap());
    let other = rows
        .iter()
        .find(|row| {
            row["attempt"]["plan"]["operation_id"] == json!(unrelated.attempt.plan.operation_id)
        })
        .unwrap();
    assert_eq!(other["attempt"], json!(unrelated.attempt));
    assert!(other["diagnostic"].is_null());
    assert_eq!(other["active_hold"], true);
}

async fn inspect_history(delivery: &Arc<RuntimeDelivery>, parent: &IssueKey) -> Vec<Value> {
    let workflow = crate::runtime_workflow::RuntimeWorkflow::new(
        delivery.runtime.clone(),
        delivery.clone(),
        delivery.clone(),
    );
    let view = crate::runtime_host::WorkflowControl::inspect(&workflow)
        .await
        .unwrap();
    inspection_rows(&view, wire::id(2), parent)
}

fn inspection_rows(
    view: &crate::runtime_host::RuntimeInspection,
    workspace: Uuid,
    parent: &IssueKey,
) -> Vec<Value> {
    view.release_history
        .iter()
        .filter(|row| row.workspace == workspace && &row.parent == parent)
        .map(|row| serde_json::to_value(row).unwrap())
        .collect()
}

struct ResumeEvidence {
    delivery: Arc<RuntimeDelivery>,
    lease: Lease,
    gate: FreshGate,
    operation: Uuid,
    admitted: std::sync::Mutex<Option<Permit>>,
    fixture_resume: bool,
}
impl DecisionEvidence for ResumeEvidence {
    fn current(&self, key: &IssueKey) -> Result<crate::reconcile::ParentDelivery> {
        self.delivery.runtime.current_delivery(key)
    }
    fn outcomes(&self) -> Result<crate::reconcile::DependencyOutcomes> {
        self.delivery.runtime.dependency_outcomes()
    }
    fn retry_target(&self, _: &WorkIdentity) -> Result<RetryTarget> {
        Err(Error::invalid("resume-only fixture"))
    }
    fn published_retry_target(&self, _: &VerifiedRetryGrant) -> Result<RetryTarget> {
        Err(Error::invalid("resume-only fixture"))
    }
    fn record_acceptance(&self, _: &IssueKey, _: VerifiedAcceptance) -> Result<()> {
        Err(Error::invalid("resume-only fixture"))
    }
    fn record_correction(&self, _: VerifiedExplicitCorrection) -> Result<()> {
        Err(Error::invalid("resume-only fixture"))
    }
    fn retry_committed(&self, _: VerifiedRetryGrant) -> Result<()> {
        Err(Error::invalid("resume-only fixture"))
    }
    fn release(&self, _: AuthorizedDecision) -> crate::runtime_host::HostFuture<'_, Receipt> {
        Box::pin(async { Err(Error::invalid("resume-only fixture")) })
    }
    fn reconcile(
        &self,
        _: AuthorizedOperationRecovery,
    ) -> crate::runtime_host::HostFuture<'_, Receipt> {
        Box::pin(async { Err(Error::invalid("resume-only fixture")) })
    }
    fn recover_release(
        &self,
        decision: AuthorizedDecision,
    ) -> crate::runtime_host::HostFuture<'_, Receipt> {
        Box::pin(async move {
            if !self.fixture_resume {
                return crate::delivery::RuntimeDeliveryDecisions::recover_release(
                    self.delivery.as_ref(),
                    decision,
                )
                .await;
            }
            let record =
                self.delivery.releases.state.lock().unwrap().attempts[&self.operation].clone();
            let original = self.delivery.runtime.control.operation(self.operation)?;
            let previous = self
                .delivery
                .runtime
                .control
                .operation(record.admission_operation)?;
            let verified = crate::control::VerifiedReleaseResume::for_decision(
                &original,
                &previous,
                &record.attempt.plan.digest()?,
                &decision,
            )?;
            let permit = self.delivery.runtime.control.admit_release_resume(
                &self.lease,
                &verified,
                |_| {
                    self.delivery
                        .runtime
                        .registration
                        .verify_active_owner(&self.lease)?;
                    (self.delivery.current_gate())(&self.gate)
                },
            )?;
            let receipt = self
                .delivery
                .runtime
                .control
                .operation(permit.operation_id)?;
            *self.admitted.lock().unwrap() = Some(permit);
            Ok(receipt)
        })
    }
}
async fn authorize_resume(
    delivery: &Arc<RuntimeDelivery>,
    lease: &Lease,
    gate: &FreshGate,
    operation: Uuid,
) -> Permit {
    execute_recovery(delivery, lease, gate, operation, "resume", true)
        .await
        .unwrap()
        .unwrap()
}
async fn execute_recovery(
    delivery: &Arc<RuntimeDelivery>,
    lease: &Lease,
    gate: &FreshGate,
    operation: Uuid,
    action: &str,
    fixture_resume: bool,
) -> Result<Option<Permit>> {
    use crate::decisions::DecisionDependencies;
    use crate::decisions::DecisionManager;
    use kegari_control_api::identity::Actor;
    use kegari_control_api::identity::Principal;
    use kegari_types::api::{CommandName, CommandRequest};
    let runtime = &delivery.runtime;
    runtime.observations.refresh().await.unwrap();
    let evidence = Arc::new(ResumeEvidence {
        delivery: delivery.clone(),
        lease: lease.clone(),
        gate: gate.clone(),
        operation,
        admitted: std::sync::Mutex::new(None),
        fixture_resume,
    });
    let services = &runtime.services;
    let manager = DecisionManager::new(DecisionDependencies {
        configuration: services.configuration.clone(),
        control: services.control.clone(),
        observations: services.observations.clone(),
        secrets: services.secrets.clone(),
        key: services.key.clone(),
        evidence: evidence.clone(),
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
        runtime_id: lease.runtime_id,
        command: CommandName::ReleaseRecover,
        args: json!({"parent":"TEST-100","operation":operation,"action":action,"reason":"Explicit fixture recovery after genuine not-sent observation"}),
        expected_config_revision: Some(
            crate::config::config_revision(&runtime.configuration.snapshot().unwrap()).unwrap(),
        ),
        expected_control_generation: Some(runtime.control.snapshot().unwrap().intent.generation),
    };
    if action == "supersede" {
        request.args["result"] = json!(
            delivery.releases.state.lock().unwrap().attempts[&operation]
                .attempt
                .plan
                .evidence_token
        );
    }
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
    // Match the real command queue's task boundary; the upstream fixture is
    // deliberately much deeper than an API command task.
    let command_parent = parent.clone();
    eprintln!("completion fixture: starting owned {action} command task");
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
    eprintln!("completion fixture: owned {action} command task completed");
    runtime
        .control
        .complete_command(
            parent.operation_id,
            result
                .clone()
                .map(|result| json!({"effect_operation_id":result.operation_id,"receipt":result})),
        )
        .unwrap();
    result?;
    Ok(evidence.admitted.lock().unwrap().take())
}
