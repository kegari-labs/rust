//! Finite child progression over actual completed run capabilities. Persisted
//! phase references are recovery facts, never replacements for execution proof.
use super::*;
use crate::{
    control::{Lease, Permit, RunReference},
    delivery::*,
    digest::canonical_digest,
    evidence::RevisionBinding,
    git::{GitExecutionScope, SourceCommitMetadata},
    runtime::VerifiedAuthorizedOutcome,
    scheduler::PhaseKey,
    worker::{WorkerAuthority, WorkerHost},
    worker_run::RunCompletion,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

pub(super) struct ActivePhase {
    pub(super) admission: DeliveryAdmission,
    pub(super) permit: Permit,
    pub(super) coordinator: Arc<DeliveryCoordinator>,
    pub(super) git: Arc<Git>,
    pub(super) workspace: Arc<WorkspaceDelivery>,
    pub(super) _active: crate::observations::ActiveSnapshot,
}
struct PhaseSetup {
    runtime: Arc<RuntimeCoordinator>,
    permit: Permit,
    armed: bool,
    failed: Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for PhaseSetup {
    fn drop(&mut self) {
        if self.armed && self.runtime.control.fence(self.permit.operation_id).is_ok() {
            let _ = self
                .runtime
                .executions
                .cancel_fenced(&self.runtime.control, self.permit.operation_id);
        }
        if self.armed {
            self.failed
                .store(true, std::sync::atomic::Ordering::Release);
        }
        // The retained permit and durable process journal remain in the host;
        // failed cancellation never becomes an exit or retirement assertion.
    }
}
pub(super) struct PhaseRequest {
    pub operation: Uuid,
    pub scope: DeliveryScope,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Stage {
    Seal,
    LaunchValidation,
    Validate,
    RetryValidation,
    LaunchValidationRetry,
    PublishBranch,
    PullRequest,
    Hosted,
    LaunchReview,
    Review,
    IntegrateStaging,
    ObserveStaging,
    Publish,
    Close,
    Complete,
    RepairLaunch,
    RepairWaiting,
    RetryReview,
    LaunchReviewRetry,
    Blocked,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChildReference {
    schema_version: u32,
    runtime: Uuid,
    operation: Uuid,
    proposal: Uuid,
    claim_digest: String,
    work: crate::evidence::WorkIdentity,
    stage: Stage,
    phase_operation: Option<Uuid>,
    validation: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validation_assignment_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validation_source_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validation_retry_operation: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validation_retry_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validation_completed_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validation_passed_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    validation_successor_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    validation_history: Vec<Uuid>,
    review: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    repair_operation: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unfinished_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_advanced_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    review_retry_operation: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    review_retry_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    review_history: Vec<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    review_round_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paired_round_digest: Option<String>,
    observed_at: chrono::DateTime<Utc>,
    error: Option<Error>,
}
struct ValidationRepairBasis {
    paired: Arc<VerifiedReviewRoundPlan>,
    basis: ValidationAdvanceBasis,
    outcome: VerifiedAuthorizedOutcome,
    completion: Arc<RunCompletion>,
}
struct ChildPipeline {
    reference: ChildReference,
    claim: Arc<CompletedChildClaim>,
    completion: Arc<RunCompletion>,
    source: Option<SealedChildSource>,
    local: Option<LocalPassed>,
    branch: Option<PublishedChildBranch>,
    pr: Option<ChildPullRequest>,
    hosted: Option<HostedPassed>,
    exact_pending: Option<PendingExactValidation>,
    exact: Option<ExactValidationPassed>,
    assignment: Option<ChildReviewAssignment>,
    review: Option<PublishedReview>,
    pending: Option<PendingStaging>,
    staged: Option<StagedDelta>,
    published: Option<Box<PublishedChild>>,
    phase: Option<ActivePhase>,
    unfinished: Option<Arc<UnfinishedChildWork>>,
    source_advanced: Option<Arc<VerifiedSourceAdvanced>>,
    validation_assignment: Option<DeliveryWorkerAssignment>,
    validation_source: Option<Arc<VerifiedSourceAdvanced>>,
    validation_retry: Option<Arc<ValidationRetryWork>>,
    validation_completed: Option<Arc<CompletedGrantedValidation>>,
    validation_passed: Option<Arc<VerifiedPassedValidation>>,
    validation_successor: Option<Arc<ConsumedValidationSuccessor>>,
    review_worker_assignment: Option<DeliveryWorkerAssignment>,
    review_retry: Option<Arc<ReviewRetryWork>>,
    review_round: Option<Arc<VerifiedReviewRoundPlan>>,
    paired_round: Option<Arc<VerifiedReviewRoundPlan>>,
}
pub(super) struct CandidateChild {
    pub(super) child: Uuid,
    pub(super) published: Option<PublishedChild>,
    pub(super) staged: Option<StagedDelta>,
}
#[derive(Default)]
pub(super) struct Children {
    pipelines: Mutex<BTreeMap<Uuid, Arc<tokio::sync::Mutex<ChildPipeline>>>>,
    advancing: tokio::sync::Mutex<()>,
}
impl RuntimeDelivery {
    fn persist_child(&self, child: &ChildPipeline) -> Result<()> {
        self.records.write(
            &format!("child-pipeline-{}.json", child.reference.operation),
            &self
                .runtime
                .services
                .key
                .sign("runtime_child_pipeline", &child.reference)?,
        )
    }
    pub(super) fn completed_claim(
        &self,
        operation: Uuid,
    ) -> Result<Option<(VerifiedAuthorizedOutcome, Arc<RunCompletion>)>> {
        let Some(completion) = self.runtime.retained_completion(operation)? else {
            return Ok(None);
        };
        let receipt = self.runtime.control.operation(operation)?;
        let records = PrivateDir::open(
            &self
                .runtime
                .registration
                .root()
                .path()
                .join("worker-records")
                .join(operation.to_string()),
        )?;
        let ids = crate::runtime_claims::proposal_ids(&records, &receipt)?;
        if ids.len() != 1 {
            return Err(Error::new(
                "worker_claim",
                "delivery requires exactly one final completed outcome proposal",
            ));
        }
        let outcome = self.runtime.authorized_outcome(operation, ids[0])?;
        let contract=completion.contract.as_ref().ok_or_else(||Error::new("reconciliation_required","actual original worker contract is unavailable; retained JSON is not execution authority"))?;
        if contract.instructions().manifest_digest != outcome.outcome().instruction_digest
            || canonical_digest(1, &contract.instructions().manifest)?
                != canonical_digest(1, &outcome.outcome().instructions)?
            || contract.authority().digest()?
                != WorkerAuthority::Authorized(outcome.original_authority().clone()).digest()?
        {
            return Err(Error::new(
                "worker_binding",
                "retained completion differs from its original authorized contract",
            ));
        }
        Ok(Some((outcome, completion)))
    }
    fn discover_children(&self, projection: &RuntimeProjection) -> Result<()> {
        for receipt in self.runtime.control.operations()? {
            let Some(run) = RunReference::from_receipt(&receipt)? else {
                continue;
            };
            if run.phase.phase != "implementation"
                || !matches!(
                    receipt.phase,
                    crate::control::ReceiptPhase::Reconciled
                        | crate::control::ReceiptPhase::Acknowledged
                )
            {
                continue;
            }
            if self
                .children
                .pipelines
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "child pipeline lock failed"))?
                .contains_key(&receipt.operation_id)
            {
                continue;
            }
            let (outcome, completion) = match self.completed_claim(receipt.operation_id) {
                Ok(Some(completed)) => completed,
                Ok(None) => continue,
                Err(error) => {
                    if let Some(parent) =
                        projection
                            .workspaces
                            .get(&run.workspace)
                            .and_then(|workspace| {
                                workspace.parents.values().find(|parent| {
                                    parent.children.values().any(|child| {
                                        child.key.identity().is_ok_and(|key| key == run.phase.work)
                                    })
                                })
                            })
                    {
                        self.runtime.invalidate_delivery(&parent.key, error)?;
                    }
                    continue;
                }
            };
            let work = outcome.effective_authority().work();
            let Some(child) = work.child else { continue };
            let parent = match projection.parent(run.workspace, work.parent) {
                Ok(parent) => parent,
                Err(_) => continue,
            };
            if self
                .runtime
                .current_delivery(&parent.key)?
                .verified_children()
                .contains_key(&child)
            {
                continue;
            }
            let result = (|| {
                let gate = parent.fresh_gate(
                    Some(child),
                    GatePurpose::NewOperation,
                    &self.runtime.dependency_outcomes()?,
                    Utc::now(),
                    Duration::seconds(30),
                )?;
                let workspace = self.workspace(projection, run.workspace)?;
                let transition = self.runtime.completed_correction_transition(&outcome)?;
                let claim = match transition {
                    Some(transition) => workspace.coordinator.accept_corrected_child_completion(
                        &gate,
                        outcome.clone(),
                        completion.clone(),
                        transition,
                    )?,
                    None => workspace.coordinator.accept_child_completion(
                        &gate,
                        outcome.clone(),
                        completion.clone(),
                    )?,
                };
                if completion.execution.is_none() {
                    return Err(Error::new(
                        "reconciliation_required",
                        "original owned execution capability is unavailable for child source sealing",
                    ));
                }
                let leaf = format!("child-pipeline-{}.json", receipt.operation_id);
                if self.records.path().join(&leaf).exists() {
                    let prior: crate::evidence::Signed<ChildReference> =
                        self.records.read(&leaf)?;
                    self.runtime
                        .services
                        .key
                        .verify("runtime_child_pipeline", &prior)?;
                    return Err(Error::new("reconciliation_required","retained child progression requires exact phase recovery before another mutation").detail(json!({"operation":receipt.operation_id,"stage":prior.payload.stage})));
                }
                let pipeline = ChildPipeline {
                    reference: ChildReference {
                        schema_version: 1,
                        runtime: receipt.runtime_id,
                        operation: receipt.operation_id,
                        proposal: outcome.outcome().proposal_id,
                        claim_digest: outcome.digest().into(),
                        work: work.clone(),
                        stage: Stage::Seal,
                        phase_operation: None,
                        validation: None,
                        validation_assignment_digest: None,
                        validation_source_digest: None,
                        validation_retry_operation: None,
                        validation_retry_digest: None,
                        validation_completed_digest: None,
                        validation_passed_digest: None,
                        validation_successor_digest: None,
                        validation_history: vec![],
                        review: None,
                        repair_operation: None,
                        unfinished_digest: None,
                        source_advanced_digest: None,
                        review_retry_operation: None,
                        review_retry_digest: None,
                        review_history: vec![],
                        review_round_digest: None,
                        paired_round_digest: None,
                        observed_at: Utc::now(),
                        error: None,
                    },
                    claim: Arc::new(claim),
                    completion,
                    source: None,
                    local: None,
                    branch: None,
                    pr: None,
                    hosted: None,
                    exact_pending: None,
                    exact: None,
                    assignment: None,
                    review: None,
                    pending: None,
                    staged: None,
                    published: None,
                    phase: None,
                    unfinished: None,
                    source_advanced: None,
                    validation_assignment: None,
                    validation_source: None,
                    validation_retry: None,
                    validation_completed: None,
                    validation_passed: None,
                    validation_successor: None,
                    review_worker_assignment: None,
                    review_retry: None,
                    review_round: None,
                    paired_round: None,
                };
                self.persist_child(&pipeline)?;
                self.children
                    .pipelines
                    .lock()
                    .map_err(|_| Error::new("runtime_unavailable", "child pipeline lock failed"))?
                    .insert(
                        receipt.operation_id,
                        Arc::new(tokio::sync::Mutex::new(pipeline)),
                    );
                Ok(())
            })();
            if let Err(error) = result {
                self.runtime.invalidate_delivery(&parent.key, error)?;
            }
        }
        Ok(())
    }
    async fn phase(
        &self,
        workspace: &WorkspaceDelivery,
        gate: &FreshGate,
        lease: &Lease,
        child: &mut ChildPipeline,
        phase: DeliveryPhase,
    ) -> Result<()> {
        if child.phase.is_some() {
            return Ok(());
        }
        let execution = child.completion.execution.as_ref().ok_or_else(|| {
            Error::new(
                "execution_ownership",
                "original execution capability unavailable",
            )
        })?;
        let contract = workspace
            .contracts
            .iter()
            .find(|c| c.repository == execution.binding.repository)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "child repository is no longer configured",
                )
            })?;
        let revision = child
            .source
            .as_ref()
            .map(|source| source.revision().clone())
            .unwrap_or_else(|| RevisionBinding {
                repository: execution.binding.repository.clone(),
                head: execution.binding.revision.commit.clone(),
                base: execution.binding.revision.commit.clone(),
                tested_commit: execution.binding.revision.commit.clone(),
                tested_tree: execution.binding.revision.tree.clone(),
            });
        let scope = DeliveryScope {
            schema_version: 1,
            runtime_id: lease.runtime_id,
            phase,
            work: gate.work().clone(),
            repository: contract.repository.clone(),
            revision,
            fresh_gate_digest: gate.digest()?,
            contract_digest: contract.digest()?,
            candidate_vector_digest: None,
        };
        let operation = child.reference.phase_operation.unwrap_or_else(Uuid::new_v4);
        child.reference.phase_operation = Some(operation);
        self.persist_child(child)?;
        if let Some(passed) = &child.validation_passed
            && child.reference.validation_passed_digest.as_deref() != Some(passed.digest())
        {
            return Err(Error::new(
                "record_corrupt",
                "passed validation reference changed",
            ));
        }
        let extra = match &child.validation_successor {
            Some(proof) => {
                if child.reference.validation_successor_digest.as_deref() != Some(proof.digest())
                    || child.reference.review != Some(proof.worker_operation())
                {
                    return Err(Error::new(
                        "record_corrupt",
                        "passed validation handoff reference changed",
                    ));
                }
                if child.reference.stage == Stage::RetryReview {
                    let receipt = self.runtime.control.operation(proof.worker_operation())?;
                    let run = RunReference::from_receipt(&receipt)?.ok_or_else(|| {
                        Error::new(
                            "worker_binding",
                            "validation successor review admission disappeared",
                        )
                    })?;
                    let grants = self.current_review_grants(&child.reference.work)?;
                    let matching: Vec<_> = grants
                        .iter()
                        .filter(|grant| grant.phase() == &run.phase)
                        .collect();
                    match matching.as_slice() {
                        [] => Some(self.consumed_validation_check(proof.clone())),
                        [grant] => {
                            let failed = self
                                .runtime
                                .retained_completion(proof.worker_operation())?
                                .ok_or_else(|| {
                                    Error::new(
                                        "reconciliation_required",
                                        "validation successor lost its joined failed review",
                                    )
                                })?;
                            Some(self.consumed_validation_retry_check(
                                proof.clone(),
                                (**grant).clone(),
                                failed,
                            ))
                        }
                        _ => {
                            return Err(Error::new(
                                "stale_result",
                                "review successor has competing current grants",
                            ));
                        }
                    }
                } else {
                    Some(self.consumed_validation_check(proof.clone()))
                }
            }
            None => child
                .validation_passed
                .as_ref()
                .map(|proof| self.passed_validation_check(proof.clone()))
                .or_else(|| {
                    child
                        .validation_completed
                        .as_ref()
                        .map(|proof| self.completed_validation_check(proof.clone()))
                }),
        };
        if let Some(proof) = &child.validation_completed
            && child.reference.validation_completed_digest.as_deref() != Some(proof.digest())
        {
            return Err(Error::new(
                "record_corrupt",
                "completed validation reference changed",
            ));
        }
        child.phase = Some(
            self.admit_delivery_phase_checked(
                workspace,
                gate,
                lease,
                PhaseRequest { operation, scope },
                extra,
            )
            .await?,
        );
        Ok(())
    }
    pub(super) async fn admit_delivery_phase(
        &self,
        workspace: &WorkspaceDelivery,
        gate: &FreshGate,
        lease: &Lease,
        request: PhaseRequest,
    ) -> Result<ActivePhase> {
        self.admit_delivery_phase_checked(workspace, gate, lease, request, None)
            .await
    }
    async fn admit_delivery_phase_checked(
        &self,
        workspace: &WorkspaceDelivery,
        gate: &FreshGate,
        lease: &Lease,
        request: PhaseRequest,
        extra: Option<GateCheck>,
    ) -> Result<ActivePhase> {
        let PhaseRequest { operation, scope } = request;
        let broker = self.provider_broker(&workspace.configuration, gate.workspace_id())?;
        let github = broker.github.clone().ok_or_else(|| {
            Error::new("missing_secret", "delivery GitHub connection unavailable")
        })?;
        let active = self.runtime.observations.capture_active(
            gate.workspace_id(),
            gate.config_revision(),
            gate.workflow_digest(),
        )?;
        let current = self.current_gate();
        let permit = self.runtime.control.admit(
            lease,
            operation,
            gate.work_key()?,
            scope.digest()?,
            || {
                self.runtime.registration.verify_active_owner(lease)?;
                current(gate)?;
                if let Some(check) = &extra {
                    check(gate)?;
                }
                Ok(())
            },
        )?;
        let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.phase_holds
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(operation, (permit.clone(), failed.clone()));
        let mut setup = PhaseSetup {
            runtime: self.runtime.clone(),
            permit: permit.clone(),
            armed: true,
            failed,
        };
        let admission = DeliveryAdmission::bind(permit.clone(), gate.clone(), scope)?;
        let host = self.runtime.control.clone();
        let registration = self.runtime.registration.clone();
        let expected = gate.clone();
        let held = permit.clone();
        let retained = self.retained_gate();
        let current: GateCheck = Arc::new(move |gate| {
            retained(gate)?;
            if let Some(check) = &extra {
                check(gate)?;
            }
            Ok(())
        });
        let effect_gate = current.clone();
        let observer_gate = current.clone();
        let gate_fn = Arc::new(move |launch: &mut dyn FnMut() -> Result<()>| {
            host.validate_permit(&held, || {
                registration.verify_active_owner(&held.lease)?;
                current(&expected)?;
                launch()
            })
        });
        let git = workspace.git.clone();
        let scoped = GitExecutionScope {
            limits: GitLimits::default(),
            start_gate: gate_fn,
            processes: self.processes.clone(),
            permit: permit.clone(),
            executions: self.runtime.executions.clone(),
        };
        let result = tokio::task::spawn_blocking(move || git.with_execution_scope(scoped))
            .await
            .map_err(|_| {
                Error::new(
                    "operation_uncertain",
                    "delivery Git phase setup did not join",
                )
            })?;
        let git = match result {
            Ok(git) => Arc::new(git),
            Err(error) => {
                self.runtime.control.fence(operation)?;
                self.runtime
                    .executions
                    .cancel_fenced(&self.runtime.control, operation)?;
                return Err(error);
            }
        };
        let coordinator = Arc::new(DeliveryCoordinator::new(
            DeliveryServices {
                host: self.runtime.control.clone(),
                broker,
                records: workspace.records.clone(),
                key: self.runtime.services.key.clone(),
                git: git.clone(),
                retained_git_sources: workspace
                    .contracts
                    .iter()
                    .map(|c| (c.repository.clone(), PathBuf::from(&c.configuration.source)))
                    .collect(),
                current_gate: effect_gate,
                executions: self.runtime.executions.clone(),
                processes: self.processes.clone(),
            },
            workspace.contracts.clone(),
        )?);
        let mut retained_workspace = workspace.clone();
        retained_workspace.coordinator = coordinator.clone();
        retained_workspace.observer = Arc::new(ProductionObserver::new(
            github,
            self.runtime.control.clone(),
            self.runtime.services.key.clone(),
            workspace.records.child("deployment")?,
            observer_gate,
        )?);
        setup.armed = false;
        self.phase_holds
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&operation);
        Ok(ActivePhase {
            admission,
            permit,
            coordinator,
            git,
            workspace: Arc::new(retained_workspace),
            _active: active,
        })
    }
    fn finish_child_phase(&self, child: &mut ChildPipeline, next: Stage) -> Result<()> {
        if let Some(phase) = &child.phase {
            self.retire_delivery_phase(
                phase,
                json!({"delivery_phase":phase.admission.scope().phase,"next":next}),
            )?;
        }
        child.phase = None;
        child.reference.phase_operation = None;
        child.reference.stage = next;
        child.reference.error = None;
        child.reference.observed_at = Utc::now();
        self.persist_child(child)
    }
    pub(super) fn retire_delivery_phase(
        &self,
        phase: &ActivePhase,
        observation: serde_json::Value,
    ) -> Result<()> {
        self.retire_phase_permit(&phase.permit, observation)
    }
    pub(super) fn retire_phase_permit(
        &self,
        permit: &Permit,
        observation: serde_json::Value,
    ) -> Result<()> {
        self.runtime
            .executions
            .observe_exited(permit.operation_id)?;
        if self
            .runtime
            .executions
            .active_operations()?
            .contains(&permit.operation_id)
        {
            return Err(Error::new(
                "operation_uncertain",
                "delivery phase still owns an unconfirmed local process",
            ));
        }
        self.runtime
            .control
            .retire_local(permit.operation_id, observation)?;
        let snapshot = self.runtime.control.snapshot()?;
        if snapshot
            .unresolved_operations
            .values()
            .any(|targets| targets.contains(&permit.work_key))
        {
            return Err(Error::new(
                "operation_uncertain",
                "delivery phase retains an unresolved provider effect",
            ));
        }
        Ok(())
    }
    pub(super) fn worker_launch_admitted(&self, operation: Uuid) -> Result<bool> {
        match self.runtime.control.operation(operation) {
            Ok(receipt) => {
                if RunReference::from_receipt(&receipt)?.is_none() {
                    return Err(Error::new(
                        "operation_conflict",
                        "launch identity belongs to another operation",
                    ));
                }
                if !self
                    .runtime
                    .control
                    .snapshot()?
                    .active_operations
                    .iter()
                    .any(|p| p.operation_id == operation)
                    && self.runtime.retained_completion(operation)?.is_none()
                {
                    return Err(Error::new(
                        "reconciliation_required",
                        "worker admission has no retained live or completed owned run",
                    ));
                }
                return Ok(true);
            }
            Err(error) if error.code == "record_missing" => {}
            Err(error) => return Err(error),
        }
        let preparation = crate::runtime_workers::preparation_operation(operation)?;
        match self.runtime.control.operation(preparation) {
            Ok(_) => Err(Error::new(
                "reconciliation_required",
                "launch preparation already has effects; recover its original operation before retrying",
            )),
            Err(error) if error.code == "record_missing" => Ok(false),
            Err(error) => Err(error),
        }
    }
    fn current_review_grants(
        &self,
        work: &crate::evidence::WorkIdentity,
    ) -> Result<Vec<crate::decisions::VerifiedRetryGrant>> {
        crate::runtime_workflow::current_retry_grants(
            &self.runtime.configuration.snapshot()?,
            &self.runtime.observations.admission_snapshot()?,
            work,
            &self.runtime.control.operations()?,
            &self.runtime.services.key,
        )
    }
    fn require_validation_retry_identity(
        &self,
        child: &ChildPipeline,
        proof: &ValidationRetryWork,
    ) -> Result<()> {
        let assignment = required(
            &child.validation_assignment,
            "original validation assignment",
        )?;
        proof.verify_assignment(assignment)?;
        if child.reference.validation_retry_digest.as_deref() != Some(proof.digest())
            || child.reference.validation_assignment_digest.as_deref()
                != Some(assignment.digest()?.as_str())
            || child.reference.validation_source_digest.as_deref()
                != proof.source_advanced().map(|p| p.digest())
            || child.validation_source.as_ref().map(|p| p.digest())
                != proof.source_advanced().map(|p| p.digest())
        {
            return Err(Error::new(
                "record_corrupt",
                "validation retry lost its exact retained source/assignment identity",
            ));
        }
        Ok(())
    }
    fn validation_retry_grant(
        &self,
        child: &ChildPipeline,
    ) -> Result<(
        Option<crate::decisions::VerifiedRetryGrant>,
        Vec<crate::decisions::VerifiedRetryGrant>,
    )> {
        let operation = child
            .reference
            .validation
            .ok_or_else(|| Error::new("record_corrupt", "failed validation identity is missing"))?;
        let receipt = self.runtime.control.operation(operation)?;
        let run = RunReference::from_receipt(&receipt)?.ok_or_else(|| {
            Error::new("worker_binding", "failed validation admission is missing")
        })?;
        let grants = self.current_review_grants(&child.reference.work)?;
        let matching: Vec<_> = grants
            .iter()
            .filter(|g| g.phase() == &run.phase)
            .cloned()
            .collect();
        let grant = match matching.as_slice() {
            [] if child
                .validation_retry
                .as_ref()
                .is_some_and(|p| p.phase() == &run.phase) =>
            {
                None
            }
            [] => {
                return Err(Error::new(
                    "retry_required",
                    "known failed validation awaits an explicit operator grant",
                ));
            }
            [grant] => Some(grant.clone()),
            _ => {
                return Err(Error::new(
                    "stale_result",
                    "validation budget has competing retry grants",
                ));
            }
        };
        Ok((grant, grants))
    }
    fn completed_validation_check(&self, proof: Arc<CompletedGrantedValidation>) -> GateCheck {
        let runtime = self.runtime.clone();
        Arc::new(move |gate| {
            let current = runtime
                .observations
                .active_gate(gate, &runtime.dependency_outcomes()?)?;
            proof
                .authority()
                .compare_current(&current, Utc::now(), Duration::seconds(30))?;
            let receipts = runtime.control.operations()?;
            let grants = crate::runtime_workflow::current_retry_grants(
                &runtime.configuration.snapshot()?,
                &runtime.observations.admission_snapshot()?,
                proof.authority().work(),
                &receipts,
                &runtime.services.key,
            )?;
            proof.verify_current(&receipts, &grants)?;
            proof.verify_joined(&|id| runtime.retained_completion(id))
        })
    }
    fn passed_validation_check(&self, proof: Arc<VerifiedPassedValidation>) -> GateCheck {
        let runtime = self.runtime.clone();
        Arc::new(move |gate| {
            let current = runtime
                .observations
                .active_gate(gate, &runtime.dependency_outcomes()?)?;
            proof
                .authority()
                .compare_current(&current, Utc::now(), Duration::seconds(30))?;
            let receipts = runtime.control.operations()?;
            let grants = crate::runtime_workflow::current_retry_grants(
                &runtime.configuration.snapshot()?,
                &runtime.observations.admission_snapshot()?,
                proof.authority().work(),
                &receipts,
                &runtime.services.key,
            )?;
            proof.verify_current(&receipts, &grants)?;
            proof.verify_joined(&|id| runtime.retained_completion(id))
        })
    }
    fn consumed_validation_check(&self, proof: Arc<ConsumedValidationSuccessor>) -> GateCheck {
        let runtime = self.runtime.clone();
        Arc::new(move |gate| {
            let current = runtime
                .observations
                .active_gate(gate, &runtime.dependency_outcomes()?)?;
            proof
                .authority()
                .compare_current(&current, Utc::now(), Duration::seconds(30))?;
            let receipts = runtime.control.operations()?;
            let grants = crate::runtime_workflow::current_retry_grants(
                &runtime.configuration.snapshot()?,
                &runtime.observations.admission_snapshot()?,
                proof.authority().work(),
                &receipts,
                &runtime.services.key,
            )?;
            proof.verify_current(&receipts, &grants)?;
            proof.verify_joined(&|id| runtime.retained_completion(id))
        })
    }
    fn consumed_validation_retry_check(
        &self,
        proof: Arc<ConsumedValidationSuccessor>,
        grant: crate::decisions::VerifiedRetryGrant,
        failed: Arc<RunCompletion>,
    ) -> GateCheck {
        let runtime = self.runtime.clone();
        Arc::new(move |gate| {
            let current = runtime
                .observations
                .active_gate(gate, &runtime.dependency_outcomes()?)?;
            proof
                .authority()
                .compare_current(&current, Utc::now(), Duration::seconds(30))?;
            let actual = runtime
                .retained_completion(proof.worker_operation())?
                .ok_or_else(|| {
                    Error::new(
                        "reconciliation_required",
                        "validation successor lost joined failed review",
                    )
                })?;
            if !Arc::ptr_eq(&actual, &failed) {
                return Err(Error::new(
                    "worker_binding",
                    "validation successor failed review identity changed",
                ));
            }
            let receipts = runtime.control.operations()?;
            let grants = crate::runtime_workflow::current_retry_grants(
                &runtime.configuration.snapshot()?,
                &runtime.observations.admission_snapshot()?,
                proof.authority().work(),
                &receipts,
                &runtime.services.key,
            )?;
            crate::runtime_workflow::require_current_grant(&grant, &grants)?;
            proof.verify_retry_preparation(&receipts, &grants, &grant, &failed)?;
            proof.verify_joined(&|id| runtime.retained_completion(id))
        })
    }
    fn observe_validation_successor(
        &self,
        child: &mut ChildPipeline,
        operation: Uuid,
    ) -> Result<()> {
        let Some(passed) = child.validation_passed.clone() else {
            if child.validation_completed.is_some()
                || child
                    .validation_source
                    .as_ref()
                    .is_some_and(|source| source.has_validation_budget())
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "completed retry lacks its genuine passed capability",
                ));
            }
            return Ok(());
        };
        if child.reference.validation_passed_digest.as_deref() != Some(passed.digest()) {
            return Err(Error::new(
                "record_corrupt",
                "passed validation reference changed",
            ));
        }
        // A completed preparation alone is not model admission. Preserve this
        // exact pending launch identity until the actual model record exists.
        match self.runtime.control.operation(operation) {
            Ok(_) => {}
            Err(error) if error.code == "record_missing" => {
                return Err(Error::new(
                    "operation_pending",
                    "validation handoff awaits exact model admission",
                ));
            }
            Err(error) => return Err(error),
        }
        let round = required(
            &child.review_round,
            "retained review round for validation handoff",
        )?
        .clone();
        if child.reference.review_round_digest.as_deref() != Some(round.digest()) {
            return Err(Error::new(
                "record_corrupt",
                "validation successor round changed",
            ));
        }
        let consumed = Arc::new(passed.consume_successor(round, operation, &self.runtime)?);
        child.reference.validation_successor_digest = Some(consumed.digest().into());
        child.validation_successor = Some(consumed);
        self.persist_child(child)
    }
    async fn launch_validation_retry(
        &self,
        lease: &Lease,
        child: &mut ChildPipeline,
    ) -> Result<()> {
        let proof = required(&child.validation_retry, "retained validation retry")?.clone();
        self.require_validation_retry_identity(child, &proof)?;
        let operation = child
            .reference
            .validation_retry_operation
            .unwrap_or_else(Uuid::new_v4);
        if child.reference.validation != Some(operation) {
            ValidationRetryWork::require_history_capacity(
                child.reference.validation_history.len(),
            )?;
        }
        let admitted = self.worker_launch_admitted(operation)?;
        let receipts = self.runtime.control.operations()?;
        let grants = self.current_review_grants(&child.reference.work)?;
        proof.verify_joined(&|id| self.runtime.retained_completion(id))?;
        if admitted {
            // A saved launch is observed, never replayed. Its exact actual run
            // must be the one bounded next admission of this opaque cause.
            proof.verify_admitted_inventory(&receipts, &grants, operation)?;
        } else {
            proof.verify_current(&receipts, &grants)?;
            let mut request = crate::runtime_workers::LaunchRequest {
                lease: lease.clone(),
                operation_id: operation,
                authority: self.current_delivery_worker_authority(proof.authority())?,
                phase: proof.phase().clone(),
                source: None,
                delivery: Some(proof.assignment().clone()),
                unfinished: None,
                source_advanced: None,
                retried_unfinished: None,
                review_retry: None,
                review_round: None,
                validation_retry: Some(proof),
                max_seconds: 3600,
                max_turns: self.runtime.configuration.snapshot()?.runtime.max_turns,
            };
            // Native adapter integration is deliberately a typed required API;
            // no ordinary-admission fallback may erase the retry/source lineage.
            let schedule = self.runtime.next_attempt_for_validation_retry(&request)?;
            if schedule.not_before > Utc::now() {
                return Err(Error::new(
                    "operation_pending",
                    "validation retry spacing has not elapsed",
                ));
            }
            request.max_turns = schedule.max_turns;
            child.reference.validation_retry_operation = Some(operation);
            self.persist_child(child)?;
            crate::runtime_workers::launch(self.runtime.clone(), request).await?;
        }
        if child.reference.validation != Some(operation) {
            ValidationRetryWork::require_history_capacity(
                child.reference.validation_history.len(),
            )?;
            child
                .reference
                .validation_history
                .push(child.reference.validation.ok_or_else(|| {
                    Error::new(
                        "record_corrupt",
                        "validation retry has no original operation",
                    )
                })?);
            child.reference.validation = Some(operation);
        }
        child.reference.validation_retry_operation = None;
        child.reference.stage = Stage::Validate;
        self.persist_child(child)
    }
    async fn launch_review_retry(&self, lease: &Lease, child: &mut ChildPipeline) -> Result<()> {
        let round = child.review_round.clone().ok_or_else(|| {
            Error::new(
                "review_round_unavailable",
                "retry requires its retained opaque review round",
            )
        })?;
        if child.reference.review_round_digest.as_deref() != Some(round.digest()) {
            return Err(Error::new("record_corrupt", "retry round proposal changed"));
        }
        let phase = round.subject_phase().clone();
        let operation = child
            .reference
            .review_retry_operation
            .unwrap_or_else(Uuid::new_v4);
        let model_admitted = match self.runtime.control.operation(operation) {
            Ok(receipt) => {
                let run = RunReference::from_receipt(&receipt)?.ok_or_else(|| {
                    Error::new(
                        "record_corrupt",
                        "review retry operation is not a model admission",
                    )
                })?;
                if run.phase != phase
                    || crate::control::ReviewRoundReference::from_receipt(&receipt)?.as_ref()
                        != Some(round.reference())
                {
                    return Err(Error::new(
                        "worker_binding",
                        "review retry receipt changed phase/round",
                    ));
                }
                self.worker_launch_admitted(operation)?
            }
            Err(error) if error.code == "record_missing" => false,
            Err(error) => return Err(error),
        };
        if !model_admitted {
            if self.worker_launch_admitted(operation)? {
                return Err(Error::new(
                    "reconciliation_required",
                    "review preparation exists without a known model admission; preserve original identity",
                ));
            }
            if self
                .current_review_grants(&child.reference.work)?
                .iter()
                .any(|g| g.phase() == &phase)
            {
                child.reference.stage = Stage::RetryReview;
                return self.persist_child(child);
            }
            let mut request = crate::runtime_workers::LaunchRequest {
                lease: lease.clone(),
                operation_id: operation,
                authority: self.current_delivery_worker_authority(round.authority())?,
                phase,
                source: None,
                delivery: Some(round.assignment().clone()),
                unfinished: None,
                source_advanced: None,
                retried_unfinished: None,
                validation_retry: None,
                review_retry: child.review_retry.clone(),
                review_round: Some(round),
                max_seconds: 3600,
                max_turns: self.runtime.configuration.snapshot()?.runtime.max_turns,
            };
            let schedule = self.runtime.next_attempt_for_review_round(&request)?;
            if schedule.not_before > Utc::now() {
                return Err(Error::new(
                    "operation_pending",
                    "review retry spacing has not elapsed",
                ));
            }
            request.max_turns = schedule.max_turns;
            child.reference.review_retry_operation = Some(operation);
            self.persist_child(child)?;
            crate::runtime_workers::launch(self.runtime.clone(), request).await?;
        }
        if child.reference.review != Some(operation) {
            child
                .reference
                .review_history
                .push(child.reference.review.ok_or_else(|| {
                    Error::new("record_corrupt", "review retry has no original operation")
                })?);
            child.reference.review = Some(operation);
        }
        self.observe_validation_successor(child, operation)?;
        child.reference.review_retry_operation = None;
        child.reference.stage = Stage::Review;
        self.persist_child(child)
    }
    pub(super) async fn launch_assignment(
        &self,
        lease: &Lease,
        operation_id: Uuid,
        assignment: DeliveryWorkerAssignment,
    ) -> Result<Uuid> {
        self.launch_assignment_with_source(lease, operation_id, assignment, None, None)
            .await
    }
    fn current_delivery_worker_authority(&self, expected: &FreshGate) -> Result<WorkerAuthority> {
        let expected = WorkerAuthority::Authorized(expected.clone());
        let current = self.runtime.current_authority(&expected)?;
        expected.compare(&current)?;
        Ok(current)
    }
    async fn launch_assignment_with_source(
        &self,
        lease: &Lease,
        operation_id: Uuid,
        assignment: DeliveryWorkerAssignment,
        source_advanced: Option<Arc<VerifiedSourceAdvanced>>,
        review_round: Option<Arc<VerifiedReviewRoundPlan>>,
    ) -> Result<Uuid> {
        let phase = if let Some(round) = &review_round {
            round.verify_assignment(&assignment)?;
            round.subject_phase().clone()
        } else if let Some(proof) = &source_advanced {
            proof.phase().clone()
        } else {
            PhaseKey {
                work: assignment.authority().work_key()?,
                generation: canonical_digest(1, &assignment.authority().work().generation)?,
                phase: assignment.phase().into(),
                retry_grant: None,
            }
        };
        let config = self.runtime.configuration.snapshot()?;
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == assignment.authority().workspace_id())
            .ok_or_else(|| Error::new("scope_violation", "assignment workspace disappeared"))?;
        let mut request = crate::runtime_workers::LaunchRequest {
            lease: lease.clone(),
            operation_id,
            authority: self.current_delivery_worker_authority(assignment.authority())?,
            phase,
            source: None,
            delivery: Some(assignment),
            unfinished: None,
            retried_unfinished: None,
            review_retry: None,
            validation_retry: None,
            source_advanced,
            review_round,
            max_seconds: 3600,
            max_turns: config.runtime.max_turns,
        };
        let schedule = if request.review_round.is_some() {
            self.runtime.next_attempt_for_review_round(&request)?
        } else if let Some(proof) = &request.source_advanced {
            proof.verify_assignment(request.delivery.as_ref().unwrap())?;
            self.runtime.next_attempt_for_source_advanced(proof)?
        } else {
            self.runtime.next_attempt(
                &request.phase,
                workspace.id,
                &config.connections[&workspace.codex_connection].account_id,
            )?
        };
        if schedule.not_before > Utc::now() {
            return Err(Error::new(
                "operation_pending",
                "delivery worker spacing has not elapsed",
            ));
        }
        request.max_turns = schedule.max_turns;
        crate::runtime_workers::launch(self.runtime.clone(), request).await
    }
    async fn advanced_assignment(
        &self,
        coordinator: &DeliveryCoordinator,
        admission: &DeliveryAdmission,
        child: &ChildPipeline,
        assignment: DeliveryWorkerAssignment,
    ) -> Result<(
        DeliveryWorkerAssignment,
        Option<Arc<VerifiedSourceAdvanced>>,
    )> {
        if let Some(proof) = &child.source_advanced
            && proof.phase().phase == assignment.phase()
        {
            proof.verify_assignment(&assignment)?;
            return Ok((proof.assignment().clone(), Some(proof.clone())));
        }
        if assignment.phase() == "child_local_validation"
            && let Some(ValidationRepairBasis {
                paired,
                basis,
                outcome: predecessor,
                completion,
            }) = self.validation_repair_basis(child).await?
        {
            let proof = coordinator
                .source_advanced_validated_repair(
                    SourceAdvancedRequest {
                        source: RepairSource {
                            admission,
                            original: child.claim.clone(),
                            source: required(&child.source, "sealed source")?,
                            completion: child.completion.clone(),
                        },
                        assignment: &assignment,
                        predecessor: &predecessor,
                        completion,
                    },
                    paired,
                    basis,
                    &self.current_review_grants(&child.reference.work)?,
                    &self.runtime,
                )
                .await?;
            return Ok((assignment, Some(Arc::new(proof))));
        }
        if assignment.phase() == "child_codex_review"
            && let Some(repair) = self.granted_review_repair(child).await?
        {
            let lineage = repair.granted_review().ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "granted review lineage disappeared",
                )
            })?;
            let receipts = self.runtime.control.operations()?;
            let grants = crate::runtime_workflow::current_retry_grants(
                &self.runtime.configuration.snapshot()?,
                &self.runtime.observations.admission_snapshot()?,
                repair.authority().work(),
                &receipts,
                &self.runtime.services.key,
            )?;
            let proof = coordinator
                .source_advanced_retried_review(
                    SourceAdvancedRequest {
                        source: RepairSource {
                            admission,
                            original: child.claim.clone(),
                            source: required(&child.source, "sealed source")?,
                            completion: child.completion.clone(),
                        },
                        assignment: &assignment,
                        predecessor: lineage.outcome(),
                        completion: lineage.completion().clone(),
                    },
                    &repair,
                    &grants,
                )
                .await?;
            return Ok((assignment, Some(Arc::new(proof))));
        }
        let phase = PhaseKey {
            work: assignment.authority().work_key()?,
            generation: canonical_digest(1, &assignment.authority().work().generation)?,
            phase: assignment.phase().into(),
            retry_grant: None,
        };
        let mut latest = None;
        for receipt in self.runtime.control.operations()? {
            if let Some(run) = RunReference::from_receipt(&receipt)?
                && run.phase == phase
                && latest
                    .as_ref()
                    .is_none_or(|(ordinal, _)| run.attempt > *ordinal)
            {
                latest = Some((run.attempt, receipt.operation_id));
            }
        }
        let Some((_, operation)) = latest else {
            return Ok((assignment, None));
        };
        let (predecessor, completion) = self.completed_claim(operation)?.ok_or_else(|| {
            Error::new(
                "operation_pending",
                "prior required check has no actual completed outcome",
            )
        })?;
        let request = SourceAdvancedRequest {
            source: RepairSource {
                admission,
                original: child.claim.clone(),
                source: required(&child.source, "sealed source")?,
                completion: child.completion.clone(),
            },
            assignment: &assignment,
            predecessor: &predecessor,
            completion,
        };
        let proof = if assignment.phase() == "child_local_validation" {
            match self.granted_review_repair(child).await? {
                Some(repair)
                    if matches!(
                        repair.granted_review(),
                        Some(CompletedReviewLineage::Round(_))
                    ) =>
                {
                    coordinator
                        .source_advanced_paired_validation(
                            request,
                            &repair,
                            &self.current_review_grants(&child.reference.work)?,
                        )
                        .await?
                }
                _ => coordinator.source_advanced(request).await?,
            }
        } else {
            coordinator.source_advanced(request).await?
        };
        Ok((assignment, Some(Arc::new(proof))))
    }
    /// The exact prior pipeline owns the consumed Findings/validation pair.
    /// Neither a phase label nor a decoded reference can select this lineage.
    async fn validation_repair_basis(
        &self,
        child: &ChildPipeline,
    ) -> Result<Option<ValidationRepairBasis>> {
        let candidates = self
            .children
            .pipelines
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "child pipeline lock failed"))?
            .iter()
            .filter(|(id, _)| **id != child.reference.operation)
            .map(|(_, value)| value.clone())
            .collect::<Vec<_>>();
        let mut selected = None;
        for candidate in candidates {
            let candidate = candidate.lock().await;
            if candidate.reference.repair_operation != Some(child.reference.operation) {
                continue;
            }
            if candidate.reference.work != child.reference.work {
                return Err(Error::new(
                    "validation_source_lineage",
                    "repair predecessor has another work identity",
                ));
            }
            let Some(paired) = &candidate.paired_round else {
                continue;
            };
            if candidate.reference.paired_round_digest.as_deref() != Some(paired.digest()) {
                return Err(Error::new(
                    "validation_source_lineage",
                    "paired repair plan changed",
                ));
            }
            let operation = candidate.reference.validation.ok_or_else(|| {
                Error::new(
                    "validation_source_lineage",
                    "paired source has no actual validator",
                )
            })?;
            let (outcome, completion) = self.completed_claim(operation)?.ok_or_else(|| {
                Error::new(
                    "operation_pending",
                    "paired validation predecessor has no actual completed result",
                )
            })?;
            let basis = if let Some(passed) = &candidate.validation_passed {
                if candidate.reference.validation_passed_digest.as_deref() != Some(passed.digest())
                    || passed.validation_outcome().digest() != outcome.digest()
                    || !Arc::ptr_eq(passed.validation_completion(), &completion)
                {
                    return Err(Error::new(
                        "validation_source_lineage",
                        "passed predecessor differs from exact actual validation",
                    ));
                }
                Some(ValidationAdvanceBasis::Passed(passed.clone()))
            } else if let Some(completed) = &candidate.validation_completed {
                if candidate.reference.validation_completed_digest.as_deref()
                    != Some(completed.digest())
                    || completed.outcome().digest() != outcome.digest()
                    || !Arc::ptr_eq(completed.completion(), &completion)
                {
                    return Err(Error::new(
                        "validation_source_lineage",
                        "completed retry predecessor changed",
                    ));
                }
                Some(ValidationAdvanceBasis::FailedChecks(completed.clone()))
            } else if let Some(previous) = &candidate.validation_source {
                if candidate.reference.validation_source_digest.as_deref()
                    != Some(previous.digest())
                {
                    return Err(Error::new(
                        "validation_source_lineage",
                        "validation source predecessor changed",
                    ));
                }
                previous
                    .has_validation_budget()
                    .then(|| ValidationAdvanceBasis::Advanced {
                        previous: previous.clone(),
                        outcome: Box::new(outcome.clone()),
                        completion: completion.clone(),
                    })
            } else {
                None
            };
            if let Some(basis) = basis
                && selected
                    .replace(ValidationRepairBasis {
                        paired: paired.clone(),
                        basis,
                        outcome,
                        completion,
                    })
                    .is_some()
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "validation source has multiple actual paired predecessors",
                ));
            }
        }
        Ok(selected)
    }
    async fn granted_review_repair(
        &self,
        child: &ChildPipeline,
    ) -> Result<Option<Arc<UnfinishedChildWork>>> {
        let candidates: Vec<_> = self
            .children
            .pipelines
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "child pipeline lock failed"))?
            .iter()
            .filter(|(operation, _)| **operation != child.reference.operation)
            .map(|(_, pipeline)| pipeline.clone())
            .collect();
        let mut found = None;
        for candidate in candidates {
            let candidate = candidate.lock().await;
            if candidate.reference.repair_operation != Some(child.reference.operation) {
                continue;
            }
            let repair = required(&candidate.unfinished, "original repair capability")?;
            if candidate.reference.work != child.reference.work
                || candidate.reference.unfinished_digest.as_deref() != Some(repair.digest())
            {
                return Err(Error::new(
                    "stale_result",
                    "repair pipeline lineage differs",
                ));
            }
            if repair.granted_review().is_some() && found.replace(repair.clone()).is_some() {
                return Err(Error::new(
                    "ownership_conflict",
                    "repair has multiple retained predecessors",
                ));
            }
        }
        Ok(found)
    }
    async fn paired_review_plan(
        &self,
        child: &ChildPipeline,
    ) -> Result<Option<Arc<VerifiedReviewRoundPlan>>> {
        let candidates: Vec<_> = self
            .children
            .pipelines
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "child pipeline lock failed"))?
            .iter()
            .filter(|(id, _)| **id != child.reference.operation)
            .map(|(_, value)| value.clone())
            .collect();
        let mut result = None;
        for candidate in candidates {
            let candidate = candidate.lock().await;
            if candidate.reference.repair_operation != Some(child.reference.operation) {
                continue;
            }
            if let Some(plan) = &candidate.paired_round
                && (candidate.reference.work != child.reference.work
                    || candidate.reference.paired_round_digest.as_deref() != Some(plan.digest())
                    || result.replace(plan.clone()).is_some())
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "paired review ancestry is ambiguous or changed",
                ));
            }
        }
        Ok(result)
    }
    fn retain_unfinished(
        &self,
        child: &mut ChildPipeline,
        proof: UnfinishedChildWork,
        error: Error,
    ) -> Result<()> {
        let digest = proof.digest().to_owned();
        let proof = Arc::new(proof);
        child.unfinished = Some(proof.clone());
        self.runtime.retain_unfinished_target(proof)?;
        child.reference.unfinished_digest = Some(digest);
        self.finish_child_phase(child, Stage::RepairLaunch)?;
        child.reference.error = Some(error);
        self.persist_child(child)
    }
    async fn launch_child_repair(&self, lease: &Lease, child: &mut ChildPipeline) -> Result<()> {
        let proof = child.unfinished.clone().ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "retained repair intent lacks actual unfinished-work capability",
            )
        })?;
        if child.reference.unfinished_digest.as_deref() != Some(proof.digest()) {
            return Err(Error::new(
                "record_corrupt",
                "repair intent differs from retained evidence",
            ));
        }
        if child.paired_round.is_none()
            && crate::control::ReviewRoundReference::from_receipt(
                &self.runtime.control.operation(proof.predecessor())?,
            )?
            .is_some()
        {
            return Err(Error::new(
                "review_round_continuation_unavailable",
                "further implementation within one pair requires exact retained pair/source continuation; original budgets and evidence remain held",
            ));
        }
        let operation = child
            .reference
            .repair_operation
            .unwrap_or_else(Uuid::new_v4);
        if self.worker_launch_admitted(operation)? {
            child.reference.stage = Stage::RepairWaiting;
            return self.persist_child(child);
        }
        let receipts = self.runtime.control.operations()?;
        let grants = crate::runtime_workflow::current_retry_grants(
            &self.runtime.configuration.snapshot()?,
            &self.runtime.observations.admission_snapshot()?,
            proof.authority().work(),
            &receipts,
            &self.runtime.services.key,
        )?;
        let matches: Vec<_> = grants
            .iter()
            .filter(|grant| grant.phase() == proof.phase())
            .collect();
        if child.paired_round.is_some() && !matches.is_empty() {
            return Err(Error::new(
                "review_round_unavailable",
                "paired repair cannot silently move to another implementation budget",
            ));
        }
        let (phase, assignment, unfinished, retried_unfinished) = match matches.as_slice() {
            [] => {
                let assignment = crate::delivery::repair_worker_assignment(&proof)?;
                proof.verify_assignment(&assignment)?;
                (proof.phase().clone(), assignment, Some(proof), None)
            }
            [grant] => {
                let workspace = self.workspace(
                    &self.runtime.observations.admission_snapshot()?,
                    proof.authority().workspace_id(),
                )?;
                let retried = Arc::new(workspace.coordinator.retry_unfinished(
                    proof.clone(),
                    (*grant).clone(),
                    &grants,
                )?);
                let assignment = crate::delivery::retried_repair_worker_assignment(&retried)?;
                retried.verify_assignment(&assignment)?;
                (retried.phase().clone(), assignment, None, Some(retried))
            }
            _ => {
                return Err(Error::new(
                    "ownership_conflict",
                    "multiple retries replace unfinished budget",
                ));
            }
        };
        let mut request = crate::runtime_workers::LaunchRequest {
            lease: lease.clone(),
            operation_id: operation,
            authority: self.current_delivery_worker_authority(assignment.authority())?,
            phase,
            source: None,
            delivery: Some(assignment),
            unfinished,
            retried_unfinished,
            review_retry: None,
            source_advanced: None,
            validation_retry: None,
            review_round: child.paired_round.clone(),
            max_seconds: 3600,
            max_turns: self.runtime.configuration.snapshot()?.runtime.max_turns,
        };
        let schedule = if request.review_round.is_some() {
            self.runtime.next_attempt_for_review_round(&request)?
        } else if let Some(retried) = &request.retried_unfinished {
            self.runtime.next_attempt_for_retried_unfinished(retried)?
        } else {
            self.runtime
                .next_attempt_for_unfinished(request.unfinished.as_ref().unwrap())?
        };
        if schedule.not_before > Utc::now() {
            return Err(Error::new(
                "operation_pending",
                "repair spacing has not elapsed",
            ));
        }
        request.max_turns = schedule.max_turns;
        child.reference.repair_operation = Some(operation);
        self.persist_child(child)?;
        crate::runtime_workers::launch(self.runtime.clone(), request).await?;
        child.reference.stage = Stage::RepairWaiting;
        self.persist_child(child)
    }
    pub(super) async fn candidate_children(
        &self,
        work: &crate::evidence::WorkIdentity,
        e2e: Uuid,
    ) -> Result<Vec<CandidateChild>> {
        let retained: Vec<_> = self
            .children
            .pipelines
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "child pipeline lock failed"))?
            .values()
            .cloned()
            .collect();
        let mut result = Vec::new();
        for retained in retained {
            let child = retained.lock().await;
            if child.reference.work.linear_endpoint != work.linear_endpoint
                || child.reference.work.linear_workspace != work.linear_workspace
                || child.reference.work.parent != work.parent
                || child.reference.work.generation != work.generation
            {
                continue;
            }
            let id = child
                .reference
                .work
                .child
                .ok_or_else(|| Error::invalid("child identity missing"))?;
            let complete = child.reference.stage == Stage::Complete && child.published.is_some();
            let changed = matches!(
                child.claim.claim().disposition,
                ChildClaimDisposition::Changed
            );
            let prepared_e2e = id == e2e
                && matches!(
                    child.reference.stage,
                    Stage::Publish | Stage::Close | Stage::Complete
                )
                && child.source.is_some()
                && child.local.is_some()
                && child.review.is_some()
                && if changed {
                    child.staged.is_some() && child.hosted.is_some()
                } else {
                    child.exact.is_some()
                };
            if complete || prepared_e2e {
                result.push(CandidateChild {
                    child: id,
                    published: child.published.as_deref().cloned(),
                    staged: child.staged.clone(),
                });
            }
        }
        Ok(result)
    }
    pub(super) async fn delivered_children(
        &self,
        work: &crate::evidence::WorkIdentity,
    ) -> Result<Vec<(PublishedChild, Option<StagedDelta>)>> {
        let pipelines: Vec<_> = self
            .children
            .pipelines
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "child pipeline lock failed"))?
            .values()
            .cloned()
            .collect();
        let mut result = Vec::new();
        for retained in pipelines {
            let child = retained.lock().await;
            if child.reference.stage == Stage::Complete
                && child.reference.work.parent == work.parent
                && child.reference.work.linear_workspace == work.linear_workspace
                && child.reference.work.generation == work.generation
                && let Some(published) = &child.published
            {
                result.push(((**published).clone(), child.staged.clone()));
            }
        }
        Ok(result)
    }
    fn block_terminal_child(&self, child: &mut ChildPipeline, error: &Error) -> Result<()> {
        if !terminal_gate_failure(error) && error.code != "review_rounds_exhausted" {
            return Ok(());
        }
        let Some(phase) = &child.phase else {
            return Ok(());
        };
        self.retire_delivery_phase(phase, json!({"delivery_phase":phase.admission.scope().phase,"outcome":"failed","error":error}))?;
        child.phase = None;
        child.reference.stage = Stage::Blocked;
        // Keep original phase_operation plus exact error as immutable failure
        // lineage. Blocked never creates a new run or grants a retry budget.
        Ok(())
    }
    pub(super) fn maintain_phase_setups(&self) -> Result<()> {
        let retained: Vec<_> = self
            .phase_holds
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "phase setup lock failed"))?
            .values()
            .cloned()
            .collect();
        for (permit, failed) in retained {
            if !failed.load(std::sync::atomic::Ordering::Acquire) {
                continue;
            }
            // A failed drop-time fence is not authority to retire. Close every
            // future spawn gate before observing an otherwise empty registry.
            self.runtime.control.fence(permit.operation_id)?;
            self.runtime
                .executions
                .observe_exited(permit.operation_id)?;
            // A never-spawned setup can retire immediately. Once a process
            // journal exists, only its original live capability can prove exit;
            // group-only recovery observations never clear ownership here.
            if !self
                .runtime
                .executions
                .active_operations()?
                .contains(&permit.operation_id)
                && self.processes.observe(permit.operation_id)?.is_empty()
            {
                self.runtime.control.retire_local(
                    permit.operation_id,
                    json!({"setup":"failed_before_spawn","local_exit":"no_spawn_record"}),
                )?;
                self.phase_holds
                    .lock()
                    .map_err(|_| Error::new("runtime_unavailable", "phase setup lock failed"))?
                    .remove(&permit.operation_id);
            }
        }
        Ok(())
    }
    pub(super) async fn maintain_child_phases(&self, projection: &RuntimeProjection) -> Result<()> {
        let _serial = self.children.advancing.lock().await;
        let retained: Vec<_> = self
            .children
            .pipelines
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "child pipeline lock failed"))?
            .values()
            .cloned()
            .collect();
        let mut reporting_error = None;
        for retained in retained {
            let mut child = retained.lock().await;
            let Some(phase) = &child.phase else {
                continue;
            };
            let lease = phase.permit.lease.clone();
            let result = if matches!(
                child.reference.stage,
                Stage::LaunchValidation | Stage::LaunchReview
            ) {
                let stage = child.reference.stage;
                self.finish_child_phase(&mut child, stage)
            } else {
                self.advance_one_child(&lease, projection, &mut child).await
            };
            if let Err(error) = result {
                if let Err(retirement) = self.block_terminal_child(&mut child, &error) {
                    reporting_error.get_or_insert(retirement);
                }
                child.reference.error = Some(error);
                if let Err(error) = self.persist_child(&child) {
                    reporting_error.get_or_insert(error);
                }
            }
        }
        reporting_error.map_or(Ok(()), Err)
    }
    pub(super) async fn advance_children(
        &self,
        lease: &Lease,
        projection: &RuntimeProjection,
    ) -> Result<()> {
        let _serial = self.children.advancing.lock().await;
        self.discover_children(projection)?;
        let retained: Vec<_> = self
            .children
            .pipelines
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "child pipeline lock failed"))?
            .values()
            .cloned()
            .collect();
        let mut reporting_error = None;
        for retained in retained {
            let mut child = retained.lock().await;
            // A preceding child may have spent a full bounded phase in Git or
            // provider I/O. New admissions use the latest complete observation.
            let result = async {
                self.runtime.registration.verify_active_owner(lease)?;
                let current = if child.phase.is_some() {
                    // Existing phases derive their actual gate and parent from
                    // the retained observation below, even on an invalid reload.
                    self.runtime.observations.snapshot()?
                } else {
                    self.runtime.observations.admission_snapshot()?
                };
                self.advance_one_child(lease, &current, &mut child).await
            }
            .await;
            if let Err(error) = result {
                if let Err(retirement) = self.block_terminal_child(&mut child, &error) {
                    reporting_error.get_or_insert(retirement);
                }
                child.reference.error = Some(error.clone());
                let reported = (|| {
                    self.persist_child(&child)?;
                    let key = crate::authority::IssueKey::new(
                        &child.reference.work.linear_endpoint,
                        child.reference.work.linear_workspace,
                        child.reference.work.parent,
                    )?;
                    self.runtime.invalidate_delivery(&key, error)
                })();
                if let Err(error) = reported {
                    reporting_error.get_or_insert(error);
                }
            }
        }
        reporting_error.map_or(Ok(()), Err)
    }
    async fn advance_one_child(
        &self,
        lease: &Lease,
        projection: &RuntimeProjection,
        child: &mut ChildPipeline,
    ) -> Result<()> {
        if matches!(
            child.reference.stage,
            Stage::Complete | Stage::Blocked | Stage::RepairWaiting
        ) {
            return Ok(());
        }
        let workspace_id = child
            .completion
            .contract
            .as_ref()
            .ok_or_else(|| Error::new("worker_binding", "original worker contract missing"))?
            .instructions()
            .manifest
            .workspace_id;
        let outcomes = self.runtime.dependency_outcomes()?;
        let parent = if let Some(phase) = &child.phase {
            self.runtime
                .observations
                .active_parent(phase.admission.fresh_gate(), &outcomes)?
        } else {
            projection
                .parent(workspace_id, child.reference.work.parent)?
                .clone()
        };
        let gate = if let Some(phase) = &child.phase {
            parent.retain_active_gate(
                phase.admission.fresh_gate(),
                &outcomes,
                Utc::now(),
                Duration::seconds(30),
            )?
        } else {
            parent.fresh_gate(
                child.reference.work.child,
                GatePurpose::NewOperation,
                &self.runtime.dependency_outcomes()?,
                Utc::now(),
                Duration::seconds(30),
            )?
        };
        if gate.work() != &child.reference.work {
            return Err(Error::new(
                "stale_result",
                "child delivery generation changed",
            ));
        }
        if child.reference.stage == Stage::RepairLaunch {
            let proof = child.unfinished.as_ref().ok_or_else(|| {
                Error::new("reconciliation_required", "repair proof is unavailable")
            })?;
            proof
                .authority()
                .compare_current(&gate, Utc::now(), Duration::seconds(30))?;
            return self.launch_child_repair(lease, child).await;
        }
        if child.reference.stage == Stage::LaunchReviewRetry {
            return self.launch_review_retry(lease, child).await;
        }
        if child.reference.stage == Stage::LaunchValidationRetry {
            return self.launch_validation_retry(lease, child).await;
        }
        let workspace = match &child.phase {
            Some(phase) => (*phase.workspace).clone(),
            None => self.workspace(projection, workspace_id)?,
        };
        let changed = matches!(
            child.claim.claim().disposition,
            ChildClaimDisposition::Changed
        );
        let phase = match child.reference.stage {
            Stage::Seal => DeliveryPhase::SourceSealing,
            Stage::LaunchValidation | Stage::Validate | Stage::RetryValidation => {
                DeliveryPhase::LocalValidation
            }
            Stage::PublishBranch | Stage::PullRequest => DeliveryPhase::PublishChild,
            Stage::Hosted => {
                if changed {
                    DeliveryPhase::HostedValidation
                } else {
                    DeliveryPhase::NoChange
                }
            }
            Stage::LaunchReview | Stage::Review | Stage::RetryReview => {
                if changed {
                    DeliveryPhase::ReviewPublication
                } else {
                    DeliveryPhase::NoChange
                }
            }
            Stage::IntegrateStaging | Stage::ObserveStaging | Stage::Publish | Stage::Close => {
                if changed {
                    DeliveryPhase::Staging
                } else {
                    DeliveryPhase::NoChange
                }
            }
            Stage::Complete
            | Stage::Blocked
            | Stage::RepairLaunch
            | Stage::RepairWaiting
            | Stage::LaunchReviewRetry
            | Stage::LaunchValidationRetry => {
                return Ok(());
            }
        };
        if child.reference.stage == Stage::Validate || child.reference.stage == Stage::Review {
            let operation = if child.reference.stage == Stage::Validate {
                child.reference.validation
            } else {
                child.reference.review
            };
            let operation = operation.ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "worker phase lacks its persisted launch identity",
                )
            })?;
            if let Some(completion) = self.runtime.retained_completion(operation)?
                && matches!(completion.outcome, crate::worker_run::RunOutcome::Failed)
            {
                let next = if child.reference.stage == Stage::Validate {
                    Stage::RetryValidation
                } else {
                    Stage::RetryReview
                };
                self.finish_child_phase(child, next)?;
                return self.persist_child(child);
            }
            if self.completed_claim(operation)?.is_none() {
                return Ok(());
            }
        }
        let pending_launch = match child.reference.stage {
            Stage::LaunchValidation => child.reference.validation.map(|id| (id, Stage::Validate)),
            Stage::LaunchReview => child.reference.review.map(|id| (id, Stage::Review)),
            _ => None,
        };
        if let Some((operation, next)) = pending_launch
            && self.worker_launch_admitted(operation)?
        {
            if next == Stage::Review {
                self.observe_validation_successor(child, operation)?;
            }
            child.reference.stage = next;
            return self.persist_child(child);
        }
        // The E2E responsibility can prepare/stage its test code before the
        // isolated vector exists, but its final completion waits for that vector.
        if child.reference.stage == Stage::Publish
            && parent
                .metadata
                .as_ref()
                .is_some_and(|m| Some(m.e2e_child) == child.reference.work.child)
            && !self
                .integrated_responsibility_ready(&workspace, &parent, &gate)
                .await?
        {
            return Ok(());
        }
        let review_grant = if child.reference.stage == Stage::RetryReview {
            let operation = child
                .reference
                .review
                .ok_or_else(|| Error::new("record_corrupt", "failed review identity missing"))?;
            let receipt = self.runtime.control.operation(operation)?;
            let run = RunReference::from_receipt(&receipt)?
                .ok_or_else(|| Error::new("worker_binding", "failed review admission missing"))?;
            let grants = self.current_review_grants(&child.reference.work)?;
            let matching: Vec<_> = grants
                .iter()
                .filter(|g| g.phase() == &run.phase)
                .cloned()
                .collect();
            match matching.as_slice() {
                [] if child
                    .review_round
                    .as_ref()
                    .is_some_and(|plan| plan.subject_phase() == &run.phase) =>
                {
                    Some((None, grants))
                }
                [] => {
                    return Err(Error::new(
                        "retry_required",
                        "known failed review awaits an explicit operator grant",
                    ));
                }
                [grant] => Some((Some(grant.clone()), grants)),
                _ => {
                    return Err(Error::new(
                        "stale_result",
                        "review budget has competing retry grants",
                    ));
                }
            }
        } else {
            None
        };
        let validation_grant = if child.reference.stage == Stage::RetryValidation {
            Some(self.validation_retry_grant(child)?)
        } else {
            None
        };
        self.phase(&workspace, &gate, lease, child, phase).await?;
        let current = child
            .phase
            .as_ref()
            .ok_or_else(|| Error::new("ownership_conflict", "delivery phase was not retained"))?;
        let coordinator = current.coordinator.clone();
        let admission = current.admission.clone();
        match child.reference.stage {
            Stage::Seal => {
                let execution = child.completion.execution.as_ref().ok_or_else(|| {
                    Error::new("worker_binding", "child execution is unavailable")
                })?;
                let isolation = child
                    .completion
                    .contract
                    .as_ref()
                    .unwrap()
                    .isolation()
                    .clone();
                let contract = workspace
                    .contracts
                    .iter()
                    .find(|c| c.repository == execution.binding.repository)
                    .unwrap();
                let git = current.git.clone();
                let source = PathBuf::from(&contract.configuration.source);
                let identity =
                    tokio::task::spawn_blocking(move || git.configured_commit_identity(&source))
                        .await
                        .map_err(|_| {
                            Error::new(
                                "operation_uncertain",
                                "commit identity observation did not join",
                            )
                        })??;
                let model = &child
                    .completion
                    .contract
                    .as_ref()
                    .unwrap()
                    .instructions()
                    .manifest
                    .profile
                    .model;
                let message = format!(
                    "feat: fulfill child responsibility\n\n{}\n\nInstruction-digest: {}\nAssisted-by: Codex ({})\nSigned-off-by: {} <{}>\n",
                    child.claim.claim().summary,
                    child
                        .completion
                        .contract
                        .as_ref()
                        .unwrap()
                        .instructions()
                        .manifest_digest,
                    model,
                    identity.name,
                    identity.email
                );
                let timestamp = chrono::DateTime::from_timestamp(Utc::now().timestamp(), 0)
                    .ok_or_else(|| Error::invalid("source commit time is outside range"))?;
                child.source = Some(
                    coordinator
                        .seal_child_source(
                            &admission,
                            &child.claim,
                            execution,
                            &isolation,
                            SourceCommitMetadata {
                                identity,
                                timestamp,
                                message,
                            },
                        )
                        .await?,
                );
                self.finish_child_phase(child, Stage::LaunchValidation)?;
            }
            Stage::LaunchValidation => {
                let assignment=coordinator.child_validation_assignment(&admission,&child.claim,required(&child.source,"sealed source")?,&child.completion,json!({"claim":child.claim.claim(),"purpose":"validate the exact sealed child source using configured local checks"}))?;
                let (assignment, advanced) = self
                    .advanced_assignment(&coordinator, &admission, child, assignment)
                    .await?;
                child.reference.source_advanced_digest =
                    advanced.as_ref().map(|p| p.digest().to_owned());
                child.validation_source = advanced.clone();
                child.reference.validation_source_digest =
                    advanced.as_ref().map(|p| p.digest().to_owned());
                child.reference.validation_assignment_digest = Some(assignment.digest()?);
                child.validation_assignment = Some(assignment.clone());
                child.source_advanced = advanced;
                self.finish_child_phase(child, Stage::LaunchValidation)?;
                let operation = child.reference.validation.unwrap_or_else(Uuid::new_v4);
                child.reference.validation = Some(operation);
                self.persist_child(child)?;
                self.launch_assignment_with_source(
                    lease,
                    operation,
                    assignment,
                    child.source_advanced.clone(),
                    None,
                )
                .await?;
                child.reference.stage = Stage::Validate;
                self.persist_child(child)?;
            }
            Stage::Validate => {
                let (outcome, completion) = self
                    .completed_claim(child.reference.validation.unwrap())?
                    .ok_or_else(|| {
                        Error::new("operation_pending", "local validation remains owned")
                    })?;
                let completed_retry = if let Some(proof) = &child.validation_retry {
                    self.require_validation_retry_identity(child, proof)?;
                    let grants = self.current_review_grants(&child.reference.work)?;
                    let completed = Arc::new(
                        coordinator
                            .completed_granted_validation(
                                &admission,
                                &outcome,
                                &completion,
                                proof,
                                &grants,
                            )
                            .await?,
                    );
                    let check = self.completed_validation_check(completed.clone());
                    check(admission.fresh_gate())?;
                    child.reference.validation_completed_digest = Some(completed.digest().into());
                    child.validation_completed = Some(completed.clone());
                    self.persist_child(child)?;
                    Some((completed, check))
                } else {
                    None
                };
                let result = coordinator
                    .advance_child(
                        &admission,
                        &child.claim,
                        &workspace.workflow,
                        ChildDeliveryAction::ValidateLocal {
                            validation: &outcome,
                            completion: &completion,
                        },
                    )
                    .await;
                if let Some((_, check)) = &completed_retry {
                    check(admission.fresh_gate())?;
                }
                let result = match result {
                    Ok(result) => result,
                    Err(error) if terminal_gate_failure(&error) => {
                        let request = ValidationRepairRequest {
                            source: RepairSource {
                                admission: &admission,
                                original: child.claim.clone(),
                                source: required(&child.source, "sealed source")?,
                                completion: child.completion.clone(),
                            },
                            workflow: &workspace.workflow,
                            outcome: &outcome,
                            completion: completion.clone(),
                        };
                        if let Some(previous) = self.paired_review_plan(child).await? {
                            let validation_source = child.validation_source.clone().ok_or_else(|| Error::new(
                                "review_round_lineage", "continued pair lacks its actual validation source capability"))?;
                            let grants = self.current_review_grants(&child.reference.work)?;
                            let config = self.runtime.configuration.snapshot()?;
                            let continued = match &completed_retry {
                                Some((completed, _)) => {
                                    coordinator
                                        .continue_paired_repair_after_granted_validation(
                                            request,
                                            previous,
                                            completed.clone(),
                                            &config,
                                            &grants,
                                        )
                                        .await
                                }
                                None => {
                                    coordinator
                                        .continue_paired_repair(
                                            request,
                                            previous,
                                            validation_source,
                                            &config,
                                            &grants,
                                        )
                                        .await
                                }
                            };
                            if let Some((_, check)) = &completed_retry {
                                check(admission.fresh_gate())?;
                            }
                            let (proof, next_pair) = match continued {
                                Ok(value) => value,
                                // A missing required check is a known terminal
                                // validation failure, not unfinished work proof.
                                // Preserve that original error so the existing
                                // outer retirement path can drain this phase.
                                Err(unavailable) if unavailable.code == "no_work_trigger" => {
                                    return Err(error);
                                }
                                Err(unavailable) => return Err(unavailable),
                            };
                            child.reference.paired_round_digest = Some(next_pair.digest().into());
                            child.paired_round = Some(Arc::new(next_pair));
                            return self.retain_unfinished(child, (*proof).clone(), error);
                        }
                        let proof = coordinator.unfinished_validation(request).await;
                        if let Some((_, check)) = &completed_retry {
                            check(admission.fresh_gate())?;
                        }
                        return match proof {
                            Ok(proof) => self.retain_unfinished(child, proof, error),
                            Err(unavailable) if unavailable.code == "no_work_trigger" => Err(error),
                            Err(unavailable) => Err(unavailable),
                        };
                    }
                    Err(error) => return Err(error),
                };
                let ChildDeliveryProgress::Local(local) = result else {
                    return Err(Error::new(
                        "delivery_protocol",
                        "local validation returned another phase",
                    ));
                };
                if let Some((completed, _)) = &completed_retry {
                    let passed = Arc::new(
                        coordinator
                            .passed_granted_validation(
                                RepairSource {
                                    admission: &admission,
                                    original: child.claim.clone(),
                                    source: required(&child.source, "sealed source")?,
                                    completion: child.completion.clone(),
                                },
                                &workspace.workflow,
                                completed.clone(),
                                &local,
                                &self.runtime,
                            )
                            .await?,
                    );
                    child.reference.validation_passed_digest = Some(passed.digest().into());
                    child.validation_passed = Some(passed);
                } else if let Some(advanced) = &child.validation_source
                    && advanced.has_validation_budget()
                {
                    let passed = Arc::new(
                        coordinator
                            .passed_source_validation(
                                RepairSource {
                                    admission: &admission,
                                    original: child.claim.clone(),
                                    source: required(&child.source, "sealed source")?,
                                    completion: child.completion.clone(),
                                },
                                &workspace.workflow,
                                advanced.clone(),
                                (&outcome, completion.clone()),
                                &local,
                                &self.runtime,
                            )
                            .await?,
                    );
                    child.reference.validation_passed_digest = Some(passed.digest().into());
                    child.validation_passed = Some(passed);
                }
                child.local = Some(local);
                self.finish_child_phase(
                    child,
                    if changed {
                        Stage::PublishBranch
                    } else {
                        Stage::Hosted
                    },
                )?;
            }
            Stage::PublishBranch => {
                let issue = parent
                    .children
                    .get(&child.reference.work.child.unwrap())
                    .and_then(|c| c.issue.as_ref())
                    .ok_or_else(|| Error::new("incomplete_snapshot", "child metadata missing"))?;
                let title = &parent
                    .issue
                    .as_ref()
                    .ok_or_else(|| Error::new("incomplete_snapshot", "parent metadata missing"))?
                    .title;
                let git = current.git.clone();
                let identifier = issue.identifier.clone();
                let title = title.clone();
                let branch =
                    tokio::task::spawn_blocking(move || git.branch_name(&identifier, &title, 255))
                        .await
                        .map_err(|_| {
                            Error::new("operation_uncertain", "branch validation did not join")
                        })??;
                let result = coordinator
                    .advance_child(
                        &admission,
                        &child.claim,
                        &workspace.workflow,
                        ChildDeliveryAction::PublishBranch {
                            source: required(&child.source, "sealed source")?,
                            local: required(&child.local, "local checks")?,
                            branch: &branch,
                            previous: None,
                        },
                    )
                    .await?;
                let ChildDeliveryProgress::Branch(branch) = result else {
                    return Err(Error::new(
                        "delivery_protocol",
                        "branch publication returned another phase",
                    ));
                };
                child.branch = Some(branch);
                self.finish_child_phase(child, Stage::PullRequest)?;
            }
            Stage::PullRequest => {
                let title = &parent
                    .issue
                    .as_ref()
                    .ok_or_else(|| Error::new("incomplete_snapshot", "parent metadata missing"))?
                    .title;
                let result = coordinator
                    .advance_child(
                        &admission,
                        &child.claim,
                        &workspace.workflow,
                        ChildDeliveryAction::OpenPullRequest {
                            branch: required(&child.branch, "published branch")?,
                            local: required(&child.local, "local checks")?,
                            title,
                        },
                    )
                    .await?;
                let ChildDeliveryProgress::PullRequest(pr) = result else {
                    return Err(Error::new(
                        "delivery_protocol",
                        "PR publication returned another phase",
                    ));
                };
                child.pr = Some(pr);
                self.finish_child_phase(child, Stage::Hosted)?;
            }
            Stage::Hosted => {
                let verifier = self.hosted_verifier(&workspace.configuration).await?;
                let result = if changed {
                    coordinator
                        .acquire_child_validation(&admission, &verifier)
                        .await
                        .map(|hosted| child.hosted = Some(hosted))
                } else {
                    if child.exact_pending.is_none() {
                        child.exact_pending =
                            Some(coordinator.dispatch_exact_validation(&admission).await?);
                    }
                    coordinator
                        .acquire_exact_validation(
                            &admission,
                            child.exact_pending.as_ref().unwrap(),
                            &verifier,
                        )
                        .await
                        .map(|exact| child.exact = Some(exact))
                };
                match result {
                    Ok(()) => self.finish_child_phase(child, Stage::LaunchReview)?,
                    Err(error) if error.code == "operation_pending" => return Ok(()),
                    Err(error) => return Err(error),
                }
            }
            Stage::LaunchReview => {
                let target = if changed {
                    ChildReviewTarget::Changed {
                        change: required(&child.source, "sealed source")?
                            .change()
                            .ok_or_else(|| {
                                Error::new(
                                    "candidate_provenance",
                                    "changed claim has no actual patch",
                                )
                            })?,
                        pull_request: required(&child.pr, "pull request")?,
                    }
                } else {
                    ChildReviewTarget::NoChange {
                        source: required(&child.source, "sealed source")?,
                    }
                };
                let review=coordinator.prepare_child_review(&admission,&child.claim,target,json!({"claim":child.claim.claim(),"purpose":"review the exact isolated child contribution and prepared criteria"}))?;
                let assignment = coordinator.child_review_worker_assignment(
                    &admission,
                    &child.claim,
                    required(&child.source, "sealed source")?,
                    &child.completion,
                    &review,
                )?;
                child.assignment = Some(review);
                let (assignment, advanced) = self
                    .advanced_assignment(&coordinator, &admission, child, assignment)
                    .await?;
                child.reference.source_advanced_digest =
                    advanced.as_ref().map(|p| p.digest().to_owned());
                child.source_advanced = advanced;
                let grants = self.current_review_grants(&child.reference.work)?;
                let input = ReviewRoundSource {
                    admission: &admission,
                    claim: child.claim.clone(),
                    source: required(&child.source, "sealed source")?,
                    completion: child.completion.clone(),
                    assignment: required(&child.assignment, "review assignment")?,
                    validated_retry: child.validation_passed.clone(),
                };
                let plan = if let Some(advanced) = &child.source_advanced {
                    let paired = self.paired_review_plan(child).await?.ok_or_else(|| {
                        Error::new(
                            "review_round_unavailable",
                            "changed review source lacks genuine paired-round ancestry",
                        )
                    })?;
                    coordinator
                        .after_repair_review_round(
                            input,
                            paired,
                            advanced.clone(),
                            &workspace.configuration,
                            &grants,
                        )
                        .await?
                } else {
                    coordinator
                        .initial_review_round(input, &workspace.configuration, &grants)
                        .await?
                };
                plan.verify_assignment(&assignment)?;
                child.reference.review_round_digest = Some(plan.digest().into());
                child.review_round = Some(Arc::new(plan));
                child.review_worker_assignment = Some(assignment.clone());
                self.finish_child_phase(child, Stage::LaunchReview)?;
                let operation = child.reference.review.unwrap_or_else(Uuid::new_v4);
                child.reference.review = Some(operation);
                self.persist_child(child)?;
                self.launch_assignment_with_source(
                    lease,
                    operation,
                    assignment,
                    child.source_advanced.clone(),
                    child.review_round.clone(),
                )
                .await?;
                self.observe_validation_successor(child, operation)?;
                child.reference.stage = Stage::Review;
                self.persist_child(child)?;
            }
            Stage::RetryValidation => {
                let (grant, grants) = validation_grant.ok_or_else(|| {
                    Error::new("stale_result", "validation grant selection is unavailable")
                })?;
                let operation = child.reference.validation.ok_or_else(|| {
                    Error::new("record_corrupt", "failed validation operation is missing")
                })?;
                let failed = self
                    .runtime
                    .retained_completion(operation)?
                    .ok_or_else(|| {
                        Error::new(
                            "reconciliation_required",
                            "actual joined failed validation is unavailable",
                        )
                    })?;
                let assignment = required(
                    &child.validation_assignment,
                    "original validation assignment",
                )?;
                if child.reference.validation_assignment_digest.as_deref()
                    != Some(assignment.digest()?.as_str())
                    || child.reference.validation_source_digest.as_deref()
                        != child.validation_source.as_ref().map(|p| p.digest())
                {
                    return Err(Error::new(
                        "record_corrupt",
                        "retained validation assignment/source changed",
                    ));
                }
                let proof = match grant {
                    Some(grant) => {
                        coordinator
                            .retry_failed_validation(ValidationRetryRequest {
                                admission: &admission,
                                assignment,
                                origin: match &child.validation_retry {
                                    Some(prior) => ValidationRetryOrigin::Retried(prior.clone()),
                                    None => ValidationRetryOrigin::Original {
                                        source_advanced: child.validation_source.clone(),
                                    },
                                },
                                failed,
                                grant,
                                current_grants: &grants,
                            })
                            .await?
                    }
                    None => {
                        coordinator
                            .continue_failed_validation(
                                &admission,
                                required(&child.validation_retry, "previous validation retry")?,
                                failed,
                                &grants,
                            )
                            .await?
                    }
                };
                proof.verify_current(
                    &self.runtime.control.operations()?,
                    &self.current_review_grants(&child.reference.work)?,
                )?;
                proof.verify_joined(&|id| self.runtime.retained_completion(id))?;
                child.reference.validation_retry_digest = Some(proof.digest().into());
                child.validation_retry = Some(Arc::new(proof));
                self.finish_child_phase(child, Stage::LaunchValidationRetry)?;
            }
            Stage::RetryReview => {
                let (grant, grants) = review_grant
                    .ok_or_else(|| Error::new("stale_result", "review grant missing"))?;
                let completion = self
                    .runtime
                    .retained_completion(child.reference.review.unwrap())?
                    .ok_or_else(|| {
                        Error::new(
                            "reconciliation_required",
                            "actual failed review completion missing",
                        )
                    })?;
                let (round, retry) = match grant {
                    Some(grant) => {
                        let proof = Arc::new(
                            coordinator
                                .retry_failed_review(ReviewRetryRequest {
                                    admission: &admission,
                                    assignment: required(
                                        &child.review_worker_assignment,
                                        "original review assignment",
                                    )?,
                                    failed: completion,
                                    grant,
                                    current_grants: &grants,
                                })
                                .await?,
                        );
                        let plan = coordinator
                            .granted_review_round(
                                ReviewRoundSource {
                                    admission: &admission,
                                    claim: child.claim.clone(),
                                    source: required(&child.source, "sealed source")?,
                                    completion: child.completion.clone(),
                                    assignment: required(&child.assignment, "review assignment")?,
                                    validated_retry: child.validation_passed.clone(),
                                },
                                &workspace.configuration,
                                proof.clone(),
                                &grants,
                            )
                            .await?;
                        (plan, Some(proof))
                    }
                    None => (
                        coordinator.transient_review_round(
                            required(&child.review_round, "previous genuine round plan")?.clone(),
                            completion,
                            &workspace.configuration,
                            &grants,
                        )?,
                        None,
                    ),
                };
                child.reference.review_round_digest = Some(round.digest().into());
                child.review_worker_assignment = Some(round.assignment().clone());
                child.review_round = Some(Arc::new(round));
                child.reference.review_retry_digest = retry.as_ref().map(|p| p.digest().into());
                child.review_retry = retry;
                child.source_advanced = None;
                child.reference.source_advanced_digest = None;
                self.finish_child_phase(child, Stage::LaunchReviewRetry)?;
                self.persist_child(child)?;
            }
            Stage::Review => {
                let (outcome, completion) = self
                    .completed_claim(child.reference.review.unwrap())?
                    .ok_or_else(|| Error::new("operation_pending", "Codex review remains owned"))?;
                let grants = self.current_review_grants(&child.reference.work)?;
                let completed = Arc::new(
                    coordinator
                        .completed_review_round(
                            &admission,
                            required(&child.assignment, "review assignment")?,
                            required(&child.review_round, "actual admitted review round")?.clone(),
                            outcome.clone(),
                            completion.clone(),
                            &grants,
                        )
                        .await?,
                );
                let checked = completed.clone();
                let runtime = self.runtime.clone();
                let current: GateCheck = Arc::new(move |_| {
                    let receipts = runtime.control.operations()?;
                    let grants = crate::runtime_workflow::current_retry_grants(
                        &runtime.configuration.snapshot()?,
                        &runtime.observations.admission_snapshot()?,
                        checked.plan().authority().work(),
                        &receipts,
                        &runtime.services.key,
                    )?;
                    checked.verify_current(&receipts, &grants)?;
                    checked.verify_joined(&|id| runtime.retained_completion(id))
                });
                let published = coordinator
                    .publish_completed_round(
                        &admission,
                        required(&child.assignment, "review assignment")?,
                        &completed,
                        current,
                    )
                    .await;
                child.review = Some(match published {
                    Ok(review) => review,
                    Err(error) if terminal_gate_failure(&error) => {
                        let request = ReviewRepairRequest {
                            source: RepairSource {
                                admission: &admission,
                                original: child.claim.clone(),
                                source: required(&child.source, "sealed source")?,
                                completion: child.completion.clone(),
                            },
                            assignment: required(&child.assignment, "review assignment")?,
                            outcome: &outcome,
                            completion: completion.clone(),
                        };
                        let proof = coordinator
                            .unfinished_completed_round(request, completed.clone(), &grants)
                            .await;
                        return match proof {
                            Ok(proof) => {
                                let proof = Arc::new(proof);
                                let pair = coordinator.paired_review_repair(
                                    completed,
                                    proof.clone(),
                                    &workspace.configuration,
                                    &grants,
                                )?;
                                child.reference.paired_round_digest = Some(pair.digest().into());
                                child.paired_round = Some(Arc::new(pair));
                                self.retain_unfinished(child, (*proof).clone(), error)
                            }
                            Err(unavailable) if unavailable.code == "no_work_trigger" => Err(error),
                            Err(unavailable) => Err(unavailable),
                        };
                    }
                    Err(error) => return Err(error),
                });
                self.finish_child_phase(
                    child,
                    if changed {
                        Stage::IntegrateStaging
                    } else {
                        Stage::Publish
                    },
                )?;
            }
            Stage::IntegrateStaging => {
                let source = required(&child.source, "sealed source")?;
                child.pending = Some(
                    coordinator
                        .request_staging_merge(
                            &admission,
                            StagingGates {
                                proposal: source.change().ok_or_else(|| {
                                    Error::new(
                                        "candidate_provenance",
                                        "staging needs actual changed source",
                                    )
                                })?,
                                pull_request: required(&child.pr, "pull request")?,
                                local: required(&child.local, "local checks")?,
                                hosted: required(&child.hosted, "hosted checks")?,
                                review: required(&child.review, "published Codex review")?,
                            },
                        )
                        .await?,
                );
                // Retain the same live admission across actual merge observation.
                child.reference.stage = Stage::ObserveStaging;
                self.persist_child(child)?;
            }
            Stage::ObserveStaging => {
                let source = required(&child.source, "sealed source")?;
                let objects = coordinator
                    .acquire_staging_source(
                        &admission,
                        required(&child.pr, "pull request")?,
                        required(&child.pending, "pending staging receipt")?,
                        self.processes.clone(),
                        self.runtime.executions.clone(),
                    )
                    .await?;
                child.staged = Some(
                    coordinator
                        .observe_staging_merge(
                            &admission,
                            objects.root(),
                            source.change().ok_or_else(|| {
                                Error::new("candidate_provenance", "staging lacks patch")
                            })?,
                            required(&child.pr, "pull request")?,
                            required(&child.pending, "pending staging receipt")?,
                        )
                        .await?,
                );
                self.finish_child_phase(child, Stage::Publish)?;
            }
            Stage::Publish => {
                let action = if changed {
                    ChildDeliveryAction::PublishChanged {
                        local: required(&child.local, "local checks")?,
                        hosted: required(&child.hosted, "hosted checks")?,
                        review: required(&child.review, "review")?,
                        staged: required(&child.staged, "staging")?,
                    }
                } else {
                    ChildDeliveryAction::PublishNoChange {
                        local: required(&child.local, "local checks")?,
                        exact: required(&child.exact, "exact validation")?,
                        review: required(&child.review, "review")?,
                    }
                };
                let result = coordinator
                    .advance_child(&admission, &child.claim, &workspace.workflow, action)
                    .await?;
                let ChildDeliveryProgress::Published(published) = result else {
                    return Err(Error::new(
                        "delivery_protocol",
                        "completion returned another phase",
                    ));
                };
                child.published = Some(published);
                self.finish_child_phase(child, Stage::Close)?;
            }
            Stage::Close => {
                coordinator
                    .close_child(
                        &admission,
                        required(&child.published, "canonical child publication")?,
                        &workspace.configuration,
                    )
                    .await?;
                self.finish_child_phase(child, Stage::Complete)?;
            }
            Stage::Complete
            | Stage::Blocked
            | Stage::RepairLaunch
            | Stage::RepairWaiting
            | Stage::LaunchReviewRetry
            | Stage::LaunchValidationRetry => {}
        }
        Ok(())
    }
    pub(super) async fn hosted_verifier(
        &self,
        configuration: &Config,
    ) -> Result<crate::providers::HostedProofVerifier> {
        let timeout = std::time::Duration::from_millis(configuration.runtime.request_timeout_ms);
        if let Some(transport) = self.runtime.services.inputs.transport("github_oidc") {
            crate::providers::HostedProofVerifier::fetch_with_transport(timeout, transport.as_ref())
                .await
        } else {
            crate::providers::HostedProofVerifier::fetch(timeout).await
        }
    }
}
pub(super) fn terminal_gate_failure(error: &Error) -> bool {
    matches!(
        error.code.as_str(),
        "validation_failed"
            | "validation_gate"
            | "review_failed"
            | "review_gate"
            | "integrated_gate"
    )
}
fn required<'a, T>(value: &'a Option<T>, name: &str) -> Result<&'a T> {
    value.as_ref().ok_or_else(|| {
        Error::new(
            "reconciliation_required",
            format!("delivery phase lacks actual retained {name}"),
        )
    })
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    #[test]
    fn absent_repair_fields_preserve_legacy_signed_pipeline_payload() {
        let digest = crate::digest::bytes_digest(b"legacy pipeline");
        let value = json!({"schema_version":1,"runtime":Uuid::from_u128(1),"operation":Uuid::from_u128(7),"proposal":Uuid::from_u128(8),"claim_digest":digest,
            "work":{"linear_endpoint":"https://linear.invalid/graphql","linear_workspace":Uuid::from_u128(2),"parent":Uuid::from_u128(10),"child":Uuid::from_u128(11),"generation":{"authorization":digest,"definition":digest,"correction":digest,"scope":digest}},
            "stage":"blocked","phase_operation":null,"validation":null,"review":null,"observed_at":"2026-01-04T00:00:00Z","error":null});
        let key = crate::evidence::AttestationKey::new("fixture".into(), vec![7; 32]).unwrap();
        let signed = key.sign("runtime_child_pipeline", &value).unwrap();
        let typed: crate::evidence::Signed<ChildReference> =
            serde_json::from_value(serde_json::to_value(signed).unwrap()).unwrap();
        key.verify("runtime_child_pipeline", &typed).unwrap();
        assert_eq!(
            canonical_digest(1, &value).unwrap(),
            canonical_digest(1, &typed.payload).unwrap()
        );
    }
    #[tokio::test]
    async fn failed_never_spawned_setup_retires_after_guard_drop_but_live_setup_is_preserved() {
        let (_root, fixture, service) = super::super::tests::fixture().await;
        let lease = super::super::tests::start_control(&fixture.runtime);
        let operation = Uuid::new_v4();
        let permit = fixture
            .runtime
            .control
            .admit(
                &lease,
                operation,
                "setup-fixture".into(),
                canonical_digest(1, &"setup").unwrap(),
                || Ok(()),
            )
            .unwrap();
        let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        service
            .phase_holds
            .lock()
            .unwrap()
            .insert(operation, (permit.clone(), failed.clone()));
        let guard = PhaseSetup {
            runtime: fixture.runtime.clone(),
            permit,
            armed: true,
            failed,
        };
        service.maintain_phase_setups().unwrap();
        assert_eq!(
            fixture
                .runtime
                .control
                .snapshot()
                .unwrap()
                .active_operations
                .len(),
            1
        );
        drop(guard);
        assert!(
            fixture
                .runtime
                .control
                .snapshot()
                .unwrap()
                .fenced_operations
                .contains(&operation)
        );
        service.maintain_phase_setups().unwrap();
        assert!(
            fixture
                .runtime
                .control
                .snapshot()
                .unwrap()
                .active_operations
                .is_empty()
        );
        assert!(service.phase_holds.lock().unwrap().is_empty());
        assert!(fixture.processes.operations().unwrap().is_empty());
    }
    #[tokio::test]
    async fn failed_setup_maintenance_fences_before_observing_any_owned_exit() {
        struct Probe {
            host: Arc<crate::control::ControlHost>,
            operation: Uuid,
            checked: Arc<std::sync::atomic::AtomicBool>,
        }
        impl crate::execution::CommandCancellation for Probe {
            fn cancel(&self) -> Result<()> {
                Ok(())
            }
            fn exited(&self) -> Result<bool> {
                if !self
                    .host
                    .snapshot()?
                    .fenced_operations
                    .contains(&self.operation)
                {
                    return Err(Error::new(
                        "fixture_order",
                        "exit observed before durable fence",
                    ));
                }
                self.checked
                    .store(true, std::sync::atomic::Ordering::Release);
                Ok(true)
            }
        }
        let (_root, fixture, service) = super::super::tests::fixture().await;
        let lease = super::super::tests::start_control(&fixture.runtime);
        let operation = Uuid::new_v4();
        let permit = fixture
            .runtime
            .control
            .admit(
                &lease,
                operation,
                "failed-fence-fixture".into(),
                canonical_digest(1, &"setup").unwrap(),
                || Ok(()),
            )
            .unwrap();
        let checked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        fixture
            .runtime
            .executions
            .register(
                operation,
                Uuid::new_v4(),
                Arc::new(Probe {
                    host: fixture.runtime.control.clone(),
                    operation,
                    checked: checked.clone(),
                }),
            )
            .unwrap();
        service.phase_holds.lock().unwrap().insert(
            operation,
            (permit, Arc::new(std::sync::atomic::AtomicBool::new(true))),
        );
        assert!(
            !fixture
                .runtime
                .control
                .snapshot()
                .unwrap()
                .fenced_operations
                .contains(&operation)
        );
        service.maintain_phase_setups().unwrap();
        assert!(checked.load(std::sync::atomic::Ordering::Acquire));
        assert!(
            fixture
                .runtime
                .control
                .snapshot()
                .unwrap()
                .active_operations
                .is_empty()
        );
    }
    #[tokio::test]
    async fn reporting_failure_keeps_actual_child_claim_and_completion_in_pipeline() {
        let (_root, fixture, service, _wire, owner) =
            super::super::tests::fixture_authorization_with_wire(false).await;
        let (lease, _ownership) = owner.start(&fixture.runtime);
        let projection = fixture.runtime.observations.snapshot().unwrap();
        let parent = projection
            .workspaces
            .values()
            .next()
            .unwrap()
            .parents
            .values()
            .next()
            .unwrap();
        let mut work = parent.work.clone().unwrap();
        work.child = Some(*parent.required_children.iter().next().unwrap());
        let (_claim_root, outcome, completion) = crate::runtime_claims::test_fixture_with_claim(
            crate::instructions::AgentRole::Implementation,
            work.clone(),
            json!({"task":"retention fault fixture"}),
            None,
            json!({"schema_version":1,"summary":"fixture claim","fulfilled_criteria":["fixture criterion"],"disposition":{"kind":"no_change","reason":"fixture responsibility requires no source change"}}),
        );
        let gate = outcome.original_authority().clone();
        let workspace = service.workspace(&projection, parent.workspace_id).unwrap();
        let coordinator = DeliveryCoordinator::new(
            DeliveryServices {
                host: service.runtime.control.clone(),
                broker: service
                    .provider_broker(&workspace.configuration, parent.workspace_id)
                    .unwrap(),
                records: workspace.records.clone(),
                key: service.runtime.services.key.clone(),
                git: workspace.git.clone(),
                retained_git_sources: BTreeMap::new(),
                current_gate: Arc::new(|_| Ok(())),
                executions: service.runtime.executions.clone(),
                processes: service.processes.clone(),
            },
            workspace.contracts.clone(),
        )
        .unwrap();
        let claim = Arc::new(
            coordinator
                .accept_child_completion(&gate, outcome.clone(), completion.clone())
                .unwrap(),
        );
        let operation = claim.operation_id();
        let pipeline = Arc::new(tokio::sync::Mutex::new(ChildPipeline {
            reference: ChildReference {
                schema_version: 1,
                runtime: service.runtime.configuration.snapshot().unwrap().runtime.id,
                operation,
                proposal: outcome.outcome().proposal_id,
                claim_digest: claim.digest().into(),
                work,
                stage: Stage::Seal,
                phase_operation: None,
                validation: None,
                validation_assignment_digest: None,
                validation_source_digest: None,
                validation_retry_operation: None,
                validation_retry_digest: None,
                validation_completed_digest: None,
                validation_passed_digest: None,
                validation_successor_digest: None,
                validation_history: vec![],
                review: None,
                repair_operation: None,
                unfinished_digest: None,
                source_advanced_digest: None,
                review_retry_operation: None,
                review_retry_digest: None,
                review_history: vec![],
                review_round_digest: None,
                paired_round_digest: None,
                observed_at: Utc::now(),
                error: None,
            },
            claim: claim.clone(),
            completion: completion.clone(),
            source: None,
            local: None,
            branch: None,
            pr: None,
            hosted: None,
            exact_pending: None,
            exact: None,
            assignment: None,
            review: None,
            pending: None,
            staged: None,
            published: None,
            phase: None,
            unfinished: None,
            source_advanced: None,
            validation_assignment: None,
            validation_source: None,
            validation_retry: None,
            validation_completed: None,
            validation_passed: None,
            validation_successor: None,
            review_worker_assignment: None,
            review_retry: None,
            review_round: None,
            paired_round: None,
        }));
        service
            .children
            .pipelines
            .lock()
            .unwrap()
            .insert(operation, pipeline.clone());
        // Replace only this test's owned protected directory. The immutable
        // PrivateDir detects its changed inode when reporting the phase failure.
        let path = service.records.path();
        let displaced = path.with_extension("retained");
        std::fs::rename(path, &displaced).unwrap();
        std::fs::create_dir(path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            service
                .advance_children(&lease, &projection)
                .await
                .unwrap_err()
                .code,
            "ownership_conflict"
        );
        let retained = service
            .children
            .pipelines
            .lock()
            .unwrap()
            .get(&operation)
            .unwrap()
            .clone();
        assert!(Arc::ptr_eq(&retained, &pipeline));
        let retained = retained.lock().await;
        assert!(Arc::ptr_eq(&retained.claim, &claim));
        assert!(Arc::ptr_eq(&retained.completion, &completion));
        assert_eq!(
            retained.reference.error.as_ref().unwrap().code,
            "worker_binding"
        );
        assert!(
            fixture
                .runtime
                .control
                .snapshot()
                .unwrap()
                .active_operations
                .is_empty()
        );
    }
}

#[cfg(test)]
#[path = "runtime_delivery_continuation_tests.rs"]
mod continuation_tests;
