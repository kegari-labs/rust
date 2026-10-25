//! Real owned Git/source seals and production contract/claim validation. The
//! completed model protocol is synthetic; no native agent qualification claimed.
use crate::{
    control::Lease,
    delivery::*,
    digest::canonical_digest,
    error::{Error, Result},
    git::Git,
    reconcile::FreshGate,
    runtime::VerifiedAuthorizedOutcome,
    scheduler::PhaseKey,
    worker::WorkerAuthority,
};
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
    worker_run::{RunCompletion, RunOutcome},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use uuid::Uuid;

/// Component adapter around the production observation task owner. The caller
/// supplies an already admitted finite phase; no worker, execution result or
/// authority is manufactured here. Finish/join observation work on either phase
/// outcome before returning to assertions in the fixture.
pub(crate) fn maintain_phase_observations<'a, T: 'a>(
    runtime: &'a crate::runtime::RuntimeCoordinator,
    phase: impl std::future::Future<Output = Result<T>> + 'a,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + 'a>> {
    // Keep the large Git/provider phase out of both nested async frames. This
    // changes fixture allocation only; the production owner still awaits it.
    let phase = Box::pin(phase);
    Box::pin(async move {
        let owner = crate::runtime_driver::ObservationMaintenance::new(
            runtime.observations.clone(),
            Arc::new(tokio::sync::Notify::new()),
        );
        let poll_ms = runtime.configuration.snapshot()?.runtime.poll_ms;
        let (result, observed) = owner.maintain_while(poll_ms, phase).await;
        observed?;
        result
    })
}

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
pub(crate) fn git(binary: &Path, cwd: &Path, args: &[&str]) {
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
pub(crate) struct WorkerFixture {
    pub(crate) contract: Arc<WorkerContract>,
    pub(crate) execution: Arc<WorkerExecution>,
}
pub(crate) struct WorkerInput<'a> {
    pub(crate) root: &'a Path,
    pub(crate) source: &'a Path,
    pub(crate) binary: &'a Path,
    pub(crate) git: Arc<Git>,
    pub(crate) gate: &'a FreshGate,
    pub(crate) workflow: &'a crate::workflow::Workflow,
    pub(crate) revision: &'a crate::git::Revision,
    pub(crate) repository: &'a crate::git::RepositoryId,
    pub(crate) runtime: Uuid,
    pub(crate) operation: Uuid,
    pub(crate) role: AgentRole,
    pub(crate) phase: &'a str,
    pub(crate) context: Value,
    pub(crate) assignment: Option<&'a DeliveryWorkerAssignment>,
    pub(crate) account: &'a str,
}
pub(crate) fn worker(input: WorkerInput<'_>) -> WorkerFixture {
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
            AgentRole::Coordination => ExecutionRole::Coordination,
            AgentRole::IndependentReview => ExecutionRole::IndependentReview,
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
            scope_paths: gate
                .child_scope()
                .map(|scope| scope.write_paths.clone())
                .unwrap_or_default(),
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
pub(crate) fn complete(
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
    complete_inner(
        host,
        lease,
        fixture,
        claim,
        FixtureRunOptions {
            unfinished,
            advanced,
            retired_retry: None,
            review_retry: None,
            max_attempts: 3,
        },
    )
}
struct FixtureRunOptions<'a> {
    unfinished: Option<&'a UnfinishedChildWork>,
    advanced: Option<&'a VerifiedSourceAdvanced>,
    retired_retry: Option<&'a RetriedUnfinishedWork>,
    review_retry: Option<&'a ReviewRetryWork>,
    max_attempts: u32,
}
pub(crate) fn complete_capped(
    host: &crate::control::ControlHost,
    lease: &Lease,
    fixture: &WorkerFixture,
    claim: Value,
    max_attempts: u32,
) -> (
    tempfile::TempDir,
    VerifiedAuthorizedOutcome,
    Arc<RunCompletion>,
) {
    complete_inner(
        host,
        lease,
        fixture,
        claim,
        FixtureRunOptions {
            unfinished: None,
            advanced: None,
            retired_retry: None,
            review_retry: None,
            max_attempts,
        },
    )
}
pub(crate) fn complete_retried(
    host: &crate::control::ControlHost,
    lease: &Lease,
    fixture: &WorkerFixture,
    claim: Value,
    proof: &RetriedUnfinishedWork,
) -> (
    tempfile::TempDir,
    VerifiedAuthorizedOutcome,
    Arc<RunCompletion>,
) {
    complete_inner(
        host,
        lease,
        fixture,
        claim,
        FixtureRunOptions {
            unfinished: None,
            advanced: None,
            retired_retry: Some(proof),
            review_retry: None,
            max_attempts: proof.grant().new_budget().max_attempts,
        },
    )
}
pub(crate) fn complete_review_retry(
    host: &crate::control::ControlHost,
    lease: &Lease,
    fixture: &WorkerFixture,
    proof: &ReviewRetryWork,
) -> (
    tempfile::TempDir,
    VerifiedAuthorizedOutcome,
    Arc<RunCompletion>,
) {
    complete_review_retry_claim(
        host,
        lease,
        fixture,
        proof,
        json!({"schema_version":1,"verdict":"pass","unresolved_findings":[],"supporting_links":[proof.grant().provenance().url]}),
    )
}
pub(crate) fn complete_review_retry_claim(
    host: &crate::control::ControlHost,
    lease: &Lease,
    fixture: &WorkerFixture,
    proof: &ReviewRetryWork,
    claim: Value,
) -> (
    tempfile::TempDir,
    VerifiedAuthorizedOutcome,
    Arc<RunCompletion>,
) {
    complete_inner(
        host,
        lease,
        fixture,
        claim,
        FixtureRunOptions {
            unfinished: None,
            advanced: None,
            retired_retry: None,
            review_retry: Some(proof),
            max_attempts: proof.grant().new_budget().max_attempts,
        },
    )
}
fn complete_inner(
    host: &crate::control::ControlHost,
    lease: &Lease,
    fixture: &WorkerFixture,
    claim: Value,
    options: FixtureRunOptions<'_>,
) -> (
    tempfile::TempDir,
    VerifiedAuthorizedOutcome,
    Arc<RunCompletion>,
) {
    let FixtureRunOptions {
        unfinished,
        advanced,
        retired_retry,
        review_retry,
        max_attempts,
    } = options;
    let contract = &fixture.contract;
    let manifest = &contract.instructions().manifest;
    let WorkerAuthority::Authorized(gate) = contract.authority() else {
        panic!()
    };
    let mut reference = RunReference {
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
        max_attempts,
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
    if let Some(proof) = retired_retry {
        reference.phase = proof.phase().clone();
        reference.attempt = 1;
        reference.max_turns = proof.grant().new_budget().max_turns;
        reference.max_review_rounds = proof.grant().new_budget().max_review_rounds;
    }
    if let Some(proof) = advanced {
        reference.phase = proof.phase().clone();
        reference.max_attempts = reference.max_attempts.min(proof.reference().max_attempts);
        reference.max_turns = reference.max_turns.min(proof.reference().max_turns);
        reference.max_review_rounds = reference
            .max_review_rounds
            .min(proof.reference().max_review_rounds);
    }
    if let Some(proof) = review_retry {
        reference.phase = proof.phase().clone();
        reference.attempt = proof.next_attempt().unwrap();
        reference.max_turns = proof.grant().new_budget().max_turns;
        reference.max_review_rounds = proof.grant().new_budget().max_review_rounds;
        for old in host
            .operations()
            .unwrap()
            .iter()
            .filter_map(|receipt| RunReference::from_receipt(receipt).unwrap())
        {
            if old.phase == reference.phase {
                reference.max_attempts = reference.max_attempts.min(old.max_attempts);
                reference.max_turns = reference.max_turns.min(old.max_turns);
                reference.max_review_rounds =
                    reference.max_review_rounds.min(old.max_review_rounds);
            }
        }
    }
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
                let inventory = host.operations().unwrap();
                host.admit_run(lease, input, || {
                    if let Some(proof) = review_retry {
                        proof.verify_contract(contract)?;
                        proof.verify_current(&inventory, std::slice::from_ref(proof.grant()))?;
                    }
                    if let Some(proof) = retired_retry {
                        proof.verify_contract(contract)?;
                        proof.verify_current(&inventory, std::slice::from_ref(proof.grant()))?;
                    }
                    Ok(())
                })
                .unwrap();
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
        json!({"outcome":"completed","session":state,"local_exit_confirmed":true}),
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

/// Component fixture: an actual bounded in-process protocol transport produces
/// the failed terminal state. No native provider/check process is launched, and
/// this completion must never be inserted into the runtime's joined-run cache.
pub(crate) async fn failed_review(
    host: &crate::control::ControlHost,
    lease: &Lease,
    fixture: &WorkerFixture,
) -> Arc<RunCompletion> {
    failed_review_inner(host, lease, fixture, None, None, None).await
}
pub(crate) async fn failed_review_retry(
    host: &crate::control::ControlHost,
    lease: &Lease,
    fixture: &WorkerFixture,
    proof: &ReviewRetryWork,
    schedule: &crate::runtime_schedule::RunSchedule,
    retry_after_ms: Option<u64>,
    failure_code: &str,
) -> Arc<RunCompletion> {
    failed_review_inner(
        host,
        lease,
        fixture,
        Some((proof, schedule)),
        retry_after_ms,
        Some(failure_code),
    )
    .await
}
async fn failed_review_inner(
    host: &crate::control::ControlHost,
    lease: &Lease,
    fixture: &WorkerFixture,
    retry: Option<(&ReviewRetryWork, &crate::runtime_schedule::RunSchedule)>,
    retry_after_ms: Option<u64>,
    failure_code: Option<&str>,
) -> Arc<RunCompletion> {
    use crate::codex::{CodexProfile, RpcClient, SessionBinding, SessionState};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    struct Deny;
    impl crate::codex::ToolBroker for Deny {
        fn call<'a>(
            &'a mut self,
            _: &'a crate::codex::ToolCall,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send + 'a>>
        {
            Box::pin(async {
                Err(Error::new(
                    "fixture_no_execution",
                    "no tools in failed review fixture",
                ))
            })
        }
    }
    let contract = &fixture.contract;
    let manifest = &contract.instructions().manifest;
    let WorkerAuthority::Authorized(gate) = contract.authority() else {
        panic!()
    };
    let mut reference = RunReference {
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
        max_attempts: 1,
        max_turns: 20,
        max_review_rounds: 1,
        attempt: 1,
    };
    if let Some((proof, schedule)) = retry {
        assert_eq!(schedule.attempt, proof.next_attempt().unwrap());
        reference.phase = proof.phase().clone();
        reference.attempt = schedule.attempt;
        reference.max_attempts = schedule.max_attempts;
        reference.max_turns = schedule.max_turns;
        reference.max_review_rounds = schedule.max_review_rounds;
    }
    let max_turns = reference.max_turns;
    host.admit_run(
        lease,
        RunAdmission {
            request_id: manifest.operation_id,
            work_key: gate.work_key().unwrap(),
            scope_digest: contract.scope_digest().into(),
            reference,
        },
        || {
            if let Some((proof, _)) = retry {
                proof.verify_contract(contract)?;
                proof.verify_current(&host.operations()?, std::slice::from_ref(proof.grant()))?;
            }
            Ok(())
        },
    )
    .unwrap();
    let mut state = SessionState::new(
        SessionBinding {
            schema_version: 1,
            account_id: contract.account_id().into(),
            work_identity: gate.work_key().unwrap(),
            operation_id: manifest.operation_id.to_string(),
            generation: manifest.work_generation.clone(),
            instruction_digest: contract.instructions().manifest_digest.clone(),
            isolation_digest: manifest.permissions.enforcement_profile.clone(),
            profile: CodexProfile {
                model: manifest.profile.model.clone(),
                reasoning: manifest.profile.reasoning.clone(),
                speed: match manifest.profile.speed {
                    crate::config::Speed::Standard => "standard",
                    crate::config::Speed::Fast => "fast",
                }
                .into(),
            },
            cwd: manifest.cwd.clone(),
        },
        max_turns,
    )
    .unwrap();
    state.thread_id = Some("failed-review-fixture".into());
    let (client, server) = tokio::io::duplex(16384);
    let (cr, cw) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let failure_code = failure_code.map(str::to_owned);
    let server = tokio::spawn(async move {
        let mut lines = BufReader::new(sr).lines();
        let init: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        sw.write_all(format!("{}\n",json!({"id":init["id"],"result":{"userAgent":"fixture","platformFamily":"unix","platformOs":"macos","codexHome":"/fixture/auth"}})).as_bytes()).await.unwrap();
        let _ = lines.next_line().await.unwrap().unwrap();
        let request: Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(request["method"], "review/start");
        sw.write_all(format!("{}\n{}\n",json!({"id":request["id"],"result":{"reviewThreadId":"failed-review-fixture","turn":{"id":"failed-review-turn","status":"inProgress"}}}),json!({"method":"turn/completed","params":{"threadId":"failed-review-fixture","turn":{"id":"failed-review-turn","status":"failed","items":[],"error":{"message":"synthetic provider failure","codexErrorInfo":failure_code}}}})).as_bytes()).await.unwrap();
    });
    let mut rpc = RpcClient::from_transport(cr, cw, std::time::Duration::from_secs(2)).unwrap();
    rpc.initialize().await.unwrap();
    rpc.review(
        &mut state,
        "Review the exact host-prepared source",
        &mut Deny,
    )
    .await
    .unwrap();
    while state.terminal.is_none() {
        rpc.pump(
            &mut state,
            &mut Deny,
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(2),
        )
        .await
        .unwrap();
    }
    server.await.unwrap();
    assert_eq!(state.terminal.as_deref(), Some("failed"));
    assert!(state.turn_id.is_none());
    let mut error = Error::new("turn_incomplete", "synthetic provider failed terminal");
    if let Some(retry_after_ms) = retry_after_ms {
        error = error.detail(json!({"retry_after_ms":retry_after_ms}));
    }
    let completion = Arc::new(RunCompletion {
        operation_id: manifest.operation_id,
        state,
        outcome: RunOutcome::Failed,
        error: Some(error),
        elapsed_ms: 1,
        hook_error: None,
        checks: vec![],
        failed_checks: vec![],
        execution: Some(fixture.execution.clone()),
        contract: Some(contract.clone()),
        local_exit_confirmed: true,
    });
    host.retire_local(manifest.operation_id, completion.observation())
        .unwrap();
    completion
}
