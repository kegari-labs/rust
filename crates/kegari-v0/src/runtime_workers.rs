//! Production worker preparation and launch. Preparation owns a distinct durable
//! permit; only a fully assembled contract consumes a model-attempt admission.
use crate::{
    authority::{ChildScope, Responsibility},
    config::{Config, Workspace},
    connections::ConnectionRegistry,
    control::{Actor, Lease, MutationRequest, Permit, RunAdmission, RunReference},
    digest::canonical_digest,
    error::{Error, Result},
    execution::{
        DirectoryIdentity, ExecutionBinding, ExecutionLimits, HookInvocation, HookLedger,
        HookPhase, LocalExecutor, OwnedDirectory, SandboxBoundary,
    },
    git::{Git, GitLimits, GitStartGate, IndependentCloneIdentity, RepositoryId, Revision},
    instructions::{AgentRole, ProviderDiscovery, RepositoryContext},
    isolation::{ExecutionRole, IsolationPlan, PathGrant, ValidatedIsolation},
    providers::ProviderBroker,
    runtime::RuntimeCoordinator,
    scheduler::PhaseKey,
    storage::PrivateDir,
    worker::{
        ReadResource, ReviewTarget, WorkerAuthority, WorkerExecution, WorkerHost, WorkerServices,
    },
    worker_run::{
        self, ExecutorHookServices, ExecutorRunHooks, NonExecutingRunHooks, Preparation, RunHooks,
        RunOptions, RunServices,
    },
    workflow::Workflow,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

/// Selected by the host from the actual configured repository/provider phase.
/// The exact commit/tree is checked against local Git objects before cloning.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSelection {
    pub repository: String,
    pub revision: Revision,
}
pub struct LaunchRequest {
    pub lease: Lease,
    pub operation_id: Uuid,
    pub authority: WorkerAuthority,
    pub phase: PhaseKey,
    pub source: Option<SourceSelection>,
    pub max_seconds: u64,
    pub max_turns: u32,
}
impl LaunchRequest {
    /// Pure validation for the scheduler before any preparation side effect.
    /// This does not replace the fresh provider/OS-owner checks at admission.
    pub fn validate_binding(&self) -> Result<()> {
        let generation = match &self.authority {
            WorkerAuthority::Authorized(gate) => canonical_digest(1, &gate.work().generation)?,
            WorkerAuthority::Independent(gate) => gate.review_identity().into(),
        };
        if self.operation_id.is_nil()
            || self.max_seconds == 0
            || self.max_seconds > 86_400
            || self.max_turns == 0
            || self.phase.work != self.authority.work_key()?
            || self.phase.generation != generation
        {
            return Err(Error::new(
                "worker_binding",
                "worker request must bind canonical work, exact generation and finite bounds",
            ));
        }
        if let Some(source) = &self.source {
            source.revision.validate()?;
        }
        validate_selection(&self.authority, self.source.as_ref())
    }
}
struct Plan {
    config: Arc<Config>,
    workspace: Workspace,
    workflow: Workflow,
    request: LaunchRequest,
    role: AgentRole,
    source: Option<SourceSelection>,
    source_path: Option<PathBuf>,
    repository_id: Option<RepositoryId>,
    context: Value,
    review: Option<ReviewTarget>,
    account: String,
    normalized_state: String,
}
struct Prepared {
    run: worker_run::PreparedRun,
    execution: Option<Arc<WorkerExecution>>,
    records: PrivateDir,
}

/// The caller retains its scheduler capacity claim throughout this future. A
/// cancellation cannot be mistaken for completion: preparation handles remain in
/// the shared owned-process registry and Sent receipts keep the canonical hold.
pub async fn launch(runtime: Arc<RuntimeCoordinator>, request: LaunchRequest) -> Result<Uuid> {
    let mut plan = Plan::read(&runtime, request)?;
    let initial_schedule =
        runtime.next_attempt(&plan.request.phase, plan.workspace.id, &plan.account)?;
    plan.context["attempt"] = json!(initial_schedule.attempt);
    let _retained = runtime.observations.capture_active(
        plan.workspace.id,
        &plan.config.revision()?,
        &plan.workflow.digest,
    )?;
    let prep_id = Uuid::new_v4();
    let prep_scope = canonical_digest(
        1,
        &json!({"kind":"worker_preparation","operation":plan.request.operation_id,"authority":plan.request.authority.digest()?,"phase":plan.request.phase,"source":plan.source,"role":plan.role}),
    )?;
    let prep_permit = runtime.control.admit(
        &plan.request.lease,
        prep_id,
        plan.request.authority.work_key()?,
        prep_scope,
        || fresh(&runtime, &plan.request),
    )?;
    let prepared = prepare_owned(&runtime, &plan, &prep_permit).await;
    let observed_exit = runtime
        .executions
        .observe_exited(prep_id)
        .and_then(|_| runtime.executions.active_operations());
    if let Err(error) = observed_exit {
        runtime.control.fence(prep_id)?;
        runtime
            .executions
            .cancel_fenced(&runtime.control, prep_id)?;
        return Err(error);
    }
    if observed_exit?.contains(&prep_id) {
        runtime.control.fence(prep_id)?;
        runtime
            .executions
            .cancel_fenced(&runtime.control, prep_id)?;
        return Err(Error::new(
            "operation_uncertain",
            "preparation retains an unconfirmed owned process and its durable effect",
        ));
    }
    runtime.control.retire_local(
        prep_id,
        match &prepared {
            Ok(_) => json!({"phase":"worker_preparation","result":"prepared"}),
            Err(error) => json!({"phase":"worker_preparation","result":"blocked","error":error}),
        },
    )?;
    let prepared = prepared?;
    let schedule = runtime.next_attempt(&plan.request.phase, plan.workspace.id, &plan.account)?;
    if plan.request.max_turns > schedule.max_turns {
        return Err(Error::new(
            "turns_exhausted",
            "requested run exceeds the original durable phase turn bound",
        ));
    }
    let contract = prepared.run.contract().clone();
    let permit = runtime.admit_worker(
        &plan.request.lease,
        RunAdmission {
            request_id: plan.request.operation_id,
            work_key: contract.work_key()?,
            scope_digest: contract.scope_digest().into(),
            reference: RunReference {
                workspace: plan.workspace.id,
                account: plan.account.clone(),
                normalized_state: Some(plan.normalized_state.clone()),
                phase: plan.request.phase.clone(),
                profile_digest: canonical_digest(1, &contract.instructions().manifest.profile)?,
                instruction_digest: contract.instructions().manifest_digest.clone(),
                max_attempts: schedule.max_attempts,
                max_turns: schedule.max_turns,
                max_review_rounds: schedule.max_review_rounds,
                attempt: schedule.attempt,
            },
        },
        &contract,
    )?;
    let start = (|| {
        runtime.register_run(permit.clone(), contract.clone())?;
        let hooks: Arc<dyn RunHooks> = if let Some(execution) = &prepared.execution {
            Arc::new(ExecutorRunHooks::new(ExecutorHookServices {
                contract: contract.clone(),
                control: runtime.control.clone(),
                host: runtime.clone(),
                execution: execution.clone(),
                registry: runtime.executions.clone(),
                records: prepared.records.child("hooks")?,
                session_id: plan.request.operation_id,
            })?)
        } else {
            Arc::new(NonExecutingRunHooks::new(
                contract.clone(),
                runtime.control.clone(),
                runtime.clone(),
            )?)
        };
        let providers = providers(&runtime, &plan.config, plan.workspace.id)?;
        let run = worker_run::run(
            prepared.run,
            permit.clone(),
            RunServices {
                worker: WorkerServices {
                    control: runtime.control.clone(),
                    providers,
                    host: runtime.clone(),
                    sink: runtime.clone(),
                    execution: prepared.execution,
                    journal: prepared.records.child("journal")?,
                    executions: runtime.executions.clone(),
                },
                hooks,
            },
            RunOptions {
                initial_input: format!(
                    "Perform the admitted {} phase using the supplied current task context and tools. Submit an honest scoped outcome proposal with observed evidence and blockers.",
                    plan.request.phase.phase
                ),
                max_seconds: plan.request.max_seconds,
                max_turns: plan.request.max_turns,
                resume: None,
            },
        )?;
        runtime.track_run(run)
    })();
    if let Err(error) = start {
        runtime.control.fence(permit.operation_id)?;
        runtime
            .executions
            .cancel_fenced(&runtime.control, permit.operation_id)?;
        // An outer setup error may follow creation of the owned run. Never infer
        // its exit from this error or silently reset the consumed attempt.
        if !runtime
            .executions
            .active_operations()?
            .contains(&permit.operation_id)
        {
            runtime.control.retire_local(
                permit.operation_id,
                json!({"outcome":"failed","error":error,"phase":"worker_start"}),
            )?;
        }
        return Err(error);
    }
    Ok(plan.request.operation_id)
}

fn fresh(runtime: &RuntimeCoordinator, request: &LaunchRequest) -> Result<()> {
    runtime.verify_owner(&request.lease)?;
    request
        .authority
        .compare(&runtime.current_authority(&request.authority)?)
}
impl Plan {
    fn read(runtime: &RuntimeCoordinator, request: LaunchRequest) -> Result<Self> {
        request.validate_binding()?;
        fresh(runtime, &request)?;
        let config = Arc::new(runtime.configuration.snapshot()?);
        let workspace_id = match &request.authority {
            WorkerAuthority::Authorized(gate) => gate.workspace_id(),
            WorkerAuthority::Independent(gate) => gate.workspace_id(),
        };
        let workspace = config
            .workspaces
            .values()
            .find(|workspace| workspace.id == workspace_id)
            .cloned()
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "worker workspace is no longer configured",
                )
            })?;
        let workflow = Workflow::load(Path::new(&workspace.workflow))?;
        let projection = runtime.observations.admission_snapshot()?;
        let (role, context, review, generation, normalized_state) = match &request.authority {
            WorkerAuthority::Authorized(gate) => {
                let parent = projection.parent(workspace.id, gate.work().parent)?;
                let role = match gate.child_scope() {
                    Some(scope) => role_for_scope(scope),
                    None => AgentRole::Coordination,
                };
                let issue = match gate.work().child {
                    Some(child) => parent
                        .children
                        .get(&child)
                        .and_then(|child| child.issue.as_ref()),
                    None => parent.issue.as_ref(),
                }
                .ok_or_else(|| {
                    Error::new("incompletely_observed", "worker's actual issue disappeared")
                })?;
                (
                    role,
                    json!({"issue":issue,"parent":parent.issue,"children":parent.children,"scope":gate.child_scope(),"phase":request.phase.phase,"workspace":{"id":workspace.id,"name":parent.workspace_name,"repositories":workspace.repositories.keys().collect::<Vec<_>>()},"dependencies":parent.prerequisites,"corrections":parent.corrections,"evidence":scoped_evidence(runtime, &parent.key)?,"attempt":null,"profile":gate.profile(),"operation":{"id":request.operation_id,"work":gate.work()}}),
                    None,
                    canonical_digest(1, &gate.work().generation)?,
                    issue.state_name.trim().to_lowercase(),
                )
            }
            WorkerAuthority::Independent(gate) => {
                let repository = projection
                    .workspaces
                    .get(&workspace.id)
                    .and_then(|workspace| workspace.repositories.get(&gate.repository_id()))
                    .ok_or_else(|| {
                        Error::new(
                            "incompletely_observed",
                            "review repository is no longer observed",
                        )
                    })?;
                let review = ReviewTarget {
                    repository: repository.identity.clone(),
                    pull_request: gate.pull_request().number,
                    head: gate.pull_request().head_sha.clone(),
                    base: gate.pull_request().base_sha.clone(),
                };
                (
                    AgentRole::IndependentReview,
                    json!({"issue":{"pull_request":gate.pull_request(),"repository":repository},"parent":null,"children":[],"workspace":{"id":workspace.id,"name":projection.workspaces[&workspace.id].workspace_name},"scope":{"repository":repository.identity,"head":gate.pull_request().head_sha,"base":gate.pull_request().base_sha,"source_writes":false},"dependencies":[],"corrections":[],"evidence":{},"attempt":null,"phase":request.phase.phase,"profile":gate.profile(),"operation":{"id":request.operation_id}}),
                    Some(review),
                    gate.review_identity().into(),
                    "independent_review".into(),
                )
            }
        };
        if request.phase.generation != generation {
            return Err(Error::new(
                "stale_generation",
                "phase budget does not identify the exact admitted work generation",
            ));
        }
        validate_selection(&request.authority, request.source.as_ref())?;
        let source = request.source.clone();
        let (source_path, repository_id) = if let Some(source) = &source {
            source.revision.validate()?;
            let repository = workspace
                .repositories
                .get(&source.repository)
                .ok_or_else(|| {
                    Error::new("scope_violation", "source repository is not configured")
                })?;
            let observed_id = source
                .repository
                .parse::<u64>()
                .map_err(|_| Error::invalid("repository requires canonical numeric GitHub ID"))?;
            if !projection
                .workspaces
                .get(&workspace.id)
                .is_some_and(|workspace| workspace.repositories.contains_key(&observed_id))
            {
                return Err(Error::new(
                    "incompletely_observed",
                    "source repository has no complete current provider observation",
                ));
            }
            (
                Some(
                    Path::new(&repository.source)
                        .canonicalize()
                        .map_err(|error| Error::io("resolve configured source", error))?,
                ),
                Some(RepositoryId {
                    hostname: repository.hostname.clone(),
                    id: source.repository.clone(),
                }),
            )
        } else {
            (None, None)
        };
        let account = config.connections[&workspace.codex_connection]
            .account_id
            .clone();
        Ok(Self {
            config,
            workspace,
            workflow,
            request,
            role,
            source,
            source_path,
            repository_id,
            context,
            review,
            account,
            normalized_state,
        })
    }
}
fn role_for_scope(scope: &ChildScope) -> AgentRole {
    match scope.responsibility {
        Responsibility::Implementation => AgentRole::Implementation,
        Responsibility::Testing => AgentRole::Testing,
        Responsibility::Documentation => AgentRole::Documentation,
    }
}
fn validate_selection(authority: &WorkerAuthority, source: Option<&SourceSelection>) -> Result<()> {
    match authority {
        WorkerAuthority::Authorized(gate) => match gate.child_scope() {
            None if source.is_none() => Ok(()),
            None => Err(Error::new(
                "scope_violation",
                "coordination has no implementation repository",
            )),
            Some(scope) => {
                let Some(source) = source else {
                    return if scope.write_repository.is_none() && scope.write_paths.is_empty() {
                        Ok(())
                    } else {
                        Err(Error::new(
                            "incompletely_prepared",
                            "source-writing child requires its authorized repository context",
                        ))
                    };
                };
                if scope.write_repository.as_deref().map_or_else(
                    || !scope.read_repositories.contains(&source.repository),
                    |repository| repository != source.repository,
                ) {
                    Err(Error::new(
                        "scope_violation",
                        "selected source differs from the child's exact repository authorization",
                    ))
                } else {
                    Ok(())
                }
            }
        },
        WorkerAuthority::Independent(gate) => {
            let source = source.ok_or_else(|| {
                Error::new(
                    "incompletely_prepared",
                    "independent review requires its exact observed head",
                )
            })?;
            if source.repository != gate.repository_id().to_string()
                || source.revision.commit != gate.pull_request().head_sha
            {
                Err(Error::new(
                    "scope_violation",
                    "review source differs from selected repository and exact head",
                ))
            } else {
                Ok(())
            }
        }
    }
}

fn providers(
    runtime: &RuntimeCoordinator,
    config: &Config,
    workspace: Uuid,
) -> Result<ProviderBroker> {
    let transports = config
        .connections
        .keys()
        .filter_map(|name| {
            runtime
                .services
                .inputs
                .transport(name)
                .map(|transport| (name.clone(), transport))
        })
        .collect();
    let registry = ConnectionRegistry::bind_with_transports(
        config,
        runtime.services.secrets.as_ref(),
        |name| runtime.services.inputs.environment(name),
        &transports,
    )?;
    let clients = registry.clients(config)?.get(&workspace).ok_or_else(|| {
        Error::new(
            "missing_secret",
            "worker provider connections are unavailable",
        )
    })?;
    Ok(ProviderBroker {
        linear: Some(clients.linear.clone()),
        github: Some(clients.github.clone()),
    })
}
fn executable(path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else if path.components().count() == 1 {
        std::env::var_os("PATH")
            .and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|root| root.join(path))
                    .find(|candidate| candidate.is_file())
            })
            .ok_or_else(|| {
                Error::new(
                    "prerequisite",
                    "configured command is absent from the host executable search path",
                )
            })?
    } else {
        return Err(Error::new(
            "prerequisite",
            "relative command paths require an explicit isolated repository path",
        ));
    };
    resolved
        .canonicalize()
        .map_err(|error| Error::io("resolve configured executable", error))
}
fn request(plan: &Plan, permit: &Permit, kind: &str, payload: Value) -> MutationRequest {
    MutationRequest {
        request_id: Uuid::new_v4(),
        runtime_id: permit.lease.runtime_id,
        actor: Actor {
            identity: format!("owner:{}", permit.lease.owner_id),
            roles: Default::default(),
            principal: None,
        },
        kind: kind.into(),
        canonical_targets: vec![permit.work_key.clone()],
        expected: BTreeMap::from([(
            "worker_operation".into(),
            plan.request.operation_id.to_string(),
        )]),
        payload,
    }
}

async fn prepare_owned(
    runtime: &Arc<RuntimeCoordinator>,
    plan: &Plan,
    permit: &Permit,
) -> Result<Prepared> {
    let mut directories = None;
    runtime.control.validate_permit(permit, || {
        fresh_preparation(runtime, &plan.request)?;
        let root = PrivateDir::create(Path::new(&plan.config.runtime.execution_root))?.child(&plan.request.operation_id.to_string())?;
        let records = runtime.registration.root().child("worker-records")?.child(&plan.request.operation_id.to_string())?;
        if std::fs::read_dir(records.path()).map_err(|error|Error::io("inspect preparation directory",error))?.next().is_some() {
            return Err(Error::new("reconciliation_required", "worker preparation already has retained records; preserve it rather than recreate"));
        }
        records.write("preparation.json", &json!({"runtime":plan.config.runtime.id,"operation":plan.request.operation_id,"preparation":permit.operation_id,"authority":plan.request.authority.digest()?,"work":permit.work_key,"source":plan.source,"phase":plan.request.phase,"role":plan.role,"root":root.path()}))?;
        let scratch = root.child("scratch")?.path().to_path_buf();
        directories = Some((root, records, scratch));
        Ok(())
    })?;
    let (root, records, scratch) = directories.ok_or_else(|| {
        Error::new(
            "ownership_conflict",
            "preparation directory gate did not run",
        )
    })?;
    let repository = root.path().join("repository");
    let git_path = executable("git")?;
    let git = Arc::new(Git::new(&git_path)?);
    let identity = match (
        &plan.request.authority,
        &plan.repository_id,
        &plan.source_path,
    ) {
        (WorkerAuthority::Authorized(gate), Some(repository_id), Some(source)) => {
            Some(DirectoryIdentity {
                schema_version: 1,
                runtime_id: plan.config.runtime.id,
                directory_id: plan.request.operation_id,
                repository: repository_id.clone(),
                work: gate.work().clone(),
                repository_path: repository.clone(),
                operator_checkout: source.clone(),
            })
        }
        _ => None,
    };
    if let (Some(source), Some(source_path)) = (&plan.source, &plan.source_path) {
        let current_runtime = runtime.clone();
        let current_permit = permit.clone();
        let authority = plan.request.authority.clone();
        let lease = plan.request.lease.clone();
        let start_gate: GitStartGate = Arc::new(move |spawn| {
            current_runtime
                .control
                .validate_permit(&current_permit, || {
                    current_runtime.verify_owner(&lease)?;
                    authority.compare(&current_runtime.retained_authority(&authority)?)?;
                    spawn()
                })
        });
        let owned_source = source_path.clone();
        let root_path = root.path().to_path_buf();
        let revision = source.revision.clone();
        let owned_identity = identity.clone();
        let independent = match &plan.request.authority {
            WorkerAuthority::Independent(gate) => Some((
                gate.clone(),
                plan.repository_id
                    .clone()
                    .ok_or_else(|| Error::invalid("review repository identity missing"))?,
                repository.clone(),
            )),
            _ => None,
        };
        let runtime_id = plan.config.runtime.id;
        let operation = plan.request.operation_id;
        let (effect, running) = runtime.control.begin_effect(
            request(
                plan,
                permit,
                "worker_clone",
                json!({"source":source,"destination":repository}),
            ),
            permit,
            || fresh_preparation(runtime, &plan.request),
            |receipt| {
                let running =
                    git.start_operation(GitLimits::default(), start_gate, move |git| {
                        if let Some(identity) = owned_identity {
                            git.prepare_execution_clone(
                                &owned_source,
                                &root_path,
                                &identity,
                                &revision,
                            )
                        } else if let Some((gate, repository_id, repository_path)) = independent {
                            let base =
                                git.exact_revision(&owned_source, &gate.pull_request().base_sha)?;
                            git.prepare_independent_review_clone(
                                &owned_source,
                                &root_path,
                                &IndependentCloneIdentity {
                                    schema_version: 1,
                                    runtime_id,
                                    workspace_id: gate.workspace_id(),
                                    directory_id: operation,
                                    repository: repository_id,
                                    pull_request: gate.pull_request().number,
                                    head: revision,
                                    base,
                                    review_identity: gate.review_identity().into(),
                                    repository_path,
                                    operator_checkout: owned_source.clone(),
                                },
                            )
                        } else {
                            Err(Error::new(
                                "scope_violation",
                                "clone lacks an authorized owned identity",
                            ))
                        }
                    })?;
                runtime.executions.register(
                    permit.operation_id,
                    receipt.operation_id,
                    Arc::new(running.cancellation()),
                )?;
                Ok(running)
            },
        )?;
        let cancellation = running.cancellation();
        let result = running.wait().await;
        if !cancellation.exited()? {
            return Err(Error::new(
                "operation_uncertain",
                "clone process has not confirmed actual exit",
            ));
        }
        runtime
            .executions
            .remove(permit.operation_id, effect.operation_id())?;
        runtime.control.complete_effect(
            effect,
            permit,
            || fresh_preparation(runtime, &plan.request),
            match &result {
                Ok(path) => json!({"clone":path,"revision":source.revision}),
                Err(error) => json!({"error":error,"partial_files":"retained"}),
            },
        )?;
        result?;
    } else {
        runtime.control.validate_permit(permit, || {
            fresh_preparation(runtime, &plan.request)?;
            let substrate = root.child("repository")?;
            if std::fs::read_dir(substrate.path()).map_err(|error|Error::io("inspect coordination substrate",error))?.next().is_some() { return Err(Error::new("ownership_conflict", "coordination substrate is not an empty newly owned directory")); }
            records.write("coordination-substrate.json", &json!({"operation":plan.request.operation_id,"authority":plan.request.authority.digest()?,"path":substrate.path(),"source_writes":false,"repository_execution":false}))
        })?;
    }
    let grants = grants(plan, &repository)?;
    let mut forbidden = forbidden_paths(runtime.registration.root().path(), &root, &plan.config)?;
    // The actual privileged journals are explicitly inaccessible, including when
    // execution_root is a child of the broader private host-state directory.
    forbidden.push(records.path().to_path_buf());
    forbidden.sort();
    forbidden.dedup();
    let mut executables = Vec::new();
    if plan.role.permits_repository_execution() && plan.repository_id.is_some() {
        executables.extend([git_path.clone(), executable("/bin/sh")?]);
        for check in plan
            .workflow
            .front_matter
            .checks
            .local
            .values()
            .chain(plan.workflow.front_matter.checks.integrated.values())
        {
            let command = check
                .argv
                .first()
                .ok_or_else(|| Error::invalid("configured check lacks an executable"))?;
            executables.push(
                if Path::new(command).is_absolute() || Path::new(command).components().count() == 1
                {
                    executable(command)?
                } else {
                    crate::isolation::contained_path(&repository, &repository.join(command))?
                },
            );
        }
        executables.sort();
        executables.dedup();
    }
    let isolation = IsolationPlan {
        schema_version: 1,
        role: match plan.role {
            AgentRole::Coordination => ExecutionRole::Coordination,
            AgentRole::IndependentReview => ExecutionRole::IndependentReview,
            _ => ExecutionRole::Child,
        },
        execution_root: root.path().to_path_buf(),
        repository: repository.clone(),
        cwd: repository.clone(),
        scratch,
        read_roots: grants.reads,
        resource_writes: grants.resource_writes,
        source_writes: grants.writes,
        forbidden_roots: forbidden,
        executables,
    };
    let boundary = isolation.validate()?;
    let execution = if let (Some(identity), Some(source)) = (identity, &plan.source) {
        let owned = Arc::new(OwnedDirectory::register(identity, &boundary)?);
        let binding = ExecutionBinding {
            runtime_id: plan.config.runtime.id,
            operation_id: plan.request.operation_id,
            work: plan
                .request
                .authority
                .work()
                .ok_or_else(|| Error::invalid("executable operation lacks authorized work"))?
                .clone(),
            repository: plan
                .repository_id
                .clone()
                .ok_or_else(|| Error::invalid("executable repository missing"))?,
            revision: source.revision.clone(),
            candidate_vector_digest: None,
            isolation_digest: boundary.digest().into(),
        };
        runtime.services.housekeeping.register_execution(
            &binding,
            &boundary,
            &owned,
            &plan.workflow,
        )?;
        let execution = Arc::new(WorkerExecution {
            git,
            git_executable: git_path,
            boundary: Arc::new(SandboxBoundary),
            ownership: owned,
            binding,
            limits: ExecutionLimits::default(),
            check_outputs: BTreeMap::new(),
        });
        let hook_records = records.child("hooks")?;
        after_create(runtime, plan, permit, &boundary, &execution, &hook_records).await?;
        Some(execution)
    } else {
        None
    };
    let scope_paths = match &plan.request.authority {
        WorkerAuthority::Authorized(gate) => gate
            .child_scope()
            .map(|scope| scope.write_paths.clone())
            .unwrap_or_default(),
        WorkerAuthority::Independent(_) => vec![".".into()],
    };
    let prepared = worker_run::prepare(Preparation {config:plan.config.clone(),workflow:plan.workflow.clone(),authority:plan.request.authority.clone(),operation_id:plan.request.operation_id,role:plan.role,phase:plan.request.phase.phase.clone(),context:plan.context.clone(),isolation,repository:plan.source.as_ref().map(|source|RepositoryContext {root:repository,revision:source.revision.commit.clone(),scope_paths}),repository_id:plan.repository_id.clone(),read_resources:grants.resources,review:plan.review.clone(),required_context:vec![],project_doc_max_bytes:32_768,provider_discovery:ProviderDiscovery::Unavailable {reason:"Pinned app-server does not expose an authenticated project-instruction discovery inventory; supplied accessible files are separately hashed".into()},known_secrets:vec![]})?;
    records.write("contract.json", &json!({"scope":prepared.contract().scope_digest(),"instructions":prepared.contract().instructions().manifest,"tool_contract":prepared.contract().tools().digest(),"isolation":prepared.isolation().plan()}))?;
    Ok(Prepared {
        run: prepared,
        execution,
        records,
    })
}

async fn after_create(
    runtime: &RuntimeCoordinator,
    plan: &Plan,
    permit: &Permit,
    isolation: &ValidatedIsolation,
    execution: &WorkerExecution,
    records: &PrivateDir,
) -> Result<()> {
    let executor = LocalExecutor {
        isolation,
        git: &execution.git,
        boundary: execution.boundary.as_ref(),
        limits: execution.limits.clone(),
        ownership: &execution.ownership,
    };
    let mut ledger = HookLedger::new(&execution.binding)?;
    let prepared = executor.prepare_hook(
        &mut ledger,
        HookInvocation {
            binding: &execution.binding,
            workflow: &plan.workflow,
            phase: HookPhase::AfterCreate,
            session: None,
            eligible_cleanup_plan: None,
        },
        |ledger| records.write("hooks.json", ledger),
    )?;
    let (effect, running) = runtime.control.begin_effect(
        request(
            plan,
            permit,
            "worker_after_create",
            json!({"workflow":plan.workflow.digest,"binding":execution.binding}),
        ),
        permit,
        || fresh_preparation(runtime, &plan.request),
        |receipt| {
            let running = executor.start_hook(prepared)?;
            if let Some(cancellation) = running.cancellation() {
                runtime.executions.register(
                    permit.operation_id,
                    receipt.operation_id,
                    cancellation,
                )?;
            }
            Ok(running)
        },
    )?;
    let cancellation = running.cancellation();
    let result = executor
        .complete_hook(&mut ledger, running, |ledger| {
            records.write("hooks.json", ledger)
        })
        .await;
    runtime
        .services
        .housekeeping
        .record_hook_ledger(execution.binding.operation_id, &ledger)?;
    if cancellation
        .as_ref()
        .map(|handle| handle.exited())
        .transpose()?
        .is_some_and(|exited| !exited)
    {
        return Err(Error::new(
            "operation_uncertain",
            "after-create hook exit remains unconfirmed",
        ));
    }
    runtime
        .executions
        .remove(permit.operation_id, effect.operation_id())?;
    runtime.control.complete_effect(
        effect,
        permit,
        || fresh_preparation(runtime, &plan.request),
        match &result {
            Ok(result) => json!({"hook":"after_create","state":result.state}),
            Err(error) => json!({"hook":"after_create","error":error}),
        },
    )?;
    if !result?.allows_next_phase {
        return Err(Error::new(
            "hook_failed",
            "after-create hook failed or is unknown; retained directory cannot launch",
        ));
    }
    Ok(())
}
struct Grants {
    reads: Vec<PathGrant>,
    resources: BTreeMap<PathBuf, ReadResource>,
    writes: Vec<PathGrant>,
    resource_writes: Vec<PathGrant>,
}
fn grants(plan: &Plan, repository: &Path) -> Result<Grants> {
    let mut result = Grants {
        reads: vec![PathGrant {
            path: repository.to_path_buf(),
            directory: true,
        }],
        resources: BTreeMap::new(),
        writes: vec![],
        resource_writes: vec![],
    };
    if let WorkerAuthority::Authorized(gate) = &plan.request.authority
        && let Some(scope) = gate.child_scope()
    {
        result.writes = scope
            .write_paths
            .iter()
            .map(|path| PathGrant {
                path: repository.join(path.trim_end_matches('/')),
                directory: path.ends_with('/'),
            })
            .collect();
        for id in &scope.read_repositories {
            if plan
                .source
                .as_ref()
                .is_some_and(|source| &source.repository == id)
            {
                continue;
            }
            let path = Path::new(
                &plan
                    .workspace
                    .repositories
                    .get(id)
                    .ok_or_else(|| {
                        Error::new(
                            "scope_violation",
                            "authorized read repository is not configured",
                        )
                    })?
                    .source,
            )
            .canonicalize()
            .map_err(|error| Error::io("resolve read repository", error))?;
            result.reads.push(PathGrant {
                path: path.clone(),
                directory: true,
            });
            result
                .resources
                .insert(path, ReadResource::Repository { id: id.clone() });
        }
        for name in &scope.test_resources {
            let resource = plan.workspace.test_resources.get(name).ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "authorized test resource is not configured",
                )
            })?;
            let path = Path::new(&resource.path)
                .canonicalize()
                .map_err(|error| Error::io("resolve test resource", error))?;
            let grant = PathGrant {
                path: path.clone(),
                directory: path.is_dir(),
            };
            if resource.writable {
                if !resource.disposable {
                    return Err(Error::new(
                        "scope_violation",
                        "writable test resource must be explicitly disposable",
                    ));
                }
                for workspace in plan.config.workspaces.values() {
                    for repository in workspace.repositories.values() {
                        let source =
                            Path::new(&repository.source)
                                .canonicalize()
                                .map_err(|error| {
                                    Error::io("resolve protected operator checkout", error)
                                })?;
                        if path.starts_with(&source) || source.starts_with(&path) {
                            return Err(Error::new(
                                "scope_violation",
                                "test-resource writes cannot overlap any configured operator checkout",
                            ));
                        }
                    }
                }
                result.resource_writes.push(grant.clone());
            }
            result.reads.push(grant);
            result.resources.insert(
                path,
                ReadResource::TestResource {
                    name: name.clone(),
                    writable: resource.writable,
                    disposable: resource.disposable,
                },
            );
        }
    }
    Ok(result)
}
fn forbidden_paths(
    state_root: &Path,
    execution: &PrivateDir,
    config: &Config,
) -> Result<Vec<PathBuf>> {
    let mut forbidden = Vec::new();
    for entry in std::fs::read_dir(state_root)
        .map_err(|error| Error::io("enumerate private host boundaries", error))?
    {
        let path = entry
            .map_err(|error| Error::io("read private host boundary", error))?
            .path();
        if execution.path().starts_with(&path) {
            continue;
        }
        forbidden.push(path);
    }
    for connection in config.connections.values() {
        if let Some(home) = &connection.auth_home {
            forbidden.push(PathBuf::from(home));
        }
    }
    Ok(forbidden)
}
fn scoped_evidence(
    runtime: &RuntimeCoordinator,
    parent: &crate::authority::IssueKey,
) -> Result<Value> {
    let inspection = runtime.inspection()?;
    let delivery = inspection.delivery.get(parent);
    Ok(json!({
        "children":delivery.map(|delivery|delivery.verified_children().values().map(|child|child.evidence()).collect::<Vec<_>>()).unwrap_or_default(),
        "parent":delivery.and_then(|delivery|delivery.verified_parent()).map(|parent|parent.manifest()),
        "acceptance":delivery.and_then(|delivery|delivery.verified_acceptance()).map(|acceptance|acceptance.acceptance()),
        "history":inspection.history.iter().filter(|row|&row.parent==parent).collect::<Vec<_>>(),
        "release":inspection.release_details.get(parent),
    }))
}

fn fresh_preparation(runtime: &RuntimeCoordinator, request: &LaunchRequest) -> Result<()> {
    runtime.verify_owner(&request.lease)?;
    request
        .authority
        .compare(&runtime.retained_authority(&request.authority)?)
}
