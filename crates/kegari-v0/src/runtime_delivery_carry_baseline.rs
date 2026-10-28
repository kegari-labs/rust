//! A prerequisite for acceptance carry, never acceptance or dispatch authority.
//!
//! This narrow producer covers baseline changes made by completed terminal
//! merges of other parents. It does not infer successful release from main,
//! tracker history, a raw artifact reference, or a partial ancestor's merge.
use crate::delivery::RepositoryContract;
use crate::deployment::VerifiedProductionBaseline;
use crate::digest::canonical_digest;
use crate::evidence::{ProductionBaseline, VerifiedParent, WorkIdentity};
use crate::git::{ProductionIntegration, RepositoryId, Revision, VerifiedProductionIntegration};
use crate::release::{ObservedEffect, ResourceKey, StepKind, StepState};
use crate::runtime_delivery::VerifiedReleaseCompletion;
use chrono::{Duration, Utc};
use kegari_config::Config;
use kegari_types::error::{Error, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Only these existing opaque inputs may supply a released link. The Git proof
/// authenticates the exact squash/object relationship; the completion proves
/// actual successful delivery and retirement, with immutable reference closure.
pub(crate) struct ReleasedCarryLink<'a> {
    pub completion: &'a VerifiedReleaseCompletion,
    pub integration: &'a VerifiedProductionIntegration,
}

#[derive(Clone, Debug, Serialize)]
struct Link {
    operation: Uuid,
    completion_digest: String,
    integration_digest: String,
    repository: RepositoryId,
    from: Revision,
    to: Revision,
}

/// Not serializable or constructible from public data. Retaining this proof
/// does not refresh provider observations or original/current parent evidence.
#[derive(Clone, Debug)]
pub(crate) struct VerifiedBaselineTransition {
    work: WorkIdentity,
    workspace: Uuid,
    original_token: String,
    configuration_revision: String,
    contracts: Vec<RepositoryContract>,
    baselines: Vec<ProductionBaseline>,
    digest: String,
    completions: Vec<VerifiedReleaseCompletion>,
}

fn blocked(message: &str) -> Error {
    Error::new("acceptance_carry_baseline", message)
}

impl VerifiedBaselineTransition {
    /// This is intentionally a relationship proof only. A later carry writer
    /// must verify the original human acceptance, exact unchanged patches,
    /// current authority and fresh integrated/production checks independently.
    pub(crate) fn observe(
        original: &VerifiedParent,
        baselines: &[VerifiedProductionBaseline],
        released: &[ReleasedCarryLink<'_>],
        contracts: &[RepositoryContract],
        configuration: &Config,
        workspace: Uuid,
    ) -> Result<Self> {
        let now = Utc::now();
        let old = original.manifest();
        if old.work.child.is_some() {
            return Err(blocked("carry baseline requires exact parent work"));
        }
        let configured = configuration
            .workspaces
            .values()
            .find(|w| w.id == workspace)
            .ok_or_else(|| blocked("carry workspace is no longer configured"))?;
        if configured.linear_workspace_id != old.work.linear_workspace {
            return Err(blocked("carry work differs from the configured workspace"));
        }
        let old_baselines = old
            .baselines
            .iter()
            .map(|b| (&b.repository, b))
            .collect::<BTreeMap<_, _>>();
        let new_baselines = baselines
            .iter()
            .map(|b| (&b.baseline().repository, b.baseline()))
            .collect::<BTreeMap<_, _>>();
        let observed = baselines
            .iter()
            .map(|b| (&b.baseline().repository, b))
            .collect::<BTreeMap<_, _>>();
        if old_baselines.is_empty()
            || old_baselines.len() != old.baselines.len()
            || new_baselines.len() != baselines.len()
            || observed.len() != baselines.len()
            || old_baselines.keys().collect::<BTreeSet<_>>() != new_baselines.keys().collect()
            || new_baselines.keys().collect::<BTreeSet<_>>() != observed.keys().collect()
        {
            return Err(blocked(
                "carry requires the complete unique repository baseline vector",
            ));
        }
        let selected_contracts = contracts
            .iter()
            .map(|c| (&c.repository, c))
            .collect::<BTreeMap<_, _>>();
        if selected_contracts.len() != contracts.len()
            || selected_contracts.keys().collect::<BTreeSet<_>>() != new_baselines.keys().collect()
        {
            return Err(blocked(
                "carry needs every current repository contract including unchanged repositories",
            ));
        }
        for (repository, contract) in &selected_contracts {
            let actual = configured
                .repositories
                .get(&contract.configuration_name)
                .ok_or_else(|| blocked("carry repository is no longer configured"))?;
            if canonical_digest(1, actual)? != canonical_digest(1, &contract.configuration)?
                || contract.configuration_name != repository.id
                || contract.configuration.hostname != repository.hostname
                || observed[repository].contract_digest() != contract.digest()?
            {
                return Err(blocked(
                    "carry repository contract differs from complete configured scope",
                ));
            }
        }
        let maximum_links = VerifiedReleaseCompletion::inventory_limit()
            .checked_mul(contracts.len())
            .ok_or_else(|| blocked("carry release inventory bound overflow"))?;
        if released.len() > maximum_links {
            return Err(blocked(
                "carry release links exceed the complete retained inventory bound",
            ));
        }
        let mut unique_links = BTreeSet::new();
        let mut links = Vec::new();
        let mut completions = BTreeMap::new();
        for supplied in released {
            supplied
                .completion
                .validate_for_configuration(configuration)?;
            let integration = supplied.integration.record();
            if supplied.completion.workspace_id() != workspace
                || supplied.completion.work().linear_endpoint != old.work.linear_endpoint
                || supplied.completion.work().linear_workspace != old.work.linear_workspace
                || supplied.completion.work().parent == old.work.parent
            {
                return Err(blocked(
                    "carry baseline supports only completed other-parent releases in the same native scope",
                ));
            }
            if !unique_links.insert((
                integration.release_operation,
                integration.repository.clone(),
            )) {
                return Err(blocked("carry release link is duplicated"));
            }
            match_completed_merge(supplied.completion, integration)?;
            if new_baselines
                .get(&integration.repository)
                .is_none_or(|baseline| integration.observed_main != baseline.production)
            {
                return Err(blocked(
                    "Git integration was not observed through this exact current main",
                ));
            }
            links.push(Link {
                operation: integration.release_operation,
                completion_digest: supplied.completion.digest().into(),
                integration_digest: integration.digest()?,
                repository: integration.repository.clone(),
                from: integration.production_parent.clone(),
                to: integration.merged.clone(),
            });
            if let Some(previous) =
                completions.insert(integration.release_operation, supplied.completion.clone())
                && previous.digest() != supplied.completion.digest()
            {
                return Err(blocked(
                    "one release operation has conflicting completion identities",
                ));
            }
        }
        if completions.len() > VerifiedReleaseCompletion::inventory_limit() {
            return Err(blocked(
                "carry releases exceed the complete retained inventory bound",
            ));
        }
        let mut consumed = BTreeSet::new();
        let mut paths = BTreeMap::new();
        let mut targets = BTreeSet::new();
        let mut changed = false;
        for (repository, new_baseline) in &new_baselines {
            let old_baseline = old_baselines[repository];
            old_baseline.validate()?;
            new_baseline.validate()?;
            let current_baseline = observed[repository];
            if current_baseline.baseline() != *new_baseline
                || current_baseline.observed_at() > now + Duration::seconds(5)
                || now - current_baseline.observed_at() > Duration::seconds(30)
                || old_baseline.deployment_free != new_baseline.deployment_free
                || old_baseline.successor_recovery_operation.is_some()
                || new_baseline.successor_recovery_operation.is_some()
            {
                return Err(blocked(
                    "carry baseline must be an exact fresh ordinary production observation",
                ));
            }
            for target in current_baseline.targets() {
                if &target.repository != *repository
                    || target.source != new_baseline.production
                    || new_baseline.source_for_target(&target.target)
                        != Some(target.source.commit.as_str())
                    || new_baseline.artifact_for_target(&target.target)
                        != Some(target.artifact_digest.as_str())
                    || !targets.insert(target.target.clone())
                {
                    return Err(blocked(
                        "carry target source or artifact differs from the complete current baseline",
                    ));
                }
            }
            if old_baseline.production == new_baseline.production {
                if old_baseline != *new_baseline {
                    return Err(blocked(
                        "same-source artifact changes need a separately verified released transition",
                    ));
                }
                continue;
            }
            changed = true;
            let path = exact_chain(
                repository,
                &old_baseline.production,
                &new_baseline.production,
                &links,
            )?;
            for index in &path {
                consumed.insert(*index);
            }
            let terminal = &links[*path
                .last()
                .ok_or_else(|| blocked("changed baseline has no released link"))?];
            let completion = completions
                .get(&terminal.operation)
                .ok_or_else(|| blocked("released baseline lost its terminal completion"))?;
            match_current_targets(completion, current_baseline)?;
            paths.insert(
                (*repository).clone(),
                path.into_iter()
                    .map(|i| links[i].clone())
                    .collect::<Vec<_>>(),
            );
        }
        if !changed || consumed.len() != links.len() || targets != old.production_targets {
            return Err(blocked(
                "carry must account for every changed baseline, supplied release and current target",
            ));
        }
        let configuration_revision = crate::config::config_revision(configuration)?;
        let current_baselines = new_baselines.into_values().cloned().collect::<Vec<_>>();
        let digest = canonical_digest(
            1,
            &serde_json::json!({
                "work":old.work,"workspace":workspace,"original":original.token(),
                "configuration":configuration_revision,
                "contracts":contracts.iter().map(RepositoryContract::digest).collect::<Result<Vec<_>>>()?,
                "original_baselines":old.baselines,"current_baselines":current_baselines,
                "released_paths":paths.into_iter().collect::<Vec<_>>(),
            }),
        )?;
        Ok(Self {
            work: old.work.clone(),
            workspace,
            original_token: original.token().into(),
            configuration_revision,
            contracts: contracts.to_vec(),
            baselines: current_baselines,
            digest,
            completions: completions.into_values().collect(),
        })
    }

    /// Immutable closure and complete configuration only, never renewed provider
    /// observations. The next admitted phase separately calls verify_current.
    pub(crate) fn revalidate(&self, configuration: &Config) -> Result<()> {
        if crate::config::config_revision(configuration)? != self.configuration_revision {
            return Err(blocked("carry baseline configuration changed"));
        }
        for completion in &self.completions {
            completion.validate_for_configuration(configuration)?;
        }
        Ok(())
    }
    pub(crate) fn verify_current(
        &self,
        configuration: &Config,
        observed: &[VerifiedProductionBaseline],
    ) -> Result<()> {
        self.revalidate(configuration)?;
        let now = Utc::now();
        if observed.len() != self.baselines.len() {
            return Err(blocked("current carry baseline inventory differs"));
        }
        let mut seen = BTreeSet::new();
        for actual in observed {
            let repository = &actual.baseline().repository;
            if !seen.insert(repository)
                || !self.baselines.contains(actual.baseline())
                || self
                    .contracts
                    .iter()
                    .find(|c| &c.repository == repository)
                    .is_none_or(|c| c.digest().ok().as_deref() != Some(actual.contract_digest()))
                || actual.observed_at() > now + Duration::seconds(5)
                || now - actual.observed_at() > Duration::seconds(30)
            {
                return Err(blocked("current carry baseline changed or expired"));
            }
        }
        Ok(())
    }
    pub(crate) fn binds_current_parent(
        &self,
        original: &VerifiedParent,
        current: &VerifiedParent,
    ) -> bool {
        self.work == original.manifest().work
            && self.work == current.manifest().work
            && self.original_token == original.token()
            && original.token() != current.token()
            && self.baselines == current.manifest().baselines
    }
    pub(crate) fn carry_references(
        &self,
        configuration: &Config,
    ) -> Result<crate::delivery::CarryReferences> {
        self.revalidate(configuration)?;
        let mut result = crate::delivery::CarryReferences::default();
        for completion in &self.completions {
            result.extend(&completion.carry_references()?)?;
        }
        Ok(result)
    }
    pub(crate) fn work(&self) -> &WorkIdentity {
        &self.work
    }
    pub(crate) fn workspace(&self) -> Uuid {
        self.workspace
    }
    pub(crate) fn original_token(&self) -> &str {
        &self.original_token
    }
    pub(crate) fn baselines(&self) -> &[ProductionBaseline] {
        &self.baselines
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
}

fn match_completed_merge(
    completion: &VerifiedReleaseCompletion,
    integration: &ProductionIntegration,
) -> Result<()> {
    let attempt = completion.terminal();
    let plan = &attempt.plan;
    if plan.operation_id != integration.release_operation
        || plan.parent_id != integration.parent_id
        || plan.evidence_token != integration.accepted_evidence_token
        || plan.acceptance_digest != integration.acceptance_digest
    {
        return Err(blocked(
            "Git integration differs from its successful release identity",
        ));
    }
    let matches = plan.steps.iter().filter(|step| match &step.kind {
        StepKind::Merge { repository, pull_request, production_base, candidate, selected_delta_digests } => {
            repository == &integration.repository && *pull_request == integration.pull_request
                && production_base == &integration.production_parent && candidate == &integration.candidate
                && selected_delta_digests == &integration.selected_delta_digests
                && matches!(attempt.steps.get(&step.id), Some(StepState::Succeeded { effect, .. })
                    if matches!(effect.as_ref(), ObservedEffect::Merge { repository, pull_request, commit, parent_commit }
                        if repository == &integration.repository && *pull_request == integration.pull_request
                            && commit == &integration.merged && parent_commit == &integration.production_parent.commit))
        }
        _ => false,
    }).count();
    if matches != 1 {
        return Err(blocked(
            "Git integration has no exact unique successful terminal merge",
        ));
    }
    Ok(())
}

fn match_current_targets(
    completion: &VerifiedReleaseCompletion,
    baseline: &VerifiedProductionBaseline,
) -> Result<()> {
    for target in baseline.targets() {
        let matches = completion.terminal().steps.values().filter(|state| matches!(state,
            StepState::Succeeded { effect, .. } if matches!(effect.as_ref(), ObservedEffect::Verification {
                repository, target: ResourceKey::Target { provider, id }, source_commit, tree, artifact_digest, ..
            } if repository == &target.repository && provider == &target.provider && id == &target.target
                && source_commit == &target.source.commit && tree == &target.source.tree
                && artifact_digest == &target.artifact_digest)
        )).count();
        if matches != 1 {
            return Err(blocked(
                "current target is not the exact successfully released serving source/artifact",
            ));
        }
    }
    Ok(())
}

// Data-only graph selection. The caller has already bound every link to actual
// opaque completion and Git proofs. Tests below never construct either proof.
fn exact_chain(
    repository: &RepositoryId,
    old: &Revision,
    new: &Revision,
    links: &[Link],
) -> Result<Vec<usize>> {
    let mut adjacent = BTreeMap::<(&str, &str), Vec<usize>>::new();
    for (index, link) in links.iter().enumerate() {
        if &link.repository == repository {
            adjacent
                .entry((&link.from.commit, &link.from.tree))
                .or_default()
                .push(index);
        }
    }
    let mut cursor = old;
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();
    while cursor != new {
        if !seen.insert(cursor.commit.clone()) {
            return Err(blocked("released baseline chain contains a cycle"));
        }
        let Some(indices) = adjacent.get(&(cursor.commit.as_str(), cursor.tree.as_str())) else {
            return Err(blocked(
                "released baseline chain is missing, duplicated or forked",
            ));
        };
        let [index] = indices.as_slice() else {
            return Err(blocked(
                "released baseline chain is missing, duplicated or forked",
            ));
        };
        let link = &links[*index];
        if link.from == link.to {
            return Err(blocked("released baseline link made no source progress"));
        }
        result.push(*index);
        cursor = &link.to;
    }
    Ok(result)
}

#[cfg(test)]
#[path = "runtime_delivery_carry_baseline_tests.rs"]
mod tests;

/// Select reference candidates only. The returned IDs confer no authority;
/// observe() subsequently requires each exact Git integration and completion.
pub(crate) fn select_released_transitions(
    original: &VerifiedParent,
    baselines: &[std::sync::Arc<VerifiedProductionBaseline>],
    completions: &[VerifiedReleaseCompletion],
) -> Result<BTreeMap<Uuid, BTreeSet<RepositoryId>>> {
    let mut links = Vec::new();
    for proof in completions {
        if proof.work().parent == original.manifest().work.parent {
            continue;
        }
        for step in &proof.terminal().plan.steps {
            let StepKind::Merge {
                repository,
                production_base,
                ..
            } = &step.kind
            else {
                continue;
            };
            let Some(StepState::Succeeded { effect, .. }) = proof.terminal().steps.get(&step.id)
            else {
                continue;
            };
            let ObservedEffect::Merge { commit, .. } = effect.as_ref() else {
                continue;
            };
            links.push(Link {
                operation: proof.terminal().plan.operation_id,
                completion_digest: proof.digest().into(),
                integration_digest: String::new(),
                repository: repository.clone(),
                from: production_base.clone(),
                to: commit.clone(),
            });
        }
    }
    let mut selected = BTreeMap::<Uuid, BTreeSet<RepositoryId>>::new();
    for baseline in baselines {
        let repository = &baseline.baseline().repository;
        let old = original
            .manifest()
            .baselines
            .iter()
            .find(|b| &b.repository == repository)
            .ok_or_else(|| blocked("new baseline expanded original repository scope"))?;
        if old.production == baseline.baseline().production {
            continue;
        }
        for index in exact_chain(
            repository,
            &old.production,
            &baseline.baseline().production,
            &links,
        )? {
            selected
                .entry(links[index].operation)
                .or_default()
                .insert(repository.clone());
        }
    }
    Ok(selected)
}
