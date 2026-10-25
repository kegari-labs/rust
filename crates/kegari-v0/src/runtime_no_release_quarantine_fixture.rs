//! Actual no-release seal and damaged immutable references. Other plans below
//! are validated queue probes only; they never mint readiness or dispatch work.
use super::*;

#[tokio::test]
async fn authenticated_no_release_damage_quarantines_only_overlap_and_exact_dependents() {
    Box::pin(scoped_cycle()).await;
}

async fn scoped_cycle() {
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
    let workspace = h
        .delivery
        .workspace(&projection, Uuid::from_u128(2))
        .unwrap();
    h.delivery
        .hydrate_no_release(&workspace, current, &mut snapshot)
        .await
        .unwrap();
    let ephemeral = snapshot
        .parent_delivery()
        .no_release_required()
        .unwrap()
        .clone();
    h.delivery.sync_no_release_completions().unwrap();
    let proof = h
        .delivery
        .no_release_completion_inventory()
        .unwrap()
        .proofs
        .remove(0);
    let scope = domain::CompletionScope::no_release(&proof).unwrap();
    let (template, _, _) = crate::delivery::observed_merge_for_runtime().await;
    assert_eq!(template.runtime_id, proof.runtime_id());
    let repository = proof
        .dependency_repositories()
        .unwrap()
        .iter()
        .next()
        .unwrap()
        .clone();
    let a = candidate(&template, repository.clone());
    let b = candidate(
        &template,
        RepositoryId {
            hostname: repository.hostname.clone(),
            id: "9001".into(),
        },
    );
    let mut dependent = b.clone();
    dependent.operation_id = Uuid::new_v4();
    dependent.dependencies = vec![domain::DependencyRequirement {
        parent_id: proof.work().parent,
        generation: proof.work().generation.clone(),
        outcome_digest: proof.outcome_digest().into(),
    }];
    dependent.validate().unwrap();
    assert!(scope.resources().is_disjoint(&b.resources));
    let mut wrong_generation = dependent.clone();
    wrong_generation.dependencies[0].generation.correction =
        canonical_digest(1, &json!("unobserved generation")).unwrap();
    let mut wrong_digest = dependent.clone();
    wrong_digest.dependencies[0].outcome_digest =
        canonical_digest(1, &json!("unobserved outcome")).unwrap();
    {
        let state = h.delivery.releases.state.lock().unwrap();
        for plan in [&a, &b, &dependent] {
            state.queue.eligible(plan, Utc::now()).unwrap();
        }
    }

    // Domain owner/hold fixtures retain the actual proof but create no actual
    // release admission. They demonstrate that quarantine cannot clear them.
    let mut owned = h.delivery.releases.state.lock().unwrap().queue.clone();
    owned.reserve(&b, Utc::now()).unwrap();
    let mut held = owned.clone();
    let mut held_record = super::super::super::tests::record(b.clone(), proof.work().clone());
    held_record.attempt.hold_reason = Some("explicit domain-only later hold".into());
    held.restore_hold(&held_record.attempt).unwrap();
    assert_eq!(
        held.eligible(&b, Utc::now()).unwrap_err().code,
        "release_hold"
    );
    let writes = h.writes();
    let receipts = canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap();
    let vector_leaf = format!(
        "parent-vector-{}.json",
        &h.parent.manifest().candidate_vector_digest().unwrap()[7..]
    );
    let vector_bytes = workspace
        .records
        .read_bytes(&vector_leaf, 1024 * 1024)
        .unwrap();
    std::fs::remove_file(workspace.records.path().join(&vector_leaf)).unwrap();
    let missing = proof.validate().unwrap_err();
    assert_eq!(missing.code, "record_missing");

    // Before any scan, the cached proof itself must detect its missing ref.
    assert_scoped(
        &h.delivery,
        &a,
        &b,
        &dependent,
        &wrong_generation,
        &wrong_digest,
        &missing,
    );
    // A new facade has neither an NR inventory nor successful NR proofs. Its
    // ordinary queue bootstrap must nevertheless import the actual runtime's
    // already-authenticated protective scope, before any NR sync on this facade.
    let shared_scope = history_fixture::restart(&h);
    {
        let state = shared_scope.releases.state.lock().unwrap();
        assert!(state.historical_no_release.anchors.is_empty());
        assert!(state.historical_no_release.proofs.is_empty());
    }
    shared_scope.restore_releases().unwrap();
    let shared_pending = Error::new(
        "record_missing",
        "shared no-release seal has not been verified in this queue inventory",
    );
    assert_scoped(
        &shared_scope,
        &a,
        &b,
        &dependent,
        &wrong_generation,
        &wrong_digest,
        &shared_pending,
    );
    {
        let state = shared_scope.releases.state.lock().unwrap();
        assert_eq!(
            state.historical_no_release.anchors[&proof.operation()].digest(),
            proof.digest()
        );
        assert!(state.historical_no_release.proofs.is_empty());
        assert!(state.no_release.is_empty());
    }
    assert_eq!(h.writes(), writes);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
    assert!(owned.owns_all(&b));
    owned.eligible(&b, Utc::now()).unwrap();
    assert_error(
        owned
            .check_no_release_resources(proof.record())
            .unwrap_err(),
        &missing,
    );
    owned
        .quarantine_completion(scope.clone(), missing.clone())
        .unwrap();
    held.quarantine_completion(scope.clone(), missing.clone())
        .unwrap();
    assert!(owned.owns_all(&b));
    assert_eq!(
        held.eligible(&b, Utc::now()).unwrap_err().code,
        "release_hold"
    );

    // The complete scan installs authenticated scope without granting credit.
    h.delivery.sync_no_release_completions().unwrap();
    assert_scoped(
        &h.delivery,
        &a,
        &b,
        &dependent,
        &wrong_generation,
        &wrong_digest,
        &missing,
    );
    let inventory = h.delivery.no_release_completion_inventory().unwrap();
    assert!(inventory.proofs.is_empty());
    assert_eq!(inventory.errors[&proof.operation()].code, "record_missing");
    assert!(inventory.sealed_operations.contains(&proof.operation()));
    assert_error(
        h.delivery
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .clone()
            .record_no_release_required(&ephemeral)
            .unwrap_err(),
        &missing,
    );
    // A fresh delivery facade must use the real signed seal to recover scope;
    // it cannot repopulate successful mutable current facts from old intent.
    let cold = history_fixture::restart(&h);
    cold.sync_no_release_completions().unwrap();
    cold.restore_releases().unwrap();
    assert_scoped(
        &cold,
        &a,
        &b,
        &dependent,
        &wrong_generation,
        &wrong_digest,
        &missing,
    );
    // The successful proof map is now empty. Protective quarantine must still
    // retain an actual reader for A's outer envelope, even for disjoint B.
    // Each corruption occurs after the scan; there is no intervening rescan to
    // turn a bad envelope into an inventory error before this queue query.
    let quarantined_seal = format!("no-release-completion-{}.json", proof.operation());
    let quarantined_bytes = h
        .delivery
        .records
        .read_bytes(&quarantined_seal, 1024 * 1024)
        .unwrap();
    for malformed in [false, true] {
        let (code, message) = if malformed {
            h.delivery
                .records
                .write(&quarantined_seal, &json!({"not_a_signed_seal":true}))
                .unwrap();
            (
                "record_corrupt",
                "control record is invalid JSON or has unknown fields",
            )
        } else {
            let mut altered: Signed<Value> = h.delivery.records.read(&quarantined_seal).unwrap();
            altered.signature = "00".repeat(32);
            h.delivery
                .records
                .write(&quarantined_seal, &altered)
                .unwrap();
            (
                "invalid_attestation",
                "authentication tag verification failed",
            )
        };
        for facade in [&h.delivery, &cold, &shared_scope] {
            let denied = facade
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&b, Utc::now())
                .unwrap_err();
            assert_eq!(denied.code, code);
            assert_eq!(denied.message, message);
        }
        h.delivery
            .records
            .write_bytes(&quarantined_seal, &quarantined_bytes)
            .unwrap();
        // Restoring only the envelope restores scoped behavior, not A's missing
        // reference or credit. The originally installed quarantine still holds.
        for (facade, expected) in [
            (&h.delivery, &missing),
            (&cold, &missing),
            (&shared_scope, &shared_pending),
        ] {
            assert_scoped(
                facade,
                &a,
                &b,
                &dependent,
                &wrong_generation,
                &wrong_digest,
                expected,
            );
        }
    }
    workspace
        .records
        .write_bytes(&vector_leaf, &vector_bytes)
        .unwrap();
    h.delivery.sync_no_release_completions().unwrap();
    {
        let state = h.delivery.releases.state.lock().unwrap();
        state.queue.eligible(&a, Utc::now()).unwrap();
        state.queue.eligible(&dependent, Utc::now()).unwrap();
    }

    // Known own-seal loss keeps its exact previously authenticated scope and
    // identity. Direct hydration cannot mint a replacement from current facts.
    let seal_leaf = format!("no-release-completion-{}.json", proof.operation());
    let seal_bytes = h
        .delivery
        .records
        .read_bytes(&seal_leaf, 1024 * 1024)
        .unwrap();
    std::fs::remove_file(h.delivery.records.path().join(&seal_leaf)).unwrap();
    let missing_seal = proof.validate().unwrap_err();
    assert_eq!(missing_seal.code, "record_missing");
    h.delivery.sync_no_release_completions().unwrap();
    assert_scoped(
        &h.delivery,
        &a,
        &b,
        &dependent,
        &wrong_generation,
        &wrong_digest,
        &missing_seal,
    );
    let (projection, mut snapshot) = h.snapshot().await;
    let current = projection
        .parent(Uuid::from_u128(2), proof.work().parent)
        .unwrap();
    let workspace = h
        .delivery
        .workspace(&projection, Uuid::from_u128(2))
        .unwrap();
    assert_error(
        h.delivery
            .hydrate_no_release(&workspace, current, &mut snapshot)
            .await
            .unwrap_err(),
        &missing_seal,
    );
    assert!(snapshot.parent_delivery().no_release_required().is_none());
    assert_eq!(
        h.delivery
            .records
            .read_bytes(&seal_leaf, 1024 * 1024)
            .unwrap_err()
            .code,
        "record_missing"
    );
    h.delivery
        .records
        .write_bytes(&seal_leaf, &seal_bytes)
        .unwrap();
    h.delivery.sync_no_release_completions().unwrap();

    // A present outer seal with bad authentication or malformed schema cannot
    // become a scoped missing-reference diagnostic merely because A is known.
    for malformed in [false, true] {
        let (code, message) = if malformed {
            h.delivery
                .records
                .write(&seal_leaf, &json!({"not_a_signed_seal":true}))
                .unwrap();
            (
                "record_corrupt",
                "control record is invalid JSON or has unknown fields",
            )
        } else {
            let mut altered: Signed<Value> = h.delivery.records.read(&seal_leaf).unwrap();
            altered.signature = "00".repeat(32);
            h.delivery.records.write(&seal_leaf, &altered).unwrap();
            (
                "invalid_attestation",
                "authentication tag verification failed",
            )
        };
        let cached = h
            .delivery
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .eligible(&b, Utc::now())
            .unwrap_err();
        assert_eq!(cached.code, code);
        assert_eq!(cached.message, message);
        let invalid = h.delivery.sync_no_release_completions().unwrap_err();
        assert_eq!(invalid.code, code);
        assert_eq!(invalid.message, message);
        assert_error(
            h.delivery
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&b, Utc::now())
                .unwrap_err(),
            &invalid,
        );
        h.delivery
            .records
            .write_bytes(&seal_leaf, &seal_bytes)
            .unwrap();
        // Restoring bytes is not a complete authenticated inventory recovery.
        assert_error(
            h.delivery
                .releases
                .state
                .lock()
                .unwrap()
                .queue
                .eligible(&b, Utc::now())
                .unwrap_err(),
            &invalid,
        );
        h.delivery.sync_no_release_completions().unwrap();
        h.delivery
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .eligible(&b, Utc::now())
            .unwrap();
    }

    // Unsafe actual storage is not merely unavailable scoped evidence.
    // The target contains the exact bytes, but a symlink must never be followed.
    let target = h._root.path().join("quarantine-vector-copy.json");
    std::fs::write(&target, &vector_bytes).unwrap();
    std::fs::remove_file(workspace.records.path().join(&vector_leaf)).unwrap();
    std::os::unix::fs::symlink(&target, workspace.records.path().join(&vector_leaf)).unwrap();
    let unsafe_error = proof.validate().unwrap_err();
    assert_eq!(unsafe_error.code, "unsafe_storage");
    assert_error(
        h.delivery
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .eligible(&b, Utc::now())
            .unwrap_err(),
        &unsafe_error,
    );
    assert_error(
        h.delivery.sync_no_release_completions().unwrap_err(),
        &unsafe_error,
    );
    assert_error(
        h.delivery
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .eligible(&b, Utc::now())
            .unwrap_err(),
        &unsafe_error,
    );
    std::fs::remove_file(workspace.records.path().join(&vector_leaf)).unwrap();
    workspace
        .records
        .write_bytes(&vector_leaf, &vector_bytes)
        .unwrap();
    h.delivery.sync_no_release_completions().unwrap();
    h.delivery
        .releases
        .state
        .lock()
        .unwrap()
        .queue
        .eligible(&b, Utc::now())
        .unwrap();

    // An unparseable inventory identity has no authenticated scope at all.
    let malformed = "no-release-not-an-operation.json";
    h.delivery.records.write_bytes(malformed, b"{}").unwrap();
    let unknown = h.delivery.sync_no_release_completions().unwrap_err();
    assert_eq!(unknown.code, "no_release_completion");
    assert_eq!(unknown.message, "no-release inventory filename is invalid");
    assert_error(
        h.delivery
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .eligible(&b, Utc::now())
            .unwrap_err(),
        &unknown,
    );
    std::fs::remove_file(h.delivery.records.path().join(malformed)).unwrap();
    assert_error(
        h.delivery
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .eligible(&b, Utc::now())
            .unwrap_err(),
        &unknown,
    );
    h.delivery.sync_no_release_completions().unwrap();
    h.delivery
        .releases
        .state
        .lock()
        .unwrap()
        .queue
        .eligible(&b, Utc::now())
        .unwrap();
    assert_eq!(h.writes(), writes);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
}

fn candidate(template: &domain::ReleasePlan, repository: RepositoryId) -> domain::ReleasePlan {
    let mut result = template.clone();
    result.operation_id = Uuid::new_v4();
    result.parent_id = Uuid::new_v4();
    result.dependencies.clear();
    result.spacing_seconds = 0;
    assert_eq!(result.steps.len(), 1);
    let domain::StepKind::Merge {
        repository: old, ..
    } = &mut result.steps[0].kind
    else {
        panic!("single actual fixture merge plan")
    };
    *old = repository.clone();
    result.resources = BTreeSet::from([domain::ResourceKey::Repository {
        repository: repository.clone(),
    }]);
    result.deployment_free = BTreeSet::from([repository]);
    result.validate().unwrap();
    result
}
fn assert_error(actual: Error, expected: &Error) {
    assert_eq!(actual.code, expected.code);
    assert_eq!(actual.message, expected.message);
}
fn assert_scoped(
    delivery: &RuntimeDelivery,
    a: &domain::ReleasePlan,
    b: &domain::ReleasePlan,
    dependent: &domain::ReleasePlan,
    wrong_generation: &domain::ReleasePlan,
    wrong_digest: &domain::ReleasePlan,
    error: &Error,
) {
    let state = delivery.releases.state.lock().unwrap();
    assert_error(state.queue.eligible(a, Utc::now()).unwrap_err(), error);
    assert_error(
        state.queue.eligible(dependent, Utc::now()).unwrap_err(),
        error,
    );
    state.queue.eligible(b, Utc::now()).unwrap();
    let now = Utc::now();
    assert_eq!(state.queue.earliest_start(b, now).unwrap(), now);
    for wrong in [wrong_generation, wrong_digest] {
        let denied = state.queue.eligible(wrong, Utc::now()).unwrap_err();
        assert_eq!(denied.code, "release_dependency");
        assert_eq!(
            denied.message,
            "required parent outcome has not been observed successfully released"
        );
    }
}
