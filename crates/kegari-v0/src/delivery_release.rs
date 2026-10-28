//! Production preparation and exact, recoverable delivery effects. The runtime
//! queue owns release admission; this adapter never manufactures a release grant.
use super::*;
use crate::deployment::{ProductionObserver, TargetDeploymentChallenge, VerifiedTargetDeployment};
use crate::release::{Dispatch, ObservedEffect, OwnedRun, ReleasePlan, ResourceKey, StepKind};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductionSource {
    record: Signed<ChildResultRecord>,
    publication: Publication,
}

/// Mandatory runtime callback checks its serialized release queue, exact
/// acceptance/carry, owner and holds immediately before provider dispatch.
pub type ReleaseStepCheck = Arc<dyn Fn(&ReleasePlan, &str) -> Result<()> + Send + Sync>;
impl DeliveryAdmission {
    pub(crate) fn verify_bound_permit(&self, permit: &Permit) -> Result<()> {
        if canonical_digest(1, permit)? != canonical_digest(1, &self.permit)? {
            return Err(production_error(
                "deployment permit differs from sealed delivery admission",
            ));
        }
        Ok(())
    }
}
impl DeliveryCoordinator {
    /// A finite release owns one canonical parent permit for its complete plan.
    /// Per-step repository scopes are derived only from that plan and the state
    /// machine's private dispatch capability; pause never creates a new permit.
    pub fn bind_release_step(
        &self,
        permit: &Permit,
        gate: &FreshGate,
        scope: DeliveryScope,
        plan: &ReleasePlan,
        dispatch: &Dispatch,
        eligibility: &crate::release::VerifiedReleaseEligibility,
    ) -> Result<DeliveryAdmission> {
        let digest = plan.digest()?;
        let scope_digest = if permit.operation_id == plan.operation_id {
            let owner = self.host.operation(permit.operation_id)?;
            if [
                "run",
                "metadata",
                "continuation",
                "source_advanced",
                "worker_preparation",
                "review_round",
                "review_round_preparation",
                "review_round_resume",
                "review_round_recovery",
                "session_resume",
                "session_recovery_preparation",
                "release_resume",
            ]
            .iter()
            .any(|key| owner.expected.contains_key(*key))
            {
                return Err(production_error(
                    "original release permit belongs to another admission kind",
                ));
            }
            digest.clone()
        } else {
            let receipt = self.host.operation(permit.operation_id)?;
            let resume = crate::control::ReleaseResumeReference::from_receipt(&receipt)?
                .ok_or_else(|| {
                    production_error("release permit has no actual resume admission lineage")
                })?;
            if resume.plan_operation_id() != plan.operation_id
                || resume.plan_digest() != digest
                || resume.request_id()? != permit.operation_id
                || receipt.runtime_id != permit.lease.runtime_id
                || receipt.canonical_targets != vec![permit.work_key.clone()]
            {
                return Err(production_error(
                    "resume admission belongs to another release plan or work",
                ));
            }
            resume.scope_digest()?
        };
        gate.bind_permit(permit, &scope_digest)?;
        let payload = dispatch.payload();
        if permit.lease.runtime_id != plan.runtime_id
            || scope.runtime_id != plan.runtime_id
            || scope.work != *gate.work()
            || scope.work.parent != plan.parent_id
            || scope.work.linear_workspace != plan.linear_workspace
            || scope.work.generation != plan.generation
            || scope.work.child.is_some()
            || scope.phase != DeliveryPhase::Release
            || scope.fresh_gate_digest != gate.digest()?
            || payload.plan_digest != digest
            || payload.operation_id != plan.operation_id
            || payload.attempt == 0
            || payload.kind.repository() != &scope.repository
            || !plan
                .steps
                .iter()
                .any(|step| step.id == payload.step_id && step.kind == payload.kind)
            || eligibility.plan_digest() != digest
            || eligibility.evidence_token() != plan.evidence_token
        {
            return Err(production_error(
                "release scope is not a step of the admitted complete plan",
            ));
        }
        scope.digest()?;
        self.host
            .validate_permit(permit, || (self.current_gate)(gate))?;
        let admission = DeliveryAdmission {
            permit: permit.clone(),
            observation: DeliveryObservation {
                gate: gate.clone(),
                scope,
                admitted: Some(permit.clone()),
            },
        };
        self.contract(&admission)?;
        Ok(admission)
    }
}
#[derive(Clone, Copy)]
pub enum ReleaseStepEvidence<'a> {
    Changed {
        prepared: &'a PreparedProductionCandidate,
        production: &'a evidence::VerifiedProduction,
    },
    Existing {
        baseline: &'a crate::deployment::VerifiedProductionBaseline,
        production: &'a evidence::VerifiedProduction,
    },
}
pub struct ReleaseRepositoryEvidence<'a> {
    pub contract: &'a RepositoryContract,
    pub evidence: ReleaseStepEvidence<'a>,
}
pub struct ReleasePlanInput<'a> {
    pub runtime_id: Uuid,
    pub operation_id: Uuid,
    pub workspace: &'a crate::config::Workspace,
    pub parent: &'a evidence::VerifiedParent,
    pub acceptance: &'a evidence::VerifiedAcceptance,
    pub carried: Option<&'a evidence::VerifiedCarry>,
    pub repositories: &'a [ReleaseRepositoryEvidence<'a>],
    /// Complete authoritative dependency requirements supplied by the runtime.
    /// This pure builder neither discovers nor satisfies dependencies.
    pub dependencies: Vec<crate::release::DependencyRequirement>,
}
/// Build the finite configured graph without acquiring locks, admitting work,
/// or performing provider operations. A model cannot select a target subset.
pub fn build_release_plan(input: ReleasePlanInput<'_>) -> Result<ReleasePlan> {
    use crate::release::ReleaseStep;
    let parent = input.parent.manifest();
    if input.runtime_id.is_nil()
        || input.operation_id.is_nil()
        || parent.work.child.is_some()
        || parent.work.linear_workspace != input.workspace.linear_workspace_id
    {
        return Err(production_error(
            "release plan identity differs from the verified parent/workspace",
        ));
    }
    let expected: BTreeSet<_> = parent
        .candidates
        .iter()
        .map(|c| c.repository.clone())
        .collect();
    let supplied: BTreeSet<_> = input
        .repositories
        .iter()
        .map(|r| r.contract.repository.clone())
        .collect();
    if expected.len() != parent.candidates.len()
        || supplied != expected
        || supplied.len() != input.repositories.len()
    {
        return Err(production_error(
            "release plan requires the complete unique parent repository vector",
        ));
    }
    let mut resources = BTreeSet::new();
    let mut deployment_free = BTreeSet::new();
    let mut steps = Vec::new();
    let mut targets = BTreeSet::new();
    for item in input.repositories {
        let contract = item.contract;
        contract.digest()?;
        contract.configuration.production_ci_policy()?;
        contract.configuration.deployment_policy()?;
        let configured = input
            .workspace
            .repositories
            .get(&contract.configuration_name)
            .ok_or_else(|| production_error("release repository is no longer configured"))?;
        if canonical_digest(1, configured)? != canonical_digest(1, &contract.configuration)? {
            return Err(production_error(
                "release repository contract differs from current workspace",
            ));
        }
        let candidate = parent
            .candidates
            .iter()
            .find(|c| c.repository == contract.repository)
            .unwrap();
        evidence::verify_accepted_candidate(
            input.parent,
            input.acceptance,
            input.carried,
            candidate,
        )?;
        let result = candidate
            .result
            .as_ref()
            .ok_or_else(|| production_error("release candidate result absent"))?;
        let production = item.evidence.production();
        if production.work() != &parent.work
            || production.revision().repository != contract.repository
            || production.revision().head != result.commit
            || production.revision().base != candidate.production.commit
            || production.revision().tested_commit != result.commit
            || production.revision().tested_tree != result.tree
        {
            return Err(production_error(
                "release production validation differs from complete accepted candidate",
            ));
        }
        let changed = !candidate.ordered_delta_digests.is_empty();
        let merge = format!(
            "merge:{}:{}",
            contract.repository.hostname, contract.repository.id
        );
        if changed {
            let (prepared, proof) = item.evidence.changed()?;
            if prepared.candidate() != candidate
                || prepared.signed.payload.evidence_token != input.parent.token()
                || prepared.signed.payload.scope.contract_digest != contract.digest()?
                || proof.is_existing_production()
            {
                return Err(production_error(
                    "changed release repository lacks exact prepared PR/production proof",
                ));
            }
            resources.insert(ResourceKey::Repository {
                repository: contract.repository.clone(),
            });
            steps.push(ReleaseStep {
                id: merge.clone(),
                depends_on: BTreeSet::new(),
                kind: StepKind::Merge {
                    repository: contract.repository.clone(),
                    pull_request: prepared.pull_request(),
                    production_base: candidate.production.clone(),
                    candidate: result.clone(),
                    selected_delta_digests: candidate.ordered_delta_digests.clone(),
                },
            });
        } else {
            let ReleaseStepEvidence::Existing { baseline, .. } = &item.evidence else {
                return Err(production_error(
                    "unchanged release repository lacks observed production baseline",
                ));
            };
            if !production.is_existing_production()
                || result != &candidate.production
                || baseline.contract_digest() != contract.digest()?
                || !parent.baselines.contains(baseline.baseline())
                || baseline.baseline().production != candidate.production
            {
                return Err(production_error(
                    "existing release repository differs from accepted actual baseline",
                ));
            }
        }
        match contract.configuration.deployment_policy()? {
            crate::config::Deployment::None(value) if value == "none" => {
                if changed {
                    deployment_free.insert(contract.repository.clone());
                }
            }
            crate::config::Deployment::None(_) => {
                return Err(production_error("unknown production behavior"));
            }
            crate::config::Deployment::Steps {
                steps: configured_steps,
            } => {
                let step_id = |prefix: &str, id: &str| {
                    format!(
                        "{prefix}:{}:{}:{id}",
                        contract.repository.hostname, contract.repository.id
                    )
                };
                let configured_ids: BTreeSet<_> =
                    configured_steps.iter().map(|s| s.id.as_str()).collect();
                if configured_steps.is_empty() || configured_ids.len() != configured_steps.len() {
                    return Err(production_error("deployment graph is empty or duplicated"));
                }
                for deployment in configured_steps {
                    if !targets.insert(deployment.target_id.clone())
                        || deployment
                            .depends_on
                            .iter()
                            .any(|id| !configured_ids.contains(id.as_str()))
                    {
                        return Err(production_error(
                            "release target is duplicated or configured dependency is missing",
                        ));
                    }
                    resources.insert(ResourceKey::Repository {
                        repository: contract.repository.clone(),
                    });
                    let target = ResourceKey::Target {
                        provider: "github".into(),
                        id: deployment.target_id.clone(),
                    };
                    resources.insert(target.clone());
                    let verification = step_id("verify", &deployment.id);
                    let mut predecessors: BTreeSet<_> = deployment
                        .depends_on
                        .iter()
                        .map(|id| step_id("verify", id))
                        .collect();
                    if changed {
                        deployment.workflow.proof.validate()?;
                        let reference=deployment.workflow.proof.workflow_ref.rsplit_once('@').and_then(|(_,r)|r.strip_prefix("refs/"))
                            .filter(|r|r.starts_with("heads/") || r.starts_with("tags/"))
                            .ok_or_else(||production_error("production workflow has no canonical pinned dispatch reference"))?;
                        let deploy = step_id("deploy", &deployment.id);
                        predecessors.insert(merge.clone());
                        steps.push(ReleaseStep {
                            id: deploy.clone(),
                            depends_on: predecessors,
                            kind: StepKind::Deploy {
                                repository: contract.repository.clone(),
                                target: target.clone(),
                                workflow_id: deployment.workflow.workflow_id,
                                workflow_revision: deployment.workflow.workflow_revision.clone(),
                                dispatch_ref: reference.into(),
                                source_input: deployment.workflow.source_input.clone(),
                                source_merge_step: merge.clone(),
                                expected_tree: result.tree.clone(),
                                expected_artifact: None,
                            },
                        });
                        steps.push(ReleaseStep {
                            id: verification,
                            depends_on: BTreeSet::from([deploy.clone()]),
                            kind: StepKind::Verify {
                                repository: contract.repository.clone(),
                                target,
                                source_merge_step: merge.clone(),
                                deployment_step: deploy,
                                expected_tree: result.tree.clone(),
                                expected_artifact: None,
                            },
                        });
                    } else {
                        let ReleaseStepEvidence::Existing { baseline, .. } = &item.evidence else {
                            unreachable!()
                        };
                        let artifact = baseline
                            .baseline()
                            .artifact_for_target(&deployment.target_id)
                            .ok_or_else(|| {
                                production_error(
                                    "existing target artifact is absent from complete baseline",
                                )
                            })?;
                        if baseline.baseline().source_for_target(&deployment.target_id)
                            != Some(result.commit.as_str())
                        {
                            let predecessor = baseline
                                .baseline()
                                .successor_recovery_operation
                                .ok_or_else(|| {
                                    production_error(
                                        "divergent target has no sealed predecessor baseline",
                                    )
                                })?;
                            if !candidate.production_integrations.iter().any(|integration| {
                                integration.release_operation == predecessor
                                    && integration.parent_id == parent.work.parent
                                    && integration.merged == *result
                                    && integration.observed_main == *result
                            }) {
                                return Err(production_error(
                                    "deployment-only successor lacks actual accepted contribution integration",
                                ));
                            }
                            deployment.workflow.proof.validate()?;
                            let reference = deployment
                                .workflow
                                .proof
                                .workflow_ref
                                .rsplit_once('@')
                                .and_then(|(_, r)| r.strip_prefix("refs/"))
                                .filter(|r| r.starts_with("heads/") || r.starts_with("tags/"))
                                .ok_or_else(|| {
                                    production_error(
                                        "existing-source workflow has no pinned dispatch reference",
                                    )
                                })?;
                            let deploy = step_id("deploy", &deployment.id);
                            steps.push(ReleaseStep {
                                id: deploy.clone(),
                                depends_on: predecessors,
                                kind: StepKind::DeployExisting {
                                    repository: contract.repository.clone(),
                                    target: target.clone(),
                                    workflow_id: deployment.workflow.workflow_id,
                                    workflow_revision: deployment
                                        .workflow
                                        .workflow_revision
                                        .clone(),
                                    dispatch_ref: reference.into(),
                                    source_input: deployment.workflow.source_input.clone(),
                                    source: result.clone(),
                                    expected_artifact: None,
                                },
                            });
                            steps.push(ReleaseStep {
                                id: verification,
                                depends_on: BTreeSet::from([deploy.clone()]),
                                kind: StepKind::VerifyDeployed {
                                    repository: contract.repository.clone(),
                                    target,
                                    source: result.clone(),
                                    deployment_step: deploy,
                                    expected_artifact: None,
                                },
                            });
                            continue;
                        }
                        steps.push(ReleaseStep {
                            id: verification,
                            depends_on: predecessors,
                            kind: StepKind::VerifyExisting {
                                repository: contract.repository.clone(),
                                target,
                                source: result.clone(),
                                artifact_digest: artifact.into(),
                            },
                        });
                    }
                }
            }
        }
    }
    if targets != parent.production_targets {
        return Err(production_error(
            "configured release target inventory differs from the accepted parent",
        ));
    }
    if steps.is_empty()
        || !steps.iter().any(|step| {
            matches!(
                step.kind,
                StepKind::Merge { .. } | StepKind::DeployExisting { .. }
            )
        })
    {
        return Err(Error::new(
            "no_release_required",
            "complete unchanged outcome requires the verified no-release-required path",
        ));
    }
    steps.sort_by(|a, b| a.id.cmp(&b.id));
    let plan = ReleasePlan {
        schema_version: 1,
        runtime_id: input.runtime_id,
        operation_id: input.operation_id,
        parent_id: parent.work.parent,
        linear_workspace: parent.work.linear_workspace,
        generation: parent.work.generation.clone(),
        evidence_token: input.parent.token().into(),
        acceptance_digest: input.acceptance.digest().into(),
        carried_acceptance: input.carried.map(|c| c.record().clone()),
        resources,
        dependencies: input.dependencies,
        spacing_seconds: input.workspace.release.spacing_seconds,
        deployment_free,
        steps,
    };
    plan.validate()?;
    Ok(plan)
}
impl<'a> ReleaseStepEvidence<'a> {
    pub fn production(&self) -> &'a evidence::VerifiedProduction {
        match self {
            Self::Changed { production, .. } | Self::Existing { production, .. } => production,
        }
    }
    fn changed(
        &self,
    ) -> Result<(
        &'a PreparedProductionCandidate,
        &'a evidence::VerifiedProduction,
    )> {
        match self {
            Self::Changed {
                prepared,
                production,
            } => Ok((prepared, production)),
            _ => Err(production_error(
                "changed release step lacks prepared production evidence",
            )),
        }
    }
}
pub struct ReleaseStepRequest<'a> {
    pub admission: &'a DeliveryAdmission,
    pub dispatch: &'a Dispatch,
    pub plan: &'a ReleasePlan,
    pub parent: &'a evidence::VerifiedParent,
    pub acceptance: &'a evidence::VerifiedAcceptance,
    pub carried: Option<&'a evidence::VerifiedCarry>,
    pub evidence: ReleaseStepEvidence<'a>,
    pub observer: &'a ProductionObserver,
    pub verifier: &'a HostedProofVerifier,
    pub final_gate: ReleaseStepCheck,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseStepOutcome {
    Pending,
    Succeeded,
    Failed(String),
}
/// Only actual provider observation constructs this value. Pending/unknown
/// actions retain their original identity; they never authorize a retry.
pub struct ObservedReleaseStep {
    plan_digest: String,
    step_id: String,
    attempt: u32,
    receipt: Option<Receipt>,
    outcome: ReleaseStepOutcome,
    effect: Option<ObservedEffect>,
    observed_at: DateTime<Utc>,
    historical_target: Option<crate::deployment::HistoricalTargetDeployment>,
}
impl ObservedReleaseStep {
    pub(crate) fn historical_target(
        &self,
    ) -> Option<&crate::deployment::HistoricalTargetDeployment> {
        self.historical_target.as_ref()
    }
    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }
    pub fn step_id(&self) -> &str {
        &self.step_id
    }
    pub fn attempt(&self) -> u32 {
        self.attempt
    }
    pub fn receipt(&self) -> Option<&Receipt> {
        self.receipt.as_ref()
    }
    pub fn outcome(&self) -> &ReleaseStepOutcome {
        &self.outcome
    }
    pub fn effect(&self) -> Option<&ObservedEffect> {
        self.effect.as_ref()
    }
    pub fn observed_at(&self) -> DateTime<Utc> {
        self.observed_at
    }
}
pub(crate) struct ReleaseHistoryReferences {
    pub files: Vec<(String, String, String)>,
    pub receipts: Vec<Receipt>,
}
impl DeliveryCoordinator {
    /// Read every immutable step ordinal, including failed attempts archived by
    /// explicit resume. This is local historical evidence, not a provider poll.
    pub(crate) fn release_history_references(
        &self,
        attempt: &crate::release::ReleaseAttempt,
    ) -> Result<ReleaseHistoryReferences> {
        use crate::release::StepState;
        attempt.validate_recovered()?;
        let plan = &attempt.plan;
        let mut result = ReleaseHistoryReferences {
            files: vec![],
            receipts: vec![],
        };
        for (step, current) in &attempt.steps {
            let mut states = attempt
                .step_history
                .get(step)
                .into_iter()
                .flatten()
                .filter(|state| {
                    matches!(
                        state,
                        StepState::Failed { .. } | StepState::Cancelled { .. }
                    )
                })
                .collect::<Vec<_>>();
            states.push(current);
            if states.len() != attempt.step_attempt(step)? as usize {
                return Err(production_error("release step history ordinal differs"));
            }
            for (index, state) in states.into_iter().enumerate() {
                let ordinal = index as u32 + 1;
                let no_dispatch = self.read_no_dispatch(plan, step, ordinal)?;
                let effect = match self.load_release_effect(plan, step, ordinal) {
                    Ok(record) => Some(record),
                    Err(error) if error.code == "record_missing" => None,
                    Err(error) => return Err(error),
                };
                if no_dispatch.is_none() && effect.is_none() {
                    if matches!(state, StepState::NotStarted | StepState::Retired { .. }) {
                        continue;
                    }
                    return Err(production_error(
                        "terminal release history has no protected step record",
                    ));
                }
                if result.files.len() >= 4095 {
                    return Err(production_error("release history exceeds reference bound"));
                }
                if let Some(observed) = &no_dispatch {
                    if !matches!(state, StepState::Failed { reason, .. } if observed.outcome() == &ReleaseStepOutcome::Failed(reason.clone()))
                    {
                        return Err(production_error(
                            "archived no-dispatch outcome differs from release history",
                        ));
                    }
                    let leaf = no_dispatch_leaf(plan.operation_id, step, ordinal);
                    let signed: Signed<NoDispatchRecord> = self.records.read(&leaf)?;
                    self.key.verify("release_no_dispatch", &signed)?;
                    result
                        .receipts
                        .push(self.host.operation(signed.payload.admission_operation)?);
                    if let Some(receipt) = observed.receipt() {
                        result.receipts.push(receipt.clone());
                    }
                    result
                        .files
                        .push((leaf, "release_no_dispatch".into(), signed.digest()?));
                }
                if let Some(effect) = effect {
                    let receipt = self.release_record_receipt(&effect)?;
                    if no_dispatch.is_none()
                        && let Some(receipt) = &receipt
                    {
                        let expected = match state {
                            StepState::Succeeded { effect, .. } => {
                                json!({"release_plan":plan.digest()?,"step":step,"outcome":ReleaseStepOutcome::Succeeded,"effect":effect})
                            }
                            StepState::Failed { reason, .. } => {
                                json!({"release_plan":plan.digest()?,"step":step,"outcome":ReleaseStepOutcome::Failed(reason.clone()),"effect":Value::Null})
                            }
                            // Cancellation needs its distinct owned cancellation proof;
                            // current runtime records provider cancellations as Failed.
                            _ => {
                                return Err(production_error(
                                    "release history outcome lacks a reconciled terminal observation",
                                ));
                            }
                        };
                        if receipt.phase != crate::control::ReceiptPhase::Reconciled
                            || receipt.observed_result.as_ref() != Some(&expected)
                        {
                            return Err(production_error(
                                "archived release effect lacks its exact reconciled outcome",
                            ));
                        }
                    }
                    result
                        .receipts
                        .push(self.host.operation(effect.admission_operation)?);
                    if let Some(receipt) = receipt {
                        result.receipts.push(receipt);
                    }
                    let leaf = release_effect_leaf(plan.operation_id, step, ordinal);
                    let signed: Signed<ReleaseEffectRecord> = self.records.read(&leaf)?;
                    self.key.verify("release_effect", &signed)?;
                    result
                        .files
                        .push((leaf, "release_effect".into(), signed.digest()?));
                }
            }
        }
        Ok(result)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseEffectRecord {
    schema_version: u32,
    plan_digest: String,
    payload: crate::release::StepPayload,
    scope: DeliveryScope,
    admission_operation: Uuid,
    admission_scope_digest: String,
    request_id: Option<Uuid>,
    command: Option<ProviderMutation>,
    deployment_step: Option<String>,
    challenge: Option<Uuid>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoDispatchRecord {
    schema_version: u32,
    payload: crate::release::StepPayload,
    runtime: Uuid,
    work: WorkIdentity,
    admission_operation: Uuid,
    admission_scope: String,
    provider_receipt: Option<(Uuid, String)>,
    reason: String,
    observed_at: DateTime<Utc>,
}
fn no_dispatch_leaf(operation: Uuid, step: &str, attempt: u32) -> String {
    format!(
        "release-no-dispatch-{operation}-{}-attempt-{attempt}.json",
        &bytes_digest(step.as_bytes())[7..]
    )
}
fn release_effect_leaf(operation: Uuid, step: &str, attempt: u32) -> String {
    format!(
        "release-effect-{}-{}-attempt-{}.json",
        operation,
        bytes_digest(step.as_bytes()).rsplit(':').next().unwrap(),
        attempt
    )
}
fn configured_deployment<'a>(
    contract: &'a RepositoryContract,
    kind: &StepKind,
) -> Result<&'a crate::config::DeploymentStep> {
    let target = match kind {
        StepKind::Deploy { target, .. }
        | StepKind::DeployExisting { target, .. }
        | StepKind::VerifyDeployed { target, .. }
        | StepKind::Verify { target, .. }
        | StepKind::VerifyExisting { target, .. } => target,
        _ => return Err(production_error("merge has no deployment contract")),
    };
    let ResourceKey::Target { provider, id } = target else {
        return Err(production_error("deployment target is not canonical"));
    };
    let crate::config::Deployment::Steps { steps } = contract.configuration.deployment_policy()?
    else {
        return Err(production_error("repository has no deployment contract"));
    };
    let matches: Vec<_> = steps
        .iter()
        .filter(|s| provider == "github" && &s.target_id == id)
        .collect();
    let [step] = matches.as_slice() else {
        return Err(production_error(
            "release target is missing or ambiguous in configuration",
        ));
    };
    if let StepKind::Deploy {
        workflow_id,
        workflow_revision,
        dispatch_ref,
        source_input,
        ..
    }
    | StepKind::DeployExisting {
        workflow_id,
        workflow_revision,
        dispatch_ref,
        source_input,
        ..
    } = kind
    {
        let reference = step
            .workflow
            .proof
            .workflow_ref
            .rsplit_once('@')
            .map(|(_, r)| r.strip_prefix("refs/").unwrap_or(r));
        if *workflow_id != step.workflow.workflow_id
            || workflow_revision != &step.workflow.workflow_revision
            || source_input != &step.workflow.source_input
            || reference != Some(dispatch_ref.as_str())
        {
            return Err(production_error(
                "release step differs from configured pinned deployment route",
            ));
        }
    }
    Ok(step)
}
impl DeliveryCoordinator {
    fn check_release_request(&self, request: &ReleaseStepRequest<'_>) -> Result<()> {
        let a = request.admission;
        let plan = request.plan;
        let payload = request.dispatch.payload();
        plan.validate()?;
        self.current(a, &[DeliveryPhase::Release, DeliveryPhase::Recovery])?;
        let production = request.evidence.production();
        let candidate = match &request.evidence {
            ReleaseStepEvidence::Changed { prepared, .. } => {
                if matches!(
                    payload.kind,
                    StepKind::VerifyExisting { .. }
                        | StepKind::DeployExisting { .. }
                        | StepKind::VerifyDeployed { .. }
                ) {
                    return Err(production_error(
                        "existing target verification cannot substitute changed candidate evidence",
                    ));
                }
                self.production_parent(a, request.parent, prepared.candidate())?;
                prepared.candidate()
            }
            ReleaseStepEvidence::Existing { baseline, .. } => {
                if !matches!(
                    payload.kind,
                    StepKind::VerifyExisting { .. }
                        | StepKind::DeployExisting { .. }
                        | StepKind::VerifyDeployed { .. }
                ) || baseline.contract_digest() != a.scope.contract_digest
                    || request.parent.manifest().work != a.scope.work
                    || !request
                        .parent
                        .manifest()
                        .baselines
                        .contains(baseline.baseline())
                {
                    return Err(production_error(
                        "existing target lacks its exact accepted parent baseline",
                    ));
                }
                let candidate = request
                    .parent
                    .manifest()
                    .candidates
                    .iter()
                    .find(|c| {
                        c.repository == a.scope.repository
                            && c.ordered_delta_digests.is_empty()
                            && c.production == baseline.baseline().production
                            && c.result.as_ref() == Some(&c.production)
                    })
                    .ok_or_else(|| {
                        production_error("existing target has no unchanged accepted candidate")
                    })?;
                if a.scope.revision.head != candidate.production.commit
                    || a.scope.revision.base != candidate.production.commit
                    || a.scope.revision.tested_commit != candidate.production.commit
                    || a.scope.revision.tested_tree != candidate.production.tree
                {
                    return Err(production_error(
                        "existing target scope differs from exact production baseline",
                    ));
                }
                candidate
            }
        };
        evidence::verify_accepted_candidate(
            request.parent,
            request.acceptance,
            request.carried,
            candidate,
        )?;
        if plan.runtime_id != a.scope.runtime_id
            || plan.parent_id != a.scope.work.parent
            || plan.linear_workspace != a.scope.work.linear_workspace
            || plan.generation != a.scope.work.generation
            || a.scope.candidate_vector_digest.as_ref()
                != Some(&request.parent.manifest().candidate_vector_digest()?)
            || plan.evidence_token != request.parent.token()
            || plan.acceptance_digest != request.acceptance.digest()
            || plan.carried_acceptance.as_ref() != request.carried.map(|c| c.record())
            || payload.operation_id != plan.operation_id
            || payload.attempt == 0
            || payload.plan_digest != plan.digest()?
            || plan
                .steps
                .iter()
                .find(|s| s.id == payload.step_id)
                .map(|s| &s.kind)
                != Some(&payload.kind)
            || payload.kind.repository() != &a.scope.repository
            || production.work() != &a.scope.work
            || production.revision() != &a.scope.revision
            || production.is_existing_production()
                != matches!(request.evidence, ReleaseStepEvidence::Existing { .. })
        {
            return Err(production_error(
                "release dispatch lacks its exact persisted plan, acceptance and tested candidate",
            ));
        }
        match &payload.kind {
            StepKind::Merge {
                pull_request,
                production_base,
                candidate,
                selected_delta_digests,
                ..
            } => {
                let (prepared, _) = request.evidence.changed()?;
                if *pull_request != prepared.pull_request()
                    || production_base != &prepared.candidate().production
                    || Some(candidate) != prepared.candidate().result.as_ref()
                    || selected_delta_digests != &prepared.candidate().ordered_delta_digests
                    || payload.exact_source_commit.is_some()
                {
                    return Err(production_error(
                        "merge step differs from exact reviewed isolated patch sequence",
                    ));
                }
            }
            StepKind::Deploy { expected_tree, .. } | StepKind::Verify { expected_tree, .. } => {
                configured_deployment(self.contract(a)?, &payload.kind)?;
                if expected_tree != &a.scope.revision.tested_tree
                    || payload.exact_source_commit.is_none()
                {
                    return Err(production_error(
                        "deployment step lacks actual observed merge source/tree",
                    ));
                }
            }
            StepKind::DeployExisting { source, target, .. }
            | StepKind::VerifyDeployed { source, target, .. } => {
                configured_deployment(self.contract(a)?, &payload.kind)?;
                let ReleaseStepEvidence::Existing { baseline, .. } = &request.evidence else {
                    return Err(production_error("existing-source baseline missing"));
                };
                let ResourceKey::Target { id, .. } = target else {
                    return Err(production_error("existing target identity invalid"));
                };
                if source != &baseline.baseline().production
                    || payload.exact_source_commit.as_ref() != Some(&source.commit)
                    || baseline.baseline().successor_recovery_operation.is_none()
                    || baseline
                        .baseline()
                        .source_for_target(id)
                        .is_none_or(|deployed| deployed == source.commit)
                    || !candidate.production_integrations.iter().any(|integration| {
                        Some(integration.release_operation)
                            == baseline.baseline().successor_recovery_operation
                            && integration.parent_id == a.scope.work.parent
                            && integration.merged == *source
                            && integration.observed_main == *source
                    })
                {
                    return Err(production_error(
                        "existing-source deployment is not exact accepted known-predecessor recovery",
                    ));
                }
            }
            StepKind::VerifyExisting {
                target,
                source,
                artifact_digest,
                ..
            } => {
                configured_deployment(self.contract(a)?, &payload.kind)?;
                let ReleaseStepEvidence::Existing { baseline, .. } = &request.evidence else {
                    return Err(production_error("existing baseline proof missing"));
                };
                let ResourceKey::Target { id, .. } = target else {
                    return Err(production_error("existing target identity invalid"));
                };
                if source != &baseline.baseline().production
                    || baseline.baseline().artifact_for_target(id) != Some(artifact_digest)
                    || payload.exact_source_commit.as_ref() != Some(&source.commit)
                {
                    return Err(production_error(
                        "existing target source or artifact changed",
                    ));
                }
            }
        }
        (request.final_gate)(plan, &payload.step_id)
    }

    /// The caller already persisted ReleaseAttempt::mark_sent. This method
    /// persists its exact wire intent before transport and cannot create a new
    /// request ID for an existing step. The final callback is cache-only.
    pub async fn dispatch_release_step(
        &self,
        request: ReleaseStepRequest<'_>,
    ) -> Result<ObservedReleaseStep> {
        let payload = request.dispatch.payload();
        if let Some(observed) =
            self.read_no_dispatch(request.plan, &payload.step_id, payload.attempt)?
        {
            return Ok(observed);
        }
        let result = self.dispatch_release_step_inner(&request).await;
        if let Err(error) = &result {
            // This owned future has ended. Intent persistence precedes every
            // broker call; absence under the same lock proves no dispatch path
            // was entered, rather than making any claim about remote absence.
            let _lock = self.records.section_lock("release-effects.lock")?;
            let leaf =
                release_effect_leaf(request.plan.operation_id, &payload.step_id, payload.attempt);
            match self.records.read::<Signed<ReleaseEffectRecord>>(&leaf) {
                Err(missing) if missing.code == "record_missing" => {
                    let owner = self.host.operation(request.admission.operation_id())?;
                    if owner.kind != "admission"
                        || [
                            "run",
                            "metadata",
                            "continuation",
                            "source_advanced",
                            "worker_preparation",
                            "review_round",
                            "review_round_preparation",
                            "review_round_resume",
                            "review_round_recovery",
                            "session_resume",
                            "session_recovery_preparation",
                        ]
                        .iter()
                        .any(|key| owner.expected.contains_key(*key))
                        || owner.runtime_id != request.plan.runtime_id
                        || owner.expected.get("scope")
                            != Some(&request.admission.permit.scope_digest)
                        || owner.canonical_targets
                            != vec![request.admission.permit.work_key.clone()]
                    {
                        return Err(production_error(
                            "failed preflight lacks actual admission lineage",
                        ));
                    }
                    let no_dispatch = NoDispatchRecord {
                        schema_version: 1,
                        payload: payload.clone(),
                        runtime: request.plan.runtime_id,
                        work: request.admission.scope.work.clone(),
                        admission_operation: request.admission.operation_id(),
                        admission_scope: request.admission.permit.scope_digest.clone(),
                        provider_receipt: None,
                        reason: format!("preflight did not dispatch: {}", error.code),
                        observed_at: Utc::now(),
                    };
                    let leaf = no_dispatch_leaf(
                        request.plan.operation_id,
                        &payload.step_id,
                        payload.attempt,
                    );
                    match self.records.read::<Signed<NoDispatchRecord>>(&leaf) {
                        Ok(_) => {}
                        Err(e) if e.code == "record_missing" => self
                            .records
                            .write(&leaf, &self.key.sign("release_no_dispatch", &no_dispatch)?)?,
                        Err(e) => return Err(e),
                    }
                }
                Ok(_) => {}
                Err(e) => return Err(e),
            }
        }
        if let Some(observed) =
            self.read_no_dispatch(request.plan, &payload.step_id, payload.attempt)?
        {
            return Ok(observed);
        }
        result
    }
    async fn dispatch_release_step_inner(
        &self,
        request: &ReleaseStepRequest<'_>,
    ) -> Result<ObservedReleaseStep> {
        self.check_release_request(request)?;
        let a = request.admission;
        let payload = request.dispatch.payload();
        let contract = self.contract(a)?;
        let leaf =
            release_effect_leaf(request.plan.operation_id, &payload.step_id, payload.attempt);
        // A retained intent, including one whose response was lost, is observed
        // only. A crash cannot turn SentUnknown back into a fresh provider call.
        match self.records.read::<Signed<ReleaseEffectRecord>>(&leaf) {
            Ok(_) => {
                return self
                    .recover_release_step(
                        a,
                        request.plan,
                        &payload.step_id,
                        payload.attempt,
                        request.observer,
                    )
                    .await;
            }
            Err(e) if e.code == "record_missing" => {}
            Err(e) => return Err(e),
        }
        let mut challenge: Option<TargetDeploymentChallenge> = None;
        let (command, deployment_step) = match &payload.kind {
            StepKind::Merge { pull_request, .. } => {
                let (prepared, production) = request.evidence.changed()?;
                let current = self
                    .hydrate_production_validation(a, prepared, request.parent)
                    .await?;
                if current.digest()? != production.digest()? {
                    return Err(production_error(
                        "production checks changed before merge dispatch",
                    ));
                }
                (
                    Some(ProviderMutation::GithubMerge {
                        repository: contract.provider.clone(),
                        pr: *pull_request,
                        expected_head: a.scope.revision.head.clone(),
                        expected_base: a.scope.revision.base.clone(),
                    }),
                    None,
                )
            }
            StepKind::Deploy { expected_tree, .. }
            | StepKind::DeployExisting {
                source:
                    crate::git::Revision {
                        tree: expected_tree,
                        ..
                    },
                ..
            } => {
                self.current_repository_protection(a, "main").await?;
                let step = configured_deployment(contract, &payload.kind)?;
                let prepared = request
                    .observer
                    .prepare_target(
                        contract,
                        a,
                        &a.permit,
                        &step.id,
                        crate::git::Revision {
                            commit: payload.exact_source_commit.clone().unwrap(),
                            tree: expected_tree.clone(),
                        },
                    )
                    .await?;
                let command = request.observer.dispatch_command(contract, &prepared)?;
                challenge = Some(prepared);
                (Some(command), Some(step.id.clone()))
            }
            StepKind::Verify { .. } | StepKind::VerifyDeployed { .. } => (
                None,
                Some(configured_deployment(contract, &payload.kind)?.id.clone()),
            ),
            StepKind::VerifyExisting { .. } => {
                let ReleaseStepEvidence::Existing { baseline, .. } = &request.evidence else {
                    return Err(production_error("existing baseline proof missing"));
                };
                request.observer.reobserve(contract, baseline).await?;
                (
                    None,
                    Some(configured_deployment(contract, &payload.kind)?.id.clone()),
                )
            }
        };
        let record = ReleaseEffectRecord {
            schema_version: 1,
            plan_digest: request.plan.digest()?,
            payload: payload.clone(),
            scope: a.scope.clone(),
            admission_operation: a.operation_id(),
            admission_scope_digest: a.permit.scope_digest.clone(),
            request_id: command.as_ref().map(|_| {
                challenge
                    .as_ref()
                    .map_or_else(Uuid::new_v4, |c| c.dispatch_request())
            }),
            command,
            deployment_step,
            challenge: challenge.as_ref().map(|c| c.challenge()),
        };
        self.host.validate_permit(&a.permit, || {
            (self.current_gate)(&a.gate)?;
            (request.final_gate)(request.plan, &payload.step_id)?;
            let _lock = self.records.section_lock("release-effects.lock")?;
            if self
                .read_no_dispatch(request.plan, &payload.step_id, payload.attempt)?
                .is_some()
            {
                return Err(Error::new(
                    "operation_conflict",
                    "step attempt already concluded without dispatch",
                ));
            }
            match self.records.read::<Signed<ReleaseEffectRecord>>(&leaf) {
                Ok(_) => Err(Error::new(
                    "operation_conflict",
                    "release step intent was concurrently claimed",
                )),
                Err(e) if e.code == "record_missing" => self
                    .records
                    .write(&leaf, &self.key.sign("release_effect", &record)?),
                Err(e) => Err(e),
            }
        })?;
        if let (Some(command), Some(request_id)) = (record.command.clone(), record.request_id) {
            let gate = a.gate.clone();
            let current = self.current_gate.clone();
            let final_gate = request.final_gate.clone();
            let plan = request.plan.clone();
            let step = payload.step_id.clone();
            let receipt = self
                .broker
                .mutate(
                    self.host.clone(),
                    MutationRequest {
                        request_id,
                        runtime_id: a.scope.runtime_id,
                        actor: Actor {
                            principal: None,
                            identity: format!("owner:{}", a.permit.lease.owner_id),
                            roles: BTreeSet::new(),
                        },
                        kind: String::new(),
                        canonical_targets: vec![],
                        expected: BTreeMap::from([
                            ("release_plan".into(), record.plan_digest.clone()),
                            ("release_step".into(), step.clone()),
                            ("release_attempt".into(), payload.attempt.to_string()),
                            ("delivery_scope".into(), a.scope.digest()?),
                        ]),
                        payload: Value::Null,
                    },
                    Some(a.permit.clone()),
                    None,
                    command,
                    move || {
                        current(&gate)?;
                        final_gate(&plan, &step)
                    },
                )
                .await?;
            if let Some(challenge) = challenge {
                // An accepted run may still be queued. Only an actual successful
                // complete attempt can mint artifact/source provenance.
                let run_id = remote_id(&receipt, "workflow_run_id")?
                    .parse()
                    .map_err(|_| production_error("deployment run ID invalid"))?;
                let run = self
                    .github()?
                    .workflow_run(&contract.provider, run_id, None)
                    .await?;
                if run.status == "completed" && run.conclusion.as_deref() == Some("success") {
                    request
                        .observer
                        .observe_target(
                            contract,
                            a,
                            &a.permit,
                            &challenge,
                            &receipt,
                            request.verifier,
                        )
                        .await?;
                }
            }
        }
        self.recover_release_step(
            a,
            request.plan,
            &payload.step_id,
            payload.attempt,
            request.observer,
        )
        .await
    }

    fn read_no_dispatch(
        &self,
        plan: &ReleasePlan,
        step: &str,
        attempt: u32,
    ) -> Result<Option<ObservedReleaseStep>> {
        let signed: Signed<NoDispatchRecord> =
            match self
                .records
                .read(&no_dispatch_leaf(plan.operation_id, step, attempt))
            {
                Ok(signed) => signed,
                Err(e) if e.code == "record_missing" => return Ok(None),
                Err(e) => return Err(e),
            };
        self.key.verify("release_no_dispatch", &signed)?;
        let r = signed.payload;
        let owner = self.host.operation(r.admission_operation)?;
        if r.schema_version != 1
            || r.runtime != plan.runtime_id
            || r.payload.plan_digest != plan.digest()?
            || r.payload.operation_id != plan.operation_id
            || r.payload.step_id != step
            || r.payload.attempt != attempt
            || attempt == 0
            || r.work.parent != plan.parent_id
            || r.work.linear_workspace != plan.linear_workspace
            || r.work.generation != plan.generation
            || r.work.child.is_some()
            || plan.steps.iter().find(|s| s.id == step).map(|s| &s.kind) != Some(&r.payload.kind)
            || owner.kind != "admission"
            || [
                "run",
                "metadata",
                "continuation",
                "source_advanced",
                "worker_preparation",
                "review_round",
                "review_round_preparation",
                "review_round_resume",
                "review_round_recovery",
                "session_resume",
                "session_recovery_preparation",
            ]
            .iter()
            .any(|key| owner.expected.contains_key(*key))
            || owner.runtime_id != plan.runtime_id
            || owner.expected.get("scope") != Some(&r.admission_scope)
            || owner.canonical_targets
                != vec![
                    crate::authority::IssueKey::new(
                        &r.work.linear_endpoint,
                        r.work.linear_workspace,
                        r.work.parent,
                    )?
                    .identity()?,
                ]
        {
            return Err(production_error(
                "no-dispatch observation lacks exact original owned step identity",
            ));
        }
        let receipt = if let Some((id, digest)) = &r.provider_receipt {
            let effect = self.load_release_effect(plan, step, attempt)?;
            let receipt = self.host.operation(*id)?;
            if effect.request_id != Some(*id)
                || effect.admission_operation != r.admission_operation
                || canonical_digest(1, &effect.payload)? != canonical_digest(1, &r.payload)?
                || receipt.payload_digest != *digest
                || receipt.phase != crate::control::ReceiptPhase::Reconciled
                || !receipt.remote_ids.is_empty()
            {
                return Err(production_error(
                    "retained provider no-dispatch evidence changed",
                ));
            }
            Some(receipt)
        } else {
            match self
                .records
                .read::<Signed<ReleaseEffectRecord>>(&release_effect_leaf(
                    plan.operation_id,
                    step,
                    attempt,
                )) {
                Ok(_) => {
                    return Err(production_error(
                        "no-dispatch and provider intent records conflict; preserve uncertainty",
                    ));
                }
                Err(error) if error.code == "record_missing" => {}
                Err(error) => return Err(error),
            }
            None
        };
        Ok(Some(ObservedReleaseStep {
            plan_digest: plan.digest()?,
            step_id: step.into(),
            attempt,
            receipt,
            outcome: ReleaseStepOutcome::Failed(r.reason),
            effect: None,
            observed_at: r.observed_at,
            historical_target: None,
        }))
    }
    fn read_release_effect(
        &self,
        observation: &DeliveryObservation,
        plan: &ReleasePlan,
        step: &str,
        attempt: u32,
    ) -> Result<ReleaseEffectRecord> {
        let r = self.load_release_effect(plan, step, attempt)?;
        if r.scope.runtime_id != observation.scope.runtime_id
            || r.scope.repository != observation.scope.repository
            || r.scope.work.linear_endpoint != observation.scope.work.linear_endpoint
            || r.scope.work.linear_workspace != observation.scope.work.linear_workspace
            || r.scope.work.parent != observation.scope.work.parent
            || r.scope.contract_digest != observation.scope.contract_digest
        {
            return Err(production_error(
                "retained effect differs from observation scope",
            ));
        }
        Ok(r)
    }
    fn load_release_effect(
        &self,
        plan: &ReleasePlan,
        step: &str,
        attempt: u32,
    ) -> Result<ReleaseEffectRecord> {
        plan.validate()?;
        let signed: Signed<ReleaseEffectRecord> =
            self.records
                .read(&release_effect_leaf(plan.operation_id, step, attempt))?;
        self.key.verify("release_effect", &signed)?;
        let r = signed.payload;
        if r.schema_version != 1
            || r.plan_digest != plan.digest()?
            || r.payload.plan_digest != r.plan_digest
            || r.payload.operation_id != plan.operation_id
            || r.payload.step_id != step
            || attempt == 0
            || r.payload
                .prerequisite_attempts
                .values()
                .any(|attempt| *attempt == 0)
            || plan
                .steps
                .iter()
                .find(|s| s.id == step)
                .map(|s| &s.depends_on)
                != Some(&r.payload.prerequisite_attempts.keys().cloned().collect())
            || r.payload.attempt != attempt
            || plan.steps.iter().find(|s| s.id == step).map(|s| &s.kind) != Some(&r.payload.kind)
            || r.scope.runtime_id != plan.runtime_id
            || r.scope.work.linear_workspace != plan.linear_workspace
            || r.scope.work.parent != plan.parent_id
            || r.scope.work.generation != plan.generation
            || r.scope.work.child.is_some()
            || r.scope.repository != *r.payload.kind.repository()
        {
            return Err(production_error(
                "retained release effect differs from canonical plan/runtime/repository",
            ));
        }
        Ok(r)
    }
    /// Poll a previously dispatched owned operation. A completed deployment may
    /// now expose its signed artifact; minting that proof requires the original
    /// still-live permit. This performs reads only and never resends the command.
    pub async fn observe_release_step(
        &self,
        admission: &DeliveryAdmission,
        plan: &ReleasePlan,
        step: &str,
        attempt: u32,
        observer: &ProductionObserver,
        verifier: &HostedProofVerifier,
    ) -> Result<ObservedReleaseStep> {
        self.current(
            admission,
            &[DeliveryPhase::Release, DeliveryPhase::Recovery],
        )?;
        let record = self.read_release_effect(admission, plan, step, attempt)?;
        if matches!(
            record.payload.kind,
            StepKind::Deploy { .. } | StepKind::DeployExisting { .. }
        ) {
            if record.admission_operation != admission.operation_id()
                || record.scope.digest()? != admission.scope.digest()?
            {
                return Err(production_error(
                    "deployment proof polling requires its original live admitted scope",
                ));
            }
            let contract = self.contract(admission)?;
            let configured = configured_deployment(contract, &record.payload.kind)?;
            let receipt = self.host.operation(
                record
                    .request_id
                    .ok_or_else(|| production_error("deployment dispatch receipt missing"))?,
            )?;
            let run_id = remote_id(&receipt, "workflow_run_id")?
                .parse()
                .map_err(|_| production_error("owned run ID invalid"))?;
            let run = self
                .github()?
                .workflow_run(&contract.provider, run_id, None)
                .await?;
            if run.status == "completed" && run.conclusion.as_deref() == Some("success") {
                match observer.recover_target(contract, &configured.id).await {
                    Ok(target)
                        if target.dispatch_request() == receipt.request_id
                            && target.run_id() == run.id
                            && target.run_attempt() == run.attempt => {}
                    _ => {
                        let challenge = observer.recover_challenge(
                            contract,
                            admission.operation_id(),
                            &configured.id,
                        )?;
                        if Some(challenge.challenge()) != record.challenge
                            || challenge.dispatch_request() != receipt.request_id
                        {
                            return Err(production_error(
                                "deployment proof challenge differs from retained release intent",
                            ));
                        }
                        observer
                            .observe_target(
                                contract,
                                admission,
                                &admission.permit,
                                &challenge,
                                &receipt,
                                verifier,
                            )
                            .await?;
                    }
                }
            }
        }
        self.recover_release_step(admission, plan, step, attempt, observer)
            .await
    }
    /// Metadata-only recovery never calls ProviderBroker or requires RUNNING.
    /// Missing proof or a changed main/target remains an explicit held result.
    pub async fn recover_release_step(
        &self,
        observation: &DeliveryObservation,
        plan: &ReleasePlan,
        step: &str,
        attempt: u32,
        observer: &ProductionObserver,
    ) -> Result<ObservedReleaseStep> {
        self.current(
            observation,
            &[DeliveryPhase::Release, DeliveryPhase::Recovery],
        )?;
        if let Some(observed) = self.read_no_dispatch(plan, step, attempt)? {
            return Ok(observed);
        }
        let r = self.read_release_effect(observation, plan, step, attempt)?;
        let result = self.observe_release_record(plan, &r, observer).await?;
        self.current(
            observation,
            &[DeliveryPhase::Release, DeliveryPhase::Recovery],
        )?;
        Ok(result)
    }
    /// Read only an already dispatched, authenticated effect. This path has no
    /// execution gate or Permit and cannot publish or refresh product readiness;
    /// it remains usable after corrections, exclusion, stop, or local retirement.
    pub async fn recover_owned_release_step(
        &self,
        plan: &ReleasePlan,
        step: &str,
        attempt: u32,
        observer: &ProductionObserver,
    ) -> Result<ObservedReleaseStep> {
        if let Some(observed) = self.read_no_dispatch(plan, step, attempt)? {
            return Ok(observed);
        }
        let r = self.load_release_effect(plan, step, attempt)?;
        self.observe_release_record(plan, &r, observer).await
    }
    /// Re-observe an already successful, retired ancestor target effect. This cannot
    /// reconcile an unknown action or produce a current target/readiness proof.
    pub(crate) async fn recover_terminal_release_step(
        &self,
        plan: &ReleasePlan,
        step: &str,
        attempt: u32,
        expected: &ObservedEffect,
        observer: &ProductionObserver,
    ) -> Result<ObservedReleaseStep> {
        let r = self.load_release_effect(plan, step, attempt)?;
        let receipt = self.release_record_receipt(&r)?;
        self.require_retired_release_effect(&r, receipt.as_ref())?;
        let contract = &self.contracts[&r.scope.repository];
        let observed = match &r.payload.kind {
            StepKind::Merge { .. } => {
                return Err(production_error(
                    "historical target recovery cannot establish a merge",
                ));
            }
            StepKind::Deploy { .. }
            | StepKind::DeployExisting { .. }
            | StepKind::Verify { .. }
            | StepKind::VerifyDeployed { .. }
            | StepKind::VerifyExisting { .. } => {
                let configured = configured_deployment(contract, &r.payload.kind)?;
                let (target, deployment) = match &r.payload.kind {
                    StepKind::VerifyExisting { .. } => {
                        let ObservedEffect::Verification {
                            evidence_digest, ..
                        } = expected
                        else {
                            return Err(production_error(
                                "historical existing target has no exact verification digest",
                            ));
                        };
                        (
                            observer
                                .historical_digest(contract, evidence_digest)
                                .await?,
                            None,
                        )
                    }
                    _ => {
                        let deployment = match &r.payload.kind {
                            StepKind::Verify { deployment_step, .. } | StepKind::VerifyDeployed { deployment_step, .. } => {
                                self.load_release_effect(plan, deployment_step, *r.payload.prerequisite_attempts.get(deployment_step)
                                    .ok_or_else(|| production_error("historical verification lacks its exact deployment attempt"))?)?
                            }
                            _ => r.clone(),
                        };
                        let dispatch = deployment.request_id.ok_or_else(|| {
                            production_error("historical deployment has no dispatch identity")
                        })?;
                        let challenge = deployment.challenge.ok_or_else(|| {
                            production_error("historical deployment has no challenge identity")
                        })?;
                        let actual = self.release_record_receipt(&deployment)?;
                        self.require_retired_release_effect(&deployment, actual.as_ref())?;
                        let target = observer
                            .historical_dispatch(contract, &configured.id, challenge, dispatch)
                            .await?;
                        if target.operation_id() != deployment.admission_operation
                            || target.dispatch_request() != dispatch
                            || target.challenge() != challenge
                        {
                            return Err(production_error(
                                "historical target differs from the original deployment admission",
                            ));
                        }
                        check_historical_target(&deployment, target.target())?;
                        let recorded = actual
                            .as_ref()
                            .and_then(|v| v.observed_result.as_ref())
                            .ok_or_else(|| {
                                production_error("historical dispatch has no reconciled effect")
                            })?;
                        if recorded["effect"]
                            != serde_json::to_value(target.deployment_effect(plan.operation_id)?)
                                .map_err(|_| {
                                    production_error("historical effect serialization failed")
                                })?
                        {
                            return Err(production_error(
                                "historical dispatch receipt records another production effect",
                            ));
                        }
                        (target, Some(deployment))
                    }
                };
                check_historical_target(&r, target.target())?;
                let effect = if matches!(
                    r.payload.kind,
                    StepKind::Deploy { .. } | StepKind::DeployExisting { .. }
                ) {
                    target.deployment_effect(plan.operation_id)?
                } else {
                    target.verification_effect()?
                };
                // Both own and prerequisite scopes were checked; retaining this
                // explicit branch prevents accepting a verification as dispatch.
                if matches!(
                    r.payload.kind,
                    StepKind::Deploy { .. } | StepKind::DeployExisting { .. }
                ) && deployment.is_none()
                {
                    return Err(production_error(
                        "historical deployment lacks exact original effect",
                    ));
                }
                ObservedReleaseStep {
                    plan_digest: r.plan_digest.clone(),
                    step_id: r.payload.step_id.clone(),
                    attempt: r.payload.attempt,
                    receipt: receipt.clone(),
                    outcome: ReleaseStepOutcome::Succeeded,
                    effect: Some(effect),
                    observed_at: Utc::now(),
                    historical_target: Some(target),
                }
            }
        };
        if observed.outcome != ReleaseStepOutcome::Succeeded
            || observed.effect.as_ref() != Some(expected)
        {
            return Err(production_error(
                "historical terminal effect differs from the recorded successful step",
            ));
        }
        // Metadata reads may race local reconciliation. Preserve exact receipts
        // rather than accepting a changed journal after provider readback.
        if canonical_digest(1, &self.release_record_receipt(&r)?)? != canonical_digest(1, &receipt)?
        {
            return Err(production_error(
                "historical effect receipt changed during observation",
            ));
        }
        self.require_retired_release_effect(&r, receipt.as_ref())?;
        Ok(observed)
    }
    fn require_retired_release_effect(
        &self,
        r: &ReleaseEffectRecord,
        receipt: Option<&Receipt>,
    ) -> Result<()> {
        let owner = self.host.operation(r.admission_operation)?;
        if owner.phase != crate::control::ReceiptPhase::Reconciled
            || owner.observed_result.as_ref().is_none_or(|v| {
                v["local_execution"] != "exited"
                    || v["unresolved_effects"]
                        .as_array()
                        .is_none_or(|effects| !effects.is_empty())
            })
        {
            return Err(production_error(
                "historical release admission is not terminal and fully reconciled",
            ));
        }
        if let Some(receipt) = receipt
            && (receipt.phase != crate::control::ReceiptPhase::Reconciled
                || receipt.observed_result.as_ref().is_none_or(|v| {
                    v["release_plan"] != r.plan_digest
                        || v["step"] != r.payload.step_id
                        || v["outcome"] != "succeeded"
                }))
        {
            return Err(production_error(
                "historical release effect is not an exactly reconciled success",
            ));
        }
        Ok(())
    }
    fn release_record_receipt(&self, r: &ReleaseEffectRecord) -> Result<Option<Receipt>> {
        let contract = self.contracts.get(&r.scope.repository).ok_or_else(|| {
            production_error("retained effect repository is no longer configured")
        })?;
        if contract.digest()? != r.scope.contract_digest
            || self.host.snapshot()?.intent.runtime_id != r.scope.runtime_id
        {
            return Err(production_error(
                "retained effect runtime or repository contract changed",
            ));
        }
        let owner = self.host.operation(r.admission_operation)?;
        let work = crate::authority::IssueKey::new(
            &r.scope.work.linear_endpoint,
            r.scope.work.linear_workspace,
            r.scope.work.parent,
        )?
        .identity()?;
        if owner.kind != "admission"
            || [
                "run",
                "metadata",
                "continuation",
                "source_advanced",
                "worker_preparation",
                "review_round",
                "review_round_preparation",
                "review_round_resume",
                "review_round_recovery",
                "session_resume",
                "session_recovery_preparation",
            ]
            .iter()
            .any(|key| owner.expected.contains_key(*key))
            || owner.runtime_id != r.scope.runtime_id
            || owner.canonical_targets != vec![work]
            || owner.expected.get("scope") != Some(&r.admission_scope_digest)
        {
            return Err(production_error(
                "retained effect lacks exact actual admission ownership",
            ));
        }

        let receipt = r.request_id.map(|id| self.host.operation(id)).transpose()?;
        if let Some(receipt) = &receipt
            && (receipt.runtime_id != r.scope.runtime_id
                || receipt.expected.get("release_plan") != Some(&r.plan_digest)
                || receipt.expected.get("release_step") != Some(&r.payload.step_id)
                || receipt.expected.get("release_attempt") != Some(&r.payload.attempt.to_string())
                || receipt.expected.get("admitted_scope") != Some(&r.admission_scope_digest)
                || receipt.expected.get("admission_operation")
                    != Some(&r.admission_operation.to_string()))
        {
            return Err(production_error(
                "release effect lacks exact durable receipt lineage",
            ));
        }
        Ok(receipt)
    }
    async fn observe_release_record(
        &self,
        plan: &ReleasePlan,
        r: &ReleaseEffectRecord,
        observer: &ProductionObserver,
    ) -> Result<ObservedReleaseStep> {
        let contract = self.contracts.get(&r.scope.repository).ok_or_else(|| {
            production_error("retained effect repository is no longer configured")
        })?;
        if contract.digest()? != r.scope.contract_digest
            || self.host.snapshot()?.intent.runtime_id != r.scope.runtime_id
        {
            return Err(production_error(
                "retained effect runtime or repository contract changed",
            ));
        }
        let receipt = self.release_record_receipt(r)?;
        let mut observed = ObservedReleaseStep {
            plan_digest: r.plan_digest.clone(),
            step_id: r.payload.step_id.clone(),
            attempt: r.payload.attempt,
            receipt: receipt.clone(),
            outcome: ReleaseStepOutcome::Pending,
            effect: None,
            observed_at: Utc::now(),
            historical_target: None,
        };
        if let Some(receipt) = receipt.as_ref().filter(|receipt| {
            receipt.phase == crate::control::ReceiptPhase::Reconciled
                && receipt
                    .observed_result
                    .as_ref()
                    .is_some_and(|result| result["effect"] == "not_sent")
        }) {
            let no_dispatch = NoDispatchRecord {
                schema_version: 1,
                payload: r.payload.clone(),
                runtime: r.scope.runtime_id,
                work: r.scope.work.clone(),
                admission_operation: r.admission_operation,
                admission_scope: r.admission_scope_digest.clone(),
                provider_receipt: Some((receipt.operation_id, receipt.payload_digest.clone())),
                reason: "authenticated provider preflight did not dispatch".into(),
                observed_at: Utc::now(),
            };
            let _lock = self.records.section_lock("release-effects.lock")?;
            let leaf = no_dispatch_leaf(plan.operation_id, &r.payload.step_id, r.payload.attempt);
            match self.records.read::<Signed<NoDispatchRecord>>(&leaf) {
                Ok(_) => {}
                Err(e) if e.code == "record_missing" => self
                    .records
                    .write(&leaf, &self.key.sign("release_no_dispatch", &no_dispatch)?)?,
                Err(e) => return Err(e),
            }
            return self
                .read_no_dispatch(plan, &r.payload.step_id, r.payload.attempt)?
                .ok_or_else(|| production_error("no-dispatch observation failed to persist"));
        }
        self.github()?.identity().await?;
        match &r.payload.kind {
            StepKind::Merge {
                repository,
                pull_request,
                production_base,
                candidate,
                ..
            } => {
                let pr = self
                    .github()?
                    .pull_request(&contract.provider, *pull_request)
                    .await?;
                if pr.head_sha != r.scope.revision.head
                    || pr.base_ref != "main"
                    || pr.head_repository_id != Some(contract.provider.id)
                {
                    return Err(production_error("observed production PR identity changed"));
                }
                if pr.merged == Some(true) {
                    let commit = pr.merge_commit_sha.ok_or_else(|| {
                        production_error("merged PR has no actual squash identity")
                    })?;
                    let object = self.github()?.commit(&contract.provider, &commit).await?;
                    if object["sha"].as_str() != Some(&commit)
                        || object.pointer("/tree/sha").and_then(Value::as_str)
                            != Some(&candidate.tree)
                        || object["parents"].as_array().is_none_or(|parents| {
                            parents.len() != 1
                                || parents[0]["sha"].as_str() != Some(&production_base.commit)
                        })
                    {
                        return Err(Error::new(
                            "release_provenance",
                            "actual production squash differs from tested tree or recorded base; retain partial effect",
                        ));
                    }
                    observed.effect = Some(ObservedEffect::Merge {
                        repository: repository.clone(),
                        pull_request: *pull_request,
                        commit: crate::git::Revision {
                            commit,
                            tree: candidate.tree.clone(),
                        },
                        parent_commit: production_base.commit.clone(),
                    });
                    observed.outcome = ReleaseStepOutcome::Succeeded;
                } else if pr.state == "closed" {
                    observed.outcome = ReleaseStepOutcome::Failed(
                        "production PR closed without its expected merge".into(),
                    );
                }
            }
            StepKind::Deploy { .. } | StepKind::DeployExisting { .. } => {
                let receipt = receipt.as_ref().ok_or_else(|| {
                    production_error("deployment has no durable dispatch receipt")
                })?;
                let run_id = remote_id(receipt, "workflow_run_id")?
                    .parse()
                    .map_err(|_| production_error("deployment run ID invalid"))?;
                let run = self
                    .github()?
                    .workflow_run(&contract.provider, run_id, None)
                    .await?;
                let configured = configured_deployment(contract, &r.payload.kind)?;
                if run.workflow_id != configured.workflow.workflow_id
                    || run.head_sha != configured.workflow.workflow_revision
                    || run.event != "workflow_dispatch"
                    || run.actor_id.to_string() != self.github()?.identity().await?.account_id
                    || run.triggering_actor_id != run.actor_id
                {
                    return Err(production_error(
                        "observed deployment is not the owned exact workflow origin",
                    ));
                }
                if run.status == "completed" {
                    let conclusion = run.conclusion.as_deref().ok_or_else(|| {
                        production_error("completed deployment workflow lacks a known conclusion")
                    })?;
                    if conclusion != "success" {
                        observed.outcome = ReleaseStepOutcome::Failed(format!(
                            "deployment workflow concluded {}",
                            conclusion
                        ));
                    } else {
                        let target = observer.recover_target(contract, &configured.id).await?;
                        if target.run_id() != run.id || target.run_attempt() != run.attempt {
                            return Err(production_error(
                                "owned deployment attempt was replaced or rerun",
                            ));
                        }
                        observed.effect =
                            Some(self.release_target_effect(plan, r, &target, false)?);
                        observed.outcome = ReleaseStepOutcome::Succeeded;
                    }
                }
            }
            StepKind::Verify { .. } | StepKind::VerifyDeployed { .. } => {
                let configured = configured_deployment(contract, &r.payload.kind)?;
                let target = observer.recover_target(contract, &configured.id).await?;
                // Verification must refer to this plan's actual deployment step,
                // not another successful deployment at the same source.
                let deployment = match &r.payload.kind {
                    StepKind::Verify {
                        deployment_step, ..
                    }
                    | StepKind::VerifyDeployed {
                        deployment_step, ..
                    } => self.load_release_effect(
                        plan,
                        deployment_step,
                        *r.payload
                            .prerequisite_attempts
                            .get(deployment_step)
                            .ok_or_else(|| {
                                production_error(
                                    "verification lacks exact deployment attempt lineage",
                                )
                            })?,
                    )?,
                    _ => unreachable!(),
                };
                self.release_target_effect(plan, &deployment, &target, false)?;
                observed.effect = Some(self.release_target_effect(plan, r, &target, true)?);
                observed.outcome = ReleaseStepOutcome::Succeeded;
            }
            StepKind::VerifyExisting {
                repository,
                target,
                source,
                artifact_digest,
            } => {
                let configured = configured_deployment(contract, &r.payload.kind)?;
                let proof = observer
                    .verify_existing_target(
                        contract,
                        &configured.target_id,
                        source,
                        artifact_digest,
                    )
                    .await?;
                observed.effect = Some(ObservedEffect::Verification {
                    repository: repository.clone(),
                    target: target.clone(),
                    source_commit: source.commit.clone(),
                    tree: source.tree.clone(),
                    artifact_digest: artifact_digest.clone(),
                    evidence_digest: proof.digest()?,
                });
                observed.outcome = ReleaseStepOutcome::Succeeded;
            }
        }
        Ok(observed)
    }
    fn release_target_effect(
        &self,
        plan: &ReleasePlan,
        r: &ReleaseEffectRecord,
        target: &VerifiedTargetDeployment,
        verification: bool,
    ) -> Result<ObservedEffect> {
        let (repository, resource, tree, artifact) = match &r.payload.kind {
            StepKind::Deploy {
                repository,
                target,
                expected_tree,
                expected_artifact,
                ..
            }
            | StepKind::Verify {
                repository,
                target,
                expected_tree,
                expected_artifact,
                ..
            } => (repository, target, expected_tree, expected_artifact),
            StepKind::DeployExisting {
                repository,
                target,
                source,
                expected_artifact,
                ..
            }
            | StepKind::VerifyDeployed {
                repository,
                target,
                source,
                expected_artifact,
                ..
            } => (repository, target, &source.tree, expected_artifact),
            _ => return Err(production_error("target proof cannot establish a merge")),
        };
        let t = target.target();
        if &t.repository != repository
            || &(ResourceKey::Target {
                provider: t.provider.clone(),
                id: t.target.clone(),
            }) != resource
            || &t.source.tree != tree
            || Some(&t.source.commit) != r.payload.exact_source_commit.as_ref()
            || artifact
                .as_ref()
                .is_some_and(|digest| digest != &t.artifact_digest)
            || (!verification
                && (target.operation_id() != r.admission_operation
                    || Some(target.dispatch_request()) != r.request_id))
        {
            return Err(production_error(
                "target proof differs from exact release source/artifact/owned dispatch",
            ));
        }
        if verification {
            Ok(ObservedEffect::Verification {
                repository: repository.clone(),
                target: resource.clone(),
                source_commit: t.source.commit.clone(),
                tree: t.source.tree.clone(),
                artifact_digest: t.artifact_digest.clone(),
                evidence_digest: target.digest()?,
            })
        } else {
            Ok(ObservedEffect::Deployment {
                repository: repository.clone(),
                target: resource.clone(),
                source_commit: t.source.commit.clone(),
                tree: t.source.tree.clone(),
                artifact_digest: t.artifact_digest.clone(),
                deployment_id: target.deployment_id().to_string(),
                run: OwnedRun {
                    repository: repository.clone(),
                    run_id: target.run_id(),
                    attempt: u32::try_from(target.run_attempt())
                        .map_err(|_| production_error("run attempt exceeds supported bound"))?,
                    operation_id: plan.operation_id,
                    actor_account_id: target.actor_account_id(),
                    origin_digest: target.origin_digest()?,
                },
            })
        }
    }
}
fn check_historical_target(
    r: &ReleaseEffectRecord,
    target: &crate::evidence::ProductionTargetBaseline,
) -> Result<()> {
    let (repository, resource, tree, artifact) = match &r.payload.kind {
        StepKind::Deploy {
            repository,
            target,
            expected_tree,
            expected_artifact,
            ..
        }
        | StepKind::Verify {
            repository,
            target,
            expected_tree,
            expected_artifact,
            ..
        } => (
            repository,
            target,
            expected_tree,
            expected_artifact.as_deref(),
        ),
        StepKind::DeployExisting {
            repository,
            target,
            source,
            expected_artifact,
            ..
        }
        | StepKind::VerifyDeployed {
            repository,
            target,
            source,
            expected_artifact,
            ..
        } => (
            repository,
            target,
            &source.tree,
            expected_artifact.as_deref(),
        ),
        StepKind::VerifyExisting {
            repository,
            target,
            source,
            artifact_digest,
        } => (
            repository,
            target,
            &source.tree,
            Some(artifact_digest.as_str()),
        ),
        _ => {
            return Err(production_error(
                "historical target cannot establish a merge",
            ));
        }
    };
    if &target.repository != repository
        || &(ResourceKey::Target {
            provider: target.provider.clone(),
            id: target.target.clone(),
        }) != resource
        || &target.source.tree != tree
        || r.payload.exact_source_commit.as_ref() != Some(&target.source.commit)
        || artifact.is_some_and(|digest| digest != target.artifact_digest)
    {
        return Err(production_error(
            "historical target differs from exact release source/artifact scope",
        ));
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductionCandidateRecord {
    schema_version: u32,
    operation: Uuid,
    scope: DeliveryScope,
    candidate: crate::git::CandidateManifest,
    evidence_token: String,
    branch: String,
    pull_request: u64,
    creation_receipt: Uuid,
    review: Signed<evidence::ReviewAttestation>,
    review_publication: Publication,
    review_status: evidence::ReviewStatus,
    sources: Vec<ProductionSource>,
    dispatches: BTreeMap<u64, Uuid>,
    challenge: HostedChallenge,
}
#[derive(Clone, Debug)]
pub struct PreparedProductionCandidate {
    signed: Signed<ProductionCandidateRecord>,
}
impl PreparedProductionCandidate {
    pub fn operation_id(&self) -> Uuid {
        self.signed.payload.operation
    }
    pub fn pull_request(&self) -> u64 {
        self.signed.payload.pull_request
    }
    pub fn revision(&self) -> &RevisionBinding {
        &self.signed.payload.scope.revision
    }
    pub fn branch(&self) -> &str {
        &self.signed.payload.branch
    }
    pub fn candidate(&self) -> &crate::git::CandidateManifest {
        &self.signed.payload.candidate
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductionValidationRecord {
    schema_version: u32,
    preparation_digest: String,
    protection: evidence::RepositoryProtection,
    checks: Vec<CheckRun>,
    proofs: Vec<crate::providers::VerifiedHostedJob>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChildValidationRecord {
    scope_digest: String,
    checks: Vec<CheckRun>,
    proofs: Vec<crate::providers::VerifiedHostedJob>,
}
fn production_error(message: &str) -> Error {
    Error::new("production_validation", message)
}
fn production_leaf(operation: Uuid) -> String {
    format!("production-candidate-{operation}.json")
}

impl DeliveryCoordinator {
    /// Ordinary changed-child hosted CI uses the same bounded private artifact
    /// protocol as production preparation. Each configured suite is dispatched
    /// once with exact source and the persisted host nonce. A pending result
    /// leaves the admitted operation and its original receipts available to poll.
    pub async fn acquire_child_validation(
        &self,
        admission: &DeliveryAdmission,
        verifier: &HostedProofVerifier,
    ) -> Result<HostedPassed> {
        self.current(admission, &[DeliveryPhase::HostedValidation])?;
        if admission.scope.work.child.is_none() {
            return Err(production_error(
                "child hosted validation needs actual child authority",
            ));
        }
        self.acquire_dispatched_validation(
            admission,
            verifier,
            self.contract(admission)?.configuration.staging_ci()?,
            CheckKind::HostedCi,
            "child",
        )
        .await
    }
    async fn acquire_dispatched_validation(
        &self,
        admission: &DeliveryAdmission,
        verifier: &HostedProofVerifier,
        suites: &[CiSuite],
        kind: CheckKind,
        prefix: &str,
    ) -> Result<HostedPassed> {
        // Cached results cannot make an unresolved current policy complete.
        let required = hosted_requirements(suites, kind.clone())?;
        let leaf = format!("{prefix}-hosted-{}.json", admission.operation_id());
        match self.records.read::<Signed<ChildValidationRecord>>(&leaf) {
            Ok(signed) => {
                self.key
                    .verify(&format!("{prefix}_hosted_validation"), &signed)?;
                if signed.payload.scope_digest != admission.scope.digest()? {
                    return Err(production_error(
                        "retained child CI belongs to another scope",
                    ));
                }
                let passed = HostedPassed {
                    work: admission.scope.work.clone(),
                    revision: admission.scope.revision.clone(),
                    kind: kind.clone(),
                    checks: signed.payload.checks,
                    proofs: signed.payload.proofs,
                };
                evidence::verify_checks(
                    &required,
                    &admission.scope.work,
                    &admission.scope.revision,
                    &CheckSnapshot {
                        pagination_complete: true,
                        observed_at: Utc::now(),
                        runs: passed.checks.clone(),
                    },
                )?;
                self.reobserve_hosted(admission, &passed).await?;
                return Ok(passed);
            }
            Err(e) if e.code == "record_missing" => {}
            Err(e) => return Err(e),
        }
        let contract = self.contract(admission)?;
        if suites
            .iter()
            .any(|s| s.proof.event_name != "workflow_dispatch")
        {
            return Err(Error::new(
                "incompletely_prepared",
                "automatic child validation requires configured pinned workflow_dispatch proof routes with source_sha and operation_id inputs",
            ));
        }
        let challenge = self.prepare_hosted_challenge(admission)?;
        let github = self.github()?;
        let mut tokens = Vec::new();
        let mut artifacts = Vec::new();
        for suite in suites {
            let (locator, reference) = suite
                .proof
                .workflow_ref
                .rsplit_once('@')
                .ok_or_else(|| production_error("child workflow ref absent"))?;
            let path = locator
                .splitn(3, '/')
                .nth(2)
                .ok_or_else(|| production_error("child workflow path absent"))?;
            let reference = reference.strip_prefix("refs/").ok_or_else(|| {
                production_error("child dispatch needs an exact pinned branch/tag reference")
            })?;
            let receipt = self
                .effect(
                    admission,
                    &format!("{prefix}-ci-{}", suite.workflow_id),
                    ProviderMutation::GithubDispatch {
                        repository: contract.provider.clone(),
                        workflow_id: suite.workflow_id,
                        workflow_path: path.into(),
                        reference: reference.into(),
                        workflow_sha: suite.proof.workflow_sha.clone(),
                        source_sha: admission.scope.revision.tested_commit.clone(),
                        expected_production: None,
                        source_input: "source_sha".into(),
                        operation_input: "operation_id".into(),
                        inputs: BTreeMap::from([
                            ("kegari_runtime_id".into(), challenge.runtime_id.to_string()),
                            (
                                "kegari_phase_operation".into(),
                                challenge.operation_id.to_string(),
                            ),
                            ("kegari_challenge".into(), challenge.challenge.to_string()),
                        ]),
                    },
                )
                .await?;
            let run_id = remote_id(&receipt, "workflow_run_id")?
                .parse()
                .map_err(|_| production_error("child CI run ID invalid"))?;
            let run = github
                .workflow_run(&contract.provider, run_id, None)
                .await?;
            if run.attempt != 1 {
                return Err(production_error(
                    "child validation run was rerun outside its owned dispatch attempt",
                ));
            }
            if run.status != "completed" {
                return Err(Error::new(
                    "operation_pending",
                    "configured child validation remains queued or running",
                ));
            }
            let name = format!(
                "kegari-{prefix}-{}-{}-{}",
                admission.operation_id(),
                suite.workflow_id,
                run.attempt
            );
            let listed = github.artifacts(&contract.provider, run.id).await?;
            let matching: Vec<_> = listed
                .iter()
                .filter(|a| a.data["name"].as_str() == Some(&name))
                .collect();
            if matching.len() != 1 {
                return Err(production_error(
                    "child CI requires one exact current proof artifact per configured suite",
                ));
            }
            let archive = github
                .artifact_archive(&contract.provider, matching[0].id, run.id, &run.head_sha)
                .await?;
            let source = read_hosted_source_archive(&archive.bytes)?;
            if source.schema_version != 1
                || source.runtime_id != admission.scope.runtime_id
                || source.phase_operation != admission.operation_id()
                || source.dispatch_request != receipt.request_id
                || source.challenge != challenge.challenge
                || source.repository_id != contract.provider.id
                || source.run_id != run.id
                || source.run_attempt != run.attempt
                || source.tested_tree != admission.scope.revision.tested_tree
                || source.outputs.get("source_sha") != Some(&admission.scope.revision.tested_commit)
                || source.proofs.is_empty()
                || source.proofs.len() > 256
            {
                return Err(production_error(
                    "child artifact differs from exact source/tree/dispatch/nonce contract",
                ));
            }
            let mut seen = BTreeSet::new();
            for proof in source.proofs {
                if proof.token.len() > 65536 || !seen.insert(proof.check_run_id) {
                    return Err(production_error(
                        "duplicate or unbounded child hosted proof",
                    ));
                }
                tokens.push(HostedToken {
                    token: proof.token,
                    expectation: HostedProofExpectation {
                        context_digest: None,
                        runtime_id: admission.scope.runtime_id,
                        operation_id: admission.operation_id(),
                        challenge: challenge.challenge,
                        repository_id: contract.provider.id,
                        repository_owner_id: contract.provider.owner_id,
                        run_id: run.id,
                        run_attempt: run.attempt,
                        check_run_id: proof.check_run_id,
                        actor_id: run.actor_id,
                        trigger_sha: run.head_sha.clone(),
                        tested_source_sha: admission.scope.revision.tested_commit.clone(),
                        workflow_ref: suite.proof.workflow_ref.clone(),
                        workflow_sha: suite.proof.workflow_sha.clone(),
                        job_workflow_ref: suite.proof.job_workflow_ref.clone(),
                        job_workflow_sha: suite.proof.job_workflow_sha.clone(),
                        event_name: suite.proof.event_name.clone(),
                    },
                });
            }
            artifacts.push(ArtifactReference {
                reference: format!(
                    "github-artifact:{}/{}/{}",
                    contract.repository.hostname, contract.provider.id, archive.artifact_id
                ),
                digest: archive.archive_digest,
            });
        }
        let mut hosted = self
            .observe_hosted_checks(admission, kind, &tokens, verifier)
            .await?;
        for check in &mut hosted.checks {
            check.artifacts.extend(artifacts.iter().cloned());
        }
        self.records.write(
            &leaf,
            &self.key.sign(
                &format!("{prefix}_hosted_validation"),
                &ChildValidationRecord {
                    scope_digest: admission.scope.digest()?,
                    checks: hosted.checks.clone(),
                    proofs: hosted.proofs.clone(),
                },
            )?,
        )?;
        Ok(hosted)
    }
    /// Validate an unchanged exact production candidate through the same
    /// reviewed private artifact protocol, without inventing a production PR.
    pub async fn acquire_existing_production_validation(
        &self,
        admission: &DeliveryAdmission,
        parent: &evidence::VerifiedParent,
        integrations: &[crate::git::VerifiedProductionIntegration],
        verifier: &HostedProofVerifier,
    ) -> Result<evidence::VerifiedProduction> {
        self.current(admission, &[DeliveryPhase::Release])?;
        self.existing_candidate(admission, parent).await?;
        let hosted = self
            .acquire_dispatched_validation(
                admission,
                verifier,
                self.contract(admission)?
                    .configuration
                    .production_ci_policy()?,
                CheckKind::ProductionCi,
                "existing",
            )
            .await?;
        self.verify_existing_validation(admission, parent, integrations, &hosted)
            .await
    }
    pub async fn hydrate_existing_production_validation(
        &self,
        observation: &DeliveryObservation,
        parent: &evidence::VerifiedParent,
        integrations: &[crate::git::VerifiedProductionIntegration],
        operation: Uuid,
    ) -> Result<evidence::VerifiedProduction> {
        self.current(
            observation,
            &[DeliveryPhase::Recovery, DeliveryPhase::Release],
        )?;
        let record: Signed<ChildValidationRecord> = self
            .records
            .read(&format!("existing-hosted-{operation}.json"))?;
        self.key.verify("existing_hosted_validation", &record)?;
        let proof = HostedPassed {
            work: observation.scope.work.clone(),
            revision: observation.scope.revision.clone(),
            kind: CheckKind::ProductionCi,
            checks: record.payload.checks,
            proofs: record.payload.proofs,
        };
        self.verify_existing_validation(observation, parent, integrations, &proof)
            .await
    }
    async fn existing_candidate(
        &self,
        observation: &DeliveryObservation,
        parent: &evidence::VerifiedParent,
    ) -> Result<()> {
        if observation.scope.work != parent.manifest().work
            || observation.scope.work.child.is_some()
            || !parent.manifest().candidates.iter().any(|candidate| {
                candidate.repository == observation.scope.repository
                    && candidate.ordered_delta_digests.is_empty()
                    && candidate.result.as_ref() == Some(&candidate.production)
                    && candidate.production.commit == observation.scope.revision.head
                    && candidate.production.commit == observation.scope.revision.base
                    && candidate.production.commit == observation.scope.revision.tested_commit
                    && candidate.production.tree == observation.scope.revision.tested_tree
            })
        {
            return Err(production_error(
                "existing production scope differs from unchanged parent candidate",
            ));
        }
        let current = self.main_revision(observation).await?;
        if current.commit != observation.scope.revision.tested_commit
            || current.tree != observation.scope.revision.tested_tree
        {
            return Err(production_error(
                "existing production changed before validation",
            ));
        }
        Ok(())
    }
    async fn verify_existing_validation(
        &self,
        observation: &DeliveryObservation,
        parent: &evidence::VerifiedParent,
        integrations: &[crate::git::VerifiedProductionIntegration],
        hosted: &HostedPassed,
    ) -> Result<evidence::VerifiedProduction> {
        self.existing_candidate(observation, parent).await?;
        let mut current = observation.clone();
        current.scope.phase = DeliveryPhase::Release;
        self.reobserve_hosted(&current, hosted).await?;
        let requirements = hosted_requirements(
            self.contract(observation)?
                .configuration
                .production_ci_policy()?,
            CheckKind::ProductionCi,
        )?;
        let verified = evidence::verify_existing_production(
            parent,
            &observation.scope.revision,
            &requirements,
            &CheckSnapshot {
                pagination_complete: true,
                observed_at: Utc::now(),
                runs: hosted.checks.clone(),
            },
            integrations,
        )?;
        self.existing_candidate(observation, parent).await?;
        Ok(verified)
    }
    fn production_parent(
        &self,
        observation: &DeliveryObservation,
        parent: &evidence::VerifiedParent,
        candidate: &crate::git::CandidateManifest,
    ) -> Result<()> {
        let manifest = parent.manifest();
        let result = candidate
            .result
            .as_ref()
            .ok_or_else(|| production_error("candidate result missing"))?;
        if observation.scope.work.child.is_some()
            || manifest.work != observation.scope.work
            || !manifest.candidates.contains(candidate)
            || candidate.ordered_delta_digests.is_empty()
            || candidate.repository != observation.scope.repository
            || result.commit != observation.scope.revision.head
            || result.commit != observation.scope.revision.tested_commit
            || result.tree != observation.scope.revision.tested_tree
            || candidate.production.commit != observation.scope.revision.base
            || observation
                .scope
                .candidate_vector_digest
                .as_ref()
                .is_some_and(|d| manifest.candidate_vector_digest().ok().as_ref() != Some(d))
        {
            return Err(production_error(
                "production candidate differs from the complete verified parent vector",
            ));
        }
        Ok(())
    }

    async fn production_sources(
        &self,
        observation: &DeliveryObservation,
        sources: &[ProductionSource],
    ) -> Result<Vec<evidence::VerifiedReview>> {
        let mut verified = Vec::new();
        let mut seen = BTreeSet::new();
        for source in sources {
            self.key.verify("child_result", &source.record)?;
            let r = &source.record.payload;
            let work = &r.evidence.work;
            if work.parent != observation.scope.work.parent
                || work.linear_endpoint != observation.scope.work.linear_endpoint
                || work.linear_workspace != observation.scope.work.linear_workspace
                || work.generation != observation.scope.work.generation
                || work
                    .child
                    .is_none_or(|id| !observation.gate.required_children().contains(&id))
                || r.evidence.revision.repository != observation.scope.repository
                || !seen.insert(source.publication.record_id.clone())
            {
                return Err(production_error(
                    "source review is duplicate or outside the current required child generation",
                ));
            }
            let mut child = observation.clone();
            child.scope.work = work.clone();
            child.scope.revision = r.evidence.revision.clone();
            child.admitted = None; // Read-only child provenance, under the current parent gate.
            self.linear_publication(
                &child,
                work.child.unwrap(),
                &source.publication.record_id,
                "child_result",
                &source.record,
            )
            .await?;
            let review = evidence::verify_review(
                &r.review,
                &r.review_publication,
                r.review_status.as_ref(),
                &self.key,
                evidence::ReviewRequirements {
                    work,
                    revision: &r.evidence.revision,
                    delta_digest: &r.review.payload.delta_digest,
                    writer: &r.review_publication.writer_account_id,
                },
            )?;
            self.reobserve_review(
                &child,
                &PublishedReview {
                    signed: r.review.clone(),
                    publication: r.review_publication.clone(),
                    status: r.review_status.clone(),
                    verified: review.clone(),
                },
            )
            .await?;
            if review.digest() != r.evidence.review_digest {
                return Err(production_error("child result review binding changed"));
            }
            verified.push(review);
        }
        Ok(verified)
    }

    async fn production_objects(
        &self,
        admission: &DeliveryAdmission,
        slice: &IsolatedSlice,
    ) -> Result<crate::git::GitPublicationObjects> {
        self.key.verify("isolated_slice", &slice.record)?;
        if slice.record.payload.candidate != *slice.manifest() {
            return Err(production_error("isolated candidate record differs"));
        }
        let path = slice.path().to_path_buf();
        let base = slice.manifest().production.clone();
        let result = slice
            .manifest()
            .result
            .clone()
            .ok_or_else(|| production_error("candidate result absent"))?;
        let gate = admission.gate.clone();
        let check = self.current_gate.clone();
        let host = self.host.clone();
        let permit = admission.permit.clone();
        let start: crate::git::GitStartGate = Arc::new(move |spawn| {
            host.validate_permit(&permit, || {
                check(&gate)?;
                spawn()
            })
        });
        let (effect, running) = self.host.begin_effect(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: admission.scope.runtime_id,
                actor: Actor {
                    principal: None,
                    identity: format!("owner:{}", admission.permit.lease.owner_id),
                    roles: BTreeSet::new(),
                },
                kind: "production_object_export".into(),
                canonical_targets: vec![admission.scope.repository.id.clone()],
                expected: BTreeMap::from([("delivery_scope".into(), admission.scope.digest()?)]),
                payload: json!({"candidate":slice.manifest()}),
            },
            &admission.permit,
            || (self.current_gate)(&admission.gate),
            |receipt| {
                let running = self.git.start_journaled_operation(
                    crate::git::GitLimits::default(),
                    start,
                    self.processes.clone(),
                    admission.permit.clone(),
                    move |git| git.publication_objects(&path, &base, &result),
                )?;
                self.executions.register(
                    admission.operation_id(),
                    receipt.operation_id,
                    Arc::new(running.cancellation()),
                )?;
                Ok(running)
            },
        )?;
        let cancellation = running.cancellation();
        let result = running.wait().await;
        if !matches!(cancellation.exited(), Ok(true)) {
            self.host.fence(admission.operation_id())?;
            self.executions
                .cancel_fenced(&self.host, admission.operation_id())?;
            return Err(Error::new(
                "operation_uncertain",
                "production object export exit is unconfirmed",
            ));
        }
        self.executions
            .remove(admission.operation_id(), effect.operation_id())?;
        self.host.complete_effect(
            effect,
            &admission.permit,
            || (self.current_gate)(&admission.gate),
            match &result {
                Ok(objects) => json!({"revision":objects.revision()}),
                Err(error) => json!({"error":error}),
            },
        )?;
        result
    }

    /// Preparation performs no production merge or deployment and requires no
    /// human acceptance. Every byte comes from the already assembled slice.
    pub async fn prepare_production_candidate(
        &self,
        admission: &DeliveryAdmission,
        parent: &evidence::VerifiedParent,
        slice: &IsolatedSlice,
        sources: &[PublishedChild],
    ) -> Result<PreparedProductionCandidate> {
        use base64::Engine as _;
        self.current(admission, &[DeliveryPhase::Release])?;
        require_effect_policy(self.contract(admission)?, DeliveryPhase::Release)?;
        self.production_parent(admission, parent, slice.manifest())?;
        if self.main_revision(admission).await? != slice.manifest().production {
            return Err(production_error(
                "main changed before production preparation",
            ));
        }
        match self
            .records
            .read::<Signed<ProductionCandidateRecord>>(&production_leaf(admission.operation_id()))
        {
            Ok(_) => {
                return self
                    .hydrate_production_candidate(admission, parent, admission.operation_id())
                    .await;
            }
            Err(e) if e.code == "record_missing" => {}
            Err(e) => return Err(e),
        }
        let source_records: Vec<_> = sources
            .iter()
            .map(|s| ProductionSource {
                record: s.signed.clone(),
                publication: s.publication.clone(),
            })
            .collect();
        let reviews = self.production_sources(admission, &source_records).await?;
        // Prove complete reviewed patch accounting before the first remote write.
        evidence::derive_review(
            parent,
            slice.manifest(),
            admission.scope.revision.clone(),
            1,
            &reviews,
            evidence::HostOperation {
                runtime_id: admission.scope.runtime_id,
                owner_generation: admission.permit.lease.owner_generation,
                operation_id: admission.operation_id(),
            },
        )?;
        let objects = self.production_objects(admission, slice).await?;
        let contract = self.contract(admission)?;
        let candidate_digest = canonical_digest(1, slice.manifest())?;
        let branch = format!(
            "kegari-release/{}/{}",
            parent.manifest().work.parent,
            candidate_digest.rsplit(':').next().unwrap()
        );
        let challenge = self.prepare_hosted_challenge(admission)?;
        for (sha, bytes) in objects.blobs() {
            let receipt = self
                .effect(
                    admission,
                    &format!("production-blob-{sha}"),
                    ProviderMutation::GithubBlob {
                        repository: contract.provider.clone(),
                        content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                    },
                )
                .await?;
            if remote_id(&receipt, "object_sha")? != *sha {
                return Err(production_error(
                    "published blob differs from exact isolated bytes",
                ));
            }
        }
        let entries = objects
            .entries()
            .iter()
            .map(|e| {
                Ok(crate::providers::TreeEntry {
                    path: e.path.clone(),
                    mode: match e.mode.as_str() {
                        "100644" => crate::providers::TreeMode::File,
                        "100755" => crate::providers::TreeMode::Executable,
                        "120000" => crate::providers::TreeMode::Symlink,
                        _ => return Err(production_error("unsupported candidate object mode")),
                    },
                    sha: e.object.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let receipt = self
            .effect(
                admission,
                "production-tree",
                ProviderMutation::GithubTree {
                    repository: contract.provider.clone(),
                    base_tree: objects.base().tree.clone(),
                    entries,
                },
            )
            .await?;
        if remote_id(&receipt, "object_sha")? != objects.revision().tree {
            return Err(production_error(
                "published tree differs from tested isolated candidate",
            ));
        }
        let identity = |i: &crate::git::GitCommitIdentity| crate::providers::CommitIdentity {
            name: i.name.clone(),
            email: i.email.clone(),
            date: i.date.clone(),
        };
        let receipt = self
            .effect(
                admission,
                "production-commit",
                ProviderMutation::GithubCommit {
                    repository: contract.provider.clone(),
                    message: objects.message().into(),
                    tree: objects.revision().tree.clone(),
                    parent: objects.base().commit.clone(),
                    author: identity(objects.author()),
                    committer: identity(objects.committer()),
                    expected_sha: objects.revision().commit.clone(),
                },
            )
            .await?;
        if remote_id(&receipt, "object_sha")? != objects.revision().commit {
            return Err(production_error(
                "published commit differs from tested Git object",
            ));
        }
        self.effect(
            admission,
            "production-branch",
            ProviderMutation::GithubCreateRef {
                repository: contract.provider.clone(),
                reference: format!("heads/{branch}"),
                head: objects.revision().commit.clone(),
            },
        )
        .await?;
        self.production_sources(admission, &source_records).await?;
        if self.main_revision(admission).await? != slice.manifest().production {
            return Err(production_error(
                "main moved during production object publication",
            ));
        }
        let correlation=self.key.sign("production_pull_request",&json!({"runtime":admission.scope.runtime_id,
            "operation":admission.operation_id(),"work":admission.scope.work,"candidate":slice.manifest(),"evidence_token":parent.token(),"challenge":challenge}))?;
        let receipt = self
            .effect(
                admission,
                "production-pr",
                ProviderMutation::GithubPullRequest {
                    repository: contract.provider.clone(),
                    head_ref: branch.clone(),
                    base_ref: "main".into(),
                    expected_head: objects.revision().commit.clone(),
                    expected_base: objects.base().commit.clone(),
                    title: format!("Release {}", parent.manifest().work.parent),
                    body: record_body("production_pull_request", &correlation)?,
                },
            )
            .await?;
        let pr = remote_id(&receipt, "pull_request")?
            .parse()
            .map_err(|_| production_error("production PR number invalid"))?;
        let reviews = self.production_sources(admission, &source_records).await?;
        let attestation = evidence::derive_review(
            parent,
            slice.manifest(),
            admission.scope.revision.clone(),
            pr,
            &reviews,
            evidence::HostOperation {
                runtime_id: admission.scope.runtime_id,
                owner_generation: admission.permit.lease.owner_generation,
                operation_id: admission.operation_id(),
            },
        )?;
        let review = self
            .publish_review(admission, &self.key.sign("codex_review", &attestation)?)
            .await?;
        let mut dispatches = BTreeMap::new();
        for suite in contract.configuration.production_ci_policy()? {
            if suite.proof.event_name == "workflow_dispatch" {
                let (locator, reference) = suite
                    .proof
                    .workflow_ref
                    .rsplit_once('@')
                    .ok_or_else(|| production_error("production workflow reference absent"))?;
                let path = locator
                    .splitn(3, '/')
                    .nth(2)
                    .ok_or_else(|| production_error("production workflow path absent"))?;
                let reference = reference.strip_prefix("refs/").ok_or_else(|| {
                    production_error("production workflow needs branch/tag dispatch reference")
                })?;
                let dispatched = self
                    .effect(
                        admission,
                        &format!("production-ci-{}", suite.workflow_id),
                        ProviderMutation::GithubDispatch {
                            repository: contract.provider.clone(),
                            workflow_id: suite.workflow_id,
                            workflow_path: path.into(),
                            reference: reference.into(),
                            workflow_sha: suite.proof.workflow_sha.clone(),
                            source_sha: admission.scope.revision.tested_commit.clone(),
                            expected_production: None,
                            source_input: "source_sha".into(),
                            operation_input: "operation_id".into(),
                            inputs: BTreeMap::from([
                                ("kegari_runtime_id".into(), challenge.runtime_id.to_string()),
                                (
                                    "kegari_phase_operation".into(),
                                    challenge.operation_id.to_string(),
                                ),
                                ("kegari_challenge".into(), challenge.challenge.to_string()),
                            ]),
                        },
                    )
                    .await?;
                dispatches.insert(suite.workflow_id, dispatched.request_id);
            }
        }
        let record = ProductionCandidateRecord {
            schema_version: 1,
            operation: admission.operation_id(),
            scope: admission.scope.clone(),
            candidate: slice.manifest().clone(),
            evidence_token: parent.token().into(),
            branch,
            pull_request: pr,
            creation_receipt: receipt.request_id,
            review: review.signed,
            review_publication: review.publication,
            review_status: review
                .status
                .ok_or_else(|| production_error("derived production status absent"))?,
            sources: source_records,
            dispatches,
            challenge,
        };
        let prepared = PreparedProductionCandidate {
            signed: self.key.sign("production_candidate", &record)?,
        };
        self.observe_production_preparation(admission, parent, &prepared)
            .await?;
        self.records
            .write(&production_leaf(admission.operation_id()), &prepared.signed)?;
        Ok(prepared)
    }

    async fn observe_production_preparation(
        &self,
        observation: &DeliveryObservation,
        parent: &evidence::VerifiedParent,
        prepared: &PreparedProductionCandidate,
    ) -> Result<PublishedReview> {
        self.observe_production_preparation_at(observation, parent, prepared, None)
            .await
    }
    async fn observe_production_preparation_at(
        &self,
        observation: &DeliveryObservation,
        parent: &evidence::VerifiedParent,
        prepared: &PreparedProductionCandidate,
        owned_merge: Option<&crate::git::Revision>,
    ) -> Result<PublishedReview> {
        self.current(
            observation,
            &[DeliveryPhase::Release, DeliveryPhase::Recovery],
        )?;
        require_effect_policy(self.contract(observation)?, DeliveryPhase::Release)?;
        self.key.verify("production_candidate", &prepared.signed)?;
        let r = &prepared.signed.payload;
        self.production_parent(observation, parent, &r.candidate)?;
        if r.schema_version != 1
            || r.scope.runtime_id != observation.scope.runtime_id
            || r.scope.work != observation.scope.work
            || r.scope.revision != observation.scope.revision
            || r.scope.contract_digest != observation.scope.contract_digest
            || r.evidence_token != parent.token()
            || r.challenge.operation_id != r.operation
            || r.challenge.admission_digest != r.scope.digest()?
        {
            return Err(production_error(
                "retained production preparation differs from current parent/contract",
            ));
        }
        let contract = self.contract(observation)?;
        let receipt = self.host.operation(r.creation_receipt)?;
        if !matches!(
            receipt.phase,
            crate::control::ReceiptPhase::Acknowledged | crate::control::ReceiptPhase::Reconciled
        ) || receipt.kind != "github_pull_request"
            || receipt.runtime_id != r.scope.runtime_id
            || receipt.expected.get("delivery_scope") != Some(&r.scope.digest()?)
            || remote_id(&receipt, "pull_request")? != r.pull_request.to_string()
        {
            return Err(production_error(
                "production PR lacks its exact durable owned creation receipt",
            ));
        }
        let current = self
            .github()?
            .pull_request(&contract.provider, r.pull_request)
            .await?;
        if current.head_sha != r.scope.revision.head
            || (owned_merge.is_none() && current.base_sha != r.scope.revision.base)
            || current.head_ref != r.branch
            || current.base_ref != "main"
            || match owned_merge {
                None => current.state != "open" || current.merged == Some(true),
                Some(source) => {
                    current.state != "closed"
                        || current.merged != Some(true)
                        || current.merge_commit_sha.as_deref() != Some(&source.commit)
                }
            }
            || current.draft
            || current.head_repository_id != Some(contract.provider.id)
            || current.author_id.to_string() != self.github()?.identity().await?.account_id
            || self.main_revision(observation).await?
                != owned_merge.unwrap_or(&r.candidate.production).clone()
        {
            return Err(production_error(
                "production PR or main no longer matches the isolated candidate",
            ));
        }
        let sources = self.production_sources(observation, &r.sources).await?;
        let expected = evidence::derive_review(
            parent,
            &r.candidate,
            r.scope.revision.clone(),
            r.pull_request,
            &sources,
            evidence::HostOperation {
                runtime_id: r.scope.runtime_id,
                owner_generation: r.review.payload.owner_generation,
                operation_id: r.operation,
            },
        )?;
        if r.review.payload != expected {
            return Err(production_error(
                "derived review no longer accounts for exactly the selected child patches",
            ));
        }
        let verified = evidence::verify_review(
            &r.review,
            &r.review_publication,
            Some(&r.review_status),
            &self.key,
            evidence::ReviewRequirements {
                work: &r.scope.work,
                revision: &r.scope.revision,
                delta_digest: &r.review.payload.delta_digest,
                writer: &r.review_publication.writer_account_id,
            },
        )?;
        let review = PublishedReview {
            signed: r.review.clone(),
            publication: r.review_publication.clone(),
            status: Some(r.review_status.clone()),
            verified,
        };
        self.reobserve_review(observation, &review).await?;
        Ok(review)
    }
    pub async fn hydrate_production_candidate(
        &self,
        observation: &DeliveryObservation,
        parent: &evidence::VerifiedParent,
        operation: Uuid,
    ) -> Result<PreparedProductionCandidate> {
        let prepared = PreparedProductionCandidate {
            signed: self.records.read(&production_leaf(operation))?,
        };
        if prepared.operation_id() != operation {
            return Err(production_error("retained preparation operation differs"));
        }
        self.observe_production_preparation(observation, parent, &prepared)
            .await?;
        Ok(prepared)
    }

    /// Download each configured suite's exact attempt artifact. The workflow
    /// contract uses source_sha/operation_id inputs and source_sha result output.
    pub async fn acquire_production_validation(
        &self,
        admission: &DeliveryAdmission,
        prepared: &PreparedProductionCandidate,
        parent: &evidence::VerifiedParent,
        verifier: &HostedProofVerifier,
    ) -> Result<evidence::VerifiedProduction> {
        let review = self
            .observe_production_preparation(admission, parent, prepared)
            .await?;
        let record = &prepared.signed.payload;
        if record.operation != admission.operation_id() {
            return Err(production_error(
                "validation must use its original admitted preparation nonce",
            ));
        }
        let contract = self.contract(admission)?;
        let github = self.github()?;
        let mut tokens = Vec::new();
        let mut artifacts = Vec::new();
        for suite in contract.configuration.production_ci_policy()? {
            let dispatch = record
                .dispatches
                .get(&suite.workflow_id)
                .map(|id| self.host.operation(*id))
                .transpose()?;
            let run = if let Some(receipt) = &dispatch {
                if receipt.expected.get("delivery_scope") != Some(&record.scope.digest()?) {
                    return Err(production_error("CI dispatch belongs to another scope"));
                }
                github
                    .workflow_run(
                        &contract.provider,
                        remote_id(receipt, "workflow_run_id")?
                            .parse()
                            .map_err(|_| production_error("CI run ID invalid"))?,
                        None,
                    )
                    .await?
            } else {
                let mut runs = github
                    .workflow_runs(
                        &contract.provider,
                        suite.workflow_id,
                        Some(&record.scope.revision.head),
                    )
                    .await?;
                runs.retain(|r| r.event == suite.proof.event_name);
                runs.sort_by_key(|r| r.id);
                runs.pop().ok_or_else(|| {
                    production_error("configured production CI has not run on this exact candidate")
                })?
            };
            let name = format!(
                "kegari-production-{}-{}-{}",
                record.operation, suite.workflow_id, run.attempt
            );
            let listed = github.artifacts(&contract.provider, run.id).await?;
            let matching: Vec<_> = listed
                .iter()
                .filter(|a| a.data["name"].as_str() == Some(&name))
                .collect();
            if matching.len() != 1 {
                return Err(production_error(
                    "production suite lacks one unique current proof artifact",
                ));
            }
            let archive = github
                .artifact_archive(&contract.provider, matching[0].id, run.id, &run.head_sha)
                .await?;
            let source = read_hosted_source_archive(&archive.bytes)?;
            if source.schema_version != 1
                || source.runtime_id != record.scope.runtime_id
                || source.phase_operation != record.operation
                || source.dispatch_request
                    != dispatch.as_ref().map_or(record.operation, |r| r.request_id)
                || source.challenge != record.challenge.challenge
                || source.repository_id != contract.provider.id
                || source.run_id != run.id
                || source.run_attempt != run.attempt
                || source.tested_tree != record.scope.revision.tested_tree
                || source.outputs.get("source_sha") != Some(&record.scope.revision.tested_commit)
                || source.proofs.is_empty()
                || source.proofs.len() > 256
            {
                return Err(production_error(
                    "production artifact source/tree/attempt/nonce differs from actual preparation",
                ));
            }
            let mut seen = BTreeSet::new();
            for proof in source.proofs {
                if proof.token.len() > 65536 || !seen.insert(proof.check_run_id) {
                    return Err(production_error("duplicate/unbounded production proof"));
                }
                tokens.push(HostedToken {
                    token: proof.token,
                    expectation: HostedProofExpectation {
                        context_digest: None,
                        runtime_id: record.scope.runtime_id,
                        operation_id: record.operation,
                        challenge: record.challenge.challenge,
                        repository_id: contract.provider.id,
                        repository_owner_id: contract.provider.owner_id,
                        run_id: run.id,
                        run_attempt: run.attempt,
                        check_run_id: proof.check_run_id,
                        actor_id: run.actor_id,
                        trigger_sha: run.head_sha.clone(),
                        tested_source_sha: record.scope.revision.tested_commit.clone(),
                        workflow_ref: suite.proof.workflow_ref.clone(),
                        workflow_sha: suite.proof.workflow_sha.clone(),
                        job_workflow_ref: suite.proof.job_workflow_ref.clone(),
                        job_workflow_sha: suite.proof.job_workflow_sha.clone(),
                        event_name: suite.proof.event_name.clone(),
                    },
                });
            }
            artifacts.push(ArtifactReference {
                reference: format!(
                    "github-artifact:{}/{}/{}",
                    contract.repository.hostname, contract.provider.id, archive.artifact_id
                ),
                digest: archive.archive_digest,
            });
        }
        let mut hosted = self
            .observe_hosted_checks(admission, CheckKind::ProductionCi, &tokens, verifier)
            .await?;
        for check in &mut hosted.checks {
            check.artifacts.extend(artifacts.iter().cloned());
        }
        let verified = self
            .production_gate(admission, parent, &review, &hosted)
            .await?;
        let protection = self
            .current_repository_protection(admission, "main")
            .await?;
        let required_contexts = self
            .contract(admission)?
            .configuration
            .production_ci_policy()?
            .iter()
            .flat_map(|suite| suite.required_checks.iter().cloned())
            .chain(std::iter::once(evidence::REVIEW_CONTEXT.into()))
            .collect();
        protection.verify(&admission.scope.revision, "main", &required_contexts)?;
        let retained = self.key.sign(
            "production_validation",
            &ProductionValidationRecord {
                schema_version: 1,
                preparation_digest: prepared.signed.digest()?,
                protection,
                checks: hosted.checks,
                proofs: hosted.proofs,
            },
        )?;
        self.records.write(
            &format!("production-validation-{}.json", prepared.operation_id()),
            &retained,
        )?;
        Ok(verified)
    }
    async fn production_gate(
        &self,
        observation: &DeliveryObservation,
        parent: &evidence::VerifiedParent,
        review: &PublishedReview,
        hosted: &HostedPassed,
    ) -> Result<evidence::VerifiedProduction> {
        hosted.matches(observation, CheckKind::ProductionCi)?;
        // Recovery is read-only; preserve the kind independently of its phase.
        let mut production = observation.clone();
        production.scope.phase = DeliveryPhase::Release;
        self.reobserve_hosted(&production, hosted).await?;
        let protection = self
            .current_repository_protection(observation, "main")
            .await?;
        let required = hosted_requirements(
            self.contract(observation)?
                .configuration
                .production_ci_policy()?,
            CheckKind::ProductionCi,
        )?;
        let contexts = required
            .iter()
            .map(|c| c.id.clone())
            .chain(std::iter::once(evidence::REVIEW_CONTEXT.into()))
            .collect();
        let result = evidence::verify_production(
            evidence::ProductionRequirements {
                parent,
                revision: &observation.scope.revision,
                checks: &required,
                required_contexts: &contexts,
            },
            &CheckSnapshot {
                pagination_complete: true,
                observed_at: Utc::now(),
                runs: hosted.checks.clone(),
            },
            &review.verified,
            &protection,
        )?;
        self.current(
            observation,
            &[DeliveryPhase::Release, DeliveryPhase::Recovery],
        )?;
        Ok(result)
    }
    pub async fn hydrate_production_validation(
        &self,
        observation: &DeliveryObservation,
        prepared: &PreparedProductionCandidate,
        parent: &evidence::VerifiedParent,
    ) -> Result<evidence::VerifiedProduction> {
        let review = self
            .observe_production_preparation(observation, parent, prepared)
            .await?;
        let signed: Signed<ProductionValidationRecord> = self.records.read(&format!(
            "production-validation-{}.json",
            prepared.operation_id()
        ))?;
        self.key.verify("production_validation", &signed)?;
        if signed.payload.schema_version != 1
            || signed.payload.preparation_digest != prepared.signed.digest()?
        {
            return Err(production_error(
                "production validation belongs to another prepared result",
            ));
        }
        self.production_gate(
            observation,
            parent,
            &review,
            &HostedPassed {
                work: observation.scope.work.clone(),
                revision: observation.scope.revision.clone(),
                kind: CheckKind::ProductionCi,
                checks: signed.payload.checks,
                proofs: signed.payload.proofs,
            },
        )
        .await
    }
}

#[cfg(test)]
#[path = "../tests/support/release_wire.rs"]
pub(super) mod tests;

#[path = "delivery_release_history.rs"]
mod history;
pub use history::MergedProductionRecovery;

#[path = "delivery_production_carry.rs"]
mod carry_production;
pub(crate) use carry_production::{CarryProduction, RetainedCarryProductionInput};
