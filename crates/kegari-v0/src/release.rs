//! Finite release plans, per-step recovery and shared-resource admission.
//!
//! The host owns provider I/O. These transitions require fresh authoritative
//! observations and a persistence callback before issuing an effect capability.
//! Pause permits an admitted plan to drain; correction, revocation, force-stop,
//! failure and uncertainty fence its remaining steps.

use crate::digest::canonical_digest;
use crate::error::{Error, Result};
use crate::evidence::{
    CarriedAcceptance, Generation, VerifiedAcceptance, VerifiedCarry, VerifiedNoReleaseRequired,
    VerifiedParent, VerifiedProduction, validate_digest,
};
use crate::git::{RepositoryId, Revision, validate_oid};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResourceKey {
    Repository { repository: RepositoryId },
    Target { provider: String, id: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepKind {
    Merge {
        repository: RepositoryId,
        pull_request: u64,
        production_base: Revision,
        candidate: Revision,
        selected_delta_digests: Vec<String>,
    },
    Deploy {
        repository: RepositoryId,
        target: ResourceKey,
        workflow_id: u64,
        workflow_revision: String,
        dispatch_ref: String,
        /// The configured workflow must consume this exact immutable source
        /// input. It is filled from the observed successful merge step.
        source_input: String,
        source_merge_step: String,
        expected_tree: String,
        expected_artifact: Option<String>,
    },
    /// Deploy the accepted source already on main after a known predecessor.
    /// There is no replacement merge or invented source step.
    DeployExisting {
        repository: RepositoryId,
        target: ResourceKey,
        workflow_id: u64,
        workflow_revision: String,
        dispatch_ref: String,
        source_input: String,
        source: Revision,
        expected_artifact: Option<String>,
    },
    VerifyDeployed {
        repository: RepositoryId,
        target: ResourceKey,
        source: Revision,
        deployment_step: String,
        expected_artifact: Option<String>,
    },
    /// Observe an already-serving accepted unchanged repository target.
    /// This step never authorizes a merge, deployment or workflow dispatch.
    VerifyExisting {
        repository: RepositoryId,
        target: ResourceKey,
        source: Revision,
        artifact_digest: String,
    },
    Verify {
        repository: RepositoryId,
        target: ResourceKey,
        source_merge_step: String,
        deployment_step: String,
        expected_tree: String,
        expected_artifact: Option<String>,
    },
}

impl StepKind {
    pub fn repository(&self) -> &RepositoryId {
        match self {
            Self::Merge { repository, .. }
            | Self::Deploy { repository, .. }
            | Self::Verify { repository, .. }
            | Self::DeployExisting { repository, .. }
            | Self::VerifyDeployed { repository, .. }
            | Self::VerifyExisting { repository, .. } => repository,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseStep {
    pub id: String,
    pub depends_on: BTreeSet<String>,
    pub kind: StepKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyRequirement {
    pub parent_id: Uuid,
    pub generation: Generation,
    pub outcome_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleasePlan {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub operation_id: Uuid,
    pub parent_id: Uuid,
    pub linear_workspace: Uuid,
    pub generation: Generation,
    pub evidence_token: String,
    pub acceptance_digest: String,
    pub carried_acceptance: Option<CarriedAcceptance>,
    pub resources: BTreeSet<ResourceKey>,
    pub dependencies: Vec<DependencyRequirement>,
    pub spacing_seconds: u64,
    pub deployment_free: BTreeSet<RepositoryId>,
    pub steps: Vec<ReleaseStep>,
}

impl ReleasePlan {
    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, self)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.runtime_id.is_nil()
            || self.operation_id.is_nil()
            || self.parent_id.is_nil()
            || self.linear_workspace.is_nil()
            || self.resources.is_empty()
            || self.steps.is_empty()
            || self.steps.len() > 1024
        {
            return Err(Error::invalid(
                "release requires a finite, identified nonempty plan and resource set",
            ));
        }
        for digest in [
            &self.evidence_token,
            &self.acceptance_digest,
            &self.generation.authorization,
            &self.generation.definition,
            &self.generation.correction,
            &self.generation.scope,
        ] {
            validate_digest(digest)?;
        }
        let mut step_map = BTreeMap::new();
        let mut merge_repositories = BTreeSet::new();
        let mut deployment_targets = BTreeSet::new();
        let mut verification_targets = BTreeSet::new();
        let mut required_resources = BTreeSet::new();
        for step in &self.steps {
            if step.id.trim().is_empty() || step_map.insert(step.id.clone(), step).is_some() {
                return Err(Error::invalid(
                    "release step identities must be nonempty and unique",
                ));
            }
            step.kind.repository().validate()?;
            required_resources.insert(ResourceKey::Repository {
                repository: step.kind.repository().clone(),
            });
            match &step.kind {
                StepKind::Merge {
                    repository,
                    pull_request,
                    production_base,
                    candidate,
                    selected_delta_digests,
                } => {
                    production_base.validate()?;
                    candidate.validate()?;
                    if *pull_request == 0
                        || selected_delta_digests.is_empty()
                        || !merge_repositories.insert(repository.clone())
                        || production_base.tree == candidate.tree
                    {
                        return Err(Error::invalid(
                            "each changed repository needs one exact nonempty production squash",
                        ));
                    }
                    let mut selected = BTreeSet::new();
                    for digest in selected_delta_digests {
                        validate_digest(digest)?;
                        if !selected.insert(digest) {
                            return Err(Error::invalid("duplicate selected delta"));
                        }
                    }
                }
                StepKind::Deploy {
                    repository,
                    target,
                    workflow_id,
                    workflow_revision,
                    dispatch_ref,
                    source_input,
                    expected_tree,
                    expected_artifact,
                    ..
                } => {
                    validate_oid(workflow_revision)?;
                    validate_oid(expected_tree)?;
                    if *workflow_id == 0
                        || dispatch_ref.trim().is_empty()
                        || source_input.trim().is_empty()
                        || self.deployment_free.contains(repository)
                        || !deployment_targets.insert(target.clone())
                    {
                        return Err(Error::invalid(
                            "deployment requires an exact configured workflow/source contract and unique target",
                        ));
                    }
                    validate_target(target)?;
                    required_resources.insert(target.clone());
                    if let Some(artifact) = expected_artifact {
                        validate_digest(artifact)?;
                    }
                }
                StepKind::DeployExisting {
                    repository,
                    target,
                    workflow_id,
                    workflow_revision,
                    dispatch_ref,
                    source_input,
                    source,
                    expected_artifact,
                } => {
                    source.validate()?;
                    validate_oid(workflow_revision)?;
                    validate_target(target)?;
                    if *workflow_id == 0
                        || dispatch_ref.trim().is_empty()
                        || source_input.trim().is_empty()
                        || self.deployment_free.contains(repository)
                        || !deployment_targets.insert(target.clone())
                    {
                        return Err(Error::invalid(
                            "existing-source deployment requires an exact configured route and unique target",
                        ));
                    }
                    required_resources.insert(target.clone());
                    if let Some(artifact) = expected_artifact {
                        validate_digest(artifact)?;
                    }
                }
                StepKind::VerifyDeployed {
                    target,
                    source,
                    expected_artifact,
                    ..
                } => {
                    source.validate()?;
                    validate_target(target)?;
                    if !verification_targets.insert(target.clone()) {
                        return Err(Error::invalid("target requires one terminal verification"));
                    }
                    required_resources.insert(target.clone());
                    if let Some(artifact) = expected_artifact {
                        validate_digest(artifact)?;
                    }
                }
                StepKind::VerifyExisting {
                    target,
                    source,
                    artifact_digest,
                    ..
                } => {
                    validate_target(target)?;
                    source.validate()?;
                    validate_digest(artifact_digest)?;
                    if !deployment_targets.insert(target.clone())
                        || !verification_targets.insert(target.clone())
                    {
                        return Err(Error::invalid(
                            "existing target must have one exclusive verification",
                        ));
                    }
                    required_resources.insert(target.clone());
                }
                StepKind::Verify {
                    target,
                    expected_tree,
                    expected_artifact,
                    ..
                } => {
                    validate_target(target)?;
                    validate_oid(expected_tree)?;
                    if !verification_targets.insert(target.clone()) {
                        return Err(Error::invalid(
                            "target must have exactly one terminal verification",
                        ));
                    }
                    required_resources.insert(target.clone());
                    if let Some(artifact) = expected_artifact {
                        validate_digest(artifact)?;
                    }
                }
            }
        }
        if deployment_targets != verification_targets {
            return Err(Error::invalid(
                "every target requires its exact terminal verification",
            ));
        }
        if self.resources != required_resources
            || !self.deployment_free.is_subset(&merge_repositories)
        {
            return Err(Error::invalid(
                "release resource set/deployment-free contract does not exactly cover its steps",
            ));
        }
        for step in &self.steps {
            for dependency in &step.depends_on {
                if dependency == &step.id || !step_map.contains_key(dependency) {
                    return Err(Error::new(
                        "dependency_cycle",
                        "release step has a self or missing dependency",
                    ));
                }
            }
        }
        let ancestors = dependency_closure(&self.steps)?;
        for step in &self.steps {
            match &step.kind {
                StepKind::Deploy { repository, source_merge_step, expected_tree, .. }
                | StepKind::Verify { repository, source_merge_step, expected_tree, .. } => {
                    let source = step_map.get(source_merge_step).ok_or_else(|| Error::invalid("source merge step is missing"))?;
                    match &source.kind {
                        StepKind::Merge { repository: source_repo, candidate, .. }
                            if source_repo == repository && candidate.tree == *expected_tree
                                && ancestors[&step.id].contains(source_merge_step) => {}
                        _ => return Err(Error::invalid("deployment/verification is not ordered after its exact source merge")),
                    }
                    if let StepKind::Verify { deployment_step, target, .. } = &step.kind
                        && (!ancestors[&step.id].contains(deployment_step)
                            || !step_map.get(deployment_step).is_some_and(|s| matches!(&s.kind, StepKind::Deploy { target: t, repository: r, .. } if t == target && r == repository))) {
                        return Err(Error::invalid("verification must follow its matching deployment"));
                    }
                }
                StepKind::DeployExisting { repository, .. } => {
                    if merge_repositories.contains(repository) || self.deployment_free.contains(repository) {
                        return Err(Error::invalid("existing-source deployment cannot share a changed or deployment-free repository"));
                    }
                }
                StepKind::VerifyDeployed { repository, target, source, deployment_step, .. } => {
                    if !ancestors[&step.id].contains(deployment_step)
                        || !step_map.get(deployment_step).is_some_and(|step| matches!(&step.kind,
                            StepKind::DeployExisting { repository: r, target: t, source: s, .. } if r == repository && t == target && s == source)) {
                        return Err(Error::invalid("existing-source verification must follow its exact configured deployment"));
                    }
                }
                StepKind::VerifyExisting { repository, .. } => {
                    if merge_repositories.contains(repository) || self.deployment_free.contains(repository) {
                        return Err(Error::invalid("existing target verification cannot share a changed or deployment-free repository"));
                    }
                }
                StepKind::Merge { repository, .. } => {
                    if !self.deployment_free.contains(repository)
                        && !self.steps.iter().any(|s| matches!(&s.kind, StepKind::Deploy { repository: r, .. } if r == repository)) {
                        return Err(Error::invalid("repository has neither deployment steps nor explicit deployment=none"));
                    }
                }
            }
        }
        let mut dependencies = BTreeSet::new();
        for dependency in &self.dependencies {
            if dependency.parent_id == self.parent_id || !dependencies.insert(dependency.parent_id)
            {
                return Err(Error::new(
                    "dependency_cycle",
                    "release parent dependency is self/duplicated",
                ));
            }
            validate_digest(&dependency.outcome_digest)?;
        }
        Ok(())
    }
}

fn validate_target(target: &ResourceKey) -> Result<()> {
    if !matches!(target, ResourceKey::Target { provider, id } if !provider.is_empty() && !id.is_empty())
    {
        return Err(Error::invalid(
            "deployment needs a canonical provider/target resource key",
        ));
    }
    Ok(())
}

fn dependency_closure(steps: &[ReleaseStep]) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut completed = BTreeMap::<String, BTreeSet<String>>::new();
    while completed.len() < steps.len() {
        let mut progress = false;
        for step in steps {
            if completed.contains_key(&step.id)
                || !step.depends_on.iter().all(|id| completed.contains_key(id))
            {
                continue;
            }
            let mut closure = step.depends_on.clone();
            for id in &step.depends_on {
                closure.extend(completed[id].iter().cloned());
            }
            completed.insert(step.id.clone(), closure);
            progress = true;
        }
        if !progress {
            return Err(Error::new(
                "dependency_cycle",
                "release plan contains a dependency cycle",
            ));
        }
    }
    Ok(completed)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedRun {
    pub repository: RepositoryId,
    pub run_id: u64,
    pub attempt: u32,
    pub operation_id: Uuid,
    pub actor_account_id: String,
    pub origin_digest: String,
}

impl OwnedRun {
    /// A human rerun with the same run ID but a later attempt is not owned.
    pub fn may_cancel(&self, current: &Self) -> bool {
        self == current
            && self.run_id != 0
            && self.attempt != 0
            && !self.operation_id.is_nil()
            && !self.actor_account_id.is_empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObservedEffect {
    Merge {
        repository: RepositoryId,
        pull_request: u64,
        commit: Revision,
        parent_commit: String,
    },
    Deployment {
        repository: RepositoryId,
        target: ResourceKey,
        source_commit: String,
        tree: String,
        artifact_digest: String,
        deployment_id: String,
        run: OwnedRun,
    },
    Verification {
        repository: RepositoryId,
        target: ResourceKey,
        source_commit: String,
        tree: String,
        artifact_digest: String,
        evidence_digest: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepState {
    NotStarted,
    Prepared {
        payload_digest: String,
    },
    SentUnknown {
        payload_digest: String,
    },
    Acknowledged {
        payload_digest: String,
        remote_id: String,
    },
    Succeeded {
        effect: Box<ObservedEffect>,
        observed_at: DateTime<Utc>,
    },
    Failed {
        reason: String,
        remote_id: Option<String>,
        observed_at: DateTime<Utc>,
    },
    Cancelled {
        remote_id: String,
        observed_at: DateTime<Utc>,
    },
    Retired {
        reason: String,
    },
}

impl StepState {
    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::SentUnknown { .. } | Self::Acknowledged { .. })
    }
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Succeeded { .. })
    }
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded { .. }
                | Self::Failed { .. }
                | Self::Cancelled { .. }
                | Self::Retired { .. }
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cancellation {
    NotRequested,
    Requested,
    Confirmed,
    Failed,
    AlreadyCompleted,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseAttempt {
    pub plan: ReleasePlan,
    pub admitted_at: DateTime<Utc>,
    pub control_generation: u64,
    pub owner_generation: u64,
    pub steps: BTreeMap<String, StepState>,
    pub step_history: BTreeMap<String, Vec<StepState>>,
    pub cancellations: BTreeMap<String, Cancellation>,
    pub hold_reason: Option<String>,
    pub abandoned: bool,
    pub resolved: bool,
    pub assigned_successor: Option<Uuid>,
    pub recovery_records: Vec<RecoveryRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRecord {
    pub actor: String,
    pub action: String,
    pub reason: String,
    pub baseline_digest: Option<String>,
    pub observed_at: DateTime<Utc>,
}

/// Freshness is evaluated immediately before every step by a complete provider
/// refresh under the host's serialized mutation gate. These values are never a
/// cache that authorizes subsequent unrefreshed steps.
pub struct FreshReleaseFacts<'a> {
    pub generation: &'a Generation,
    pub evidence_token: &'a str,
    pub owner_generation: u64,
    pub authority_current: bool,
    pub pending_correction: bool,
    pub provider_reads_complete: bool,
    pub resource_lock_owned: bool,
    pub credentials_usable: bool,
    pub repository_policy_current: bool,
    pub force_stopped: bool,
    pub current_revisions: &'a BTreeMap<RepositoryId, Revision>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepPayload {
    pub operation_id: Uuid,
    pub step_id: String,
    pub attempt: u32,
    pub prerequisite_attempts: BTreeMap<String, u32>,
    pub plan_digest: String,
    pub kind: StepKind,
    pub exact_source_commit: Option<String>,
}

/// Can only be obtained after persisting sent/unknown. Dropping or timing out a
/// capability does not permit blind replay; reconcile the recorded operation.
#[derive(Debug)]
pub struct Dispatch {
    payload: StepPayload,
}

impl Dispatch {
    pub fn payload(&self) -> &StepPayload {
        &self.payload
    }
}

pub struct AdmissionFacts {
    pub running: bool,
    pub included: bool,
    pub authorized: bool,
    pub no_pending_correction: bool,
    pub preparation_complete: bool,
    pub provider_reads_complete: bool,
    pub repository_policy_usable: bool,
    pub credentials_usable: bool,
    pub manual_mode: bool,
    pub explicit_manual_request: Option<Uuid>,
    pub actor_may_release: bool,
    pub owner_generation: u64,
    pub control_generation: u64,
    /// From the validated operator delivery contract, including every required
    /// verification step. An agent-proposed subset is not this inventory.
    pub required_step_ids: BTreeSet<String>,
}

pub struct DeliveryEvidence<'a> {
    pub parent: &'a VerifiedParent,
    pub acceptance: &'a VerifiedAcceptance,
    pub carried: Option<&'a VerifiedCarry>,
    pub production: &'a [VerifiedProduction],
}

pub struct RecoveryDecision<'a> {
    pub actor: &'a str,
    pub reason: &'a str,
    pub now: DateTime<Utc>,
}

/// Read-only readiness is separate from authority to dispatch. In manual mode
/// this proof may truthfully report that the release command is still awaited.
#[derive(Clone, Debug)]
pub struct VerifiedReleaseEligibility {
    acceptance_digest: String,
    plan_digest: String,
    evidence_token: String,
    checked_at: DateTime<Utc>,
    awaiting_manual_command: bool,
}
impl VerifiedReleaseEligibility {
    pub fn acceptance_digest(&self) -> &str {
        &self.acceptance_digest
    }
    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }
    pub fn evidence_token(&self) -> &str {
        &self.evidence_token
    }
    pub fn checked_at(&self) -> DateTime<Utc> {
        self.checked_at
    }
    pub fn awaiting_manual_command(&self) -> bool {
        self.awaiting_manual_command
    }
}

pub fn verify_release_eligibility(
    plan: &ReleasePlan,
    evidence: DeliveryEvidence<'_>,
    facts: AdmissionFacts,
    queue: &ReleaseQueue,
    now: DateTime<Utc>,
) -> Result<VerifiedReleaseEligibility> {
    queue.eligible(plan, now)?;
    let awaiting_manual_command =
        facts.manual_mode && (facts.explicit_manual_request.is_none() || !facts.actor_may_release);
    // The evaluated draft is not returned or persisted and grants no dispatch
    // capability. All ordinary admission predicates remain identical.
    let acceptance_digest = evidence.acceptance.digest().to_owned();
    ReleaseAttempt::evaluate(plan.clone(), evidence, facts, now, false)?;
    Ok(VerifiedReleaseEligibility {
        acceptance_digest,
        plan_digest: plan.digest()?,
        evidence_token: plan.evidence_token.clone(),
        checked_at: now,
        awaiting_manual_command,
    })
}

/// Re-evaluate an explicit continuation without waiving another attempt's hold.
/// The returned proof remains read-only; the control host must separately admit
/// the authenticated resume lineage before any uncompleted step can dispatch.
pub fn verify_release_resume_eligibility(
    attempt: &ReleaseAttempt,
    evidence: DeliveryEvidence<'_>,
    facts: AdmissionFacts,
    queue: &ReleaseQueue,
    now: DateTime<Utc>,
) -> Result<VerifiedReleaseEligibility> {
    attempt.validate_recovered()?;
    if attempt.has_unknown()
        || attempt.abandoned
        || attempt.resolved
        || attempt.assigned_successor.is_some()
        || attempt.completed()
    {
        return Err(Error::new(
            "recovery_blocked",
            "resume requires an unfinished current plan with known outcomes",
        ));
    }
    let mut continuation = queue.clone();
    if let Some(hold) = continuation.holds.remove(&attempt.plan.operation_id)
        && (hold.resources != attempt.plan.resources || hold.parent_id != attempt.plan.parent_id)
    {
        return Err(Error::new(
            "ownership_conflict",
            "retained hold differs from original release plan",
        ));
    }
    verify_release_eligibility(&attempt.plan, evidence, facts, &continuation, now)
}

impl ReleaseAttempt {
    pub fn admit(
        plan: ReleasePlan,
        evidence: DeliveryEvidence<'_>,
        facts: AdmissionFacts,
        now: DateTime<Utc>,
    ) -> Result<Self> {
        Self::evaluate(plan, evidence, facts, now, true)
    }

    fn evaluate(
        plan: ReleasePlan,
        evidence: DeliveryEvidence<'_>,
        facts: AdmissionFacts,
        now: DateTime<Utc>,
        dispatch_authority: bool,
    ) -> Result<Self> {
        let DeliveryEvidence {
            parent,
            acceptance,
            carried,
            production,
        } = evidence;
        plan.validate()?;
        let a = acceptance.acceptance();
        if facts.required_step_ids != plan.steps.iter().map(|s| s.id.clone()).collect() {
            return Err(Error::new(
                "release_incomplete",
                "release plan omits or adds steps relative to the configured delivery contract",
            ));
        }
        if !facts.running
            || !facts.included
            || !facts.authorized
            || !facts.no_pending_correction
            || !facts.preparation_complete
            || !facts.provider_reads_complete
            || !facts.repository_policy_usable
            || !facts.credentials_usable
            || facts.owner_generation == 0
            || facts.control_generation == 0
        {
            return Err(Error::new(
                "release_ineligible",
                "runtime intent, inclusion, authority, preparation or current provider gates prevent release",
            ));
        }
        if dispatch_authority
            && facts.manual_mode
            && (facts.explicit_manual_request.is_none() || !facts.actor_may_release)
        {
            return Err(Error::new(
                "release_authorization_required",
                "manual delivery requires one authenticated current-result release command",
            ));
        }
        if plan.parent_id != parent.manifest().work.parent
            || plan.linear_workspace != parent.manifest().work.linear_workspace
            || plan.generation != parent.manifest().work.generation
            || plan.evidence_token != parent.token()
            || plan.acceptance_digest != acceptance.digest()
            || a.work.parent != plan.parent_id
            || a.work.linear_workspace != plan.linear_workspace
            || a.work.generation != plan.generation
        {
            return Err(Error::new(
                "stale_result",
                "release plan, candidate and acceptance identities differ",
            ));
        }
        if a.reviewed_token != plan.evidence_token {
            let carry = plan.carried_acceptance.as_ref().ok_or_else(|| {
                Error::new(
                    "stale_result",
                    "acceptance names an older result without valid carry lineage",
                )
            })?;
            if carry.schema_version != 1
                || carry.original_reviewed_token != a.reviewed_token
                || carry.original_acceptance_digest != acceptance.digest()
                || carry.current_evidence_token != plan.evidence_token
                || carried.is_none_or(|verified| verified.record() != carry)
            {
                return Err(Error::new(
                    "stale_result",
                    "carry lineage does not bind this original decision and current candidate",
                ));
            }
            validate_digest(&carry.unchanged_patch_proof)?;
            validate_digest(&carry.production_validation_digest)?;
        } else if plan.carried_acceptance.is_some() || carried.is_some() {
            return Err(Error::new(
                "stale_result",
                "unnecessary carry lineage is inconsistent with direct acceptance",
            ));
        }
        let candidate_repositories: BTreeSet<_> = parent
            .manifest()
            .candidates
            .iter()
            .filter(|c| !c.ordered_delta_digests.is_empty())
            .map(|c| &c.repository)
            .collect();
        let planned_repositories: BTreeSet<_> = plan
            .steps
            .iter()
            .filter_map(|s| match &s.kind {
                StepKind::Merge { repository, .. } => Some(repository),
                _ => None,
            })
            .collect();
        let planned_targets: BTreeSet<_> = plan
            .resources
            .iter()
            .filter_map(|r| match r {
                ResourceKey::Target { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        if planned_repositories != candidate_repositories
            || planned_targets != parent.manifest().production_targets
        {
            return Err(Error::new(
                "scope_violation",
                "release must cover exactly every accepted candidate repository and production target",
            ));
        }
        for step in &plan.steps {
            if let StepKind::Merge {
                repository,
                production_base,
                candidate,
                selected_delta_digests,
                ..
            } = &step.kind
                && (!parent.manifest().candidates.iter().any(|c| {
                    &c.repository == repository
                        && &c.production == production_base
                        && c.result.as_ref() == Some(candidate)
                        && &c.ordered_delta_digests == selected_delta_digests
                }) || !production.iter().any(|p| {
                    p.work() == &parent.manifest().work
                        && !p.is_existing_production()
                        && p.revision().repository == *repository
                        && p.revision().base == production_base.commit
                        && p.revision().head == candidate.commit
                        && p.revision().tested_tree == candidate.tree
                }))
            {
                return Err(Error::new(
                    "scope_violation",
                    "production merge lacks its accepted isolated candidate and full verified production validation",
                ));
            }
        }
        for step in &plan.steps {
            if let StepKind::DeployExisting {
                repository,
                target: ResourceKey::Target { id, .. },
                source,
                ..
            }
            | StepKind::VerifyDeployed {
                repository,
                target: ResourceKey::Target { id, .. },
                source,
                ..
            } = &step.kind
            {
                let baseline = parent
                    .manifest()
                    .baselines
                    .iter()
                    .find(|baseline| &baseline.repository == repository)
                    .ok_or_else(|| {
                        Error::new(
                            "scope_violation",
                            "existing-source deployment baseline missing",
                        )
                    })?;
                let predecessor = baseline.successor_recovery_operation.ok_or_else(|| {
                    Error::new(
                        "scope_violation",
                        "existing-source deployment requires known predecessor lineage",
                    )
                })?;
                let candidate_matches = parent.manifest().candidates.iter().any(|candidate| {
                    &candidate.repository == repository
                        && candidate.ordered_delta_digests.is_empty()
                        && &candidate.production == source
                        && candidate.result.as_ref() == Some(source)
                        && candidate.production_integrations.iter().any(|integration| {
                            integration.release_operation == predecessor
                                && integration.parent_id == plan.parent_id
                                && &integration.merged == source
                                && &integration.observed_main == source
                        })
                });
                let validation_matches = production.iter().any(|proof| {
                    proof.is_existing_production()
                        && proof.work() == &parent.manifest().work
                        && &proof.revision().repository == repository
                        && proof.revision().head == source.commit
                        && proof.revision().base == source.commit
                        && proof.revision().tested_commit == source.commit
                        && proof.revision().tested_tree == source.tree
                });
                if !candidate_matches
                    || !validation_matches
                    || baseline.production != *source
                    || baseline.deployment_free
                    || baseline
                        .source_for_target(id)
                        .is_none_or(|current| current == source.commit)
                    || baseline.artifact_for_target(id).is_none()
                {
                    return Err(Error::new(
                        "scope_violation",
                        "existing-source deployment lacks exact accepted integration, divergent target and complete production validation",
                    ));
                }
            }
        }
        for step in &plan.steps {
            if let StepKind::VerifyExisting {
                repository,
                target,
                source,
                artifact_digest,
            } = &step.kind
            {
                let ResourceKey::Target { id, .. } = target else {
                    unreachable!("validated target")
                };
                let candidate_matches = parent.manifest().candidates.iter().any(|candidate| {
                    &candidate.repository == repository
                        && candidate.ordered_delta_digests.is_empty()
                        && &candidate.production == source
                        && candidate.result.as_ref() == Some(source)
                });
                let baseline_matches = parent.manifest().baselines.iter().any(|baseline| {
                    &baseline.repository == repository
                        && &baseline.production == source
                        && !baseline.deployment_free
                        && baseline.source_for_target(id) == Some(source.commit.as_str())
                        && baseline.target_artifacts.get(id) == Some(artifact_digest)
                });
                let validation_matches = production.iter().any(|proof| {
                    proof.is_existing_production()
                        && proof.work() == &parent.manifest().work
                        && &proof.revision().repository == repository
                        && proof.revision().head == source.commit
                        && proof.revision().base == source.commit
                        && proof.revision().tested_commit == source.commit
                        && proof.revision().tested_tree == source.tree
                });
                if !candidate_matches || !baseline_matches || !validation_matches {
                    return Err(Error::new(
                        "scope_violation",
                        "existing target verification lacks its exact accepted unchanged source, artifact and production validation",
                    ));
                }
            }
        }
        let steps = plan
            .steps
            .iter()
            .map(|s| (s.id.clone(), StepState::NotStarted))
            .collect();
        Ok(Self {
            plan,
            admitted_at: now,
            control_generation: facts.control_generation,
            owner_generation: facts.owner_generation,
            steps,
            step_history: BTreeMap::new(),
            cancellations: BTreeMap::new(),
            hold_reason: None,
            abandoned: false,
            resolved: false,
            assigned_successor: None,
            recovery_records: vec![],
        })
    }

    pub fn validate_recovered(&self) -> Result<()> {
        self.plan.validate()?;
        if self.steps.keys().collect::<BTreeSet<_>>()
            != self
                .plan
                .steps
                .iter()
                .map(|s| &s.id)
                .collect::<BTreeSet<_>>()
            || self.owner_generation == 0
            || self.control_generation == 0
            || self.assigned_successor == Some(self.plan.operation_id)
        {
            return Err(Error::new(
                "corrupt_record",
                "release receipt step/owner set is inconsistent",
            ));
        }
        for (id, state) in &self.steps {
            self.step_attempt(id)?;
            match state {
                StepState::Prepared { payload_digest }
                | StepState::SentUnknown { payload_digest }
                | StepState::Acknowledged { payload_digest, .. } => {
                    validate_digest(payload_digest)?
                }
                StepState::Succeeded { effect, .. } => self.verify_effect(id, effect)?,
                _ => {}
            }
        }
        Ok(())
    }

    pub fn completed(&self) -> bool {
        !self.abandoned && self.steps.values().all(StepState::is_success)
    }
    pub fn has_unknown(&self) -> bool {
        self.steps.values().any(StepState::is_unknown)
            || self.cancellations.iter().any(|(id, c)| {
                matches!(
                    c,
                    Cancellation::Requested | Cancellation::Unknown | Cancellation::Failed
                ) && self.steps.get(id).is_none_or(|s| !s.is_terminal())
            })
    }

    fn set_step(&mut self, id: String, state: StepState) {
        if let Some(previous) = self.steps.insert(id.clone(), state) {
            self.step_history.entry(id).or_default().push(previous);
        }
    }

    pub fn next_steps(&self) -> Vec<&ReleaseStep> {
        if self.hold_reason.is_some()
            || self.abandoned
            || self.resolved
            || self.assigned_successor.is_some()
        {
            return vec![];
        }
        self.plan
            .steps
            .iter()
            .filter(|step| {
                matches!(self.steps[&step.id], StepState::NotStarted)
                    && step.depends_on.iter().all(|id| self.steps[id].is_success())
            })
            .collect()
    }

    fn verify_freshness(&self, step: &ReleaseStep, facts: &FreshReleaseFacts<'_>) -> Result<()> {
        if facts.generation != &self.plan.generation
            || facts.evidence_token != self.plan.evidence_token
            || facts.owner_generation != self.owner_generation
            || !facts.authority_current
            || facts.pending_correction
            || !facts.provider_reads_complete
            || !facts.resource_lock_owned
            || !facts.credentials_usable
            || !facts.repository_policy_current
            || facts.force_stopped
        {
            return Err(Error::new(
                "release_fenced",
                "fresh authority, generation, owner, exact result or provider prerequisites changed",
            ));
        }
        match &step.kind {
            StepKind::Merge {
                repository,
                production_base,
                ..
            } => {
                if facts.current_revisions.get(repository) != Some(production_base) {
                    return Err(Error::new(
                        "stale_result",
                        "production base moved; reassemble and revalidate before merging",
                    ));
                }
            }
            StepKind::VerifyExisting {
                repository, source, ..
            }
            | StepKind::DeployExisting {
                repository, source, ..
            }
            | StepKind::VerifyDeployed {
                repository, source, ..
            } => {
                if facts.current_revisions.get(repository) != Some(source) {
                    return Err(Error::new(
                        "stale_result",
                        "already-serving target source moved since acceptance",
                    ));
                }
            }
            StepKind::Deploy {
                repository,
                source_merge_step,
                ..
            }
            | StepKind::Verify {
                repository,
                source_merge_step,
                ..
            } => {
                let expected = match self.steps.get(source_merge_step) {
                    Some(StepState::Succeeded { effect, .. }) => match effect.as_ref() {
                        ObservedEffect::Merge { commit, .. } => Some(commit),
                        _ => None,
                    },
                    _ => None,
                };
                if expected.is_none() || facts.current_revisions.get(repository) != expected {
                    return Err(Error::new(
                        "stale_result",
                        "production source moved after this release's merge",
                    ));
                }
            }
        }
        // No RUNNING predicate here: an already admitted finite operation drains
        // during pause/graceful stop, unless one of the fences above changed.
        Ok(())
    }

    fn source_commit(&self, step: &ReleaseStep) -> Result<Option<String>> {
        let id = match &step.kind {
            StepKind::Merge { .. } => return Ok(None),
            StepKind::VerifyExisting { source, .. }
            | StepKind::DeployExisting { source, .. }
            | StepKind::VerifyDeployed { source, .. } => return Ok(Some(source.commit.clone())),
            StepKind::Deploy {
                source_merge_step, ..
            }
            | StepKind::Verify {
                source_merge_step, ..
            } => source_merge_step,
        };
        match self.steps.get(id) {
            Some(StepState::Succeeded { effect, .. }) => match effect.as_ref() {
                ObservedEffect::Merge { commit, .. } => Ok(Some(commit.commit.clone())),
                _ => Err(Error::new(
                    "release_dependency",
                    "source step is not a merge",
                )),
            },
            _ => Err(Error::new(
                "release_dependency",
                "deployment source merge has not been observed successful",
            )),
        }
    }

    /// Ordinals change only after explicit resume archives a known terminal
    /// failure/cancellation. Pending and repeatedly observed effects retain the
    /// same immutable identity; a prepared-but-unsent retry does not invent one.
    pub fn step_attempt(&self, id: &str) -> Result<u32> {
        if !self.steps.contains_key(id) {
            return Err(Error::invalid("unknown release step"));
        }
        let failures = self
            .step_history
            .get(id)
            .into_iter()
            .flatten()
            .filter(|state| {
                matches!(
                    state,
                    StepState::Failed { .. } | StepState::Cancelled { .. }
                )
            })
            .count();
        let resumes = self
            .recovery_records
            .iter()
            .filter(|record| record.action == "resume")
            .count();
        if failures > resumes || failures >= 4096 {
            return Err(Error::new(
                "corrupt_record",
                "release attempt history lacks explicit resume lineage or exceeds its retained bound",
            ));
        }
        Ok(failures as u32 + 1)
    }
    pub fn prepare_step(
        &mut self,
        id: &str,
        facts: &FreshReleaseFacts<'_>,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<StepPayload> {
        let step = self
            .next_steps()
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| {
                Error::new(
                    "release_step_blocked",
                    "step is not newly eligible; uncertain/sent steps require reconciliation",
                )
            })?;
        self.verify_freshness(step, facts)?;
        let payload = StepPayload {
            operation_id: self.plan.operation_id,
            step_id: id.into(),
            attempt: self.step_attempt(id)?,
            prerequisite_attempts: step
                .depends_on
                .iter()
                .map(|id| Ok((id.clone(), self.step_attempt(id)?)))
                .collect::<Result<_>>()?,
            plan_digest: self.plan.digest()?,
            kind: step.kind.clone(),
            exact_source_commit: self.source_commit(step)?,
        };
        let mut next = self.clone();
        next.set_step(
            id.into(),
            StepState::Prepared {
                payload_digest: canonical_digest(1, &payload)?,
            },
        );
        persist(&next)?;
        *self = next;
        Ok(payload)
    }

    pub fn mark_sent(
        &mut self,
        payload: &StepPayload,
        facts: &FreshReleaseFacts<'_>,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<Dispatch> {
        let step = self
            .plan
            .steps
            .iter()
            .find(|s| s.id == payload.step_id)
            .ok_or_else(|| Error::new("release_step_blocked", "unknown release step"))?;
        self.verify_freshness(step, facts)?;
        let digest = canonical_digest(1, payload)?;
        if self.hold_reason.is_some()
            || self.abandoned
            || self.assigned_successor.is_some()
            || payload.operation_id != self.plan.operation_id
            || payload.attempt != self.step_attempt(&payload.step_id)?
            || payload.prerequisite_attempts
                != step
                    .depends_on
                    .iter()
                    .map(|id| Ok((id.clone(), self.step_attempt(id)?)))
                    .collect::<Result<BTreeMap<_, _>>>()?
            || payload.plan_digest != self.plan.digest()?
            || payload.kind != step.kind
            || payload.exact_source_commit != self.source_commit(step)?
            || !matches!(self.steps.get(&payload.step_id), Some(StepState::Prepared { payload_digest }) if payload_digest == &digest)
        {
            return Err(Error::new(
                "operation_uncertain",
                "step is not an unchanged prepared action; reconcile rather than repeat it",
            ));
        }
        let mut next = self.clone();
        next.set_step(
            payload.step_id.clone(),
            StepState::SentUnknown {
                payload_digest: digest,
            },
        );
        persist(&next)?;
        *self = next;
        Ok(Dispatch {
            payload: payload.clone(),
        })
    }

    pub fn acknowledge(
        &mut self,
        id: &str,
        remote_id: String,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        let digest = match self.steps.get(id) {
            Some(StepState::SentUnknown { payload_digest }) if !remote_id.is_empty() => {
                payload_digest.clone()
            }
            Some(StepState::Acknowledged {
                remote_id: existing,
                ..
            }) if existing == &remote_id => return Ok(()),
            _ => {
                return Err(Error::new(
                    "operation_conflict",
                    "acknowledgement does not match a sent step",
                ));
            }
        };
        let mut next = self.clone();
        next.set_step(
            id.into(),
            StepState::Acknowledged {
                payload_digest: digest,
                remote_id,
            },
        );
        persist(&next)?;
        *self = next;
        Ok(())
    }

    fn verify_effect(&self, id: &str, effect: &ObservedEffect) -> Result<()> {
        let step = self
            .plan
            .steps
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| Error::new("operation_conflict", "unknown observed step"))?;
        let valid = match (&step.kind, effect) {
            (
                StepKind::Merge {
                    repository,
                    pull_request,
                    production_base,
                    candidate,
                    ..
                },
                ObservedEffect::Merge {
                    repository: actual_repo,
                    pull_request: actual_pr,
                    commit,
                    parent_commit,
                },
            ) => {
                commit.validate()?;
                repository == actual_repo
                    && pull_request == actual_pr
                    && production_base.commit == *parent_commit
                    && candidate.tree == commit.tree
            }
            (
                StepKind::Deploy {
                    repository,
                    target,
                    expected_tree,
                    expected_artifact,
                    ..
                },
                ObservedEffect::Deployment {
                    repository: actual_repo,
                    target: actual_target,
                    source_commit,
                    tree,
                    artifact_digest,
                    deployment_id,
                    run,
                },
            ) => {
                validate_oid(source_commit)?;
                validate_digest(artifact_digest)?;
                repository == actual_repo
                    && target == actual_target
                    && expected_tree == tree
                    && self.source_commit(step)?.as_ref() == Some(source_commit)
                    && expected_artifact
                        .as_ref()
                        .is_none_or(|expected| expected == artifact_digest)
                    && !deployment_id.is_empty()
                    && run.repository == *repository
                    && run.operation_id == self.plan.operation_id
                    && run.run_id > 0
                    && run.attempt > 0
            }
            (
                StepKind::Verify {
                    repository,
                    target,
                    expected_tree,
                    expected_artifact,
                    deployment_step,
                    ..
                },
                ObservedEffect::Verification {
                    repository: actual_repo,
                    target: actual_target,
                    source_commit,
                    tree,
                    artifact_digest,
                    evidence_digest,
                },
            ) => {
                validate_oid(source_commit)?;
                validate_digest(artifact_digest)?;
                validate_digest(evidence_digest)?;
                repository == actual_repo
                    && target == actual_target
                    && expected_tree == tree
                    && self.source_commit(step)?.as_ref() == Some(source_commit)
                    && expected_artifact
                        .as_ref()
                        .is_none_or(|expected| expected == artifact_digest)
                    && matches!(self.steps.get(deployment_step), Some(StepState::Succeeded { effect, .. }) if matches!(effect.as_ref(), ObservedEffect::Deployment { artifact_digest: deployed, .. } if deployed == artifact_digest))
            }
            (
                StepKind::DeployExisting {
                    repository,
                    target,
                    source,
                    expected_artifact,
                    ..
                },
                ObservedEffect::Deployment {
                    repository: actual_repo,
                    target: actual_target,
                    source_commit,
                    tree,
                    artifact_digest,
                    deployment_id,
                    run,
                },
            ) => {
                validate_digest(artifact_digest)?;
                repository == actual_repo
                    && target == actual_target
                    && source.commit == *source_commit
                    && source.tree == *tree
                    && expected_artifact
                        .as_ref()
                        .is_none_or(|expected| expected == artifact_digest)
                    && !deployment_id.is_empty()
                    && run.repository == *repository
                    && run.operation_id == self.plan.operation_id
                    && run.run_id > 0
                    && run.attempt > 0
            }
            (
                StepKind::VerifyDeployed {
                    repository,
                    target,
                    source,
                    expected_artifact,
                    deployment_step,
                },
                ObservedEffect::Verification {
                    repository: actual_repo,
                    target: actual_target,
                    source_commit,
                    tree,
                    artifact_digest,
                    evidence_digest,
                },
            ) => {
                validate_digest(artifact_digest)?;
                validate_digest(evidence_digest)?;
                repository == actual_repo
                    && target == actual_target
                    && source.commit == *source_commit
                    && source.tree == *tree
                    && expected_artifact
                        .as_ref()
                        .is_none_or(|expected| expected == artifact_digest)
                    && matches!(self.steps.get(deployment_step), Some(StepState::Succeeded { effect, .. }) if matches!(effect.as_ref(), ObservedEffect::Deployment { artifact_digest: deployed, .. } if deployed == artifact_digest))
            }
            (
                StepKind::VerifyExisting {
                    repository,
                    target,
                    source,
                    artifact_digest: expected,
                },
                ObservedEffect::Verification {
                    repository: actual_repo,
                    target: actual_target,
                    source_commit,
                    tree,
                    artifact_digest,
                    evidence_digest,
                },
            ) => {
                validate_digest(evidence_digest)?;
                repository == actual_repo
                    && target == actual_target
                    && source.commit == *source_commit
                    && source.tree == *tree
                    && expected == artifact_digest
            }
            _ => false,
        };
        if !valid {
            return Err(Error::new(
                "release_provenance",
                "observed production effect differs from the tested exact source/tree/target contract",
            ));
        }
        Ok(())
    }

    /// A successful API acknowledgement is insufficient. The adapter supplies
    /// an observed terminal effect, and its commit/tree/source identity is checked.
    pub fn observe_success(
        &mut self,
        id: &str,
        effect: ObservedEffect,
        now: DateTime<Utc>,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        if matches!(self.steps.get(id), Some(StepState::Succeeded { effect: existing, .. }) if existing.as_ref() == &effect)
        {
            return Ok(());
        }
        if !matches!(
            self.steps.get(id),
            Some(StepState::SentUnknown { .. } | StepState::Acknowledged { .. })
        ) {
            return Err(Error::new(
                "operation_conflict",
                "successful observation has no matching sent/unknown action",
            ));
        }
        if let Err(error) = self.verify_effect(id, &effect) {
            let mut next = self.clone();
            next.hold_reason = Some("observed production provenance mismatch".into());
            persist(&next)?;
            *self = next;
            return Err(error);
        }
        let mut next = self.clone();
        next.set_step(
            id.into(),
            StepState::Succeeded {
                effect: Box::new(effect),
                observed_at: now,
            },
        );
        if matches!(
            next.cancellations.get(id),
            Some(Cancellation::Requested | Cancellation::Unknown | Cancellation::Failed)
        ) {
            next.cancellations
                .insert(id.into(), Cancellation::AlreadyCompleted);
        }
        persist(&next)?;
        *self = next;
        Ok(())
    }

    pub fn observe_failure(
        &mut self,
        id: &str,
        reason: String,
        remote_id: Option<String>,
        now: DateTime<Utc>,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        if matches!(self.steps.get(id), Some(StepState::Failed {reason: old, remote_id: existing, ..}) if old == &reason && existing == &remote_id)
        {
            return Ok(());
        }
        if reason.trim().is_empty()
            || !matches!(
                self.steps.get(id),
                Some(StepState::SentUnknown { .. } | StepState::Acknowledged { .. })
            )
        {
            return Err(Error::new(
                "operation_conflict",
                "failure observation has no matching remote action/reason",
            ));
        }
        let mut next = self.clone();
        next.set_step(
            id.into(),
            StepState::Failed {
                reason: reason.clone(),
                remote_id,
                observed_at: now,
            },
        );
        next.hold_reason = Some(reason);
        persist(&next)?;
        *self = next;
        Ok(())
    }

    pub fn fence(
        &mut self,
        reason: String,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        if reason.trim().is_empty() {
            return Err(Error::invalid("release fence requires a reason"));
        }
        let mut next = self.clone();
        next.hold_reason = Some(reason);
        persist(&next)?;
        *self = next;
        Ok(())
    }

    pub fn record_cancellation(
        &mut self,
        id: &str,
        state: Cancellation,
        remote_id: Option<String>,
        now: DateTime<Utc>,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        if !self.steps.contains_key(id) {
            return Err(Error::invalid("unknown cancellation step"));
        }
        if self.steps[id].is_success() && state != Cancellation::AlreadyCompleted {
            return Err(Error::new(
                "operation_conflict",
                "completed remote effect cannot be described as cancelled or rolled back",
            ));
        }
        let mut next = self.clone();
        if state == Cancellation::Confirmed {
            if !matches!(
                self.steps[id],
                StepState::SentUnknown { .. } | StepState::Acknowledged { .. }
            ) {
                return Err(Error::new(
                    "operation_conflict",
                    "confirmed cancellation has no active remote action",
                ));
            }
            let remote_id = remote_id.filter(|s| !s.is_empty()).ok_or_else(|| {
                Error::invalid("confirmed cancellation requires observed remote identity")
            })?;
            next.set_step(
                id.into(),
                StepState::Cancelled {
                    remote_id,
                    observed_at: now,
                },
            );
        }
        next.cancellations.insert(id.into(), state);
        next.hold_reason
            .get_or_insert_with(|| "release interrupted; reconcile each remote effect".into());
        persist(&next)?;
        *self = next;
        Ok(())
    }

    /// Abandon retires known unstarted work only. Unknown remote work stays
    /// unknown with its hold; the command never claims cancellation by assertion.
    pub fn abandon(
        &mut self,
        actor: &str,
        reason: &str,
        now: DateTime<Utc>,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        validate_recovery_actor(actor, reason)?;
        let mut next = self.clone();
        for id in next.steps.keys().cloned().collect::<Vec<_>>() {
            if matches!(
                next.steps[&id],
                StepState::NotStarted | StepState::Prepared { .. }
            ) {
                next.set_step(
                    id,
                    StepState::Retired {
                        reason: reason.into(),
                    },
                );
            }
        }
        next.abandoned = true;
        next.hold_reason.get_or_insert_with(|| {
            "abandoned partial release requires known baseline reconciliation".into()
        });
        next.recovery_records.push(RecoveryRecord {
            actor: actor.into(),
            action: "abandon".into(),
            reason: reason.into(),
            baseline_digest: None,
            observed_at: now,
        });
        persist(&next)?;
        *self = next;
        Ok(())
    }

    pub fn resume(
        &mut self,
        actor: &str,
        reason: &str,
        current_parent: &VerifiedParent,
        fresh_gates_passed: bool,
        now: DateTime<Utc>,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        validate_recovery_actor(actor, reason)?;
        if self.has_unknown()
            || self.abandoned
            || self.resolved
            || self.assigned_successor.is_some()
            || !fresh_gates_passed
            || current_parent.token() != self.plan.evidence_token
            || current_parent.manifest().work.generation != self.plan.generation
        {
            return Err(Error::new(
                "recovery_blocked",
                "resume requires the same still-current accepted result and known remote outcomes",
            ));
        }
        let mut next = self.clone();
        for id in next.steps.keys().cloned().collect::<Vec<_>>() {
            if matches!(
                next.steps[&id],
                StepState::Failed { .. } | StepState::Cancelled { .. } | StepState::Prepared { .. }
            ) {
                next.set_step(id, StepState::NotStarted);
            }
        }
        next.hold_reason = None;
        next.recovery_records.push(RecoveryRecord {
            actor: actor.into(),
            action: "resume".into(),
            reason: reason.into(),
            baseline_digest: None,
            observed_at: now,
        });
        persist(&next)?;
        *self = next;
        Ok(())
    }

    pub fn resolve(
        &mut self,
        actor: &str,
        reason: &str,
        baseline: &ReconciledBaseline,
        now: DateTime<Utc>,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        validate_recovery_actor(actor, reason)?;
        baseline.validate(&self.plan.resources)?;
        if self.has_unknown()
            || !baseline.coherent
            || !self.steps.values().all(StepState::is_terminal)
            || self.hold_reason.is_none()
            || self.completed()
        {
            return Err(Error::new(
                "recovery_blocked",
                "resolve requires known terminal actions and a verified coherent baseline; it cannot mark delivery successful",
            ));
        }
        let mut next = self.clone();
        next.resolved = true;
        next.hold_reason = None;
        next.recovery_records.push(RecoveryRecord {
            actor: actor.into(),
            action: "resolve".into(),
            reason: reason.into(),
            baseline_digest: Some(canonical_digest(1, baseline)?),
            observed_at: now,
        });
        persist(&next)?;
        *self = next;
        Ok(())
    }

    pub fn supersede(
        &mut self,
        decision: RecoveryDecision<'_>,
        successor: &ReleaseAttempt,
        acceptance: &VerifiedAcceptance,
        baseline: &ReconciledBaseline,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        self.supersede_plan(decision, &successor.plan, acceptance, baseline, persist)
    }

    /// Bind a corrected successor plan without claiming it has an execution
    /// admission. A control-host permit is still required before dispatch.
    pub fn supersede_plan(
        &mut self,
        decision: RecoveryDecision<'_>,
        successor: &ReleasePlan,
        acceptance: &VerifiedAcceptance,
        baseline: &ReconciledBaseline,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        successor.validate()?;
        let RecoveryDecision { actor, reason, now } = decision;
        validate_recovery_actor(actor, reason)?;
        baseline.validate(&self.plan.resources)?;
        let a = acceptance.acceptance();
        if self.has_unknown()
            || self.hold_reason.is_none()
            || self.resolved
            || self
                .assigned_successor
                .is_some_and(|id| id != successor.operation_id)
            || successor.runtime_id != self.plan.runtime_id
            || successor.parent_id != self.plan.parent_id
            || successor.linear_workspace != self.plan.linear_workspace
            || successor.generation == self.plan.generation
            || successor.operation_id == self.plan.operation_id
            || successor.generation != a.work.generation
            || successor.evidence_token != a.reviewed_token
            || successor.acceptance_digest != acceptance.digest()
            || successor.carried_acceptance.is_some()
            || successor.resources != self.plan.resources
        {
            return Err(Error::new(
                "recovery_blocked",
                "supersede requires known old outcomes and this parent's freshly accepted corrected successor within the same resources",
            ));
        }
        // The successor may explicitly represent a known main/deployed mismatch;
        // no stale predecessor deployment is started merely to erase divergence.
        let mut next = self.clone();
        for id in next.steps.keys().cloned().collect::<Vec<_>>() {
            if matches!(
                next.steps[&id],
                StepState::NotStarted | StepState::Prepared { .. }
            ) {
                next.set_step(
                    id,
                    StepState::Retired {
                        reason: reason.into(),
                    },
                );
            }
        }
        next.assigned_successor = Some(successor.operation_id);
        next.recovery_records.push(RecoveryRecord {
            actor: actor.into(),
            action: "supersede".into(),
            reason: reason.into(),
            baseline_digest: Some(canonical_digest(1, baseline)?),
            observed_at: now,
        });
        persist(&next)?;
        *self = next;
        Ok(())
    }
}

fn validate_recovery_actor(actor: &str, reason: &str) -> Result<()> {
    if actor.trim().is_empty() || reason.trim().is_empty() {
        return Err(Error::invalid(
            "explicit authenticated recovery actor and reason are required",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineObservation {
    pub resource: ResourceKey,
    pub production_commit: String,
    pub deployed_commit: Option<String>,
    pub artifact_digest: Option<String>,
    pub provider_record: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconciledBaseline {
    pub provider_reads_complete: bool,
    pub coherent: bool,
    pub observations: Vec<BaselineObservation>,
    pub observed_at: DateTime<Utc>,
}

impl ReconciledBaseline {
    fn validate(&self, resources: &BTreeSet<ResourceKey>) -> Result<()> {
        let mut seen = BTreeSet::new();
        for observation in &self.observations {
            if !seen.insert(observation.resource.clone()) || observation.provider_record.is_empty()
            {
                return Err(Error::new(
                    "recovery_blocked",
                    "baseline observations are duplicated or lack provider evidence",
                ));
            }
            validate_oid(&observation.production_commit)?;
            if let Some(commit) = &observation.deployed_commit {
                validate_oid(commit)?;
            }
            if let Some(digest) = &observation.artifact_digest {
                validate_digest(digest)?;
            }
            if matches!(observation.resource, ResourceKey::Target { .. })
                && (observation.deployed_commit.is_none() || observation.artifact_digest.is_none())
            {
                return Err(Error::new(
                    "recovery_blocked",
                    "deployment baseline is unknown",
                ));
            }
            if self.coherent
                && observation
                    .deployed_commit
                    .as_ref()
                    .is_some_and(|sha| sha != &observation.production_commit)
            {
                return Err(Error::new(
                    "recovery_blocked",
                    "claimed coherent baseline has main/deployed divergence",
                ));
            }
        }
        if !self.provider_reads_complete || seen != *resources {
            return Err(Error::new(
                "recovery_blocked",
                "every held resource needs a complete authoritative baseline observation",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuccessfulRelease {
    pub operation_id: Uuid,
    pub parent_id: Uuid,
    pub generation: Generation,
    pub outcome_digest: String,
    pub resources: BTreeSet<ResourceKey>,
    pub completed_at: DateTime<Utc>,
    pub spacing_seconds: u64,
    pub no_release_required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceHold {
    pub operation_id: Uuid,
    pub parent_id: Uuid,
    pub resources: BTreeSet<ResourceKey>,
    pub assigned_successor: Option<Uuid>,
}

/// Rebuilt from verified release records on recovery; this is an in-memory
/// projection, not an editable workflow database. Mutations require `&mut self`
/// under the single control-host mutation gate.
#[derive(Clone, Default, Debug)]
pub struct ReleaseQueue {
    owners: BTreeMap<ResourceKey, Uuid>,
    holds: BTreeMap<Uuid, ResourceHold>,
    successes: Vec<SuccessfulRelease>,
    completions: BTreeMap<Uuid, crate::runtime_delivery::VerifiedReleaseCompletion>,
}

impl ReleaseQueue {
    fn validate_completions(&self) -> Result<()> {
        for proof in self.completions.values() {
            proof.validate()?;
        }
        Ok(())
    }
    /// Remove only the exact ancestor holds of a successful, retired chain.
    /// Other owners/holds remain intact; only the terminal's ordinary success
    /// contributes dependency history and spacing during the subsequent rebuild.
    pub(crate) fn apply_completion(
        &mut self,
        proof: &crate::runtime_delivery::VerifiedReleaseCompletion,
    ) -> Result<()> {
        let expected = proof.validate()?;
        let terminal = proof.terminal();
        if let Some(prior) = self.completions.get(&terminal.plan.operation_id) {
            if prior.digest() != proof.digest() {
                return Err(Error::new("release_hold", "terminal completion changed"));
            }
            return Ok(());
        }
        if expected.is_empty()
            || expected.iter().any(|(operation, hold)| {
                self.holds.get(operation) != Some(hold)
                    || hold.resources != terminal.plan.resources
                    || *operation == terminal.plan.operation_id
            })
        {
            return Err(Error::new(
                "release_hold",
                "queue differs from exact completed ancestor hold inventory",
            ));
        }
        for operation in expected.keys() {
            self.holds.remove(operation);
        }
        self.completions
            .insert(terminal.plan.operation_id, proof.clone());
        Ok(())
    }
    pub fn restore_hold(&mut self, attempt: &ReleaseAttempt) -> Result<()> {
        attempt.validate_recovered()?;
        if attempt.hold_reason.is_some() && !attempt.resolved {
            self.holds.insert(
                attempt.plan.operation_id,
                ResourceHold {
                    operation_id: attempt.plan.operation_id,
                    parent_id: attempt.plan.parent_id,
                    resources: attempt.plan.resources.clone(),
                    assigned_successor: attempt.assigned_successor,
                },
            );
        }
        Ok(())
    }

    pub fn record_success(&mut self, attempt: &ReleaseAttempt) -> Result<SuccessfulRelease> {
        if !attempt.completed() || attempt.has_unknown() || attempt.hold_reason.is_some() {
            return Err(Error::new(
                "partial_release",
                "every required production step must be observed successful",
            ));
        }
        attempt.validate_recovered()?;
        let completed_at = attempt
            .steps
            .values()
            .filter_map(|s| match s {
                StepState::Succeeded { observed_at, .. } => Some(*observed_at),
                _ => None,
            })
            .max()
            .ok_or_else(|| Error::new("partial_release", "no observed completion"))?;
        let record = SuccessfulRelease {
            operation_id: attempt.plan.operation_id,
            parent_id: attempt.plan.parent_id,
            generation: attempt.plan.generation.clone(),
            outcome_digest: canonical_digest(1, &attempt.steps)?,
            resources: attempt.plan.resources.clone(),
            completed_at,
            spacing_seconds: attempt.plan.spacing_seconds,
            no_release_required: false,
        };
        if let Some(previous) = self
            .successes
            .iter()
            .find(|s| s.operation_id == record.operation_id)
        {
            if previous != &record {
                return Err(Error::new(
                    "operation_conflict",
                    "release completion changed after recording",
                ));
            }
            return Ok(previous.clone());
        }
        self.owners
            .retain(|_, owner| *owner != attempt.plan.operation_id);
        self.holds.remove(&attempt.plan.operation_id);
        self.successes.push(record.clone());
        Ok(record)
    }

    /// A verified accepted already-production outcome satisfies dependencies,
    /// but creates no merge/deployment activity or spacing event. Unknown
    /// overlapping effects still have to be reconciled first.
    pub fn record_no_release_required(
        &mut self,
        verified: &VerifiedNoReleaseRequired,
    ) -> Result<SuccessfulRelease> {
        let proof = verified.record();
        self.check_no_release_resources(proof)?;
        let mut resources: BTreeSet<_> = proof
            .baselines
            .iter()
            .map(|b| ResourceKey::Repository {
                repository: b.repository.clone(),
            })
            .collect();
        resources.extend(proof.targets.iter().map(|t| ResourceKey::Target {
            provider: t.provider.clone(),
            id: t.target.clone(),
        }));
        let record = SuccessfulRelease {
            operation_id: proof.operation_id,
            parent_id: proof.work.parent,
            generation: proof.work.generation.clone(),
            outcome_digest: verified.digest().into(),
            resources,
            completed_at: proof.recorded_at,
            spacing_seconds: 0,
            no_release_required: true,
        };
        if let Some(previous) = self
            .successes
            .iter()
            .find(|s| s.operation_id == record.operation_id)
        {
            if previous != &record {
                return Err(Error::new(
                    "operation_conflict",
                    "already-production decision changed after recording",
                ));
            }
            return Ok(previous.clone());
        }
        self.successes.push(record.clone());
        Ok(record)
    }

    pub(crate) fn check_no_release_resources(
        &self,
        record: &crate::evidence::NoReleaseRequired,
    ) -> Result<()> {
        self.validate_completions()?;
        let mut resources: BTreeSet<_> = record
            .baselines
            .iter()
            .map(|b| ResourceKey::Repository {
                repository: b.repository.clone(),
            })
            .collect();
        resources.extend(record.targets.iter().map(|t| ResourceKey::Target {
            provider: t.provider.clone(),
            id: t.target.clone(),
        }));
        if self.owners.keys().any(|r| resources.contains(r))
            || self
                .holds
                .values()
                .any(|h| !h.resources.is_disjoint(&resources))
        {
            return Err(Error::new(
                "release_hold",
                "already-production classification cannot clear active or unresolved delivery",
            ));
        }
        Ok(())
    }

    pub fn earliest_start(&self, plan: &ReleasePlan, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
        self.validate_completions()?;
        let mut earliest = now;
        for previous in &self.successes {
            if previous.no_release_required || previous.resources.is_disjoint(&plan.resources) {
                continue;
            }
            let spacing = previous.spacing_seconds.max(plan.spacing_seconds);
            let seconds = i64::try_from(spacing)
                .map_err(|_| Error::invalid("release spacing is out of range"))?;
            let end = previous
                .completed_at
                .checked_add_signed(Duration::seconds(seconds))
                .ok_or_else(|| Error::invalid("release spacing exceeds supported date range"))?;
            earliest = earliest.max(end);
        }
        Ok(earliest)
    }

    /// Ephemeral eligibility projection for one exact, authenticated chain.
    /// The caller's queue and every durable ancestor hold remain unchanged.
    pub(crate) fn successor_view(
        &self,
        plan: &ReleasePlan,
        proof: &crate::runtime_delivery::VerifiedSuccessorHolds,
    ) -> Result<Self> {
        let expected = proof.validate(plan)?;
        let mut view = self.clone();
        for (operation, hold) in expected {
            if self.holds.get(operation) != Some(hold) {
                return Err(Error::new(
                    "release_hold",
                    "queue hold differs from verified successor lineage",
                ));
            }
            view.holds
                .get_mut(operation)
                .expect("checked hold")
                .assigned_successor = Some(plan.operation_id);
        }
        Ok(view)
    }

    pub fn eligible(&self, plan: &ReleasePlan, now: DateTime<Utc>) -> Result<()> {
        self.validate_completions()?;
        plan.validate()?;
        for dependency in &plan.dependencies {
            if !self.successes.iter().any(|s| {
                s.parent_id == dependency.parent_id
                    && s.generation == dependency.generation
                    && s.outcome_digest == dependency.outcome_digest
            }) {
                return Err(Error::new(
                    "release_dependency",
                    "required parent outcome has not been observed successfully released",
                ));
            }
        }
        for hold in self.holds.values() {
            let dependent = plan
                .dependencies
                .iter()
                .any(|d| d.parent_id == hold.parent_id);
            if (!hold.resources.is_disjoint(&plan.resources) || dependent)
                && hold.assigned_successor != Some(plan.operation_id)
            {
                return Err(Error::new(
                    "release_hold",
                    "overlapping or dependent delivery is held by an unresolved attempt",
                ));
            }
        }
        if plan.resources.iter().any(|r| {
            self.owners
                .get(r)
                .is_some_and(|owner| *owner != plan.operation_id)
        }) {
            return Err(Error::new(
                "release_locked",
                "another admitted release owns an overlapping resource",
            ));
        }
        if self.earliest_start(plan, now)? > now {
            return Err(Error::new(
                "release_spacing",
                "completion-to-start spacing has not elapsed",
            ));
        }
        Ok(())
    }

    pub fn reserve(&mut self, plan: &ReleasePlan, now: DateTime<Utc>) -> Result<()> {
        self.eligible(plan, now)?;
        // No mutation before all keys are checked; stable BTreeSet order avoids
        // partial acquisition and is shared across workspace projections.
        for resource in &plan.resources {
            self.owners.insert(resource.clone(), plan.operation_id);
        }
        Ok(())
    }

    pub fn owns_all(&self, plan: &ReleasePlan) -> bool {
        self.validate_completions().is_ok()
            && !plan.resources.is_empty()
            && plan
                .resources
                .iter()
                .all(|r| self.owners.get(r) == Some(&plan.operation_id))
    }

    pub fn reconcile_attempt(&mut self, attempt: &ReleaseAttempt) -> Result<()> {
        attempt.validate_recovered()?;
        // Validate the complete recovered acquisition before releasing any
        // existing ownership. A conflicting snapshot must leave this projection
        // unchanged, including resources still owned by the recovering attempt.
        if !attempt.resolved && attempt.hold_reason.is_none() && !attempt.completed() {
            for resource in &attempt.plan.resources {
                if self
                    .owners
                    .get(resource)
                    .is_some_and(|owner| *owner != attempt.plan.operation_id)
                {
                    return Err(Error::new(
                        "ownership_conflict",
                        "recovered release resource has another owner",
                    ));
                }
            }
        }
        self.owners
            .retain(|_, owner| *owner != attempt.plan.operation_id);
        if attempt.resolved {
            self.holds.remove(&attempt.plan.operation_id);
        } else if attempt.hold_reason.is_some() {
            self.restore_hold(attempt)?;
        } else if !attempt.completed() {
            self.holds.remove(&attempt.plan.operation_id);
            for resource in &attempt.plan.resources {
                self.owners
                    .insert(resource.clone(), attempt.plan.operation_id);
            }
        }
        Ok(())
    }
}
