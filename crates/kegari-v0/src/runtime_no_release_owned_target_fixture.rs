//! Actual offline target challenge/OIDC/native proof and closed owned receipts.
//! Initial target authority and parent integrated checks are explicitly fixture
//! upstream inputs, not a qualified native release or confined execution.
use super::*;
use crate::delivery::{DeliveryPhase, DeliveryScope, RuntimeLineageWire, RuntimeSeedWire};
use crate::runtime_delivery::{child::PhaseRequest, worker_fixture};

pub(super) struct OwnedTargetFixture {
    github: RuntimeSeedWire,
    native: Arc<RuntimeLineageWire>,
    source: std::path::PathBuf,
    original: crate::git::Revision,
    owner: Mutex<Option<Uuid>>,
}
impl OwnedTargetFixture {
    pub(super) fn wire(
        binary: &std::path::Path,
        source: &std::path::Path,
        revision: &crate::git::Revision,
        data: &Arc<Mutex<crate::corrections::wire_fixture::Fixture>>,
        multiple_targets: bool,
    ) -> Self {
        let github = RuntimeSeedWire::new(revision);
        let native = RuntimeLineageWire::new(&github, revision.clone());
        if multiple_targets {
            native.enable_distinct_targets();
        }
        native.capture_objects(binary, source, std::slice::from_ref(revision));
        data.lock().unwrap().github_transport = Some(native.clone());
        Self {
            github,
            native,
            source: source.to_owned(),
            original: revision.clone(),
            owner: Mutex::new(None),
        }
    }
    pub(super) async fn seed(
        &self,
        delivery: &RuntimeDelivery,
        workspace: &WorkspaceDelivery,
        gate: &FreshGate,
        lease: &Lease,
        source: &crate::git::Revision,
    ) -> Result<()> {
        let contract = &workspace.contracts[0];
        // An actual owned upstream fixture effect. Recovery scope avoids
        // inventing a parent vector before the baseline can be assembled.
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
                        phase: DeliveryPhase::Recovery,
                        work: gate.work().clone(),
                        repository: contract.repository.clone(),
                        revision: evidence::RevisionBinding {
                            repository: contract.repository.clone(),
                            head: source.commit.clone(),
                            base: source.commit.clone(),
                            tested_commit: source.commit.clone(),
                            tested_tree: source.tree.clone(),
                        },
                        fresh_gate_digest: gate.digest()?,
                        contract_digest: contract.digest()?,
                        candidate_vector_digest: None,
                    },
                },
            )
            .await?;
        Box::pin(self.native.seed_all_targets(
            &phase.coordinator,
            &workspace.observer,
            &phase.admission,
        ))
        .await;
        delivery.retire_delivery_phase(&phase, json!({
            "fixture_upstream":"actual offline target proof; synthetic initial deployment authority"
        }))?;
        let receipt = delivery
            .runtime
            .control
            .operation(phase.permit.operation_id)?;
        assert_eq!(receipt.phase, ReceiptPhase::Reconciled);
        assert_eq!(
            receipt.observed_result.as_ref().unwrap()["local_execution"],
            "exited"
        );
        assert_eq!(
            receipt.observed_result.as_ref().unwrap()["unresolved_effects"],
            json!([])
        );
        assert!(
            self.owner
                .lock()
                .unwrap()
                .replace(receipt.operation_id)
                .is_none()
        );
        Ok(())
    }
    fn move_external_fixture_source(&self, binary: &std::path::Path) -> crate::git::Revision {
        // Explicit later upstream state, never controller rollback/deployment.
        std::fs::write(
            self.source.join("later-production.txt"),
            "later external production\n",
        )
        .unwrap();
        worker_fixture::git(binary, &self.source, &["add", "later-production.txt"]);
        worker_fixture::git(
            binary,
            &self.source,
            &[
                "commit",
                "--signoff",
                "-m",
                "test: move external fixture production",
            ],
        );
        let next = crate::git::Git::new(binary)
            .unwrap()
            .revision(&self.source, "HEAD")
            .unwrap();
        self.native
            .capture_objects(binary, &self.source, &[self.original.clone(), next.clone()]);
        self.github.set_production(&next);
        self.native.move_fixture_target(next.clone());
        next
    }
}

#[tokio::test]
async fn owned_target_no_release_history_keeps_exact_closed_provenance_after_native_movement() {
    Box::pin(owned_target_cycle()).await;
}

async fn owned_target_cycle() {
    let h = Box::pin(Harness::new_inner(true)).await;
    let target = h.owned_target.as_ref().unwrap();
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
    assert_eq!(baseline.targets().len(), 1);
    assert_eq!(baseline.targets()[0].source, target.original);
    assert!(!baseline.baseline().deployment_free);
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
        panic!("one actual target-bearing completion required")
    };
    let proof = proof.clone();
    assert_eq!(proof.record().targets.len(), 1);
    assert_eq!(proof.record().targets[0].source, target.original);
    let successful = proof.successful_history().unwrap();
    assert!(successful.resources.contains(&domain::ResourceKey::Target {
        provider: proof.record().targets[0].provider.clone(),
        id: proof.record().targets[0].target.clone(),
    }));
    assert_eq!(successful.spacing_seconds, 0);
    let outcome = proof.outcome_digest().to_owned();
    h.delivery.sync_no_release_completions().unwrap();
    history_fixture::assert_credit(&h, &key, &proof.work().generation, Some(&outcome)).await;
    let cold = history_fixture::restart(&h);
    cold.sync_no_release_completions().unwrap();
    cold.restore_releases().unwrap();
    assert_eq!(
        cold.no_release_completion_inventory().unwrap().proofs[0].digest(),
        proof.digest()
    );

    let seal_leaf = format!("no-release-completion-{}.json", proof.operation());
    let signed: Signed<Value> = h.delivery.records.read(&seal_leaf).unwrap();
    h.built
        .runtime
        .services
        .key
        .verify("no_release_completion", &signed)
        .unwrap();
    let refs = signed.payload["owned_targets"].as_array().unwrap();
    assert_eq!(refs.len(), 1);
    assert!(signed.payload["origin"]["targets_digest"].is_string());
    let files = refs[0]["files"].as_array().unwrap();
    assert!(files.iter().any(|v| v["purpose"] == "target_deployment"));
    assert!(files.iter().any(|v| v["purpose"] == "deployment_challenge"));
    assert!(
        !files
            .iter()
            .any(|v| v["leaf"].as_str().unwrap().starts_with("target-index-"))
    );
    let receipt_ids: Vec<Uuid> = refs[0]["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| serde_json::from_value(v["operation"].clone()).unwrap())
        .collect();
    assert!(receipt_ids.contains(&target.owner.lock().unwrap().unwrap()));
    assert!(
        receipt_ids
            .iter()
            .any(|id| h.built.runtime.control.operation(*id).unwrap().kind == "github_dispatch")
    );

    h.stop();
    let writes = h.writes();
    let effects = target.github.release_effect_count();
    let receipt_digest =
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap();
    let next = target.move_external_fixture_source(&h.delivery.git_path().unwrap());
    assert_ne!(next, target.original);
    let unavailable = workspace
        .observer
        .observe(&workspace.contracts[0])
        .await
        .unwrap_err();
    assert_eq!(unavailable.code, "production_baseline");
    assert_eq!(
        unavailable.message,
        "current target has no retained authenticated deployment proof"
    );
    proof.validate().unwrap();
    cold.sync_no_release_completions().unwrap();
    history_fixture::assert_credit(&h, &key, &proof.work().generation, Some(&outcome)).await;

    let deployment =
        crate::storage::PrivateDir::open(&workspace.records.path().join("deployment")).unwrap();
    let pointers: Vec<_> = deployment
        .records_bounded(65_536)
        .unwrap()
        .into_iter()
        .filter(|leaf| leaf.starts_with("target-index-"))
        .collect();
    assert_eq!(pointers.len(), 1);
    let pointer = &pointers[0];
    let pointer_bytes = deployment.read_bytes(pointer, 1024 * 1024).unwrap();
    std::fs::remove_file(deployment.path().join(pointer)).unwrap();
    proof.validate().unwrap();
    cold.sync_no_release_completions().unwrap();
    history_fixture::assert_credit(&h, &key, &proof.work().generation, Some(&outcome)).await;
    deployment.write_bytes(pointer, &pointer_bytes).unwrap();

    let receipts = crate::storage::PrivateDir::open(
        &h.built.runtime.registration.root().path().join("receipts"),
    )
    .unwrap();
    let required: Vec<_> = files
        .iter()
        .map(|v| (false, v["leaf"].as_str().unwrap().to_owned()))
        .chain(receipt_ids.iter().map(|id| (true, format!("{id}.json"))))
        .collect();
    for (receipt, leaf) in required {
        let directory = if receipt { &receipts } else { &deployment };
        let bytes = directory.read_bytes(&leaf, 1024 * 1024).unwrap();
        std::fs::remove_file(directory.path().join(&leaf)).unwrap();
        assert_eq!(
            proof.validate().unwrap_err().code,
            "record_missing",
            "required {leaf}"
        );
        for service in [h.delivery.clone(), history_fixture::restart(&h)] {
            let inventory = service.no_release_completion_inventory().unwrap();
            assert!(inventory.proofs.is_empty());
            assert_eq!(inventory.errors[&proof.operation()].code, "record_missing");
            assert!(inventory.rows[0].completion.is_none());
            service.sync_no_release_completions().unwrap();
            history_fixture::assert_credit(&h, &key, &proof.work().generation, None).await;
            history_fixture::assert_history_queue_refusal(
                &service,
                &proof,
                &inventory.errors[&proof.operation()],
            );
        }
        directory.write_bytes(&leaf, &bytes).unwrap();
        cold.sync_no_release_completions().unwrap();
        history_fixture::assert_credit(&h, &key, &proof.work().generation, Some(&outcome)).await;
    }
    assert_eq!(h.writes(), writes);
    assert_eq!(target.github.release_effect_count(), effects);
    assert_eq!(
        canonical_digest(1, &h.built.runtime.control.operations().unwrap()).unwrap(),
        receipt_digest
    );
}

#[path = "runtime_no_release_multi_target_fixture.rs"]
mod multiple;
