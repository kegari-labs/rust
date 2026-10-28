//! Durable no-release evidence from actual publication/readback/retirement and
//! real owned Git/vector provenance. Upstream checks and correction coordination
//! are explicitly synthetic; no native joined or confined execution is created.
use super::*;
use crate::authority::IssueKey;

#[path = "runtime_no_release_cold_runtime_fixture.rs"]
mod cold_runtime_fixture;

#[tokio::test]
async fn sealed_no_release_survives_restart_correction_and_later_main_without_current_credit() {
    Box::pin(history_cycle()).await;
}

async fn history_cycle() {
    let h = Harness::new().await;
    // Match public release progression before directly exercising its private
    // no-release phase; a genuinely cold facade below must remain poisoned.
    h.delivery.restore_releases().unwrap();
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
    let current = projection
        .parent(Uuid::from_u128(2), h.parent.manifest().work.parent)
        .unwrap();
    let key = current.key.clone();
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
    let inventory = h.delivery.no_release_completion_inventory().unwrap();
    assert!(inventory.errors.is_empty(), "{:?}", inventory.errors);
    assert_eq!(inventory.proofs.len(), 1);
    let proof = inventory.proofs[0].clone();
    // Empty optional fields preserve the original deployment-free schema-1
    // signing bytes. The real inventory reader above deserialized the full
    // typed seal and verified this same HMAC/digest without adding null/[] fields.
    let legacy: Signed<Value> = h
        .delivery
        .records
        .read(&format!("no-release-completion-{}.json", proof.operation()))
        .unwrap();
    assert!(legacy.payload.get("owned_targets").is_none());
    assert!(legacy.payload["origin"].get("targets_digest").is_none());
    h.built
        .runtime
        .services
        .key
        .verify("no_release_completion", &legacy)
        .unwrap();
    assert_eq!(legacy.digest().unwrap(), proof.digest());
    assert_eq!(proof.work(), &h.parent.manifest().work);
    assert_eq!(proof.outcome_digest(), outcome);
    assert_eq!(
        proof.record().recorded_at,
        inventory.rows[0].completion.as_ref().unwrap().completed_at
    );
    assert_eq!(
        inventory.rows[0].publication.effect,
        proof.publication_operation()
    );
    assert!(inventory.rows[0].publication.comment.is_some());
    assert!(
        inventory.rows[0]
            .publication
            .url
            .as_ref()
            .unwrap()
            .starts_with("https://linear.invalid/comment/")
    );
    assert_eq!(h.writes(), 3);
    Box::pin(produced_seal_reference_loss_keeps_original_identity(
        &h, &workspace, &proof,
    ))
    .await;
    h.delivery.sync_no_release_completions().unwrap();
    assert_credit(&h, &key, &proof.work().generation, Some(&outcome)).await;
    let cold = restart(&h);
    cold.sync_no_release_completions().unwrap();
    assert_eq!(
        cold.releases
            .state
            .lock()
            .unwrap()
            .queue
            .check_no_release_resources(proof.record())
            .unwrap_err()
            .code,
        "release_recovery_pending",
        "historical credit does not restore the unknown cold release queue"
    );
    // Runtime credit is independently usable while dispatch remains poisoned.
    assert_credit(&h, &key, &proof.work().generation, Some(&outcome)).await;
    cold.restore_releases().unwrap();
    cold.releases
        .state
        .lock()
        .unwrap()
        .queue
        .check_no_release_resources(proof.record())
        .unwrap();
    let recovered = cold.no_release_completion_inventory().unwrap();
    assert!(recovered.errors.is_empty());
    assert_eq!(recovered.proofs.len(), 1);
    assert_eq!(recovered.proofs[0].digest(), proof.digest());
    assert_eq!(
        recovered.rows[0].completion.as_ref().unwrap().completed_at,
        proof.record().recorded_at
    );
    assert_credit(&h, &key, &proof.work().generation, Some(&outcome)).await;
    Box::pin(assert_domain_history_preserves_later_owner_and_hold(&proof)).await;

    // An actual malformed inventory name is a global scan failure. Removing
    // the bad file alone cannot authorize a rebuild from an incomplete map.
    let malformed = "no-release-not-a-uuid.json";
    h.delivery.records.write_bytes(malformed, b"{}").unwrap();
    let scan_error = h.delivery.sync_no_release_completions().unwrap_err();
    assert_eq!(scan_error.code, "no_release_completion");
    assert_eq!(
        scan_error.message,
        "no-release inventory filename is invalid"
    );
    std::fs::remove_file(h.delivery.records.path().join(malformed)).unwrap();
    assert_history_queue_refusal(&h.delivery, &proof, &scan_error);
    // NR failure must not be copied into the independent ordinary inventory.
    h.built
        .runtime
        .require_released_inventory_available()
        .unwrap();
    assert!(
        h.delivery
            .releases
            .state
            .lock()
            .unwrap()
            .completion_inventory_error
            .is_none()
    );
    assert_eq!(
        h.built
            .runtime
            .require_no_release_inventory_available()
            .unwrap_err()
            .message,
        scan_error.message
    );
    assert_credit(&h, &key, &proof.work().generation, None).await;
    h.delivery.sync_no_release_completions().unwrap();
    h.delivery
        .releases
        .state
        .lock()
        .unwrap()
        .queue
        .check_no_release_resources(proof.record())
        .unwrap();
    assert_credit(&h, &key, &proof.work().generation, Some(&outcome)).await;

    // Actual native comment -> signed publisher route/history. Its classifier
    // completion is the explicit synthetic coordination fixture, not a worker.
    let corrected = correct_parent(&h.built, &h.wire, &h.lease).await;
    assert_ne!(corrected.work().generation, proof.work().generation);
    assert_credit(&h, &key, &proof.work().generation, Some(&outcome)).await;
    assert_credit(&h, &key, &corrected.work().generation, None).await;
    assert!(
        h.built
            .runtime
            .current_delivery(&key)
            .unwrap()
            .no_release_required()
            .is_none()
    );

    // An explicit external fixture change updates actual disposable Git plus
    // the native provider ref together. It is no controller rollback/deploy.
    let config = h.built.runtime.configuration.snapshot().unwrap();
    let source = std::path::Path::new(&config.workspaces["example"].repositories["42"].source);
    let binary = h.delivery.git_path().unwrap();
    std::fs::write(
        source.join("behavior.txt"),
        "later independently published behavior\n",
    )
    .unwrap();
    crate::runtime_delivery::worker_fixture::git(&binary, source, &["add", "behavior.txt"]);
    crate::runtime_delivery::worker_fixture::git(
        &binary,
        source,
        &[
            "commit",
            "--signoff",
            "-m",
            "test: simulate later external production",
        ],
    );
    let later = crate::git::Git::new(&binary)
        .unwrap()
        .revision(source, "HEAD")
        .unwrap();
    assert_ne!(later, proof.record().baselines[0].production);
    {
        let mut wire = h.wire.lock().unwrap();
        wire.github_values.insert(
            "/repos/fixture/repo/git/ref/heads/main".into(),
            json!({"ref":"refs/heads/main","object":{"type":"commit","sha":later.commit}}),
        );
        wire.github_values.insert(
            format!("/repos/fixture/repo/git/commits/{}", later.commit),
            json!({"sha":later.commit,"tree":{"sha":later.tree},"parents":[]}),
        );
    }
    h.built.runtime.observations.refresh().await.unwrap();
    for delivery in [&h.delivery, &cold] {
        delivery.sync_no_release_completions().unwrap();
        assert_eq!(
            delivery.no_release_completion_inventory().unwrap().proofs[0].digest(),
            proof.digest()
        );
        assert_credit(&h, &key, &proof.work().generation, Some(&outcome)).await;
        assert_credit(&h, &key, &corrected.work().generation, None).await;
    }
    h.stop();
    let receipts = canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap();
    let writes = h.writes();
    let vector_leaf = format!(
        "parent-vector-{}.json",
        &h.parent.manifest().candidate_vector_digest().unwrap()[7..]
    );
    let intent = intent_leaf(proof.operation());
    for (directory, leaf) in [
        (&workspace.records, vector_leaf.as_str()),
        (&h.delivery.records, intent.as_str()),
    ] {
        let bytes = directory.read_bytes(leaf, 1024 * 1024).unwrap();
        std::fs::remove_file(directory.path().join(leaf)).unwrap();
        // A root shadow must not replace the exact workspace vector namespace.
        let shadow = leaf == vector_leaf;
        if shadow {
            h.delivery.records.write_bytes(leaf, &bytes).unwrap();
        }
        assert_eq!(proof.validate().unwrap_err().code, "record_missing");
        for delivery in [h.delivery.clone(), restart(&h)] {
            let inventory = delivery.no_release_completion_inventory().unwrap();
            assert!(inventory.proofs.is_empty());
            assert_eq!(inventory.errors[&proof.operation()].code, "record_missing");
            let [row] = inventory.rows.as_slice() else {
                panic!("one exact retained diagnostic row required")
            };
            assert_eq!(row.admission, proof.operation());
            assert_eq!(row.work, *proof.work());
            assert!(row.completion.is_none());
            assert_eq!(row.diagnostic.as_ref().unwrap().code, "record_missing");
            delivery.sync_no_release_completions().unwrap();
            assert_history_queue_refusal(&delivery, &proof, &inventory.errors[&proof.operation()]);
            assert!(
                !delivery
                    .releases
                    .state
                    .lock()
                    .unwrap()
                    .no_release
                    .contains_key(&proof.operation()),
                "a formerly sealed outcome cannot fall back to current cached evidence"
            );
            assert_credit(&h, &key, &proof.work().generation, None).await;
            Box::pin(assert_diagnostic_facade(
                &h,
                &delivery,
                &proof,
                &inventory.errors[&proof.operation()],
            ))
            .await;
        }
        if shadow {
            std::fs::remove_file(h.delivery.records.path().join(leaf)).unwrap();
        }
        directory.write_bytes(leaf, &bytes).unwrap();
        cold.sync_no_release_completions().unwrap();
        assert_credit(&h, &key, &proof.work().generation, Some(&outcome)).await;
    }
    let receipt_directory = crate::storage::PrivateDir::open(
        &h.built.runtime.registration.root().path().join("receipts"),
    )
    .unwrap();
    for operation in [proof.operation(), proof.publication_operation()] {
        let leaf = format!("{operation}.json");
        let bytes = receipt_directory.read_bytes(&leaf, 1024 * 1024).unwrap();
        std::fs::remove_file(receipt_directory.path().join(&leaf)).unwrap();
        assert_eq!(proof.validate().unwrap_err().code, "record_missing");
        for delivery in [h.delivery.clone(), restart(&h)] {
            let inventory = delivery.no_release_completion_inventory().unwrap();
            assert!(inventory.proofs.is_empty());
            assert_eq!(inventory.errors[&proof.operation()].code, "record_missing");
            let [row] = inventory.rows.as_slice() else {
                panic!("missing final receipt keeps one exact historical diagnostic")
            };
            assert_eq!(row.admission, proof.operation());
            assert_eq!(row.work, *proof.work());
            assert!(row.completion.is_none());
            assert_eq!(row.diagnostic.as_ref().unwrap().code, "record_missing");
            delivery.sync_no_release_completions().unwrap();
            assert_history_queue_refusal(&delivery, &proof, &inventory.errors[&proof.operation()]);
            assert!(
                !delivery
                    .releases
                    .state
                    .lock()
                    .unwrap()
                    .no_release
                    .contains_key(&proof.operation()),
                "a formerly sealed outcome cannot fall back to current cached evidence"
            );
            assert_credit(&h, &key, &proof.work().generation, None).await;
            Box::pin(assert_diagnostic_facade(
                &h,
                &delivery,
                &proof,
                &inventory.errors[&proof.operation()],
            ))
            .await;
        }
        // Restore only bytes read from the actual protected receipt beforehand.
        receipt_directory.write_bytes(&leaf, &bytes).unwrap();
        cold.sync_no_release_completions().unwrap();
        assert_credit(&h, &key, &proof.work().generation, Some(&outcome)).await;
    }
    assert_eq!(h.writes(), writes);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
    {
        let mut removed = config.clone();
        removed.workspaces.clear();
        *h.built.configuration.config.lock().unwrap() = removed;
    }
    h.built.runtime.observations.refresh().await.unwrap();
    for delivery in [h.delivery.clone(), restart(&h)] {
        delivery.sync_no_release_completions().unwrap();
        assert_credit(&h, &key, &proof.work().generation, None).await;
        assert_removed_workspace_facade(&delivery, proof.operation(), proof.record(), &outcome)
            .await;
    }
    assert_eq!(h.writes(), writes);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
    *h.built.configuration.config.lock().unwrap() = config;
}

async fn produced_seal_reference_loss_keeps_original_identity(
    h: &Harness,
    workspace: &WorkspaceDelivery,
    proof: &crate::runtime_delivery::VerifiedNoReleaseCompletion,
) {
    // The genuine producer now shares immutable identity before any sync.
    // Retaining it is protective provenance, independent of historical credit.
    let known = h.built.runtime.known_no_release_completions().unwrap();
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].operation(), proof.operation());
    assert_eq!(known[0].digest(), proof.digest());
    let vector_leaf = format!(
        "parent-vector-{}.json",
        &h.parent.manifest().candidate_vector_digest().unwrap()[7..]
    );
    let vector = workspace
        .records
        .read_bytes(&vector_leaf, 1024 * 1024)
        .unwrap();
    let seal_leaf = format!("no-release-completion-{}.json", proof.operation());
    let seal = h
        .delivery
        .records
        .read_bytes(&seal_leaf, 1024 * 1024)
        .unwrap();
    let signed_seal: Signed<serde_json::Value> = h.delivery.records.read(&seal_leaf).unwrap();
    h.built
        .runtime
        .services
        .key
        .verify("no_release_completion", &signed_seal)
        .unwrap();
    let writes = h.writes();
    let receipts = canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap();
    std::fs::remove_file(workspace.records.path().join(&vector_leaf)).unwrap();
    let first = restart(h);
    let missing_vector = proof.validate().unwrap_err();
    assert_eq!(missing_vector.code, "record_missing");
    // No inventory or runtime sync has seeded this facade. Its shared original
    // identity and actual seal must prevent fallback to current ephemeral proof.
    Box::pin(assert_direct_hydration_refusal(
        h,
        &first,
        proof,
        &missing_vector,
    ))
    .await;
    let inventory = first.no_release_completion_inventory().unwrap();
    assert!(inventory.proofs.is_empty());
    assert_eq!(
        inventory.anchors.len(),
        1,
        "an authenticated seal with a missing ref still establishes immutable identity"
    );
    assert_eq!(inventory.anchors[0].digest(), proof.digest());
    assert_eq!(inventory.errors[&proof.operation()].code, "record_missing");
    first.sync_no_release_completions().unwrap();
    let known = h.built.runtime.known_no_release_completions().unwrap();
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].digest(), proof.digest());
    Box::pin(assert_diagnostic_facade(
        h,
        &first,
        proof,
        &inventory.errors[&proof.operation()],
    ))
    .await;
    workspace
        .records
        .write_bytes(&vector_leaf, &vector)
        .unwrap();

    for changed in [false, true] {
        if changed {
            // Explicit protected-record tampering: no capability is built from
            // this newly signed payload. The known genuine digest must reject it.
            let mut value = signed_seal.clone();
            value.payload["display"]["title"] = json!("Altered retained display");
            let changed = h
                .built
                .runtime
                .services
                .key
                .sign("no_release_completion", &value.payload)
                .unwrap();
            h.delivery.records.write(&seal_leaf, &changed).unwrap();
        } else {
            std::fs::remove_file(h.delivery.records.path().join(&seal_leaf)).unwrap();
        }
        let newer = restart(h);
        let unavailable = proof.validate().unwrap_err();
        Box::pin(assert_direct_hydration_refusal(
            h,
            &newer,
            proof,
            &unavailable,
        ))
        .await;
        newer.sync_no_release_completions().unwrap();
        let inventory = newer.no_release_completion_inventory().unwrap();
        assert!(inventory.proofs.is_empty());
        assert_eq!(inventory.anchors[0].digest(), proof.digest());
        let error = &inventory.errors[&proof.operation()];
        assert_eq!(
            error.code,
            if changed {
                "no_release_completion"
            } else {
                "record_missing"
            }
        );
        if changed {
            assert_eq!(
                error.message,
                "previously observed immutable no-release seal changed"
            );
        }
        assert_history_queue_refusal(&newer, proof, error);
        Box::pin(assert_diagnostic_facade(h, &newer, proof, error)).await;
        let (projection, mut snapshot) = h.snapshot().await;
        let current = projection
            .parent(Uuid::from_u128(2), proof.work().parent)
            .unwrap();
        let current_workspace = newer.workspace(&projection, Uuid::from_u128(2)).unwrap();
        let hydrated = newer
            .hydrate_no_release(&current_workspace, current, &mut snapshot)
            .await
            .unwrap_err();
        assert_eq!(hydrated.code, error.code);
        assert!(snapshot.parent_delivery().no_release_required().is_none());
        assert!(newer.releases.state.lock().unwrap().no_release.is_empty());
        if changed {
            let current: Signed<serde_json::Value> = h.delivery.records.read(&seal_leaf).unwrap();
            assert_ne!(current.digest().unwrap(), proof.digest());
            assert_eq!(
                current.payload["display"]["title"],
                "Altered retained display"
            );
        } else {
            assert_eq!(
                h.delivery
                    .records
                    .read_bytes(&seal_leaf, 1024 * 1024)
                    .unwrap_err()
                    .code,
                "record_missing",
                "history hydration cannot remint a previously sealed operation"
            );
        }
        h.delivery.records.write_bytes(&seal_leaf, &seal).unwrap();
        newer.sync_no_release_completions().unwrap();
        assert_eq!(
            newer.no_release_completion_inventory().unwrap().proofs[0].digest(),
            proof.digest()
        );
    }
    assert_eq!(h.writes(), writes);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
}

pub(super) fn assert_history_queue_refusal(
    delivery: &RuntimeDelivery,
    proof: &crate::runtime_delivery::VerifiedNoReleaseCompletion,
    expected: &Error,
) {
    // A new facade must reconstruct actual queue state before comparing its
    // affected resource refusal; pending bootstrap is not scoped history.
    if let Err(error) = delivery.restore_releases() {
        // A globally failed inventory also refuses restoration. Preserve that
        // exact failure while checking the retained queue and a fresh rebuild.
        assert_eq!(error.code, expected.code);
        assert_eq!(error.message, expected.message);
    }
    let state = delivery.releases.state.lock().unwrap();
    let current = state
        .queue
        .check_no_release_resources(proof.record())
        .unwrap_err();
    assert_eq!(current.code, expected.code);
    assert_eq!(current.message, expected.message);
    let rebuilt = match super::super::super::rebuild_queue(
        &state.attempts,
        &state.no_release,
        &state.completions,
        &state.completion_anchors,
        &state.completion_errors,
        state.completion_inventory_error.as_ref(),
        &state.historical_no_release,
    ) {
        Ok(queue) => queue
            .check_no_release_resources(proof.record())
            .unwrap_err(),
        Err(error) => error,
    };
    assert_eq!(
        rebuilt.code, expected.code,
        "a rebuild cannot clear the affected authenticated scope refusal"
    );
    assert_eq!(rebuilt.message, expected.message);
}

async fn assert_domain_history_preserves_later_owner_and_hold(
    proof: &crate::runtime_delivery::VerifiedNoReleaseCompletion,
) {
    // This is solely a domain reservation/hold fixture around a genuine opaque
    // historical no-release seal. It creates no release admission or dispatch
    // in the no-release runtime. The plan comes from the existing offline
    // adapter fixture; the hold is explicit synthetic unresolved domain state.
    let (plan, work, _) = crate::delivery::observed_merge_for_runtime().await;
    assert!(plan.resources.iter().any(|resource| matches!(resource,
        domain::ResourceKey::Repository { repository }
        if proof.dependency_repositories().unwrap().contains(repository))));
    let now = Utc::now();
    let mut owned = domain::ReleaseQueue::default();
    owned.reserve(&plan, now).unwrap();
    assert!(owned.owns_all(&plan));
    let success = owned.record_no_release_completion(proof).unwrap();
    assert!(success.no_release_required);
    assert_eq!(success.spacing_seconds, 0);
    assert_eq!(success.completed_at, proof.record().recorded_at);
    assert!(
        owned.owns_all(&plan),
        "past success cannot release a later owner"
    );
    assert_eq!(
        owned
            .check_no_release_resources(proof.record())
            .unwrap_err()
            .code,
        "release_hold"
    );

    let mut retained = super::super::super::tests::record(plan.clone(), work);
    retained.attempt.hold_reason = Some("explicit synthetic later unresolved hold".into());
    let mut held = domain::ReleaseQueue::default();
    held.restore_hold(&retained.attempt).unwrap();
    let before = held.eligible(&plan, now).unwrap_err();
    assert_eq!(before.code, "release_hold");
    held.record_no_release_completion(proof).unwrap();
    let after = held.eligible(&plan, now).unwrap_err();
    assert_eq!(after.code, before.code);
    assert_eq!(after.message, before.message);
    assert_eq!(
        held.check_no_release_resources(proof.record())
            .unwrap_err()
            .code,
        "release_hold"
    );

    let mut history_only = domain::ReleaseQueue::default();
    history_only.record_no_release_completion(proof).unwrap();
    assert_eq!(
        history_only.earliest_start(&plan, now).unwrap(),
        now,
        "historical no-release completion never creates a spacing interval"
    );
    history_only.eligible(&plan, now).unwrap();
}

pub(super) fn restart(h: &Harness) -> Arc<RuntimeDelivery> {
    RuntimeDelivery::with_fixture(
        h.built.runtime.clone(),
        h.delivery.processes.clone(),
        h.delivery.git_path().unwrap(),
    )
    .unwrap()
}
pub(super) async fn assert_credit(
    h: &Harness,
    key: &IssueKey,
    generation: &evidence::Generation,
    expected: Option<&str>,
) {
    h.built.runtime.observations.refresh().await.unwrap();
    assert_eq!(
        h.built
            .runtime
            .dependency_outcomes()
            .unwrap()
            .parent_outcome(key, generation),
        expected
    );
}
async fn assert_direct_hydration_refusal(
    h: &Harness,
    delivery: &Arc<RuntimeDelivery>,
    proof: &crate::runtime_delivery::VerifiedNoReleaseCompletion,
    expected: &Error,
) {
    let writes = h.writes();
    let receipts = canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap();
    let (projection, mut snapshot) = h.snapshot().await;
    let current = projection
        .parent(proof.workspace_id(), proof.work().parent)
        .unwrap();
    let workspace = delivery
        .workspace(&projection, proof.workspace_id())
        .unwrap();
    let before = {
        let state = delivery.releases.state.lock().unwrap();
        assert!(state.no_release.is_empty());
        format!("{:?}", state.queue)
    };
    let error = delivery
        .hydrate_no_release(&workspace, current, &mut snapshot)
        .await
        .unwrap_err();
    assert_eq!(error.code, expected.code);
    assert_eq!(error.message, expected.message);
    assert!(snapshot.parent_delivery().no_release_required().is_none());
    {
        let state = delivery.releases.state.lock().unwrap();
        assert!(state.no_release.is_empty());
        assert_eq!(format!("{:?}", state.queue), before);
    }
    assert_eq!(h.writes(), writes);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
}

async fn assert_diagnostic_facade(
    h: &Harness,
    delivery: &Arc<RuntimeDelivery>,
    proof: &crate::runtime_delivery::VerifiedNoReleaseCompletion,
    expected: &Error,
) {
    let writes = h.writes();
    let receipts = canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap();
    let workflow = crate::runtime_workflow::RuntimeWorkflow::new(
        delivery.runtime.clone(),
        delivery.clone(),
        delivery.clone(),
    );
    let view = crate::app::WorkflowControl::inspect(&workflow)
        .await
        .unwrap();
    let config = delivery.runtime.configuration.snapshot().unwrap();
    let selected = crate::inspection::resolve_history_parent(
        &config,
        &view.projection,
        &view.release_history,
        &view.no_release_history,
        &format!("{}/{}", proof.workspace_id(), proof.work().parent),
        None,
    )
    .unwrap();
    let rows = crate::inspection::no_release_parent_rows(&view.no_release_history, &selected);
    let [row] = rows.as_slice() else {
        panic!("read-only facade must retain one exact diagnostic no-release row")
    };
    assert_eq!(row.admission, proof.operation());
    assert_eq!(row.work, *proof.work());
    assert_eq!(row.record_digest, proof.outcome_digest());
    assert!(row.completion.is_none());
    let diagnostic = row.diagnostic.as_ref().unwrap();
    assert_eq!(diagnostic.code, expected.code);
    assert_eq!(diagnostic.message, expected.message);
    assert!(row.display.title.is_some());
    assert_eq!(
        crate::inspection::observed_history_url(None, &[], &rows).unwrap(),
        row.display.url.as_ref().unwrap().as_str()
    );
    assert!(
        view.outcomes
            .parent_outcome(&row.parent, &proof.work().generation)
            .is_none()
    );
    assert!(
        delivery
            .runtime
            .current_delivery(&row.parent)
            .unwrap()
            .no_release_required()
            .is_none()
    );
    let overview = crate::inspection::overview(
        &view,
        Some(proof.workspace_id()),
        &delivery.runtime.control.snapshot().unwrap(),
        Utc::now(),
        Duration::seconds(30),
    )
    .unwrap();
    let overview = serde_json::to_value(overview).unwrap();
    assert!(
        !overview["history"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["outcome"] == "no-release-required"
                && row["generation"] == json!(proof.work().generation)
                && row["parent_reference"]
                    == format!("{}/{}", proof.workspace_id(), proof.work().parent)),
        "diagnostic display cannot supply successful history"
    );
    // A scoped historical failure no longer aborts refresh for unrelated work.
    // The actual affected parent still supplies neither readiness nor credit;
    // its exact immutable-ref error remains asserted on the authenticated row.
    delivery.refresh(&view.projection).await.unwrap();
    let affected = delivery.runtime.current_delivery(&row.parent).unwrap();
    assert!(affected.no_release_required().is_none());
    assert!(affected.release_eligibility().is_none());
    assert!(
        delivery
            .runtime
            .dependency_outcomes()
            .unwrap()
            .parent_outcome(&row.parent, &proof.work().generation)
            .is_none()
    );
    assert_eq!(h.writes(), writes);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
}

async fn assert_removed_workspace_facade(
    delivery: &Arc<RuntimeDelivery>,
    operation: Uuid,
    original: &evidence::NoReleaseRequired,
    digest: &str,
) {
    let workflow = crate::runtime_workflow::RuntimeWorkflow::new(
        delivery.runtime.clone(),
        delivery.clone(),
        delivery.clone(),
    );
    let view = crate::app::WorkflowControl::inspect(&workflow)
        .await
        .unwrap();
    assert!(view.projection.workspaces.is_empty());
    let config = delivery.runtime.configuration.snapshot().unwrap();
    let selected = crate::inspection::resolve_history_parent(
        &config,
        &view.projection,
        &view.release_history,
        &view.no_release_history,
        &format!("{}/{}", Uuid::from_u128(2), original.work.parent),
        None,
    )
    .unwrap();
    let rows = crate::inspection::no_release_parent_rows(&view.no_release_history, &selected);
    let [row] = rows.as_slice() else {
        panic!("one exact no-release history row required")
    };
    assert_eq!(row.admission, operation);
    assert_eq!(row.work, original.work);
    assert_eq!(row.record_digest, digest);
    assert_eq!(
        row.completion.as_ref().unwrap().completed_at,
        original.recorded_at
    );
    assert!(row.diagnostic.is_none());
    assert!(row.display.title.is_some());
    assert_eq!(
        crate::inspection::observed_history_url(None, &[], &rows).unwrap(),
        row.display.url.as_ref().unwrap().as_str()
    );
    let overview = crate::inspection::overview(
        &view,
        Some(Uuid::from_u128(2)),
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
            .any(|row| row["outcome"] == "no-release-required"
                && row["generation"] == json!(original.work.generation)
                && row["completed_at"] == json!(original.recorded_at))
    );
}
async fn correct_parent(
    built: &crate::runtime_fixture::RuntimeFixture,
    data: &Arc<std::sync::Mutex<crate::corrections::wire_fixture::Fixture>>,
    lease: &Lease,
) -> FreshGate {
    let comment = Uuid::new_v4();
    let body = "Validate the already-present behavior and keep this correction within the existing test responsibility.";
    data.lock().unwrap().comments.entry(Uuid::from_u128(100)).or_default().push(json!({"id":comment,"issue":{"id":Uuid::from_u128(100)},
        "body":body,"createdAt":Utc::now(),"updatedAt":Utc::now(),"editedAt":null,"user":{"id":Uuid::from_u128(5)},
        "onBehalfOf":null,"botActor":null,"url":"https://linear.invalid/comment/correction"}));
    let projection = built.runtime.observations.refresh().await.unwrap();
    let parent = projection
        .parent(Uuid::from_u128(2), Uuid::from_u128(100))
        .unwrap();
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
        "source_comment":comment,"source_revision":kegari_digest::bytes_digest(body.as_bytes()),"class":"corrective",
        "rationale":"The native request changes validation expectations within existing scope",
        "routes":[{"child":Uuid::from_u128(101),"instructions":"Validate existing behavior without adding an unrelated source change.",
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
        .parent(Uuid::from_u128(2), Uuid::from_u128(100))
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

#[tokio::test]
async fn history_native_read_cannot_publish_current_credit_after_acceptor_reload() {
    Box::pin(history_hydration_race()).await;
}

async fn history_hydration_race() {
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
    let current = projection
        .parent(Uuid::from_u128(2), h.parent.manifest().work.parent)
        .unwrap();
    let workspace = h
        .delivery
        .workspace(&projection, Uuid::from_u128(2))
        .unwrap();
    let config = h.built.runtime.configuration.snapshot().unwrap();
    let before = {
        let state = h.delivery.releases.state.lock().unwrap();
        assert!(state.no_release.is_empty());
        format!("{:?}", state.queue)
    };
    let receipts = canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap();
    let writes = h.writes();
    let fired = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = fired.clone();
    let configuration = h.built.configuration.clone();
    // This one-shot callback is armed only after the real current snapshot and
    // Git baseline are prepared. The first additional accepted-origin identity
    // response performs a real configuration reload during historical await.
    h.wire.lock().unwrap().before_identity_read = Some(Arc::new(move || {
        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        configuration
            .config
            .lock()
            .unwrap()
            .workspaces
            .get_mut("example")
            .unwrap()
            .actors
            .acceptors = [Uuid::from_u128(6)].into();
        Ok(())
    }));
    let error = h
        .delivery
        .hydrate_no_release(&workspace, current, &mut snapshot)
        .await
        .unwrap_err();
    assert_eq!(fired.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(error.code, "stale_config");
    assert_eq!(error.message, "delivery configuration changed");
    assert!(snapshot.parent_delivery().no_release_required().is_none());
    {
        let state = h.delivery.releases.state.lock().unwrap();
        assert!(state.no_release.is_empty());
        assert_eq!(format!("{:?}", state.queue), before);
    }
    assert!(
        !h.delivery
            .records
            .records_bounded(65_536)
            .unwrap()
            .iter()
            .any(|leaf| leaf.starts_with("no-release-completion-"))
    );
    assert_eq!(h.writes(), writes);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
    // A later genuine current snapshot may reuse the same existing publication;
    // neither the failed read nor the recovered hydration sends another write.
    *h.built.configuration.config.lock().unwrap() = config;
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
    assert_eq!(
        h.delivery
            .no_release_completion_inventory()
            .unwrap()
            .proofs
            .len(),
        1
    );
    assert_eq!(h.writes(), writes);
}

#[tokio::test]
async fn direct_no_release_seal_is_shared_before_sync_and_cannot_be_reminted_by_new_facade() {
    Box::pin(shared_seal_before_sync()).await;
}

async fn shared_seal_before_sync() {
    let h = Harness::new().await;
    h.delivery.restore_releases().unwrap();
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
    let current = projection
        .parent(Uuid::from_u128(2), h.parent.manifest().work.parent)
        .unwrap();
    let key = current.key.clone();
    let workspace = h
        .delivery
        .workspace(&projection, Uuid::from_u128(2))
        .unwrap();
    // This is the actual production seal-writing path. Deliberately do not call
    // history inventory or sync: neither may be needed to preserve its identity.
    h.delivery
        .hydrate_no_release(&workspace, current, &mut snapshot)
        .await
        .unwrap();
    let current_no_release = snapshot.parent_delivery().no_release_required().unwrap();
    let operation = current_no_release.record().operation_id;
    let work = current_no_release.record().work.clone();
    let outcome_digest = current_no_release.digest().to_owned();
    let leaf = format!("no-release-completion-{operation}.json");
    let original_bytes = h.delivery.records.read_bytes(&leaf, 1024 * 1024).unwrap();
    let original: Signed<Value> = h.delivery.records.read(&leaf).unwrap();
    h.built
        .runtime
        .services
        .key
        .verify("no_release_completion", &original)
        .unwrap();
    let original_digest = original.digest().unwrap();
    let prior_anchors = h.built.runtime.known_no_release_completions().unwrap();
    assert_eq!(h.writes(), 3);
    let writes = h.writes();
    let receipts = canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap();
    // An anchor is protective provenance only, not installed historical credit.
    assert!(
        h.built
            .runtime
            .dependency_outcomes()
            .unwrap()
            .parent_outcome(&key, &work.generation)
            .is_none()
    );

    // Install only the actual freshly hydrated snapshot via the production
    // cache setter. Its current fact must be usable while its known seal exists;
    // this is distinct from an anchor granting historical credit by itself.
    h.built.runtime.retain_delivery_snapshot(snapshot).unwrap();
    assert_eq!(
        h.built
            .runtime
            .current_delivery(&key)
            .unwrap()
            .no_release_required()
            .unwrap()
            .digest(),
        outcome_digest,
    );
    assert_eq!(
        h.built
            .runtime
            .dependency_outcomes()
            .unwrap()
            .parent_outcome(&key, &work.generation),
        Some(outcome_digest.as_str())
    );
    std::fs::remove_file(h.delivery.records.path().join(&leaf)).unwrap();
    let missing = h.delivery.records.read::<Signed<Value>>(&leaf).unwrap_err();
    assert_eq!(missing.code, "record_missing");
    // No rescan, synchronization, invalidation call or new facade has run yet.
    // Every-use evidence projection must block the old current-snapshot fallback
    // from this runtime's already authenticated but now missing own seal.
    assert!(
        h.built
            .runtime
            .dependency_outcomes()
            .unwrap()
            .parent_outcome(&key, &work.generation)
            .is_none()
    );
    assert!(
        h.built
            .runtime
            .current_delivery(&key)
            .unwrap()
            .no_release_required()
            .is_none()
    );
    assert_eq!(h.writes(), writes);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
    let newer = restart(&h);
    // Restore the genuine release queue without scanning or syncing no-release
    // history. This avoids confusing cold queue bootstrap with missing identity.
    newer.restore_releases().unwrap();
    let (projection, mut empty_snapshot) = h.snapshot().await;
    assert!(
        empty_snapshot
            .parent_delivery()
            .no_release_required()
            .is_none()
    );
    let current = projection.parent(Uuid::from_u128(2), work.parent).unwrap();
    let workspace = newer.workspace(&projection, Uuid::from_u128(2)).unwrap();
    let refused = newer
        .hydrate_no_release(&workspace, current, &mut empty_snapshot)
        .await
        .unwrap_err();
    assert_eq!(refused.code, missing.code);
    assert_eq!(refused.message, missing.message);
    assert!(
        empty_snapshot
            .parent_delivery()
            .no_release_required()
            .is_none()
    );
    assert!(newer.releases.state.lock().unwrap().no_release.is_empty());
    assert_eq!(
        newer
            .records
            .read_bytes(&leaf, 1024 * 1024)
            .unwrap_err()
            .code,
        "record_missing",
        "a new facade cannot recreate a seal already produced by this runtime"
    );
    let after_anchors = h.built.runtime.known_no_release_completions().unwrap();
    // Assert after the attempted remint, so unfixed source reaches that boundary
    // rather than failing early solely on the formerly empty shared anchor map.
    assert_eq!(prior_anchors.len(), 1);
    assert_eq!(after_anchors.len(), 1);
    assert_eq!(prior_anchors[0].operation(), operation);
    assert_eq!(prior_anchors[0].digest(), original_digest);
    assert_eq!(after_anchors[0].operation(), operation);
    assert_eq!(after_anchors[0].digest(), original_digest);
    assert_eq!(after_anchors[0].outcome_digest(), outcome_digest);
    assert!(
        h.built
            .runtime
            .dependency_outcomes()
            .unwrap()
            .parent_outcome(&key, &work.generation)
            .is_none()
    );
    assert_eq!(h.writes(), writes);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );

    // Restore actual original bytes. Only the explicit successful inventory sync
    // below installs historical credit; remembering alone did not do so.
    h.delivery
        .records
        .write_bytes(&leaf, &original_bytes)
        .unwrap();
    newer.sync_no_release_completions().unwrap();
    let recovered = newer.no_release_completion_inventory().unwrap();
    assert!(recovered.errors.is_empty());
    assert_eq!(recovered.proofs.len(), 1);
    assert_eq!(recovered.proofs[0].digest(), original_digest);
    assert_credit(&h, &key, &work.generation, Some(&outcome_digest)).await;
    assert_eq!(h.writes(), writes);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
}
