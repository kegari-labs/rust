//! Real owned Git/source seals and production contract/claim validation. The
//! completed model protocol is synthetic; no native agent qualification claimed.
use super::*;
use crate::test_process_probe as probe;
use crate::{
    control::{RunAdmission, RunReference},
    execution::{
        CommandBoundary, DirectoryIdentity, ExecutionBinding, ExecutionEvidenceClass,
        ExecutionLimits, OwnedDirectory, StartedCommand,
    },
    instructions::{
        AgentRole, AssemblyInput, PermissionManifest, ProviderDiscovery, RepositoryContext,
        WorkerNetwork, assemble,
    },
    isolation::{ExecutionRole, IsolationPlan, PathGrant, ValidatedIsolation},
    worker::{ContractInput, WorkerContract, WorkerExecution, operation_tool_definitions},
    worker_run::RunOutcome,
};
use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
struct NoCommands;
impl CommandBoundary for NoCommands {
    fn evidence_class(&self) -> ExecutionEvidenceClass {
        ExecutionEvidenceClass::DeterministicFixture
    }
    fn start(
        &self,
        _: &ValidatedIsolation,
        _: &[String],
        _: std::time::Duration,
        _: usize,
    ) -> Result<Box<dyn StartedCommand>> {
        Err(Error::new(
            "fixture_no_execution",
            "review fixture cannot execute repository commands",
        ))
    }
}
fn git(binary: &Path, cwd: &Path, args: &[&str]) {
    let output = Command::new(binary)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Disposable Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Disposable Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
struct WorkerFixture {
    contract: Arc<WorkerContract>,
    execution: Arc<WorkerExecution>,
}
struct WorkerInput<'a> {
    root: &'a Path,
    source: &'a Path,
    binary: &'a Path,
    git: Arc<Git>,
    gate: &'a FreshGate,
    workflow: &'a crate::workflow::Workflow,
    revision: &'a crate::git::Revision,
    repository: &'a crate::git::RepositoryId,
    runtime: Uuid,
    operation: Uuid,
    role: AgentRole,
    phase: &'a str,
    context: Value,
    assignment: Option<&'a DeliveryWorkerAssignment>,
    account: &'a str,
}
fn worker(input: WorkerInput<'_>) -> WorkerFixture {
    let WorkerInput {
        root,
        source,
        binary,
        git: tool,
        gate,
        workflow,
        revision,
        repository,
        runtime,
        operation,
        role,
        phase,
        context: task,
        assignment,
        account,
    } = input;
    fs::create_dir(root).unwrap();
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    let repo = root.join("repository");
    git(
        binary,
        root,
        &[
            "clone",
            "--local",
            "--no-hardlinks",
            source.to_str().unwrap(),
            repo.to_str().unwrap(),
        ],
    );
    fs::set_permissions(&repo, fs::Permissions::from_mode(0o700)).unwrap();
    let scratch = root.join("scratch");
    let auth = root.join("auth");
    fs::create_dir(&scratch).unwrap();
    fs::create_dir(&auth).unwrap();
    let writes = if role.permits_source_writes() {
        gate.child_scope()
            .unwrap()
            .write_paths
            .iter()
            .map(|p| PathGrant {
                path: repo.join(p.trim_end_matches('/')),
                directory: p.ends_with('/'),
            })
            .collect()
    } else {
        vec![]
    };
    let isolation = IsolationPlan {
        schema_version: 1,
        role: match role {
            AgentRole::Review => ExecutionRole::Review,
            AgentRole::Validation => ExecutionRole::Validation,
            _ => ExecutionRole::Child,
        },
        execution_root: root.into(),
        repository: repo.clone(),
        cwd: repo.clone(),
        scratch: scratch.clone(),
        read_roots: vec![PathGrant {
            path: repo.clone(),
            directory: true,
        }],
        resource_writes: vec![],
        source_writes: writes,
        forbidden_roots: vec![source.into(), auth.clone()],
        executables: vec![],
    }
    .validate()
    .unwrap();
    let mut context = json!({"issue":{"id":gate.work().child,"title":"Disposable assigned source fixture"},"parent":{"id":gate.work().parent},"children":[],"workspace":{"id":gate.workspace_id(),"name":"fixture"},"scope":gate.child_scope(),"dependencies":[],"corrections":[],"evidence":{"task":if assignment.is_some() {Value::Null}else{task}},"attempt":1,"phase":phase,"profile":gate.profile(),"operation":{"id":operation}});
    if let Some(assignment) = assignment {
        context = assignment.bind_context(context, operation).unwrap();
    }
    let assembled = assemble(AssemblyInput {
        workflow,
        role,
        phase,
        context: &context,
        workspace_id: gate.workspace_id(),
        operation_id: operation,
        work_generation: &gate.work().digest().unwrap(),
        correction_revision: &gate.work().generation.correction,
        profile: gate.profile(),
        cwd: &repo,
        codex_home: &auth,
        codex_version: crate::instructions::SUPPORTED_CODEX_VERSION,
        repository: Some(RepositoryContext {
            root: repo.clone(),
            revision: revision.commit.clone(),
            scope_paths: gate.child_scope().unwrap().write_paths.clone(),
        }),
        required_context: &[],
        project_doc_max_bytes: 32768,
        permissions: PermissionManifest {
            read_roots: vec![repo.clone()],
            resource_write_paths: vec![],
            source_write_paths: isolation
                .plan()
                .source_writes
                .iter()
                .map(|g| g.path.clone())
                .collect(),
            scratch_roots: vec![scratch],
            repository_code_execution: false,
            enforcement_profile: isolation.digest().into(),
            network: WorkerNetwork::BrokerOnly,
        },
        tools: operation_tool_definitions(role, role.permits_source_writes(), false)
            .into_iter()
            .map(|(tool, _)| tool)
            .collect(),
        provider_discovery: ProviderDiscovery::Unavailable {
            reason: "synthetic protocol fixture".into(),
        },
        known_secrets: &[],
    })
    .unwrap();
    let contract = Arc::new(
        WorkerContract::new(ContractInput {
            assembled,
            authority: WorkerAuthority::Authorized(gate.clone()),
            isolation: isolation.clone(),
            workflow: workflow.clone(),
            context,
            review: None,
            account_id: account.into(),
            repository: Some(repository.clone()),
            read_resources: BTreeMap::new(),
        })
        .unwrap(),
    );
    let ownership = Arc::new(
        OwnedDirectory::register(
            DirectoryIdentity {
                schema_version: 1,
                runtime_id: runtime,
                directory_id: Uuid::new_v4(),
                repository: repository.clone(),
                work: gate.work().clone(),
                repository_path: repo,
                operator_checkout: source.into(),
            },
            &isolation,
        )
        .unwrap(),
    );
    WorkerFixture {
        contract,
        execution: Arc::new(WorkerExecution {
            git: tool.clone(),
            git_executable: binary.into(),
            boundary: Arc::new(NoCommands),
            ownership,
            binding: ExecutionBinding {
                runtime_id: runtime,
                operation_id: operation,
                work: gate.work().clone(),
                repository: repository.clone(),
                revision: revision.clone(),
                candidate_vector_digest: None,
                isolation_digest: isolation.digest().into(),
            },
            limits: ExecutionLimits::default(),
            check_outputs: BTreeMap::new(),
        }),
    }
}
fn complete(
    host: &crate::control::ControlHost,
    lease: &Lease,
    fixture: &WorkerFixture,
    claim: Value,
    unfinished: Option<&UnfinishedChildWork>,
    advanced: Option<&VerifiedSourceAdvanced>,
) -> (
    tempfile::TempDir,
    VerifiedAuthorizedOutcome,
    Arc<RunCompletion>,
) {
    let contract = &fixture.contract;
    let manifest = &contract.instructions().manifest;
    let WorkerAuthority::Authorized(gate) = contract.authority() else {
        panic!()
    };
    let reference = RunReference {
        workspace: gate.workspace_id(),
        account: contract.account_id().into(),
        normalized_state: Some("in progress".into()),
        phase: PhaseKey {
            work: gate.work_key().unwrap(),
            generation: canonical_digest(1, &gate.work().generation).unwrap(),
            phase: manifest.phase.clone(),
            retry_grant: None,
        },
        profile_digest: canonical_digest(1, &manifest.profile).unwrap(),
        instruction_digest: contract.instructions().manifest_digest.clone(),
        max_attempts: 3,
        max_turns: 20,
        max_review_rounds: 3,
        attempt: unfinished.map_or_else(
            || {
                advanced.map_or_else(
                    || {
                        host.operations()
                            .unwrap()
                            .iter()
                            .filter_map(|receipt| RunReference::from_receipt(receipt).unwrap())
                            .filter(|run| {
                                run.phase.phase == manifest.phase
                                    && run.phase.work == gate.work_key().unwrap()
                            })
                            .map(|run| run.attempt)
                            .max()
                            .unwrap_or(0)
                            + 1
                    },
                    |proof| proof.reference().attempt + 1,
                )
            },
            |proof| proof.reference().attempt + 1,
        ),
    };
    let input = RunAdmission {
        request_id: manifest.operation_id,
        work_key: gate.work_key().unwrap(),
        scope_digest: contract.scope_digest().into(),
        reference,
    };
    match unfinished {
        Some(proof) => {
            host.admit_run_continued_observed(
                lease,
                input,
                crate::control::RunContinuationReference {
                    predecessor: proof.predecessor(),
                    predecessor_receipt_digest: proof.predecessor_receipt_digest().into(),
                    trigger_digest: proof.digest().into(),
                    progress_digest: proof.progress_digest().into(),
                    failure_operation: proof.failure_operation(),
                    review_rounds_consumed: proof.review_rounds_consumed(),
                },
                |_| proof.verify_contract(contract),
            )
            .unwrap();
        }
        None => match advanced {
            Some(proof) => {
                let inventory = host.operations().unwrap();
                host.admit_run_source_advanced_observed(
                    lease,
                    input,
                    crate::control::SourceAdvancedReference {
                        predecessor: proof.predecessor(),
                        predecessor_receipt_digest: proof.predecessor_receipt_digest().into(),
                        source_operation: proof.source_operation(),
                        source_receipt_digest: proof.source_receipt_digest().into(),
                        trigger_digest: proof.digest().into(),
                        progress_digest: proof.progress_digest().into(),
                        review_rounds_consumed: proof.review_rounds_consumed(),
                    },
                    |_| {
                        proof.verify_contract(contract)?;
                        proof.verify_predecessor_inventory(&inventory)
                    },
                )
                .unwrap();
            }
            None => {
                host.admit_run(lease, input, || Ok(())).unwrap();
            }
        },
    }
    let (files, outcome, state) = crate::runtime_claims::test_completed_contract_fixture(
        contract,
        host.operation(manifest.operation_id).unwrap(),
        claim,
    )
    .unwrap();
    host.retire_local(
        manifest.operation_id,
        json!({"outcome":"completed","session":state}),
    )
    .unwrap();
    let completion = Arc::new(RunCompletion {
        operation_id: manifest.operation_id,
        state,
        outcome: RunOutcome::Completed,
        error: None,
        elapsed_ms: 1,
        hook_error: None,
        checks: vec![],
        failed_checks: vec![],
        execution: Some(fixture.execution.clone()),
        contract: Some(contract.clone()),
        local_exit_confirmed: true,
    });
    (files, outcome, completion)
}
#[tokio::test]
async fn exact_review_findings_produce_repair_from_real_owned_source_and_contracts() {
    let (root, built, old_service, _wire, owner) =
        super::super::tests::fixture_authorization_with_wire(true).await;
    let binary = old_service.git_path().unwrap();
    let root = root.path().canonicalize().unwrap();
    let source = root.join("source");
    git(&binary, &source, &["init", "-b", "main"]);
    fs::create_dir(source.join("tests")).unwrap();
    fs::write(source.join("tests/main.txt"), "initial\n").unwrap();
    git(&binary, &source, &["add", "tests"]);
    git(
        &binary,
        &source,
        &[
            "commit",
            "--signoff",
            "-m",
            "test: initialize disposable continuation source",
        ],
    );
    let tool = Arc::new(Git::new(&binary).unwrap());
    let revision = tool.revision(&source, "HEAD").unwrap();
    let processes = Arc::new(
        crate::process_records::ProcessJournal::with_probe(
            built
                .runtime
                .registration
                .root()
                .child("continuation-process-fixture")
                .unwrap(),
            built.runtime.configuration.snapshot().unwrap().runtime.id,
            Arc::new(probe::FixtureBootProbe::new()),
        )
        .unwrap(),
    );
    let service =
        RuntimeDelivery::with_fixture(built.runtime.clone(), processes, binary.clone()).unwrap();
    let projection = built.runtime.observations.snapshot().unwrap();
    let parent = projection
        .parent(Uuid::from_u128(2), Uuid::from_u128(100))
        .unwrap();
    let gate = parent
        .fresh_gate(
            Some(Uuid::from_u128(101)),
            GatePurpose::NewOperation,
            &crate::reconcile::DependencyOutcomes::default(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    let workspace = service.workspace(&projection, gate.workspace_id()).unwrap();
    let repository = workspace.contracts[0].repository.clone();
    let (lease, _lock) = owner.start(&built.runtime);
    let runtime = lease.runtime_id;
    let account = workspace.configuration.connections
        [&workspace.configuration.workspaces["example"].codex_connection]
        .account_id
        .clone();
    let original = worker(WorkerInput {
        root: &root.join("implementation"),
        source: &source,
        binary: &binary,
        git: tool.clone(),
        gate: &gate,
        workflow: &workspace.workflow,
        revision: &revision,
        repository: &repository,
        runtime,
        operation: Uuid::new_v4(),
        role: AgentRole::Testing,
        phase: "implementation",
        context: json!({"task":"check the required behavior"}),
        assignment: None,
        account: &account,
    });
    let (_original_records, outcome, completion) = complete(
        &built.runtime.control,
        &lease,
        &original,
        json!({"schema_version":1,"summary":"existing source assessed","fulfilled_criteria":["required behavior"],"disposition":{"kind":"no_change","reason":"claimed existing behavior suffices"}}),
        None,
        None,
    );
    let claim = Arc::new(
        workspace
            .coordinator
            .accept_child_completion(&gate, outcome, completion.clone())
            .unwrap(),
    );
    let binding = RevisionBinding {
        repository: repository.clone(),
        head: revision.commit.clone(),
        base: revision.commit.clone(),
        tested_commit: revision.commit.clone(),
        tested_tree: revision.tree.clone(),
    };
    let scope = |phase| DeliveryScope {
        schema_version: 1,
        runtime_id: runtime,
        phase,
        work: gate.work().clone(),
        repository: repository.clone(),
        revision: binding.clone(),
        fresh_gate_digest: gate.digest().unwrap(),
        contract_digest: workspace.contracts[0].digest().unwrap(),
        candidate_vector_digest: None,
    };
    let sealing = service
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: scope(DeliveryPhase::SourceSealing),
            },
        )
        .await
        .unwrap();
    let source_proof=sealing.coordinator.seal_child_source(&sealing.admission,&claim,&original.execution,original.contract.isolation(),crate::git::SourceCommitMetadata {identity:crate::git::CommitIdentity {name:"Disposable Fixture".into(),email:"fixture@example.invalid".into()},timestamp:chrono::DateTime::from_timestamp(1_767_225_600,0).unwrap(),message:"test: seal disposable source\n\nSigned-off-by: Disposable Fixture <fixture@example.invalid>\n".into()}).await.unwrap();
    service
        .retire_delivery_phase(&sealing, json!({"sealed":true}))
        .unwrap();
    built.runtime.observations.refresh().await.unwrap();
    let validation_phase = service
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: scope(DeliveryPhase::LocalValidation),
            },
        )
        .await
        .unwrap();
    let validation_assignment = validation_phase
        .coordinator
        .child_validation_assignment(
            &validation_phase.admission,
            &claim,
            &source_proof,
            &completion,
            json!({"task":"required local checks"}),
        )
        .unwrap();
    service
        .retire_delivery_phase(&validation_phase, json!({"assignment_prepared":true}))
        .unwrap();
    let validator = worker(WorkerInput {
        root: &root.join("validation"),
        source: &source,
        binary: &binary,
        git: tool.clone(),
        gate: &gate,
        workflow: &workspace.workflow,
        revision: &revision,
        repository: &repository,
        runtime,
        operation: Uuid::new_v4(),
        role: AgentRole::Validation,
        phase: "child_local_validation",
        context: Value::Null,
        assignment: Some(&validation_assignment),
        account: &account,
    });
    let (_validation_records, validation_outcome, validation_completion) = complete(
        &built.runtime.control,
        &lease,
        &validator,
        json!({"kind":"validation","summary":"synthetic protocol; no passed check claimed"}),
        None,
        None,
    );
    built.runtime.observations.refresh().await.unwrap();
    let review_phase = service
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: scope(DeliveryPhase::NoChange),
            },
        )
        .await
        .unwrap();
    let review = review_phase
        .coordinator
        .prepare_child_review(
            &review_phase.admission,
            &claim,
            ChildReviewTarget::NoChange {
                source: &source_proof,
            },
            json!({"task":"review required behavior"}),
        )
        .unwrap();
    let review_worker = review_phase
        .coordinator
        .child_review_worker_assignment(
            &review_phase.admission,
            &claim,
            &source_proof,
            &completion,
            &review,
        )
        .unwrap();
    service
        .retire_delivery_phase(&review_phase, json!({"assignment_prepared":true}))
        .unwrap();
    let reviewer = worker(WorkerInput {
        root: &root.join("review"),
        source: &source,
        binary: &binary,
        git: tool.clone(),
        gate: &gate,
        workflow: &workspace.workflow,
        revision: &revision,
        repository: &repository,
        runtime,
        operation: Uuid::new_v4(),
        role: AgentRole::Review,
        phase: "child_codex_review",
        context: review.context().clone(),
        assignment: Some(&review_worker),
        account: &account,
    });
    let (_review_records, review_outcome, review_completion) = complete(
        &built.runtime.control,
        &lease,
        &reviewer,
        json!({"schema_version":1,"verdict":"findings","unresolved_findings":["Required empty-input behavior has no implementation in tests/main.txt"],"supporting_links":[]}),
        None,
        None,
    );
    built.runtime.observations.refresh().await.unwrap();
    let review_phase = service
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: scope(DeliveryPhase::NoChange),
            },
        )
        .await
        .unwrap();
    let proof = review_phase
        .coordinator
        .unfinished_review(ReviewRepairRequest {
            source: RepairSource {
                admission: &review_phase.admission,
                original: claim,
                source: &source_proof,
                completion,
            },
            assignment: &review,
            outcome: &review_outcome,
            completion: review_completion.clone(),
        })
        .await
        .unwrap();
    assert_eq!(proof.predecessor(), original.execution.binding.operation_id);
    assert_eq!(proof.phase().phase, "implementation");
    assert_eq!(proof.review_rounds_consumed(), 1);
    assert_eq!(proof.source().revision(), &revision);
    assert_eq!(proof.reference().max_attempts, 3);
    let assignment = repair_worker_assignment(&proof).unwrap();
    proof.verify_assignment(&assignment).unwrap();
    let repair = worker(WorkerInput {
        root: &root.join("repair"),
        source: proof.source().root(),
        binary: &binary,
        git: tool.clone(),
        gate: &gate,
        workflow: &workspace.workflow,
        revision: &revision,
        repository: &repository,
        runtime,
        operation: Uuid::new_v4(),
        role: AgentRole::Testing,
        phase: "implementation",
        context: proof.remaining_task().clone(),
        assignment: Some(&assignment),
        account: &account,
    });
    proof.verify_contract(&repair.contract).unwrap();
    assert!(proof.verify_contract(&original.contract).is_err());
    let proof = Arc::new(proof);
    let request = crate::runtime_workers::LaunchRequest {
        lease: lease.clone(),
        operation_id: repair.execution.binding.operation_id,
        authority: WorkerAuthority::Authorized(gate.clone()),
        phase: proof.phase().clone(),
        source: None,
        delivery: Some(assignment),
        unfinished: Some(proof.clone()),
        source_advanced: None,
        max_seconds: 60,
        max_turns: 20,
    };
    request.validate_binding().unwrap();
    proof
        .verify_failure_inventory(&built.runtime.control.operations().unwrap())
        .unwrap();
    service
        .retire_delivery_phase(&review_phase, json!({"findings":true}))
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    fs::write(
        repair
            .contract
            .isolation()
            .plan()
            .repository
            .join("tests/main.txt"),
        "required empty-input case covered\n",
    )
    .unwrap();
    let (_repair_records, repair_outcome, repair_completion) = complete(
        &built.runtime.control,
        &lease,
        &repair,
        json!({"schema_version":1,"summary":"required finding corrected","fulfilled_criteria":["required behavior"],"disposition":{"kind":"changed"}}),
        Some(&proof),
        None,
    );
    let repair_claim = Arc::new(
        workspace
            .coordinator
            .accept_child_completion(&gate, repair_outcome, repair_completion.clone())
            .unwrap(),
    );
    built.runtime.observations.refresh().await.unwrap();
    let repair_sealing = service
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: scope(DeliveryPhase::SourceSealing),
            },
        )
        .await
        .unwrap();
    let changed=repair_sealing.coordinator.seal_child_source(&repair_sealing.admission,&repair_claim,&repair.execution,repair.contract.isolation(),crate::git::SourceCommitMetadata {identity:crate::git::CommitIdentity {name:"Disposable Fixture".into(),email:"fixture@example.invalid".into()},timestamp:chrono::DateTime::from_timestamp(1_767_225_601,0).unwrap(),message:"test: repair disposable required case\n\nSigned-off-by: Disposable Fixture <fixture@example.invalid>\n".into()}).await.unwrap();
    service
        .retire_delivery_phase(&repair_sealing, json!({"sealed":true}))
        .unwrap();
    assert_ne!(changed.revision().tested_tree, revision.tree);
    let mut advanced_scope = scope(DeliveryPhase::LocalValidation);
    advanced_scope.revision = changed.revision().clone();
    built.runtime.observations.refresh().await.unwrap();
    let revalidate = service
        .admit_delivery_phase(
            &workspace,
            &gate,
            &lease,
            PhaseRequest {
                operation: Uuid::new_v4(),
                scope: advanced_scope,
            },
        )
        .await
        .unwrap();
    let assignment = revalidate
        .coordinator
        .child_validation_assignment(
            &revalidate.admission,
            &repair_claim,
            &changed,
            &repair_completion,
            json!({"task":"validate corrected exact source"}),
        )
        .unwrap();
    let advanced = revalidate
        .coordinator
        .source_advanced(SourceAdvancedRequest {
            source: RepairSource {
                admission: &revalidate.admission,
                original: repair_claim,
                source: &changed,
                completion: repair_completion,
            },
            assignment: &assignment,
            predecessor: &validation_outcome,
            completion: validation_completion,
        })
        .await
        .unwrap();
    assert_eq!(
        advanced.predecessor(),
        validator.execution.binding.operation_id
    );
    assert_eq!(
        advanced.source_operation(),
        repair.execution.binding.operation_id
    );
    assert_eq!(advanced.phase().phase, "child_local_validation");
    assert_eq!(advanced.reference().max_attempts, 3);
    advanced.verify_assignment(&assignment).unwrap();
    advanced
        .verify_predecessor_inventory(&built.runtime.control.operations().unwrap())
        .unwrap();
    service
        .retire_delivery_phase(&revalidate, json!({"advanced_assignment_prepared":true}))
        .unwrap();
    // This fixture has a synthetic completed provider protocol, not an actual
    // owned driver join. Do not manufacture runtime-cache process authority.
    assert_eq!(
        built
            .runtime
            .next_attempt_for_source_advanced(&advanced)
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    let revalidator = worker(WorkerInput {
        root: &root.join("revalidation"),
        source: advanced.source().root(),
        binary: &binary,
        git: tool.clone(),
        gate: &gate,
        workflow: &workspace.workflow,
        revision: advanced.source().revision(),
        repository: &repository,
        runtime,
        operation: Uuid::new_v4(),
        role: AgentRole::Validation,
        phase: "child_local_validation",
        context: Value::Null,
        assignment: Some(&assignment),
        account: &account,
    });
    advanced.verify_contract(&revalidator.contract).unwrap();
    let prior_validation = built
        .runtime
        .control
        .operation(advanced.predecessor())
        .unwrap();
    let (_revalidation_records, _revalidation_outcome, _) = complete(
        &built.runtime.control,
        &lease,
        &revalidator,
        json!({"kind":"validation","summary":"synthetic completed protocol; no passed execution proof claimed"}),
        None,
        Some(&advanced),
    );
    let admitted = RunReference::from_receipt(
        &built
            .runtime
            .control
            .operation(revalidator.execution.binding.operation_id)
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(admitted.attempt, 2);
    assert_eq!(admitted.max_attempts, 3);
    assert_eq!(admitted.phase, advanced.reference().phase);
    assert_eq!(
        advanced
            .verify_predecessor_inventory(&built.runtime.control.operations().unwrap())
            .unwrap_err()
            .code,
        "stale_result"
    );
    assert_eq!(
        canonical_digest(
            1,
            &built
                .runtime
                .control
                .operation(advanced.predecessor())
                .unwrap()
        )
        .unwrap(),
        canonical_digest(1, &prior_validation).unwrap()
    );
    // A subsequent clean same-head review must invalidate the retained failure,
    // even though its original failed-review receipt remains byte-for-byte equal.
    let clean_reviewer = worker(WorkerInput {
        root: &root.join("clean-review"),
        source: &source,
        binary: &binary,
        git: tool,
        gate: &gate,
        workflow: &workspace.workflow,
        revision: &revision,
        repository: &repository,
        runtime,
        operation: Uuid::new_v4(),
        role: AgentRole::Review,
        phase: "child_codex_review",
        context: Value::Null,
        assignment: Some(&review_worker),
        account: &account,
    });
    let prior_failure = built
        .runtime
        .control
        .operation(proof.failure_operation())
        .unwrap();
    let (_clean_records, _clean_outcome, _) = complete(
        &built.runtime.control,
        &lease,
        &clean_reviewer,
        json!({"schema_version":1,"verdict":"pass","unresolved_findings":[],"supporting_links":[]}),
        None,
        None,
    );
    assert_eq!(
        proof
            .verify_failure_inventory(&built.runtime.control.operations().unwrap())
            .unwrap_err()
            .code,
        "stale_result"
    );
    assert_eq!(
        canonical_digest(
            1,
            &built
                .runtime
                .control
                .operation(proof.failure_operation())
                .unwrap()
        )
        .unwrap(),
        canonical_digest(1, &prior_failure).unwrap()
    );
    fs::write(
        repair
            .contract
            .isolation()
            .plan()
            .repository
            .join("tests/main.txt"),
        "changed after source seal\n",
    )
    .unwrap();
    assert!(
        advanced
            .source()
            .verify_for_clone(&repair.execution.git)
            .is_err()
    );
}
