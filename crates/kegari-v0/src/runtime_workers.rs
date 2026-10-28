//! Production worker preparation and launch. Preparation owns a distinct durable
//! permit; only a fully assembled contract consumes a model-attempt admission.
use crate::{
    authority::{ChildScope, Responsibility},
    connections::ConnectionRegistry,
    control::{
        Actor, Lease, MutationRequest, Permit, PreparationAdmission, PreparationReference,
        RunAdmission, RunReference,
    },
    digest::canonical_digest,
    execution::{
        DirectoryIdentity, ExecutionBinding, ExecutionLimits, HookInvocation, HookLedger,
        HookPhase, JournaledSandboxBoundary, LocalExecutor, OwnedDirectory,
    },
    git::{
        Git, GitExecutionScope, GitLimits, GitStartGate, IndependentCloneIdentity, RepositoryId,
        Revision,
    },
    housekeeping::{ReadOnlyIdentity, ReadOnlyPreparation},
    instructions::{AgentRole, ProviderDiscovery, RepositoryContext},
    isolation::{
        BoundManagedExecution, ExecutionRole, IsolationPlan, PathGrant, ValidatedIsolation,
    },
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
use kegari_config::{Config, Workspace};
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;
#[path = "runtime_workers_evaluation_return.rs"]
mod evaluation_return;
pub(crate) use evaluation_return::{
    EvaluationPreparationReturn, EvaluationWorkerTransfer, PreparationReturned,
    TrialTransferFailure, prepare_evaluation_admitted,
};
#[path = "runtime_workers_resume.rs"]
mod session_resume;
pub(crate) use session_resume::prepare_paired_resumed;
pub use session_resume::prepare_resumed;

/// Selected by the host from the actual configured repository/provider phase.
/// The exact commit/tree is checked against local Git objects before cloning.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSelection {
    pub repository: String,
    pub revision: Revision,
}

/// Resolve immutable source identities from the selected provider. This reads no
/// operator checkout, fetches no objects and never treats a local branch as fresh.
/// Preparation must subsequently verify that those exact Git objects are present.
pub async fn resolve_source(
    runtime: &RuntimeCoordinator,
    authority: &WorkerAuthority,
) -> Result<Option<SourceSelection>> {
    authority.compare(&runtime.current_authority(authority)?)?;
    let config = runtime.configuration.snapshot()?;
    let workspace_id = match authority {
        WorkerAuthority::Authorized(gate) => gate.workspace_id(),
        WorkerAuthority::Independent(gate) => gate.workspace_id(),
    };
    let repository_id = match authority {
        WorkerAuthority::Authorized(gate) => gate
            .child_scope()
            .and_then(|scope| scope.write_repository.clone()),
        WorkerAuthority::Independent(gate) => Some(gate.repository_id().to_string()),
    };
    let Some(repository_id) = repository_id else {
        return Ok(None);
    };
    let workspace = config
        .workspaces
        .values()
        .find(|workspace| workspace.id == workspace_id)
        .ok_or_else(|| Error::new("scope_violation", "source workspace is not configured"))?;
    let configured = workspace.repositories.get(&repository_id).ok_or_else(|| {
        Error::new(
            "source_unavailable",
            "source repository is not configured for owned preparation",
        )
    })?;
    let number = repository_id
        .parse::<u64>()
        .map_err(|_| Error::invalid("source requires canonical repository identity"))?;
    let projection = runtime.observations.admission_snapshot()?;
    let repository = &projection
        .workspaces
        .get(&workspace_id)
        .and_then(|workspace| workspace.repositories.get(&number))
        .ok_or_else(|| {
            Error::new(
                "incompletely_observed",
                "source repository lacks complete current provider identity",
            )
        })?
        .identity;
    let github = providers(runtime, &config, workspace_id)?
        .github
        .ok_or_else(|| Error::new("missing_secret", "source provider is unavailable"))?;
    let bound = std::time::Duration::from_millis(config.runtime.request_timeout_ms)
        .min(std::time::Duration::from_secs(30));
    let target = match authority {
        WorkerAuthority::Authorized(_) => ProviderSource::Staging(&configured.staging),
        WorkerAuthority::Independent(gate) => ProviderSource::Review(gate),
    };
    let result = tokio::time::timeout(bound, provider_revision(&github, repository, target))
        .await
        .map_err(|_| {
            Error::new(
                "timeout",
                "source identity resolution exceeded its finite bound",
            )
        })??;
    if crate::config::config_revision(&runtime.configuration.snapshot()?)?
        != crate::config::config_revision(&config)?
    {
        return Err(Error::new(
            "stale_config",
            "source configuration changed during provider observation",
        ));
    }
    authority.compare(&runtime.current_authority(authority)?)?;
    Ok(Some(SourceSelection {
        repository: repository_id,
        revision: result,
    }))
}

enum ProviderSource<'a> {
    Staging(&'a str),
    Review(&'a crate::reconcile::IndependentReviewGate),
}
async fn provider_revision(
    github: &crate::providers::GithubClient,
    repository: &crate::providers::RepositoryIdentity,
    target: ProviderSource<'_>,
) -> Result<Revision> {
    let commit = match &target {
        ProviderSource::Staging(branch) => {
            github
                .reference(repository, &format!("heads/{branch}"))
                .await?
        }
        ProviderSource::Review(expected) => {
            require_current_review(
                &github
                    .independent_review_pull_request(repository, expected.pull_request().number)
                    .await?,
                expected,
            )?;
            expected.pull_request().head_sha.clone()
        }
    };
    let observed = github.commit(repository, &commit).await?;
    let revision = Revision {
        commit,
        tree: observed
            .pointer("/tree/sha")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new("provider_response", "source commit tree is missing"))?
            .into(),
    };
    revision.validate()?;
    match target {
        ProviderSource::Staging(branch)
            if github
                .reference(repository, &format!("heads/{branch}"))
                .await?
                != revision.commit =>
        {
            return Err(Error::new(
                "stale_revision",
                "configured staging moved during source resolution",
            ));
        }
        ProviderSource::Review(expected) => require_current_review(
            &github
                .independent_review_pull_request(repository, expected.pull_request().number)
                .await?,
            expected,
        )?,
        _ => {}
    }
    Ok(revision)
}

fn require_current_review(
    actual: &crate::providers::IndependentPullRequest,
    expected: &crate::reconcile::IndependentReviewGate,
) -> Result<()> {
    actual.text.validate()?;
    if &actual.text != expected.pull_request_text() {
        return Err(Error::new(
            "stale_revision",
            "independent review task text changed during source resolution",
        ));
    }
    let actual = &actual.pull_request;
    let expected = expected.pull_request();
    if actual.repository_id != expected.repository_id
        || actual.id != expected.id
        || actual.number != expected.number
        || actual.head_sha != expected.head_sha
        || actual.base_sha != expected.base_sha
        || actual.head_repository_id != expected.head_repository_id
        || actual.head_ref != expected.head_ref
        || actual.base_ref != expected.base_ref
        || actual.author_id != expected.author_id
        || actual.state != "open"
        || actual.draft
        || actual.merged == Some(true)
    {
        return Err(Error::new(
            "stale_revision",
            "independent review source changed during resolution",
        ));
    }
    Ok(())
}
#[derive(Clone)]
pub struct LaunchRequest {
    pub lease: Lease,
    pub operation_id: Uuid,
    pub authority: WorkerAuthority,
    pub phase: PhaseKey,
    pub source: Option<SourceSelection>,
    /// Opaque delivery assignment; cannot be constructed from worker/CLI JSON.
    pub delivery: Option<crate::delivery::DeliveryWorkerAssignment>,
    /// Exact host-verified remaining obligation; worker/CLI text cannot mint it.
    pub unfinished: Option<Arc<crate::delivery::UnfinishedChildWork>>,
    /// Actual repaired source authorizing a repeated required check/review phase.
    pub source_advanced: Option<Arc<crate::delivery::VerifiedSourceAdvanced>>,
    /// A current native operator grant for the retained exact unfinished task.
    pub retried_unfinished: Option<Arc<crate::delivery::RetriedUnfinishedWork>>,
    pub review_retry: Option<Arc<crate::delivery::ReviewRetryWork>>,
    /// Exact failed-validation source/session and current native retry grant.
    pub validation_retry: Option<Arc<crate::delivery::ValidationRetryWork>>,
    /// Exact review/fix round, orthogonal to its existing readmission cause.
    pub review_round: Option<Arc<crate::delivery::VerifiedReviewRoundPlan>>,
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
            || [
                self.unfinished.is_some(),
                self.source_advanced.is_some(),
                self.retried_unfinished.is_some(),
                self.review_retry.is_some(),
                self.validation_retry.is_some(),
            ]
            .into_iter()
            .filter(|present| *present)
            .count()
                > 1
        {
            return Err(Error::new(
                "worker_binding",
                "worker request must bind canonical work, exact generation and finite bounds",
            ));
        }
        if let Some(assignment) = self.delivery.as_ref()
            && let Some(proof) = assignment.parent_reassembly()
        {
            if self.review_round.is_some()
                || self.unfinished.is_some()
                || self.source_advanced.is_some()
                || self.retried_unfinished.is_some()
                || self.review_retry.is_some()
                || self.validation_retry.is_some()
                || proof.reference().operation != self.operation_id
                || proof.phase() != &self.phase
            {
                return Err(Error::new(
                    "worker_binding",
                    "parent reassembly cannot replace or mix dispatch causes",
                ));
            }
            proof.verify_assignment(assignment)?;
            WorkerAuthority::Authorized(proof.authority().clone()).compare(&self.authority)?;
        }
        if let Some(source) = &self.source {
            source.revision.validate()?;
        }
        if self.review_round.is_none()
            && (self
                .unfinished
                .as_ref()
                .is_some_and(|proof| proof.requires_review_round())
                || self
                    .source_advanced
                    .as_ref()
                    .is_some_and(|proof| proof.requires_review_round())
                || self
                    .review_retry
                    .as_ref()
                    .is_some_and(|proof| proof.requires_review_round()))
        {
            return Err(Error::new(
                "worker_binding",
                "common review lineage cannot omit its opaque round plan",
            ));
        }
        if let Some(round) = &self.review_round {
            let assignment = self.delivery.as_ref().ok_or_else(|| {
                Error::new(
                    "worker_binding",
                    "review round requires its actual delivery assignment",
                )
            })?;
            if round.subject_phase() != &self.phase
                || self.retried_unfinished.is_some()
                || self.validation_retry.is_some()
            {
                return Err(Error::new(
                    "worker_binding",
                    "review round changes its exact subject phase",
                ));
            }
            round.verify_assignment(assignment)?;
            WorkerAuthority::Authorized(round.authority().clone()).compare(&self.authority)?;
            use crate::control::ReviewRoundCause;
            let compatible = match &round.reference().cause {
                ReviewRoundCause::Initial {} | ReviewRoundCause::TransientRetry { .. } => {
                    self.unfinished.is_none() && self.source_advanced.is_none()
                }
                ReviewRoundCause::PairedRepair { .. } => self.unfinished.is_some(),
                ReviewRoundCause::RepairContinuation {
                    current_source,
                    validation,
                    ..
                } => self.unfinished.as_ref().is_some_and(|unfinished| {
                    unfinished.predecessor() == current_source.implementation.operation
                        && unfinished.predecessor_receipt_digest()
                            == current_source.implementation.receipt_digest
                        && unfinished.failure_operation() == validation.validation.operation
                }),
                ReviewRoundCause::AfterRepair { .. } => self.source_advanced.is_some(),
            };
            if !compatible {
                return Err(Error::new(
                    "worker_binding",
                    "review round differs from retained source cause",
                ));
            }
        }
        if let Some(proof) = &self.unfinished {
            let assignment = self.delivery.as_ref().ok_or_else(|| {
                Error::new(
                    "worker_binding",
                    "unfinished work requires its sealed repair assignment",
                )
            })?;
            if proof.phase() != &self.phase || proof.predecessor() == self.operation_id {
                return Err(Error::new(
                    "worker_binding",
                    "repair cannot replace its original phase or reuse the predecessor operation",
                ));
            }
            proof.verify_assignment(assignment)?;
            WorkerAuthority::Authorized(proof.authority().clone()).compare(&self.authority)?;
        }
        if let Some(proof) = &self.source_advanced {
            let assignment = self.delivery.as_ref().ok_or_else(|| {
                Error::new(
                    "worker_binding",
                    "advanced source requires its exact delivery assignment",
                )
            })?;
            if proof.phase() != &self.phase
                || proof.predecessor() == self.operation_id
                || proof.source_operation() == self.operation_id
            {
                return Err(Error::new(
                    "worker_binding",
                    "advanced source cannot replace its phase or reuse an operation",
                ));
            }
            proof.verify_assignment(assignment)?;
            WorkerAuthority::Authorized(proof.authority().clone()).compare(&self.authority)?;
        }
        if let Some(proof) = &self.retried_unfinished {
            let assignment = self.delivery.as_ref().ok_or_else(|| {
                Error::new(
                    "worker_binding",
                    "explicit retry requires its sealed repair assignment",
                )
            })?;
            if proof.phase() != &self.phase
                || self.phase.retry_grant != Some(proof.grant().record().operation_id)
                || proof.original().predecessor() == self.operation_id
                || proof.original().failure_operation() == self.operation_id
            {
                return Err(Error::new(
                    "worker_binding",
                    "explicit retry differs from its actual granted phase",
                ));
            }
            proof.verify_assignment(assignment)?;
            WorkerAuthority::Authorized(proof.authority().clone()).compare(&self.authority)?;
        }
        if let Some(proof) = &self.review_retry {
            let assignment = self.delivery.as_ref().ok_or_else(|| {
                Error::new("worker_binding", "review retry requires exact assignment")
            })?;
            if proof.phase() != &self.phase || proof.predecessor() == self.operation_id {
                return Err(Error::new(
                    "worker_binding",
                    "review retry differs from actual granted phase",
                ));
            }
            proof.verify_assignment(assignment)?;
            WorkerAuthority::Authorized(proof.authority().clone()).compare(&self.authority)?;
        }
        if let Some(proof) = &self.validation_retry {
            let assignment = self.delivery.as_ref().ok_or_else(|| {
                Error::new(
                    "worker_binding",
                    "validation retry requires its exact sealed assignment",
                )
            })?;
            if self.review_round.is_some()
                || proof.phase() != &self.phase
                || self.phase.phase != "child_local_validation"
                || self.phase.retry_grant != Some(proof.grant().record().operation_id)
                || proof.predecessor() == self.operation_id
            {
                return Err(Error::new(
                    "worker_binding",
                    "validation retry changes its exact granted phase or round lineage",
                ));
            }
            proof.verify_assignment(assignment)?;
            WorkerAuthority::Authorized(proof.authority().clone()).compare(&self.authority)?;
        }
        if let Some(assignment) = &self.delivery {
            let WorkerAuthority::Authorized(gate) = &self.authority else {
                return Err(Error::new(
                    "worker_binding",
                    "delivery assignment cannot grant independent authority",
                ));
            };
            assignment.verify_binding(gate, &self.phase.phase)?;
            if let Some(selected) = &self.source
                && !assignment.source().is_some_and(|source| {
                    source.repository().id == selected.repository
                        && source.revision() == &selected.revision
                })
            {
                return Err(Error::new(
                    "worker_binding",
                    "caller source differs from sealed delivery assignment",
                ));
            }
            let selected = assignment.source().map(|source| SourceSelection {
                repository: source.repository().id.clone(),
                revision: source.revision().clone(),
            });
            validate_selection(&self.authority, selected.as_ref())
        } else {
            validate_selection(&self.authority, self.source.as_ref())
        }
    }
}
struct Plan {
    canary_operand: Option<Arc<crate::evaluation::OwnedCanaryPathOperand>>,
    // Preparation binds the installed backend definition before the owned
    // repository/scratch exist. Their actual domain is constructed afterward.
    command_policy_digest: Option<String>,
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
    processes: Arc<crate::process_records::ProcessJournal>,
    run: worker_run::PreparedRun,
    execution: Option<Arc<WorkerExecution>>,
    records: PrivateDir,
}

/// Stable linkage between a launch intent and its preparation effects. Presence
/// of this receipt prohibits replaying preparation under a replacement identity.
pub(crate) fn preparation_operation(worker: Uuid) -> Result<Uuid> {
    crate::control::preparation_operation(worker)
}
/// The caller retains its scheduler capacity claim throughout this future. A
/// cancellation cannot be mistaken for completion: preparation handles remain in
/// the shared owned-process registry and Sent receipts keep the canonical hold.
pub async fn launch(runtime: Arc<RuntimeCoordinator>, request: LaunchRequest) -> Result<Uuid> {
    prepare_admitted(runtime, request).await?.run()
}
/// Perform actual owned preparation, immutable assembly and atomic admission.
/// The returned capability can enter production or an explicitly authorized
/// evaluation; neither path reconstructs a permit or substitutes a tool broker.
pub async fn prepare_admitted(
    runtime: Arc<RuntimeCoordinator>,
    request: LaunchRequest,
) -> Result<AdmittedWorker> {
    prepare_admitted_inner(runtime, request, None, None).await
}
async fn prepare_admitted_inner(
    runtime: Arc<RuntimeCoordinator>,
    request: LaunchRequest,
    mut trace: Option<&mut evaluation_return::PreparationTrace>,
    operand: Option<Arc<crate::evaluation::OwnedCanaryPathOperand>>,
) -> Result<AdmittedWorker> {
    let mut plan = Plan::read(&runtime, request)?;
    runtime.verify_evaluation_context_operand(&plan.request, operand.as_ref())?;
    if let Some(operand) = &operand {
        operand.augment_context(&mut plan.context)?;
    }
    plan.canary_operand = operand;
    if let Some(proof) = plan
        .request
        .delivery
        .as_ref()
        .and_then(|a| a.parent_reassembly())
    {
        proof
            .refresh_current(&runtime.configuration.snapshot()?)
            .await?;
    }
    let initial_schedule = plan.schedule(&runtime)?;
    runtime.check_worker_capacity(
        plan.workspace.id,
        &plan.account,
        &plan.request.phase,
        &plan.normalized_state,
    )?;
    if initial_schedule.not_before > chrono::Utc::now() {
        return Err(Error::new(
            "operation_pending",
            "worker retry spacing has not elapsed",
        ));
    }
    if plan.request.delivery.is_none() {
        plan.context["attempt"] = json!(initial_schedule.attempt);
    }
    let _retained = runtime.observations.capture_active(
        plan.workspace.id,
        &crate::config::config_revision(&plan.config)?,
        &plan.workflow.digest,
    )?;
    let prep_id = preparation_operation(plan.request.operation_id)?;
    let mut prep_binding = json!({"kind":"worker_preparation","operation":plan.request.operation_id,"authority":plan.request.authority.digest()?,"phase":plan.request.phase,"source":plan.source,"role":plan.role,"assignment":plan.request.delivery.as_ref().map(|assignment|assignment.digest()).transpose()?});
    if let Some(round) = &plan.request.review_round {
        prep_binding["review_round"] = json!(round.digest());
    }
    if let Some(policy_digest) = &plan.command_policy_digest {
        prep_binding["managed_execution"] = json!({
            "schema_version": 5,
            "policy_digest": policy_digest,
        });
    }
    let prep_scope = canonical_digest(1, &prep_binding)?;
    let prep_admission = PreparationAdmission {
        request_id: prep_id,
        work_key: plan.request.authority.work_key()?,
        scope_digest: prep_scope,
        reference: PreparationReference {
            worker_operation: plan.request.operation_id,
            workspace: plan.workspace.id,
            account: plan.account.clone(),
            phase: plan.request.phase.clone(),
        },
    };
    let prep_gate = |snapshot: &crate::control::ControlSnapshot| {
        plan.revalidate_command_policy()?;
        runtime.check_evaluation_context_operand(
            snapshot,
            &plan.request,
            plan.canary_operand.as_ref(),
        )?;
        let current = runtime.plan_worker_preparation(
            &plan.request,
            plan.workspace.id,
            &plan.account,
            snapshot,
        )?;
        require_preparation_schedule(&initial_schedule, &current, plan.request.max_turns)
    };
    let prep_reference = prep_admission.reference.clone();
    let prep_permit = if let Some(proof) = plan
        .request
        .delivery
        .as_ref()
        .and_then(|a| a.parent_reassembly())
    {
        runtime
            .control
            .admit_parent_reassembly_preparation_observed(
                &plan.request.lease,
                prep_admission,
                proof.reference().clone(),
                prep_gate,
            )?
    } else if let Some(round) = &plan.request.review_round {
        runtime.control.admit_round_preparation_observed(
            &plan.request.lease,
            prep_admission,
            round.reference().clone(),
            prep_gate,
        )?
    } else {
        runtime.control.admit_preparation_observed(
            &plan.request.lease,
            prep_admission,
            prep_gate,
        )?
    };
    if let Some(trace) = trace.as_deref_mut() {
        trace.preparation(&prep_permit, prep_reference, &runtime.control)?;
    }
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
    let prep_observation = match &prepared {
        Ok(_) => json!({"phase":"worker_preparation","result":"prepared"}),
        Err(error) => json!({"phase":"worker_preparation","result":"blocked","error":error}),
    };
    if let Some(trace) = trace.as_deref_mut() {
        trace.preparation_retiring(prep_observation.clone());
    }
    let prep_closed = runtime.control.retire_local(prep_id, prep_observation)?;
    if let Some(trace) = trace.as_deref_mut() {
        trace.preparation_closed(prep_closed);
    }
    let mut prepared = prepared?;
    if let Some(proof) = plan
        .request
        .delivery
        .as_ref()
        .and_then(|a| a.parent_reassembly())
    {
        proof
            .refresh_current(&runtime.configuration.snapshot()?)
            .await?;
    }
    let schedule = plan.schedule(&runtime)?;
    if plan.request.max_turns > schedule.max_turns {
        return Err(Error::new(
            "turns_exhausted",
            "requested run exceeds the original durable phase turn bound",
        ));
    }
    runtime.verify_evaluation_context_operand(&plan.request, plan.canary_operand.as_ref())?;
    let contract = prepared.run.contract().clone();
    let admission = RunAdmission {
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
    };
    let admitted_budget = admission.reference.clone();
    let permit = if plan
        .request
        .delivery
        .as_ref()
        .is_some_and(|a| a.parent_reassembly().is_some())
    {
        let preparation = runtime.control.operation(prep_id)?;
        runtime.admit_parent_reassembly_worker(admission, &contract, &plan.request, &preparation)?
    } else if plan.request.review_round.is_some() {
        // Read the actual closed canonical preparation; Control checks its whole
        // digest again atomically with the model admission and current proof.
        let preparation = runtime.control.operation(prep_id)?;
        runtime.admit_review_round_worker(admission, &contract, &plan.request, &preparation)?
    } else {
        match (
            &plan.request.unfinished,
            &plan.request.source_advanced,
            &plan.request.retried_unfinished,
            &plan.request.review_retry,
            &plan.request.validation_retry,
        ) {
            (Some(proof), None, None, None, None) => {
                runtime.admit_unfinished_worker(&plan.request.lease, admission, &contract, proof)?
            }
            (None, Some(proof), None, None, None) => runtime.admit_source_advanced_worker(
                &plan.request.lease,
                admission,
                &contract,
                proof,
            )?,
            (None, None, Some(proof), None, None) => runtime.admit_retried_unfinished_worker(
                &plan.request.lease,
                admission,
                &contract,
                proof,
            )?,
            (None, None, None, Some(proof), None) => runtime.admit_review_retry_worker(
                &plan.request.lease,
                admission,
                &contract,
                proof,
            )?,
            (None, None, None, None, Some(proof)) => runtime.admit_validation_retry_worker(
                &plan.request.lease,
                admission,
                &contract,
                proof,
            )?,
            (None, None, None, None, None) => {
                runtime.admit_worker(&plan.request.lease, admission, &contract)?
            }
            _ => {
                return Err(Error::new(
                    "worker_binding",
                    "conflicting readmission causes",
                ));
            }
        }
    };
    if let Some(trace) = trace.as_deref_mut() {
        trace.admitted(&permit, &contract, admitted_budget, &runtime.control)?;
    }
    let start: Result<AdmittedWorker> = (|| {
        if let Some(assignment) = &plan.request.delivery
            && matches!(
                assignment.phase(),
                "parent_integrated_validation"
                    | "parent_integrated_review"
                    | "parent_evidence_review"
            )
        {
            let preparation = runtime.control.operation(prep_id)?;
            runtime.register_parent_run(
                permit.clone(),
                contract.clone(),
                crate::runtime::ParentRunRegistrationInput {
                    assignment,
                    configuration: &plan.config,
                    preparation: &preparation,
                    records: &prepared.records,
                },
            )?;
        } else {
            runtime.register_run(permit.clone(), contract.clone())?;
        }
        if let Some(round) = &plan.request.review_round {
            runtime.register_review_round(&permit, &contract, round.clone())?;
        }
        // Preparation and model execution have distinct permits. Reusing an
        // owned checkout never transfers the preparation process authority.
        prepared.execution = prepared
            .execution
            .as_ref()
            .map(|execution| {
                Ok(Arc::new(WorkerExecution {
                    git: Arc::new(execution.git.with_execution_scope(GitExecutionScope {
                        limits: GitLimits::default(),
                        start_gate: scoped_git_gate(
                            runtime.clone(),
                            permit.clone(),
                            plan.request.authority.clone(),
                        ),
                        processes: prepared.processes.clone(),
                        permit: permit.clone(),
                        executions: runtime.executions.clone(),
                    })?),
                    git_executable: execution.git_executable.clone(),
                    boundary: Arc::new(JournaledSandboxBoundary::new(
                        prepared.processes.clone(),
                        permit.clone(),
                    )),
                    ownership: execution.ownership.clone(),
                    binding: execution.binding.clone(),
                    limits: execution.limits.clone(),
                    check_outputs: execution.check_outputs.clone(),
                }))
            })
            .transpose()?;
        let hooks: Arc<dyn RunHooks> = if let Some(execution) = &prepared.execution
            && contract
                .instructions()
                .manifest
                .permissions
                .repository_code_execution
        {
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
        let services = RunServices {
            processes: prepared.processes,
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
        };
        let options = RunOptions {
            initial_input: format!(
                "Perform the admitted {} phase using the supplied current task context and tools. Submit an honest scoped outcome proposal with observed evidence and blockers.",
                plan.request.phase.phase
            ),
            max_seconds: plan.request.max_seconds,
            max_turns: plan.request.max_turns,
            resume: None,
        };
        Ok(AdmittedWorker {
            runtime: runtime.clone(),
            permit: permit.clone(),
            input: Some((prepared.run, services, options)),
            transferred: false,
        })
    })();
    if let Err(error) = &start {
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
            let observation = json!({"outcome":"failed","error":error,"phase":"worker_start"});
            if let Some(trace) = trace.as_deref_mut() {
                trace.model_retiring(observation.clone());
            }
            let closed = runtime
                .control
                .retire_local(permit.operation_id, observation)?;
            if let Some(trace) = trace {
                trace.model_closed(closed);
            }
        }
        return Err(error.clone());
    }
    start
}

/// One concrete registered admission, consumed once. Abandonment fences before
/// cancelling owned handles and never clears uncertain native or remote effects.
pub struct AdmittedWorker {
    runtime: Arc<RuntimeCoordinator>,
    permit: Permit,
    input: Option<(worker_run::PreparedRun, RunServices, RunOptions)>,
    transferred: bool,
}
impl AdmittedWorker {
    pub fn operation_id(&self) -> Uuid {
        self.permit.operation_id
    }
    pub fn run(mut self) -> Result<Uuid> {
        let (prepared, services, options) = self
            .input
            .take()
            .ok_or_else(|| Error::new("ownership_conflict", "admitted worker already consumed"))?;
        self.runtime.ordinary_worker_transfer(
            self.permit.operation_id,
            prepared.contract().instructions().manifest.workspace_id,
        )?;
        let run = worker_run::run(prepared, self.permit.clone(), services, options)?;
        self.runtime.track_run(run)?;
        self.transferred = true;
        Ok(self.permit.operation_id)
    }
    pub fn into_trial(mut self) -> Result<worker_run::AdmittedTrial> {
        let (prepared, services, options) = self
            .input
            .take()
            .ok_or_else(|| Error::new("ownership_conflict", "admitted worker already consumed"))?;
        self.runtime
            .evaluation_trial_transfer(&self.permit, prepared.contract(), &options)?;
        let trial =
            worker_run::AdmittedTrial::new(prepared, self.permit.clone(), services, options)?;
        self.transferred = true;
        Ok(trial)
    }
}
impl Drop for AdmittedWorker {
    fn drop(&mut self) {
        if !self.transferred && self.runtime.control.fence(self.permit.operation_id).is_ok() {
            let _ = self
                .runtime
                .executions
                .cancel_fenced(&self.runtime.control, self.permit.operation_id);
            if self
                .runtime
                .executions
                .observe_exited(self.permit.operation_id)
                .is_ok()
                && self
                    .runtime
                    .executions
                    .active_operations()
                    .is_ok_and(|active| !active.contains(&self.permit.operation_id))
            {
                let _ = self.runtime.control.retire_local(
                    self.permit.operation_id,
                    json!({"outcome":"not_started","phase":"admitted_worker_abandoned"}),
                );
            }
        }
    }
}

fn fresh(runtime: &RuntimeCoordinator, request: &LaunchRequest) -> Result<()> {
    runtime.verify_owner(&request.lease)?;
    let current = runtime.current_authority(&request.authority)?;
    request.authority.compare(&current)?;
    if let Some(assignment) = &request.delivery {
        let WorkerAuthority::Authorized(gate) = &current else {
            return Err(Error::new(
                "worker_binding",
                "delivery authority changed class",
            ));
        };
        assignment.verify_binding(gate, &request.phase.phase)?;
    }
    Ok(())
}
fn require_preparation_schedule(
    expected: &crate::runtime_schedule::RunSchedule,
    current: &crate::runtime_schedule::RunSchedule,
    requested_turns: u32,
) -> Result<()> {
    if current.attempt != expected.attempt
        || current.max_attempts != expected.max_attempts
        || current.max_turns != expected.max_turns
        || current.max_review_rounds != expected.max_review_rounds
        || current.trigger != expected.trigger
        || requested_turns > current.max_turns
    {
        return Err(Error::new(
            "admission_budget",
            "preparation no longer binds its exact selected attempt, trigger and finite caps",
        ));
    }
    if current.not_before > chrono::Utc::now() {
        return Err(Error::new(
            "retry_wait",
            "current preparation trigger is not yet eligible",
        ));
    }
    Ok(())
}
impl Plan {
    fn revalidate_command_policy(&self) -> Result<()> {
        if let Some(expected) = &self.command_policy_digest
            && expected != &BoundManagedExecution::automatic_policy_digest()?
        {
            return Err(Error::new(
                "worker_binding",
                "installed command backend differs from the preparation binding",
            ));
        }
        Ok(())
    }
    fn schedule(
        &self,
        runtime: &RuntimeCoordinator,
    ) -> Result<crate::runtime_schedule::RunSchedule> {
        if self
            .request
            .delivery
            .as_ref()
            .is_some_and(|a| a.parent_reassembly().is_some())
        {
            return runtime.next_attempt_for_parent_reassembly(&self.request);
        }
        if self.request.review_round.is_some() {
            return runtime.next_attempt_for_review_round(&self.request);
        }
        match (
            &self.request.unfinished,
            &self.request.source_advanced,
            &self.request.retried_unfinished,
            &self.request.review_retry,
            &self.request.validation_retry,
        ) {
            (Some(proof), None, None, None, None) => runtime.next_attempt_for_unfinished(proof),
            (None, Some(proof), None, None, None) => {
                runtime.next_attempt_for_source_advanced(proof)
            }
            (None, None, Some(proof), None, None) => {
                runtime.next_attempt_for_retried_unfinished(proof)
            }
            (None, None, None, Some(proof), None) => runtime.next_attempt_for_review_retry(proof),
            (None, None, None, None, Some(_)) => {
                runtime.next_attempt_for_validation_retry(&self.request)
            }
            (None, None, None, None, None) => {
                runtime.next_attempt(&self.request.phase, self.workspace.id, &self.account)
            }
            _ => Err(Error::new(
                "worker_binding",
                "conflicting readmission causes",
            )),
        }
    }
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
        let workflow = crate::config::load_workflow(&workspace)?;
        let projection = runtime.observations.admission_snapshot()?;
        let (mut role, mut context, review, generation, normalized_state) = match &request.authority
        {
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
                    authorized_context(
                        parent,
                        gate,
                        &request.phase.phase,
                        request.operation_id,
                        &workspace.repositories.keys().cloned().collect::<Vec<_>>(),
                        scoped_evidence(runtime, workspace.id, &parent.key)?,
                    )?,
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
                    json!({"issue":{"pull_request":gate.context_pull_request(),"repository":repository},"parent":null,"children":[],"workspace":{"id":workspace.id,"name":projection.workspaces[&workspace.id].workspace_name},"scope":{"repository":repository.identity,"head":gate.pull_request().head_sha,"base":gate.pull_request().base_sha,"source_writes":false},"dependencies":[],"corrections":[],"evidence":{},"attempt":null,"phase":request.phase.phase,"profile":gate.profile(),"operation":{"id":request.operation_id}}),
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
        let source = if let Some(assignment) = &request.delivery {
            role = assignment.role();
            context = assignment.bind_context(context, request.operation_id)?;
            assignment.source().map(|source| SourceSelection {
                repository: source.repository().id.clone(),
                revision: source.revision().clone(),
            })
        } else {
            request.source.clone()
        };
        validate_selection(&request.authority, source.as_ref())?;
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
                    if let Some(assigned) = request
                        .delivery
                        .as_ref()
                        .and_then(|assignment| assignment.source())
                    {
                        if assigned.repository().hostname != repository.hostname
                            || assigned.repository().id != source.repository
                        {
                            return Err(Error::new(
                                "worker_binding",
                                "delivery source repository identity differs from current configuration",
                            ));
                        }
                        assigned.revalidate_filesystem()?;
                        assigned.root().to_path_buf()
                    } else {
                        Path::new(&repository.source)
                            .canonicalize()
                            .map_err(|error| Error::io("resolve configured source", error))?
                    },
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
        let command_policy_digest =
            if role.permits_repository_execution() && repository_id.is_some() {
                Some(BoundManagedExecution::automatic_policy_digest()?)
            } else {
                None
            };
        Ok(Self {
            canary_operand: None,
            command_policy_digest,
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
        fresh_plan_preparation(runtime, plan)?;
        let root = PrivateDir::create(Path::new(&plan.config.runtime.execution_root))?.child(&plan.request.operation_id.to_string())?;
        let records = runtime.registration.root().child("worker-records")?.child(&plan.request.operation_id.to_string())?;
        if std::fs::read_dir(records.path()).map_err(|error|Error::io("inspect preparation directory",error))?.next().is_some() {
            return Err(Error::new("reconciliation_required", "worker preparation already has retained records; preserve it rather than recreate"));
        }
        records.write("preparation.json", &json!({"runtime":plan.config.runtime.id,"operation":plan.request.operation_id,"preparation":permit.operation_id,"authority":plan.request.authority.digest()?,"work":permit.work_key,"source":plan.source,"phase":plan.request.phase,"role":plan.role,"root":root.path()}))?;
        let scratch = root.child("scratch")?.path().to_path_buf();
        let processes = Arc::new(crate::process_records::ProcessJournal::new(runtime.registration.root().child("worker-processes")?, plan.config.runtime.id)?);
        directories = Some((root, records, scratch, processes));
        Ok(())
    })?;
    let (root, records, scratch, processes) = directories.ok_or_else(|| {
        Error::new(
            "ownership_conflict",
            "preparation directory gate did not run",
        )
    })?;
    let repository = root.path().join("repository");
    let git_path = plan
        .source
        .as_ref()
        .map(|_| executable("git"))
        .transpose()?;
    let mut git = git_path
        .as_ref()
        .map(Git::from_pinned_executable)
        .transpose()?
        .map(Arc::new);
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
    let mut independent_proof = None;
    let mut read_clones = Vec::new();
    if let (Some(source), Some(source_path)) = (&plan.source, &plan.source_path) {
        let start_gate = scoped_git_gate(
            runtime.clone(),
            permit.clone(),
            plan.request.authority.clone(),
        );
        git = Some(Arc::new(
            git.as_ref()
                .ok_or_else(|| Error::new("source_unavailable", "source Git adapter is missing"))?
                .with_execution_scope(GitExecutionScope {
                    limits: GitLimits::default(),
                    start_gate: start_gate.clone(),
                    processes: processes.clone(),
                    permit: permit.clone(),
                    executions: runtime.executions.clone(),
                })?,
        ));
        let acquired = if plan
            .request
            .delivery
            .as_ref()
            .and_then(|assignment| assignment.source())
            .is_none()
        {
            Some(Arc::new(
                acquire_worker_source(
                    runtime,
                    plan,
                    permit,
                    git_path.as_ref().ok_or_else(|| {
                        Error::new("source_unavailable", "source Git path is missing")
                    })?,
                    processes.clone(),
                    &records,
                )
                .await?,
            ))
        } else {
            None
        };
        let owned_source = acquired
            .as_ref()
            .map_or_else(|| source_path.clone(), |source| source.root().to_path_buf());
        let operator_checkout = source_path.clone();
        if let Some(source) = &acquired {
            records.write("acquired-source.json",&json!({"source_digest":source.digest()?,"repository":source.repository(),"revision":source.revision(),"operator_checkout":operator_checkout}))?;
        }
        let root_path = root.path().to_path_buf();
        let revision = source.revision.clone();
        let owned_identity = identity.clone();
        let delivery = plan.request.delivery.clone();
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
            || fresh_plan_preparation(runtime, plan),
            |receipt| {
                let git = git.as_ref().ok_or_else(|| Error::new("source_unavailable", "source Git adapter is missing"))?;
                let reservation = runtime.executions.reserve(permit.operation_id, receipt.operation_id)?;
                let running = git.start_journaled_operation(
                    GitLimits::default(),
                    start_gate,
                    processes.clone(),
                    permit.clone(),
                    move |git| {
                        if let Some(source)=delivery.as_ref().and_then(|assignment|assignment.source()) {source.verify_for_clone(&git)?;}
                        if let Some(source)=&acquired { source.verify_for_clone(&git)?; }
                        let mut cloned_reads=Vec::new();
                        if let Some(assignment)=&delivery {
                            if assignment.read_sources().len()>64 {return Err(Error::new("record_limit","delivery assignment read source inventory exceeds limit"));}
                            for (index,source) in assignment.read_sources().iter().enumerate() {
                                source.verify_for_clone(&git)?;
                                let clone_root=root_path.join(format!("read-source-{index}"));
                                let clone_identity=DirectoryIdentity {schema_version:1,runtime_id,directory_id:Uuid::new_v4(),repository:source.repository().clone(),work:assignment.authority().work().clone(),repository_path:clone_root.join("repository"),operator_checkout:source.root().to_path_buf()};
                                let path=git.prepare_execution_clone(source.root(),&clone_root,&clone_identity,source.revision())?;
                                cloned_reads.push((source.repository().id.clone(),path));
                            }
                        }
                        let actual = git
                            .exact_revision(&owned_source, &revision.commit)
                            .map_err(source_error)?;
                        if actual != revision {
                            return Err(Error::new(
                                "source_unavailable",
                                "local Git objects do not match the provider-selected commit/tree",
                            ));
                        }
                        if let Some(identity) = owned_identity {
                            match &acquired {
                                Some(source)=>git.prepare_acquired_execution_clone(source,&root_path,&identity),
                                None=>git.prepare_execution_clone(&owned_source,&root_path,&identity,&revision),
                            }.map(|path| (path, None, cloned_reads))
                        } else if let Some((gate, repository_id, repository_path)) = independent {
                            let base = match &acquired {
                                Some(source)=>source.required_revision(&gate.pull_request().base_sha).ok_or_else(||Error::new("source_unavailable","acquired independent review lacks its exact admitted base object"))?,
                                None=>git.exact_revision(&owned_source,&gate.pull_request().base_sha).map_err(source_error)?,
                            };
                            let identity = IndependentCloneIdentity {
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
                                operator_checkout: operator_checkout.clone(),
                            };
                            let (path,proof) = match &acquired {
                                Some(source)=>(git.prepare_acquired_independent_review_clone(source,&root_path,&identity)?,git.observe_acquired_independent_review_clone(source,&root_path,&identity)?),
                                None=>(git.prepare_independent_review_clone(&owned_source,&root_path,&identity)?,git.observe_independent_review_clone(&root_path,&identity)?),
                            };
                            Ok((path, Some(proof), cloned_reads))
                        } else {
                            Err(Error::new(
                                "scope_violation",
                                "clone lacks an authorized owned identity",
                            ))
                        }
                    },
                )?;
                reservation.attach(Arc::new(running.cancellation()))?;
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
            || fresh_plan_preparation(runtime, plan),
            match &result {
                Ok((path, _, reads)) => {
                    json!({"clone":path,"revision":source.revision,"read_clones":reads})
                }
                Err(error) => json!({"error":error,"partial_files":"retained"}),
            },
        )?;
        let (_, proof, reads) = result?;
        independent_proof = proof;
        read_clones = reads;
    } else {
        runtime.control.validate_permit(permit, || {
            fresh_plan_preparation(runtime, plan)?;
            let substrate = root.child("repository")?;
            if std::fs::read_dir(substrate.path()).map_err(|error|Error::io("inspect coordination substrate",error))?.next().is_some() { return Err(Error::new("ownership_conflict", "coordination substrate is not an empty newly owned directory")); }
            records.write("coordination-substrate.json", &json!({"operation":plan.request.operation_id,"authority":plan.request.authority.digest()?,"path":substrate.path(),"source_writes":false,"repository_execution":false}))
        })?;
    }
    let mut grants = grants(plan, &repository)?;
    for (id, path) in read_clones {
        grants.reads.retain(|grant| !matches!(grants.resources.get(&grant.path),Some(ReadResource::Repository{id:prior}) if prior == &id));
        grants.resources.retain(
            |_, resource| !matches!(resource,ReadResource::Repository{id:prior} if prior == &id),
        );
        grants.reads.push(PathGrant {
            path: path.clone(),
            directory: true,
        });
        grants
            .resources
            .insert(path, ReadResource::Repository { id });
    }
    let mut forbidden = forbidden_paths(runtime.registration.root().path(), &root, &plan.config)?;
    // The actual privileged journals are explicitly inaccessible, including when
    // execution_root is a child of the broader private host-state directory.
    forbidden.push(records.path().to_path_buf());
    forbidden.sort();
    forbidden.dedup();
    let mut isolation = IsolationPlan {
        schema_version: 1,
        role: match plan.role {
            AgentRole::Coordination => ExecutionRole::Coordination,
            AgentRole::IndependentReview => ExecutionRole::IndependentReview,
            AgentRole::Review => ExecutionRole::Review,
            AgentRole::Validation => ExecutionRole::Validation,
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
        managed_execution: None,
    };
    if let Some(expected) = &plan.command_policy_digest {
        // Actual roots and admitted resources now exist. This bounded capture
        // stays outside Control; later effect gates recheck retained identities.
        let domain = BoundManagedExecution::automatic(&isolation)?;
        if domain.policy_digest() != expected {
            return Err(Error::new(
                "worker_binding",
                "actual command domain differs from the admitted backend policy",
            ));
        }
        isolation.managed_execution = Some(domain);
    }
    let boundary = isolation.validate()?;
    if let Some(proof) = &independent_proof {
        records.write(
            "independent-clone-observation.json",
            &json!({"identity":proof.identity(),"digest":proof.digest()?}),
        )?;
    }
    if identity.is_none() {
        let read_only = match &plan.request.authority {
            WorkerAuthority::Authorized(gate) if gate.work().child.is_none() => {
                ReadOnlyIdentity::Coordination {
                    workspace: gate.workspace_id(),
                    parent: crate::authority::IssueKey::new(
                        &gate.work().linear_endpoint,
                        gate.work().linear_workspace,
                        gate.work().parent,
                    )?,
                }
            }
            WorkerAuthority::Authorized(gate) => ReadOnlyIdentity::ChildReasoning {
                workspace: gate.workspace_id(),
                work: gate.work().clone(),
            },
            WorkerAuthority::Independent(_) => ReadOnlyIdentity::IndependentReview(
                independent_proof
                    .as_ref()
                    .ok_or_else(|| {
                        Error::new(
                            "execution_ownership",
                            "review clone lacks its observed ownership capability",
                        )
                    })?
                    .identity()
                    .clone(),
            ),
        };
        let preserved = runtime
            .services
            .housekeeping
            .register_readonly_preparation(ReadOnlyPreparation {
                preparation: permit,
                operation: plan.request.operation_id,
                authority: &plan.request.authority,
                identity: read_only,
                independent_clone: independent_proof.as_ref(),
                isolation: &boundary,
                workflow: &plan.workflow,
            })?;
        records.write("preserved-preparation.json", &json!({"operation":preserved.operation_id(),"digest":preserved.digest(),"cleanup_authorized":false}))?;
    }
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
            candidate_vector_digest: plan
                .request
                .delivery
                .as_ref()
                .and_then(|assignment| assignment.candidate_vector_digest())
                .map(str::to_owned),
            isolation_digest: boundary.digest().into(),
        };
        runtime.services.housekeeping.register_execution(
            &binding,
            &boundary,
            &owned,
            &plan.workflow,
        )?;
        let execution = Arc::new(WorkerExecution {
            git: git.ok_or_else(|| Error::invalid("execution source lacks Git"))?,
            git_executable: git_path
                .ok_or_else(|| Error::invalid("execution source lacks pinned Git path"))?,
            boundary: Arc::new(JournaledSandboxBoundary::new(
                processes.clone(),
                permit.clone(),
            )),
            ownership: owned,
            binding,
            limits: ExecutionLimits::default(),
            check_outputs: BTreeMap::new(),
        });
        let hook_records = records.child("hooks")?;
        if plan.role.permits_repository_execution() {
            after_create(runtime, plan, permit, &boundary, &execution, &hook_records).await?;
        }
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
    if let Some(operand) = &plan.canary_operand {
        operand.verify_isolation(&isolation.validate()?)?;
        runtime.verify_evaluation_context_operand(&plan.request, Some(operand))?;
    }
    let prepared = worker_run::prepare(Preparation {config:plan.config.clone(),workflow:plan.workflow.clone(),authority:plan.request.authority.clone(),operation_id:plan.request.operation_id,role:plan.role,phase:plan.request.phase.phase.clone(),context:plan.context.clone(),isolation,repository:plan.source.as_ref().map(|source|RepositoryContext {root:repository,revision:source.revision.commit.clone(),scope_paths}),repository_id:plan.repository_id.clone(),read_resources:grants.resources,review:plan.review.clone(),required_context:vec![],project_doc_max_bytes:32_768,provider_discovery:ProviderDiscovery::Unavailable {reason:"Pinned app-server does not expose an authenticated project-instruction discovery inventory; supplied accessible files are separately hashed".into()},known_secrets:vec![]})?;
    records.write("contract.json", &json!({"scope":prepared.contract().scope_digest(),"instructions":prepared.contract().instructions().manifest,"tool_contract":prepared.contract().tools().digest(),"isolation":prepared.isolation().plan()}))?;
    Ok(Prepared {
        processes,
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
        || fresh_plan_preparation(runtime, plan),
        |receipt| {
            executor.preflight_hook(&prepared)?;
            let reservation = runtime
                .executions
                .reserve(permit.operation_id, receipt.operation_id)?;
            let running = executor.start_hook(prepared)?;
            if let Some(cancellation) = running.cancellation() {
                reservation.attach(cancellation)?;
            } else {
                reservation.never_started();
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
        .register_hook_ledger_location(execution.binding.operation_id, records)?;
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
        || fresh_plan_preparation(runtime, plan),
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
            .filter(|_| plan.role.permits_source_writes())
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
            if plan.request.delivery.as_ref().is_some_and(|assignment| {
                assignment
                    .read_sources()
                    .iter()
                    .any(|source| &source.repository().id == id)
            }) {
                // The exact candidate is copied under this owned execution;
                // never require or grant an unrelated operator checkout here.
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
            if resource.writable
                && plan.repository_id.is_some()
                && plan.role.permits_repository_execution()
            {
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
/// Complete documented context envelope used by actual authorized worker
/// preparation. Native observations remain nested data alongside verified evidence.
pub fn authorized_context(
    parent: &crate::reconcile::ParentProjection,
    gate: &crate::reconcile::FreshGate,
    phase: &str,
    operation: Uuid,
    repositories: &[String],
    mut evidence: Value,
) -> Result<Value> {
    if operation.is_nil() || phase.is_empty() {
        return Err(Error::new(
            "worker_binding",
            "authorized context needs actual operation and phase",
        ));
    }
    let native = native_context(parent, gate)?;
    let issue = match gate.work().child {
        Some(child) => parent
            .children
            .get(&child)
            .and_then(|child| child.issue.as_ref()),
        None => parent.issue.as_ref(),
    }
    .ok_or_else(|| Error::new("incompletely_observed", "worker's actual issue disappeared"))?;
    let records = evidence
        .as_object_mut()
        .ok_or_else(|| Error::new("worker_binding", "scoped evidence must be an object"))?;
    if records.contains_key("native_context") {
        return Err(Error::new(
            "worker_binding",
            "scoped evidence already contains native context",
        ));
    }
    records.insert("native_context".into(), native);
    crate::delivery::public_model_context(
        &json!({"issue":issue,"parent":parent.issue,"children":parent.children,
        "scope":gate.child_scope(),"phase":phase,
        "workspace":{"id":gate.workspace_id(),"name":parent.workspace_name,"repositories":repositories},
        "dependencies":parent.prerequisites,"corrections":parent.corrections,
        "evidence":evidence,"attempt":null,"profile":gate.profile(),
        "operation":{"id":operation,"work":gate.work()}}),
    )
}

/// Native source material for the exact admitted parent/child; it grants no authority.
pub fn native_context(
    parent: &crate::reconcile::ParentProjection,
    gate: &crate::reconcile::FreshGate,
) -> Result<Value> {
    if parent.workspace_id != gate.workspace_id() || parent.key.issue != gate.work().parent {
        return Err(Error::new(
            "scope_violation",
            "native context belongs to another admitted parent",
        ));
    }
    let mut selected = std::collections::BTreeSet::from([parent.key.issue]);
    if let Some(child) = gate.work().child {
        selected.insert(child);
        if let Some(child) = parent.children.get(&child) {
            selected.extend(child.prerequisites.iter().copied());
        }
    } else {
        selected.extend(parent.required_children.iter().copied());
    }
    let mut pending: Vec<_> = selected.iter().copied().collect();
    while let Some(issue) = pending.pop() {
        if let Some(observation) = parent.observations.get(&issue) {
            for relation in &observation.dependencies {
                if relation.relation_type == "blocks"
                    && relation.related_issue_id == issue
                    && selected.insert(relation.issue_id)
                {
                    pending.push(relation.issue_id);
                }
            }
        }
    }
    let observations: BTreeMap<_, _> = parent
        .observations
        .iter()
        .filter(|(issue, _)| selected.contains(issue))
        .collect();
    let pending = parent.corrections.invalidations().pending;
    let mut comments = Vec::new();
    for (id, expected) in &pending {
        let found: Vec<_> = observations
            .values()
            .flat_map(|observation| &observation.comments)
            .filter(|comment| comment.id == *id)
            .collect();
        if found.is_empty() && gate.work().child.is_some() {
            continue;
        }
        let [comment] = found.as_slice() else {
            return Err(Error::new(
                "incompletely_observed",
                "pending correction lacks one complete native comment",
            ));
        };
        if comment.body_digest != expected.body_digest
            || comment.user_id != expected.author
            || comment.edited_at != expected.edited_at
            || crate::digest::bytes_digest(comment.body.as_bytes()) != comment.body_digest
            || !selected.contains(&comment.issue_id)
        {
            return Err(Error::new(
                "stale_correction",
                "native pending text differs from the admitted revision",
            ));
        }
        comments.push(*comment);
    }
    Ok(
        json!({"authority":"untrusted_native_source_material","observations":observations,
        "pending_correction_comments":comments}),
    )
}

fn scoped_evidence(
    runtime: &RuntimeCoordinator,
    workspace: Uuid,
    parent: &crate::authority::IssueKey,
) -> Result<Value> {
    let inspection = runtime.inspection()?;
    let delivery = inspection.delivery.get(parent);
    Ok(json!({
        "children":delivery.map(|delivery|delivery.verified_children().values().map(|child|child.evidence()).collect::<Vec<_>>()).unwrap_or_default(),
        "parent":delivery.and_then(|delivery|delivery.verified_parent()).map(|parent|parent.manifest()),
        "acceptance":delivery.and_then(|delivery|delivery.verified_acceptance()).map(|acceptance|acceptance.acceptance()),
        "history":crate::inspection::history_for_parent(&inspection.history,workspace,parent),
        "release":inspection.release_details.get(&(workspace,parent.clone())),
    }))
}

fn fresh_plan_preparation(runtime: &RuntimeCoordinator, plan: &Plan) -> Result<()> {
    fresh_preparation(runtime, &plan.request)?;
    plan.revalidate_command_policy()
}

fn fresh_preparation(runtime: &RuntimeCoordinator, request: &LaunchRequest) -> Result<()> {
    runtime.verify_owner(&request.lease)?;
    let current = runtime.retained_authority(&request.authority)?;
    request.authority.compare(&current)?;
    if let Some(assignment) = &request.delivery {
        let WorkerAuthority::Authorized(gate) = &current else {
            return Err(Error::new(
                "worker_binding",
                "delivery authority changed class",
            ));
        };
        assignment.verify_binding(gate, &request.phase.phase)?;
    }
    Ok(())
}

async fn acquire_worker_source(
    runtime: &Arc<RuntimeCoordinator>,
    plan: &Plan,
    permit: &Permit,
    git_path: &Path,
    processes: Arc<crate::process_records::ProcessJournal>,
    records: &PrivateDir,
) -> Result<crate::remote_objects::VerifiedObjectSource> {
    use crate::remote_objects::{
        AcquisitionServices, AuthorizedObjectRequest, ObjectAcquisition, ObjectLimits,
    };
    let selected = plan.source.as_ref().ok_or_else(|| {
        Error::new(
            "source_unavailable",
            "acquisition requires a selected exact source",
        )
    })?;
    let configuration = plan
        .workspace
        .repositories
        .get(&selected.repository)
        .ok_or_else(|| {
            Error::new(
                "scope_violation",
                "source repository configuration is missing",
            )
        })?
        .clone();
    let projection = runtime.observations.snapshot()?;
    let provider = projection
        .workspaces
        .get(&plan.workspace.id)
        .and_then(|workspace| {
            workspace
                .repositories
                .get(&selected.repository.parse::<u64>().ok()?)
        })
        .ok_or_else(|| {
            Error::new(
                "incompletely_observed",
                "source acquisition requires current provider repository identity",
            )
        })?
        .identity
        .clone();
    let contract = crate::delivery::RepositoryContract {
        configuration_name: selected.repository.clone(),
        repository: plan
            .repository_id
            .clone()
            .ok_or_else(|| Error::new("scope_violation", "source repository identity missing"))?,
        provider,
        configuration,
    };
    let object_request = AuthorizedObjectRequest::for_worker(
        &contract,
        selected.revision.clone(),
        permit,
        &plan.request.authority,
    )?;
    let expected = plan.request.clone();
    let expected_permit = permit.clone();
    let expected_contract = contract.clone();
    let workspace = plan.workspace.id;
    let current_runtime = runtime.clone();
    let current = Arc::new(move |request: &AuthorizedObjectRequest| {
        fresh_preparation(&current_runtime, &expected)?;
        if request.permit().operation_id != expected_permit.operation_id
            || request.permit().lease != expected_permit.lease
            || request.permit().work_key != expected_permit.work_key
            || request.permit().scope_digest != expected_permit.scope_digest
            || request.authority_digest() != expected.authority.digest()?
            || request.contract_digest()? != expected_contract.digest()?
            || request.repository() != &expected_contract.repository
        {
            return Err(Error::new(
                "scope_violation",
                "object acquisition differs from original preparation authority",
            ));
        }
        let config = current_runtime.configuration.snapshot()?;
        let configured = config
            .workspaces
            .values()
            .find(|value| value.id == workspace)
            .and_then(|workspace| {
                workspace
                    .repositories
                    .get(&expected_contract.configuration_name)
            })
            .ok_or_else(|| Error::new("scope_violation", "source repository access was removed"))?;
        if canonical_digest(1, configured)?
            != canonical_digest(1, &expected_contract.configuration)?
        {
            return Err(Error::new(
                "stale_config",
                "source repository contract changed during acquisition",
            ));
        }
        Ok(())
    });
    let github = providers(runtime, &plan.config, workspace)?
        .github
        .ok_or_else(|| Error::new("missing_secret", "object source provider is unavailable"))?;
    let mut object_records = None;
    runtime.control.validate_permit(permit, || {
        fresh_plan_preparation(runtime, plan)?;
        object_records = Some(records.child("object-sources")?);
        Ok(())
    })?;
    ObjectAcquisition::new(AcquisitionServices {
        control: runtime.control.clone(),
        github,
        git: Git::from_pinned_executable(git_path)?,
        records: object_records
            .ok_or_else(|| Error::new("ownership_conflict", "object records gate did not run"))?,
        key: runtime.services.key.clone(),
        processes,
        executions: runtime.executions.clone(),
        current,
        limits: ObjectLimits::default(),
    })?
    .acquire(object_request)
    .await
}

fn source_error(error: Error) -> Error {
    if error.code == "git_operation_failed" {
        Error::new(
            "source_unavailable",
            "the configured local source lacks the exact selected objects; an owned authenticated object cache is required",
        )
    } else {
        error
    }
}

fn scoped_git_gate(
    runtime: Arc<RuntimeCoordinator>,
    permit: Permit,
    authority: WorkerAuthority,
) -> GitStartGate {
    Arc::new(move |spawn| {
        runtime.control.validate_permit(&permit, || {
            runtime.verify_owner(&permit.lease)?;
            if runtime
                .current_correction_transition(permit.operation_id)?
                .is_some()
            {
                crate::worker::effective_authority(
                    runtime.as_ref(),
                    permit.operation_id,
                    &authority,
                )?;
            } else {
                authority.compare(&runtime.retained_authority(&authority)?)?;
            }
            spawn()
        })
    })
}

#[cfg(test)]
mod source_tests {
    use super::*;
    use crate::providers::{
        Credential, GithubClient, HttpPolicy, HttpRequest, HttpResponse, HttpTransport,
        RepositoryIdentity,
    };
    use std::{collections::VecDeque, future::Future, pin::Pin, sync::Mutex, time::Duration};
    #[tokio::test]
    async fn unresolved_delivery_policy_reaches_current_scoped_worker_plan_without_effects() {
        use crate::app::ConfigurationControl;
        use crate::reconcile::{DependencyOutcomes, GatePurpose};
        let (_temp, fixture, _delivery, wire, owner) =
            crate::runtime_delivery::tests::fixture_authorization_with_configuration(
                true,
                |config| {
                    let workspace = config.workspaces.get_mut("example").unwrap();
                    workspace.workflow = None;
                    let repository = workspace.repositories.get_mut("42").unwrap();
                    repository.local_checks.clear();
                    repository.integrated_checks.clear();
                    repository.ci = None;
                    repository.production_ci = None;
                    repository.no_change_validation = None;
                    repository.deployment = None;
                },
            )
            .await;
        // All omissions precede the real projection and signed authorization.
        let config = fixture.configuration.snapshot().unwrap();
        config.validate().unwrap();
        let config_revision = crate::config::config_revision(&config).unwrap();
        let projection = fixture.runtime.observations.snapshot().unwrap();
        assert_eq!(projection.config_revision, config_revision);
        let parent = projection
            .parent(Uuid::from_u128(2), Uuid::from_u128(100))
            .unwrap();
        let gate = parent
            .fresh_gate(
                Some(Uuid::from_u128(101)),
                GatePurpose::NewOperation,
                &DependencyOutcomes::default(),
                chrono::Utc::now(),
                chrono::Duration::seconds(30),
            )
            .unwrap();
        let authority = WorkerAuthority::Authorized(gate.clone());
        let revision = Revision {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        {
            let mut wire = wire.lock().unwrap();
            wire.github_values.insert("/repos/fixture/repo/git/ref/heads/staging".into(),
                json!({"ref":"refs/heads/staging","object":{"type":"commit","sha":revision.commit}}));
            wire.github_values.insert(
                format!("/repos/fixture/repo/git/commits/{}", revision.commit),
                json!({"sha":revision.commit,"tree":{"sha":revision.tree},"parents":[]}),
            );
        }
        let selected = resolve_source(&fixture.runtime, &authority)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(selected.repository, "42");
        assert_eq!(selected.revision, revision);
        let (lease, _owner_lock) = owner.start(&fixture.runtime);
        let operations_before =
            canonical_digest(1, &fixture.runtime.control.operations().unwrap()).unwrap();
        let request = LaunchRequest {
            lease,
            operation_id: Uuid::new_v4(),
            authority,
            phase: PhaseKey {
                work: gate.work_key().unwrap(),
                generation: canonical_digest(1, &gate.work().generation).unwrap(),
                phase: "implementation".into(),
                retry_grant: None,
            },
            source: Some(selected),
            delivery: None,
            unfinished: None,
            source_advanced: None,
            retried_unfinished: None,
            review_retry: None,
            validation_retry: None,
            review_round: None,
            max_seconds: 30,
            max_turns: 1,
        };
        let plan = Plan::read(&fixture.runtime, request.clone()).unwrap();
        assert_eq!(plan.role, AgentRole::Testing);
        assert_eq!(plan.source.as_ref().unwrap().revision, revision);
        assert_eq!(
            crate::config::config_revision(&plan.config).unwrap(),
            config_revision
        );
        assert_eq!(
            plan.request.authority.digest().unwrap(),
            gate.digest().unwrap()
        );
        assert!(plan.workflow.front_matter.checks.local.is_empty());
        assert!(plan.workflow.front_matter.checks.integrated.is_empty());
        assert_eq!(
            plan.workspace.repositories["42"]
                .staging_ci()
                .unwrap_err()
                .code,
            "delivery_policy_unresolved"
        );
        assert!(plan.context.get("issue").is_some());
        assert!(
            fixture
                .runtime
                .current_delivery(&parent.key)
                .unwrap()
                .verified_parent()
                .is_none()
        );
        assert!(
            fixture
                .runtime
                .current_delivery(&parent.key)
                .unwrap()
                .verified_acceptance()
                .is_none()
        );
        // The actual runtime plan accepts neither an unobserved source nor a
        // caller-selected repository outside the original signed child scope.
        let mut wrong_source = request.clone();
        wrong_source.source.as_mut().unwrap().repository = "99".into();
        assert_eq!(
            Plan::read(&fixture.runtime, wrong_source)
                .err()
                .unwrap()
                .code,
            "scope_violation"
        );
        let mut missing_source = request.clone();
        missing_source.source = None;
        assert_eq!(
            Plan::read(&fixture.runtime, missing_source)
                .err()
                .unwrap()
                .code,
            "incompletely_prepared"
        );
        fixture
            .configuration
            .fences
            .lock()
            .unwrap()
            .workspaces
            .insert(Uuid::from_u128(2));
        assert_eq!(
            Plan::read(&fixture.runtime, request).err().unwrap().code,
            "permission_fenced"
        );
        assert_eq!(
            canonical_digest(1, &fixture.runtime.control.operations().unwrap()).unwrap(),
            operations_before
        );
        assert!(fixture.processes.operations().unwrap().is_empty());
        assert!(
            fixture
                .runtime
                .executions
                .active_operations()
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn preparation_rechecks_selected_attempt_caps_trigger_and_spacing_before_receipt() {
        use crate::control::{ControlHost, Intent, Lifecycle};
        let temp = tempfile::tempdir().unwrap();
        let root =
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("control")).unwrap();
        let runtime_id = Uuid::new_v4();
        let host = ControlHost::setup(root, runtime_id, Uuid::new_v4()).unwrap();
        host.lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id,
                actor: Actor {
                    identity: "preparation_fixture".into(),
                    roles: ["lifecycle".into()].into(),
                    principal: None,
                },
                kind: "lifecycle".into(),
                canonical_targets: vec![runtime_id.to_string()],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            Lifecycle::Start,
            0,
        )
        .unwrap();
        let lease = Lease {
            runtime_id,
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        host.attach_reconciled(lease.clone()).unwrap();
        let expected = crate::runtime_schedule::RunSchedule {
            attempt: 2,
            max_attempts: 3,
            max_turns: 4,
            max_review_rounds: 2,
            trigger: crate::scheduler::Trigger::VerifiedUnfinished,
            not_before: chrono::Utc::now() - chrono::Duration::seconds(1),
        };
        require_preparation_schedule(&expected, &expected, 4).unwrap();
        for field in [
            "attempt",
            "attempt_cap",
            "turn_cap",
            "review_cap",
            "trigger",
            "spacing",
        ] {
            let mut current = expected.clone();
            match field {
                "attempt" => current.attempt += 1,
                "attempt_cap" => current.max_attempts -= 1,
                "turn_cap" => current.max_turns -= 1,
                "review_cap" => current.max_review_rounds -= 1,
                "trigger" => current.trigger = crate::scheduler::Trigger::OperatorRetry,
                "spacing" => {
                    current.not_before = chrono::Utc::now() + chrono::Duration::seconds(30)
                }
                _ => unreachable!(),
            }
            let reference = PreparationReference {
                worker_operation: Uuid::new_v4(),
                workspace: Uuid::new_v4(),
                account: "fixture".into(),
                phase: crate::scheduler::PhaseKey {
                    work: "fixture_work".into(),
                    generation: canonical_digest(1, &"generation").unwrap(),
                    phase: "implementation".into(),
                    retry_grant: None,
                },
            };
            let request_id = reference.operation_id().unwrap();
            let error = host
                .admit_preparation_observed(
                    &lease,
                    PreparationAdmission {
                        request_id,
                        work_key: reference.phase.work.clone(),
                        scope_digest: canonical_digest(1, &"scope").unwrap(),
                        reference,
                    },
                    |snapshot| {
                        assert_eq!(snapshot.intent.desired, Intent::Running);
                        assert!(snapshot.active_operations.is_empty());
                        require_preparation_schedule(&expected, &current, 4)
                    },
                )
                .unwrap_err();
            assert_eq!(
                error.code,
                if field == "spacing" {
                    "retry_wait"
                } else {
                    "admission_budget"
                }
            );
            assert_eq!(
                host.operation(request_id).unwrap_err().code,
                "record_missing"
            );
            assert!(host.snapshot().unwrap().active_operations.is_empty());
            assert!(host.snapshot().unwrap().unresolved_operations.is_empty());
        }
        assert_eq!(
            require_preparation_schedule(&expected, &expected, 5)
                .unwrap_err()
                .code,
            "admission_budget"
        );
    }
    struct Wire {
        responses: Mutex<VecDeque<Value>>,
        paths: Mutex<Vec<String>>,
    }
    impl HttpTransport for Wire {
        fn send(
            &self,
            request: HttpRequest,
        ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
            Box::pin(async move {
                assert_eq!(request.method, crate::providers::Method::Get);
                self.paths.lock().unwrap().push(request.url.path().into());
                let value = self
                    .responses
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected source request");
                Ok(HttpResponse {
                    status: 200,
                    headers: reqwest::header::HeaderMap::new(),
                    body: serde_json::to_vec(&value).unwrap(),
                })
            })
        }
    }
    fn fixture(
        last_commit: &str,
        returned_commit: &str,
    ) -> (GithubClient, Arc<Wire>, RepositoryIdentity) {
        let mut values = VecDeque::new();
        for response in [
            json!({"ref":"refs/heads/staging","object":{"type":"commit","sha":"a".repeat(40)}}),
            json!({"sha":returned_commit,"tree":{"sha":"b".repeat(40)},"parents":[]}),
            json!({"ref":"refs/heads/staging","object":{"type":"commit","sha":last_commit}}),
        ] {
            values.push_back(json!({"id":7,"login":"fixture","type":"User"}));
            values.push_back(json!({"id":42,"name":"repo","owner":{"id":7,"login":"fixture","type":"User"},"default_branch":"main","private":true,"archived":false,"disabled":false,"permissions":{"pull":true}}));
            values.push_back(response);
        }
        let wire = Arc::new(Wire {
            responses: Mutex::new(values),
            paths: Mutex::new(vec![]),
        });
        let client = GithubClient::with_transport(
            HttpPolicy::new(
                "https://fixture.invalid/api/v3",
                "7".into(),
                Duration::from_secs(1),
            )
            .unwrap(),
            kegari_config::GithubSelection {
                repositories: vec!["42".into()],
                organizations: vec![],
                pr_authors: vec![],
            },
            Credential::new("fixture-only".into()).unwrap(),
            Some(wire.clone()),
        )
        .unwrap();
        (
            client,
            wire,
            RepositoryIdentity {
                id: 42,
                owner: "fixture".into(),
                name: "repo".into(),
                owner_id: 7,
                organization_id: None,
            },
        )
    }
    #[tokio::test]
    async fn source_resolution_requires_stable_provider_branch_and_exact_commit_tree() {
        let (client, wire, repository) = fixture(&"a".repeat(40), &"a".repeat(40));
        let result = provider_revision(&client, &repository, ProviderSource::Staging("staging"))
            .await
            .unwrap();
        assert_eq!(
            result,
            Revision {
                commit: "a".repeat(40),
                tree: "b".repeat(40)
            }
        );
        assert!(wire.responses.lock().unwrap().is_empty());
        assert_eq!(
            wire.paths
                .lock()
                .unwrap()
                .iter()
                .filter(|path| path.ends_with("/git/ref/heads/staging"))
                .count(),
            2
        );
        let (client, _, repository) = fixture(&"c".repeat(40), &"a".repeat(40));
        assert_eq!(
            provider_revision(&client, &repository, ProviderSource::Staging("staging"))
                .await
                .unwrap_err()
                .code,
            "stale_revision"
        );
        let (client, _, repository) = fixture(&"a".repeat(40), &"c".repeat(40));
        assert_eq!(
            provider_revision(&client, &repository, ProviderSource::Staging("staging"))
                .await
                .unwrap_err()
                .code,
            "identity_mismatch"
        );
    }
}
