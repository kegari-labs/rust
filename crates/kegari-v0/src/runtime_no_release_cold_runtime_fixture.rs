//! Cold runtime evidence caches, not a new OS/control process. The fixture keeps
//! the actual registered ControlHost and disk records, constructs a separate
//! ObservationCache and RuntimeCoordinator, and transfers no old completion or
//! delivery snapshot capability. Native confinement remains unqualified.
use super::*;

#[tokio::test]
async fn saved_no_release_reconstructs_new_runtime_caches_and_rejects_missing_refs() {
    Box::pin(cold_runtime_cycle()).await;
}

async fn cold_runtime_cycle() {
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
    let original = h.delivery.no_release_completion_inventory().unwrap();
    let [proof] = original.proofs.as_slice() else {
        panic!("actual publication and native readback must have sealed one completion")
    };
    let operation = proof.operation();
    let digest = proof.digest().to_owned();
    let outcome = proof.outcome_digest().to_owned();
    let work = proof.work().clone();
    let record = proof.record().clone();
    let key = current.key.clone();
    // Only historical identity values are used below, never the original proof.
    drop(original);
    drop(snapshot);
    h.stop();
    let writes = h.writes();
    let receipts = canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap();
    let cold = new_runtime_evidence(&h).await;
    assert!(
        cold.runtime
            .known_no_release_completions()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        cold.runtime.observations.health().unwrap()["status"],
        "observation_pending"
    );
    cold.runtime.observations.refresh().await.unwrap();
    assert!(
        cold.runtime
            .dependency_outcomes()
            .unwrap()
            .parent_outcome(&key, &work.generation)
            .is_none()
    );
    assert!(
        cold.runtime
            .current_delivery(&key)
            .unwrap()
            .no_release_required()
            .is_none()
    );
    cold.sync_no_release_completions().unwrap();
    let known = cold.runtime.known_no_release_completions().unwrap();
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].operation(), operation);
    assert_eq!(known[0].digest(), digest);
    assert_eq!(known[0].record(), &record);
    assert_eq!(
        cold.runtime
            .dependency_outcomes()
            .unwrap()
            .parent_outcome(&key, &work.generation),
        Some(outcome.as_str())
    );
    assert!(
        cold.runtime
            .current_delivery(&key)
            .unwrap()
            .no_release_required()
            .is_none(),
        "historical cache recovery must not reconstruct a current parent gate"
    );
    assert_eq!(
        cold.releases
            .state
            .lock()
            .unwrap()
            .queue
            .check_no_release_resources(&record)
            .unwrap_err()
            .code,
        "release_recovery_pending"
    );
    cold.restore_releases().unwrap();
    cold.releases
        .state
        .lock()
        .unwrap()
        .queue
        .check_no_release_resources(&record)
        .unwrap();

    let vector_leaf = format!(
        "parent-vector-{}.json",
        &h.parent.manifest().candidate_vector_digest().unwrap()[7..]
    );
    let vector = workspace
        .records
        .read_bytes(&vector_leaf, 1024 * 1024)
        .unwrap();
    std::fs::remove_file(workspace.records.path().join(&vector_leaf)).unwrap();
    let missing = new_runtime_evidence(&h).await;
    assert!(
        missing
            .runtime
            .known_no_release_completions()
            .unwrap()
            .is_empty()
    );
    missing.runtime.observations.refresh().await.unwrap();
    missing.sync_no_release_completions().unwrap();
    let inventory = missing.no_release_completion_inventory().unwrap();
    assert!(inventory.proofs.is_empty());
    assert_eq!(inventory.errors[&operation].code, "record_missing");
    assert_eq!(inventory.anchors.len(), 1);
    assert_eq!(inventory.anchors[0].digest(), digest);
    assert!(
        missing
            .runtime
            .dependency_outcomes()
            .unwrap()
            .parent_outcome(&key, &work.generation)
            .is_none()
    );
    missing.restore_releases().unwrap();
    assert_eq!(
        missing
            .releases
            .state
            .lock()
            .unwrap()
            .queue
            .check_no_release_resources(&record)
            .unwrap_err()
            .code,
        "record_missing"
    );
    workspace
        .records
        .write_bytes(&vector_leaf, &vector)
        .unwrap();
    missing.sync_no_release_completions().unwrap();
    missing.runtime.observations.refresh().await.unwrap();
    assert_eq!(
        missing
            .runtime
            .dependency_outcomes()
            .unwrap()
            .parent_outcome(&key, &work.generation),
        Some(outcome.as_str())
    );
    assert_eq!(h.writes(), writes);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
}

async fn new_runtime_evidence(h: &Harness) -> Arc<RuntimeDelivery> {
    let previous = &h.built.runtime;
    let configuration = Arc::new(crate::runtime_fixture::FixtureConfiguration {
        config: Mutex::new(previous.configuration.snapshot().unwrap()),
        fences: Mutex::new(previous.configuration.authority_fences().unwrap()),
    });
    let observations = Arc::new(
        crate::observations::ObservationCache::with_inputs(
            configuration.clone(),
            previous.services.secrets.clone(),
            previous.services.key.clone(),
            previous.registration.root().child("references").unwrap(),
            previous.services.inputs.clone(),
            previous.control.clone(),
        )
        .await
        .unwrap(),
    );
    let probe = Arc::new(crate::test_process_probe::FixtureBootProbe::new());
    let git = h.delivery.git_path().unwrap();
    let housekeeping = Arc::new(
        crate::housekeeping::Housekeeping::new(
            kegari_storage::PrivateDir::open(previous.registration.root().path()).unwrap(),
            previous.control.clone(),
            probe.clone(),
            git.clone(),
        )
        .unwrap(),
    );
    let runtime = crate::runtime::RuntimeCoordinator::new(crate::host::RuntimeDependencies {
        configuration,
        control: previous.control.clone(),
        registration: previous.registration.clone(),
        observations,
        secrets: previous.services.secrets.clone(),
        key: previous.services.key.clone(),
        housekeeping,
        logs: Arc::new(crate::housekeeping::LogSink::new(
            previous.registration.root().child("logs").unwrap(),
        )),
        inputs: previous.services.inputs.clone(),
        wake: Arc::new(tokio::sync::Notify::new()),
    })
    .unwrap();
    assert!(!Arc::ptr_eq(&runtime, previous));
    assert!(!Arc::ptr_eq(&runtime.observations, &previous.observations));
    assert!(!Arc::ptr_eq(&runtime.executions, &previous.executions));
    let processes = Arc::new(
        crate::process_records::ProcessJournal::with_probe(
            previous
                .registration
                .root()
                .child("no-release-cold-processes")
                .unwrap(),
            runtime.configuration.snapshot().unwrap().runtime.id,
            probe,
        )
        .unwrap(),
    );
    RuntimeDelivery::with_fixture(runtime, processes, git).unwrap()
}
