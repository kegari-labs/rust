//! Actual DeliveryCoordinator assignment through the public worker tool path.
//! Only command execution is injected; no vector, assignment, production
//! execution class, completed native turn or adequacy review is manufactured.
use crate::pinned_git;
use kegari_v0::evaluation::TrialHost;
#[path = "worker_bundle.rs"]
#[allow(dead_code)] // Also included by targets that exercise only ordinary checks.
mod worker_bundle;
use kegari_v0::{
    codex::*,
    config::Speed,
    control::*,
    error::{Error, Result},
    execution::*,
    git::Git,
    instructions::*,
    isolation::*,
    providers::ProviderBroker,
    reconcile::ParentProjection,
    storage::PrivateDir,
    worker::*,
    workflow::Workflow,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration as StdDuration,
};
use uuid::Uuid;
include!("worker_components.rs");

pub(super) async fn exercise_assignment(
    root: &Path,
    parent: &ParentProjection,
    assignment: &kegari_v0::delivery::DeliveryWorkerAssignment,
    workflow: Workflow,
    control: Arc<ControlHost>,
    lease: &Lease,
) -> Vec<VerifiedCheckExecution> {
    assert_eq!(assignment.role(), AgentRole::Validation);
    assert_eq!(assignment.phase(), "parent_integrated_validation");
    assert!(assignment.authority().work().child.is_some());
    assert_eq!(
        assignment.context()["validation_assignment"]["configured_checks"],
        json!([])
    );
    assert!(assignment.read_sources().is_empty()); // This existing parent fixture has one repository.
    let source = assignment.source().unwrap();
    let original_vector = assignment.candidate_vector_digest().unwrap();
    let git_path = pinned_git::prepare(root);
    let git = Arc::new(Git::new(&git_path).unwrap());
    source.verify_for_clone(&git).unwrap();

    let execution_root = root.join("integrated-worker");
    std::fs::create_dir(&execution_root).unwrap();
    let repository = execution_root.join("repository");
    git_run(
        &execution_root,
        &[
            "clone",
            "--local",
            "--no-hardlinks",
            source.root().to_str().unwrap(),
            repository.to_str().unwrap(),
        ],
    );
    let scratch = execution_root.join("scratch");
    std::fs::create_dir(&scratch).unwrap();
    let private = PrivateDir::create(&root.join("integrated-worker-private")).unwrap();
    let auth = private.child("auth").unwrap();
    let mut isolation = IsolationPlan {
        schema_version: 1,
        role: ExecutionRole::Validation,
        execution_root,
        repository: repository.clone(),
        cwd: repository.clone(),
        scratch: scratch.clone(),
        read_roots: vec![PathGrant {
            path: repository.clone(),
            directory: true,
        }],
        source_writes: vec![],
        resource_writes: vec![],
        forbidden_roots: vec![source.root().into(), private.path().into()],
        managed_execution: None,
    };
    isolation.managed_execution = Some(BoundManagedExecution::automatic(&isolation).unwrap());
    let isolation = isolation.validate().unwrap();
    let revision = git.revision(&repository, "HEAD").unwrap();
    assert_eq!(&revision, source.revision());
    let operation = Uuid::new_v4();
    let authority = WorkerAuthority::Authorized(assignment.authority().clone());
    let native = kegari_v0::runtime_workers::authorized_context(
        parent,
        assignment.authority(),
        assignment.phase(),
        operation,
        &[source.repository().id.clone()],
        json!({}),
    )
    .unwrap();
    let context = assignment.bind_context(native, operation).unwrap();
    assignment
        .verify_bound_context(&context, operation)
        .unwrap();
    let work_generation = assignment.authority().work().digest().unwrap();
    let profile = assignment.authority().profile();
    let assembled = assemble(AssemblyInput {
        workflow: &workflow,
        role: assignment.role(),
        phase: assignment.phase(),
        context: &context,
        workspace_id: assignment.authority().workspace_id(),
        operation_id: operation,
        work_generation: &work_generation,
        correction_revision: &assignment.authority().work().generation.correction,
        profile,
        cwd: &repository,
        codex_home: auth.path(),
        codex_version: kegari_v0::instructions::SUPPORTED_CODEX_VERSION,
        repository: Some(RepositoryContext {
            root: repository.clone(),
            revision: revision.commit.clone(),
            scope_paths: vec![],
        }),
        required_context: &[],
        project_doc_max_bytes: 32768,
        permissions: PermissionManifest {
            read_roots: vec![repository.clone()],
            source_write_paths: vec![],
            resource_write_paths: vec![],
            scratch_roots: vec![scratch],
            repository_code_execution: true,
            enforcement_profile: isolation.digest().into(),
            network: WorkerNetwork::BrokerOnly,
        },
        tools: operation_tool_definitions(assignment.role(), false, true)
            .into_iter()
            .map(|(tool, _)| tool)
            .collect(),
        provider_discovery: ProviderDiscovery::Unavailable {
            reason: "actual assignment with an injected command boundary; no native Codex run"
                .into(),
        },
        known_secrets: &[],
    })
    .unwrap();
    let contract = Arc::new(
        WorkerContract::new(ContractInput {
            assembled,
            authority: authority.clone(),
            isolation: isolation.clone(),
            workflow,
            context,
            review: None,
            account_id: "codex-fixture".into(),
            repository: Some(source.repository().clone()),
            read_resources: BTreeMap::new(),
        })
        .unwrap(),
    );
    let permit = control
        .admit(
            lease,
            operation,
            contract.work_key().unwrap(),
            contract.scope_digest().into(),
            || Ok(()),
        )
        .unwrap();
    let boundary = Arc::new(Boundary {
        fail: AtomicBool::new(false),
        block: AtomicBool::new(false),
        unknown: AtomicBool::new(false),
        last_cancel: Mutex::new(None),
        bundle: None,
    });
    let execution = Arc::new(WorkerExecution {
        git,
        git_executable: git_path,
        boundary: boundary.clone(),
        ownership: Arc::new(
            OwnedDirectory::register(
                DirectoryIdentity {
                    schema_version: 1,
                    runtime_id: lease.runtime_id,
                    directory_id: Uuid::new_v4(),
                    repository: source.repository().clone(),
                    work: assignment.authority().work().clone(),
                    repository_path: repository.clone(),
                    operator_checkout: source.root().into(),
                },
                &isolation,
            )
            .unwrap(),
        ),
        binding: ExecutionBinding {
            runtime_id: lease.runtime_id,
            operation_id: operation,
            work: assignment.authority().work().clone(),
            repository: source.repository().clone(),
            revision,
            candidate_vector_digest: Some(original_vector.into()),
            isolation_digest: isolation.digest().into(),
        },
        limits: ExecutionLimits {
            output_bytes: 32768,
            ..Default::default()
        },
        check_outputs: BTreeMap::new(),
    });
    let registry = Arc::new(ExecutionRegistry::default());
    let sink = Arc::new(Sink::default());
    let journal = private.child("journal").unwrap();
    let mut worker = ScopedWorker::new(
        contract.clone(),
        permit,
        WorkerServices {
            control: control.clone(),
            providers: ProviderBroker::default(),
            host: Arc::new(Host {
                authority: Mutex::new(authority),
                revoked: AtomicBool::new(false),
                owner: AtomicBool::new(true),
                rates: AtomicBool::new(false),
                waiting: AtomicBool::new(false),
            }),
            sink: sink.clone(),
            execution: Some(execution.clone()),
            journal,
            executions: registry.clone(),
        },
    )
    .unwrap();
    let mut state = SessionState::new(
        SessionBinding {
            schema_version: 1,
            account_id: "codex-fixture".into(),
            work_identity: contract.work_key().unwrap(),
            operation_id: operation.to_string(),
            generation: work_generation,
            instruction_digest: contract.instructions().manifest_digest.clone(),
            isolation_digest: isolation.digest().into(),
            profile: CodexProfile {
                model: profile.model.clone(),
                reasoning: profile.reasoning.clone(),
                speed: match profile.speed {
                    Speed::Standard => "standard",
                    Speed::Fast => "fast",
                }
                .into(),
            },
            cwd: repository,
        },
        2,
    )
    .unwrap();
    state.thread_id = Some("integrated-component-thread".into());
    state.turn_id = Some("integrated-component-turn".into());
    state.turns_started = 1;
    worker.checkpoint(&state, None).unwrap();
    let call = |name: &str, arguments: Value| ToolCall {
        thread_id: state.thread_id.clone().unwrap(),
        turn_id: state.turn_id.clone().unwrap(),
        call_id: Uuid::new_v4().to_string(),
        name: name.into(),
        arguments,
    };
    let source_read = worker
        .call(&call("source_read", json!({"path":"source.txt"})))
        .await
        .unwrap();
    assert_eq!(source_read["content"], "production\n");
    let discovered_argv = vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        "test \"$(cat source.txt)\" = production".to_string(),
    ];
    // The fixture proposer chooses this from the actual observed file/task,
    // without a configured command. Execution is still explicitly injected.
    assert_ne!(
        discovered_argv,
        super::workflow().front_matter.checks.integrated["e2e"].argv
    );
    let observed = worker
        .call(&call("context_read", json!({"key":"validation_evidence"})))
        .await
        .unwrap();
    assert_eq!(observed["total_reads"], 1);
    let read_id = observed["reads"][0]["id"].as_str().unwrap();
    let proposed = worker.call(&call("validation_propose", json!({
        "previous":null,"reason":"Validate the actual assembled repository against its observed production source",
        "supporting_reads":[read_id],"requirements":[{
            "id":"candidate-source-integrity","statement":"assembled candidate retains the observed production source contents",
            "rationale":"source_read observed production in source.txt; the task requires validating the exact assembled candidate",
            "argv":discovered_argv,"cwd":".","services":[],"artifacts":[],
        }],
    }))).await.unwrap();
    let plan_digest = proposed["plan_digest"].as_str().unwrap();
    let ran = worker
        .call(&call(
            "validation_run",
            json!({
                "plan_digest":plan_digest,"obligation_id":"candidate-source-integrity",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(ran["successful"], true);
    let checks = worker.take_verified_checks();
    assert_eq!(checks.len(), 1);
    let plan = checks[0].validation_plan().unwrap();
    assert_eq!(plan.digest(), plan_digest);
    assert_eq!(
        plan.record().kind,
        kegari_v0::evidence::CheckKind::Integrated
    );
    assert_eq!(plan.record().original_binding, execution.binding);
    assert_eq!(
        plan.record().binding.candidate_vector_digest.as_deref(),
        Some(original_vector)
    );
    assert_eq!(plan.record().supporting_reads[0].digest().unwrap(), read_id);
    let record = checks[0].record();
    assert_eq!(record.command.argv, discovered_argv);
    assert_eq!(record.schema_version, 4);
    assert_eq!(
        record.evidence_class,
        ExecutionEvidenceClass::DeterministicFixture
    );
    assert_eq!(record.command.cwd, isolation.plan().repository);
    assert!(record.discovered.as_ref().unwrap().transcript.is_file());
    assert!(registry.active_operations().unwrap().is_empty());
    assert!(
        control
            .operations()
            .unwrap()
            .iter()
            .any(|receipt| receipt.kind == "worker_validation"
                && receipt.phase == ReceiptPhase::Acknowledged)
    );
    // A model's success claim remains pending. This test deliberately creates
    // no completed native turn, protected accepted outcome or adequacy review.
    let claim = worker
        .call(&call(
            "outcome_submit",
            json!({
                "claim":{"ready":true},"artifact_refs":[],
            }),
        ))
        .await
        .unwrap();
    assert_eq!(claim["state"], "pending_host_verification");
    assert!(sink.0.lock().unwrap().is_empty());
    source.revalidate_filesystem().unwrap();
    checks
}
