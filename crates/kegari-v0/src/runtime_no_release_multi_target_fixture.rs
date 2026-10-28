//! Complete two-target no-release proof with different artifact identities.
//! Native execution/check inputs remain the same explicitly synthetic upstream
//! fixture boundary as the single-target producer; no capabilities are decoded.
use super::*;

#[tokio::test]
async fn distinct_target_artifacts_survive_no_release_sealing_without_common_artifact_fallback() {
    Box::pin(multiple_targets()).await;
}

async fn multiple_targets() {
    let h = Box::pin(Harness::new_targets(true, true)).await;
    h.delivery.restore_releases().unwrap();
    let projection = h.install_current().await;
    let current = projection
        .parent(Uuid::from_u128(2), h.parent.manifest().work.parent)
        .unwrap();
    let workspace = h
        .delivery
        .workspace(&projection, Uuid::from_u128(2))
        .unwrap();
    let baseline = workspace
        .observer
        .observe(&workspace.contracts[0])
        .await
        .unwrap();
    assert_eq!(baseline.targets().len(), 2);
    let artifacts: BTreeMap<_, _> = baseline
        .targets()
        .iter()
        .map(|target| (target.target.clone(), target.artifact_digest.clone()))
        .collect();
    assert_eq!(artifacts.len(), 2);
    assert_ne!(
        artifacts["service-production"],
        artifacts["service-secondary"]
    );
    assert_eq!(baseline.baseline().target_artifacts, artifacts);
    assert!(baseline.baseline().deployed_artifact.is_none());
    for target in baseline.targets() {
        assert_eq!(target.source, baseline.baseline().production);
        assert_eq!(
            baseline.baseline().artifact_for_target(&target.target),
            Some(target.artifact_digest.as_str())
        );
        assert_eq!(
            baseline.baseline().source_for_target(&target.target),
            Some(target.source.commit.as_str())
        );
    }
    let refs = workspace
        .observer
        .owned_baseline_references(&workspace.contracts[0], &baseline)
        .unwrap();
    assert_eq!(refs.len(), 2);
    let deployment =
        kegari_storage::PrivateDir::open(&workspace.records.path().join("deployment")).unwrap();
    // These are ordinary serialized retained references, never proof constructors.
    // Each must bind its own target+artifact rather than borrow its neighbor's.
    for index in 0..2 {
        let mut wrong = serde_json::to_value(&refs[index]).unwrap();
        wrong["target"] = serde_json::to_value(refs[1 - index].target()).unwrap();
        let wrong: crate::deployment::OwnedTargetObservationRef =
            serde_json::from_value(wrong).unwrap();
        let error = wrong
            .validate_retained(
                &deployment,
                &h.built.runtime.control,
                &h.built.runtime.services.key,
            )
            .unwrap_err();
        assert_eq!(error.code, "production_baseline");
        assert_eq!(
            error.message,
            "owned baseline proof differs from retained target identity"
        );
    }
    h.delivery
        .advance_no_release(&h.lease, &projection, Uuid::from_u128(2), current.key.issue)
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
    let inventory = h.delivery.no_release_completion_inventory().unwrap();
    assert!(inventory.errors.is_empty(), "{:?}", inventory.errors);
    let [proof] = inventory.proofs.as_slice() else {
        panic!("one exact two-target completion required")
    };
    let proof = proof.clone();
    assert_eq!(proof.record().targets.len(), 2);
    assert_eq!(proof.record().baselines[0].target_artifacts, artifacts);
    assert!(proof.record().baselines[0].deployed_artifact.is_none());
    let resources = proof.successful_history().unwrap().resources;
    for target in artifacts.keys() {
        assert!(resources.contains(&domain::ResourceKey::Target {
            provider: "github".into(),
            id: target.clone()
        }));
    }
    h.delivery.sync_no_release_completions().unwrap();
    let cold = history_fixture::restart(&h);
    cold.sync_no_release_completions().unwrap();
    cold.restore_releases().unwrap();
    assert_eq!(
        cold.no_release_completion_inventory().unwrap().proofs[0].digest(),
        proof.digest()
    );
    history_fixture::assert_credit(
        &h,
        &key,
        &proof.work().generation,
        Some(proof.outcome_digest()),
    )
    .await;
    h.stop();
    let writes = h.writes();
    let target = h.owned_target.as_ref().unwrap();
    let effects = target.github.release_effect_count();
    let receipts = canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap();
    // The second target's actual retained proof is independently required; the
    // valid first target and shared source may not stand in for it.
    let second = refs
        .iter()
        .find(|reference| reference.target().target == "service-secondary")
        .unwrap();
    let retained = serde_json::to_value(second).unwrap();
    let leaf = retained["proof_leaf"].as_str().unwrap();
    let bytes = deployment.read_bytes(leaf, 1_048_576).unwrap();
    std::fs::remove_file(deployment.path().join(leaf)).unwrap();
    assert_eq!(proof.validate().unwrap_err().code, "record_missing");
    for service in [h.delivery.clone(), cold.clone()] {
        let invalid = service.no_release_completion_inventory().unwrap();
        assert!(invalid.proofs.is_empty());
        assert_eq!(invalid.errors[&proof.operation()].code, "record_missing");
        assert!(invalid.rows[0].completion.is_none());
        service.sync_no_release_completions().unwrap();
        history_fixture::assert_credit(&h, &key, &proof.work().generation, None).await;
        history_fixture::assert_history_queue_refusal(
            &service,
            &proof,
            &invalid.errors[&proof.operation()],
        );
    }
    deployment.write_bytes(leaf, &bytes).unwrap();
    cold.sync_no_release_completions().unwrap();
    history_fixture::assert_credit(
        &h,
        &key,
        &proof.work().generation,
        Some(proof.outcome_digest()),
    )
    .await;
    assert_eq!(h.writes(), writes);
    assert_eq!(target.github.release_effect_count(), effects);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
}
