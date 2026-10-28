//! Actual owned Git/no-change vector and native baseline. The returned parent
//! check/review facts are explicitly synthetic and cannot qualify integrated
//! execution, provider startup, or the runtime joined-completion cache.
use super::*;
use crate::delivery::{DeliveryPhase, DeliveryScope, ParentCandidateVector, SliceAssembly};
use crate::runtime_delivery::{child::PhaseRequest, worker_fixture};

pub(super) struct PreparedParent {
    pub parent: VerifiedParent,
    pub child: VerifiedChild,
    pub vector: ParentCandidateVector,
    pub body: String,
    pub owned_target: Option<owned_target_fixture::OwnedTargetFixture>,
}

pub(super) fn prepare<'a>(
    built: &'a RuntimeFixture,
    delivery: &'a RuntimeDelivery,
    wire: &'a Arc<Mutex<crate::corrections::wire_fixture::Fixture>>,
    lease: &'a Lease,
    with_owned_target: bool,
    multiple_targets: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = PreparedParent> + 'a>> {
    Box::pin(async move {
        let configuration = built.runtime.configuration.snapshot().unwrap();
        let source =
            std::path::Path::new(&configuration.workspaces["example"].repositories["42"].source);
        let binary = delivery.git_path().unwrap();
        // Disposable upstream setup only. Production assembly below uses its
        // own actual admitted, journaled Git processes and never writes source.
        worker_fixture::git(&binary, source, &["init", "-b", "main"]);
        std::fs::write(source.join("behavior.txt"), "known production fixture\n").unwrap();
        worker_fixture::git(&binary, source, &["add", "behavior.txt"]);
        worker_fixture::git(
            &binary,
            source,
            &[
                "commit",
                "--signoff",
                "-m",
                "test: initialize no-change production fixture",
            ],
        );
        let revision = crate::git::Git::new(&binary)
            .unwrap()
            .revision(source, "HEAD")
            .unwrap();
        {
            let mut data = wire.lock().unwrap();
            data.github_values.insert(
                "/repos/fixture/repo/git/ref/heads/main".into(),
                json!({"ref":"refs/heads/main","object":{"type":"commit","sha":revision.commit}}),
            );
            data.github_values.insert(
                format!("/repos/fixture/repo/git/commits/{}", revision.commit),
                json!({"sha":revision.commit,"tree":{"sha":revision.tree},"parents":[]}),
            );
        }
        let owned_target = with_owned_target.then(|| {
            owned_target_fixture::OwnedTargetFixture::wire(
                &binary,
                source,
                &revision,
                wire,
                multiple_targets,
            )
        });
        let projection = built.runtime.observations.refresh().await.unwrap();
        let observed = projection
            .parent(Uuid::from_u128(2), Uuid::from_u128(100))
            .unwrap();
        let gate = observed
            .fresh_gate(
                None,
                GatePurpose::NewOperation,
                &built.runtime.dependency_outcomes().unwrap(),
                Utc::now(),
                Duration::seconds(30),
            )
            .unwrap();
        let workspace = delivery
            .workspace(&projection, gate.workspace_id())
            .unwrap();
        let contract = &workspace.contracts[0];
        worker_fixture::maintain_phase_observations(&built.runtime, async {
            if let Some(target) = &owned_target {
                Box::pin(target.seed(delivery, &workspace, &gate, lease, &revision)).await?;
            }
            let phase = delivery
                .admit_delivery_phase(
                    &workspace,
                    &gate,
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
                            fresh_gate_digest: gate.digest()?,
                            contract_digest: contract.digest()?,
                            candidate_vector_digest: None,
                        },
                    },
                )
                .await?;
            let root = source.parent().unwrap().join("no-release-candidate");
            kegari_storage::PrivateDir::create(&root)?;
            let slice = phase
                .coordinator
                .assemble_isolated_slice(
                    &phase.admission,
                    SliceAssembly {
                        execution_root: &root,
                        staged: &[],
                        required_children: gate.required_children(),
                        child_dependencies: &BTreeMap::new(),
                        // Upstream child completion is synthetic fixture input.
                        // It cannot create a local-check or integrated capability.
                        satisfied_children: gate.required_children(),
                        production_integrations: &[],
                        identity: kegari_git::CommitIdentity {
                            name: "Disposable Fixture Human".into(),
                            email: "fixture@example.test".into(),
                        },
                    },
                )
                .await?;
            assert!(slice.manifest().ordered_delta_digests.is_empty());
            assert_eq!(slice.manifest().result.as_ref(), Some(&revision));
            let baseline = workspace.observer.observe(contract).await?;
            let parent_contract =
                phase
                    .coordinator
                    .parent_contract(gate.clone(), observed, &workspace.workflow)?;
            let vector = phase
                .coordinator
                .prepare_parent_vector(
                    &parent_contract,
                    &workspace.observer,
                    &[&slice],
                    &[&baseline],
                )
                .await?;
            assert_eq!(vector.candidates(), &[slice.manifest().clone()]);
            assert_eq!(vector.baselines(), &[baseline.baseline().clone()]);
            let current = built.runtime.observations.refresh().await?;
            let e2e = current
                .parent(gate.workspace_id(), gate.work().parent)?
                .fresh_gate(
                    Some(parent_contract.e2e_child()),
                    GatePurpose::NewOperation,
                    &built.runtime.dependency_outcomes()?,
                    Utc::now(),
                    Duration::seconds(30),
                )?;
            let missing = match phase
                .coordinator
                .verify_integrated_checks(&parent_contract, &e2e, &vector, &workspace.observer, &[])
                .await
            {
                Ok(_) => panic!("actual integrated checks remain unavailable"),
                Err(error) => error,
            };
            assert_eq!(missing.code, "integrated_gate");
            assert_eq!(
                missing.message,
                "every configured integrated check must execute exactly once"
            );
            let (parent, child) = parent_fixture::parent(
                gate.work(),
                observed.children[&parent_contract.e2e_child()]
                    .work
                    .as_ref()
                    .unwrap(),
                &built.runtime.services.key,
                slice.manifest().clone(),
                vector.baselines().to_vec(),
                gate.dependency_digest().to_owned(),
            );
            let body = phase.coordinator.fixture_parent_body_for_vector(
                parent.manifest(),
                &vector,
                lease.runtime_id,
            )?;
            delivery.retire_delivery_phase(
                &phase,
                json!({
                    "candidate_vector":vector.digest(),
                    "fixture_upstream":"synthetic checks; actual integrated gate remains blocked"
                }),
            )?;
            assert_eq!(
                built
                    .runtime
                    .control
                    .operation(phase.permit.operation_id)?
                    .phase,
                crate::control::ReceiptPhase::Reconciled
            );
            Ok(PreparedParent {
                parent,
                child,
                vector,
                body,
                owned_target,
            })
        })
        .await
        .unwrap()
    })
}
