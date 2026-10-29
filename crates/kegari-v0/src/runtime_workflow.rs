//! Shared control-host decision facade. Provider publication and release work
//! remain concrete services; cache insertion accepts only sealed evidence.
use crate::authority::IssueKey;
use crate::control::Receipt;
use crate::control::ReceiptPhase;
use crate::decisions::AuthorizedDecision;
use crate::decisions::AuthorizedOperationRecovery;
use crate::decisions::DecisionDependencies;
use crate::decisions::DecisionEvidence;
use crate::decisions::DecisionManager;
use crate::decisions::RetryTarget;
use crate::decisions::VerifiedExplicitCorrection;
use crate::decisions::VerifiedRetryGrant;
use crate::delivery::RuntimeDeliveryDecisions;
use crate::evidence::Publication;
use crate::evidence::VerifiedAcceptance;
use crate::evidence::WorkIdentity;
use crate::evidence::decode_signed_publication;
use crate::reconcile::DependencyOutcomes;
use crate::reconcile::ParentDelivery;
use crate::runtime::RuntimeCoordinator;
use crate::runtime_host::HostFuture;
use crate::runtime_host::RuntimeInspection;
use crate::runtime_host::WorkflowControl;
use kegari_control_api::identity::Actor;
use kegari_digest::canonical_digest;
use kegari_secrets::AttestationKey;
use kegari_types::api::CommandRequest;
use kegari_types::error::{Error, Result};
use kegari_types::run::PhaseKey;
use kegari_types::run::RunReference;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use uuid::Uuid;

pub struct RuntimeWorkflow {
    runtime: Arc<RuntimeCoordinator>,
    decisions: DecisionManager,
    delivery: Arc<dyn RuntimeDeliveryDecisions>,
    progression: Arc<dyn crate::runtime_driver::RuntimeProgression>,
    driver: Option<Arc<crate::runtime_driver::RuntimeDriver>>,
}
impl RuntimeWorkflow {
    /// The release/recovery implementation is mandatory. Construction cannot
    /// substitute success or an unsupported-operation handler for that backend.
    pub fn new(
        runtime: Arc<RuntimeCoordinator>,
        delivery: Arc<dyn RuntimeDeliveryDecisions>,
        progression: Arc<dyn crate::runtime_driver::RuntimeProgression>,
    ) -> Self {
        let services = &runtime.services;
        let decisions = DecisionManager::new(DecisionDependencies {
            configuration: services.configuration.clone(),
            control: services.control.clone(),
            observations: services.observations.clone(),
            secrets: services.secrets.clone(),
            key: services.key.clone(),
            evidence: Arc::new(RuntimeDecisionEvidence {
                runtime: runtime.clone(),
                delivery: delivery.clone(),
            }),
            housekeeping: services.housekeeping.clone(),
            inputs: services.inputs.clone(),
        });
        Self {
            runtime,
            decisions,
            delivery,
            progression,
            driver: None,
        }
    }

    pub(crate) fn with_driver(mut self, driver: Arc<crate::runtime_driver::RuntimeDriver>) -> Self {
        self.driver = Some(driver);
        self
    }
}
impl WorkflowControl for RuntimeWorkflow {
    fn inspect(&self) -> HostFuture<'_, RuntimeInspection> {
        Box::pin(async {
            let runtime = self.runtime.clone();
            tokio::task::spawn_blocking(move || runtime.recover_provider_holds())
                .await
                .map_err(|_| {
                    Error::new(
                        "reconciliation_required",
                        "provider journal recovery task failed",
                    )
                })??;
            self.progression.reconcile_for_inspection().await?;
            let mut view = self.runtime.inspection()?;
            view.release_history = self.delivery.release_inspection()?;
            view.no_release_history = self.delivery.no_release_inspection()?;
            crate::inspection::validate_history(
                view.projection.runtime_id,
                &view.release_history,
                &view.no_release_history,
            )?;
            crate::inspection::sort_rows(&mut view.release_history);
            crate::inspection::sort_no_release_rows(&mut view.no_release_history);
            Ok(view)
        })
    }
    fn health(&self) -> Result<Value> {
        Ok(json!({"providers":self.runtime.observations.health()?,
            "accounts":self.runtime.account_observations()?,
            "driver":self.driver.as_ref().map(|driver| driver.diagnostics()).transpose()?}))
    }
    fn command_targets(&self, request: &CommandRequest) -> Result<Vec<String>> {
        if request.command == kegari_types::api::CommandName::EvaluationRun {
            return self.runtime.evaluation_targets(request);
        }
        self.decisions.command_targets(request)
    }
    fn execute(
        &self,
        actor: Actor,
        request: CommandRequest,
        parent: Receipt,
    ) -> HostFuture<'_, Receipt> {
        if request.command == kegari_types::api::CommandName::EvaluationRun {
            return Box::pin(self.runtime.execute_evaluation(actor, request, parent));
        }
        Box::pin(self.decisions.execute(actor, request, parent))
    }
    fn cleanup_plan(&self, operation: Uuid) -> HostFuture<'_, Value> {
        Box::pin(self.decisions.cleanup_plan(operation))
    }
}

/// Recover a grant from a current complete provider comment and its actual
/// local publication/admission lineage. HMAC-valid text alone cannot establish
/// that this runtime acknowledged that comment or consumed the named budget.
pub(crate) fn recover_retry_comment(
    runtime: Uuid,
    workspace: Uuid,
    writer: &str,
    work: &WorkIdentity,
    comment: &kegari_providers::Comment,
    receipts: &[Receipt],
    key: &AttestationKey,
) -> Result<VerifiedRetryGrant> {
    let signed = decode_signed_publication::<crate::decisions::RetryGrant>(
        &comment.body,
        "operator_retry",
        key,
    )?;
    if comment.bot
        || comment.delegated
        || comment.issue_id != work.child.unwrap_or(work.parent)
        || comment.user_id.map(|id| id.to_string()).as_deref() != Some(writer)
        || comment.body_digest != kegari_digest::bytes_digest(comment.body.as_bytes())
        || signed.payload.workspace_id != workspace
        || signed.payload.provider_writer != writer
    {
        return Err(Error::new(
            "invalid_attestation",
            "retry comment target or native writer differs",
        ));
    }
    let publication = Publication {
        provider: "linear".into(),
        record_id: comment.id.to_string(),
        writer_account_id: writer.into(),
        payload_digest: canonical_digest(1, &signed)?,
        observed_at: chrono::Utc::now(),
        url: comment.url.clone(),
    };
    let grant = VerifiedRetryGrant::verify(&signed, &publication, work, runtime, key)?;
    let receipt = receipts
        .iter()
        .find(|r| r.operation_id == grant.record().operation_id)
        .ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "retry publication receipt is missing",
            )
        })?;
    if receipt.runtime_id != runtime
        || receipt.kind != "linear_comment"
        || !matches!(
            receipt.phase,
            ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
        )
        || receipt.remote_ids.get("comment_id") != Some(&comment.id.to_string())
        || receipt.expected.get("provider_body_digest") != Some(&comment.body_digest)
        || receipt.expected.get("parent_command_id") != Some(&grant.record().command_id.to_string())
        || receipt.expected.get("provider_writer").map(String::as_str) != Some(writer)
        || !receipt.canonical_targets.contains(&format!(
            "linear:{}:{}:{}",
            work.linear_endpoint,
            work.linear_workspace,
            work.child.unwrap_or(work.parent)
        ))
    {
        return Err(Error::new(
            "reconciliation_required",
            "retry publication acknowledgement differs from current provider record",
        ));
    }
    let mut actual = BTreeMap::new();
    let mut ordinals = BTreeSet::new();
    for prior in receipts {
        if let Some(run) = crate::control::run_reference_from_receipt(prior)?
            && &run.phase == grant.phase()
        {
            if prior.runtime_id != runtime
                || run.workspace != workspace
                || !ordinals.insert(run.attempt)
                || !matches!(
                    prior.phase,
                    ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                )
                || prior.observed_result.as_ref().is_none_or(|r| {
                    r["local_execution"] != "exited"
                        || r["unresolved_effects"]
                            .as_array()
                            .is_none_or(|v| !v.is_empty())
                })
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "retry prior admissions are not completely reconciled",
                ));
            }
            crate::runtime_schedule::require_provider_clear(
                runtime,
                &run.account,
                &run.phase.work,
                receipts,
            )?;
            actual.insert(prior.operation_id, prior.payload_digest.clone());
        }
    }
    if actual != grant.record().prior_admissions
        || ordinals.iter().copied().ne(1..=ordinals.len() as u32)
    {
        return Err(Error::new(
            "reconciliation_required",
            "retry publication does not cover the actual complete phase history",
        ));
    }
    Ok(grant)
}
pub(crate) fn current_retry_grants(
    config: &kegari_config::Config,
    projection: &crate::reconcile::RuntimeProjection,
    work: &WorkIdentity,
    receipts: &[Receipt],
    key: &AttestationKey,
) -> Result<Vec<VerifiedRetryGrant>> {
    if projection.runtime_id != config.runtime.id
        || projection.config_revision != crate::config::config_revision(config)?
    {
        return Err(Error::new(
            "stale_config",
            "retry observation predates current configuration",
        ));
    }
    let mut matches = projection
        .workspaces
        .values()
        .filter(|w| w.complete)
        .filter_map(|workspace| {
            let parent = workspace.parents.get(&work.parent)?;
            let selected = if let Some(child) = work.child {
                parent.children.get(&child)?.work.as_ref()
            } else {
                parent.work.as_ref()
            };
            (selected == Some(work)).then_some((workspace, parent))
        });
    let (workspace, parent) = matches.next().ok_or_else(|| {
        Error::new(
            "stale_result",
            "retry work is not current in a complete provider observation",
        )
    })?;
    if matches.next().is_some() {
        return Err(Error::new(
            "ownership_conflict",
            "retry work is selected by multiple workspaces",
        ));
    }
    let configured = config
        .workspaces
        .values()
        .find(|w| w.id == workspace.workspace_id)
        .ok_or_else(|| Error::new("stale_config", "retry workspace was removed"))?;
    let writer = &config.connections[&configured.linear_connection].account_id;
    let target = work.child.unwrap_or(work.parent);
    let observed = parent.observations.get(&target).ok_or_else(|| {
        Error::new(
            "incomplete_snapshot",
            "retry work has no current comment observation",
        )
    })?;
    recover_retry_comments(
        config.runtime.id,
        workspace.workspace_id,
        writer,
        work,
        &observed.comments,
        receipts,
        key,
    )
}
/// Observe exact native grants for a retained parent/worker identity. This is
/// a data reader, not an admission gate: the active adapter must first verify
/// its actual owner/permit, pinned configuration and registered active snapshot.
/// In particular, this never substitutes a current projection for retained work.
pub(crate) fn retained_parent_retry_grants(
    config: &kegari_config::Config,
    parent: &crate::reconcile::ParentProjection,
    work: &WorkIdentity,
    receipts: &[Receipt],
    key: &AttestationKey,
) -> Result<Vec<VerifiedRetryGrant>> {
    let configured = config
        .workspaces
        .values()
        .find(|workspace| workspace.id == parent.workspace_id)
        .ok_or_else(|| Error::new("stale_config", "retained retry workspace is absent"))?;
    let connection = config
        .connections
        .get(&configured.linear_connection)
        .ok_or_else(|| Error::new("stale_config", "retained retry connection is absent"))?;
    let endpoint = connection
        .endpoint
        .as_deref()
        .ok_or_else(|| Error::new("identity_mismatch", "retained retry endpoint is absent"))?;
    let key_parent = IssueKey::new(endpoint, configured.linear_workspace_id, work.parent)?;
    let selected = match work.child {
        Some(child) => parent
            .children
            .get(&child)
            .and_then(|child| child.work.as_ref()),
        None => parent.work.as_ref(),
    };
    if connection.kind != kegari_config::ProviderKind::Linear
        || parent.key != key_parent
        || work.linear_endpoint != key_parent.endpoint
        || work.linear_workspace != key_parent.workspace
        || selected != Some(work)
    {
        return Err(Error::new(
            "identity_mismatch",
            "retained retry observation does not bind the exact admitted work",
        ));
    }
    let observed = parent
        .observations
        .get(&work.child.unwrap_or(work.parent))
        .ok_or_else(|| Error::new("incomplete_snapshot", "retained retry comments are absent"))?;
    recover_retry_comments(
        config.runtime.id,
        parent.workspace_id,
        &connection.account_id,
        work,
        &observed.comments,
        receipts,
        key,
    )
}

fn recover_retry_comments(
    runtime: Uuid,
    workspace: Uuid,
    writer: &str,
    work: &WorkIdentity,
    comments: &[kegari_providers::Comment],
    receipts: &[Receipt],
    key: &AttestationKey,
) -> Result<Vec<VerifiedRetryGrant>> {
    let mut grants = BTreeMap::new();
    for comment in comments.iter().filter(|c| {
        c.body.starts_with("<!-- kegari:v0:operator_retry -->")
            && c.user_id.map(|id| id.to_string()).as_deref() == Some(writer)
    }) {
        let signed = decode_signed_publication::<crate::decisions::RetryGrant>(
            &comment.body,
            "operator_retry",
            key,
        )?;
        let old = &signed.payload.work;
        if old != work
            && old.linear_endpoint == work.linear_endpoint
            && old.linear_workspace == work.linear_workspace
            && old.parent == work.parent
            && old.child == work.child
        {
            continue;
        }
        let grant =
            recover_retry_comment(runtime, workspace, writer, work, comment, receipts, key)?;
        if grants.insert(grant.record().operation_id, grant).is_some() {
            return Err(Error::new(
                "ownership_conflict",
                "multiple native publications claim one retry grant",
            ));
        }
    }
    Ok(grants.into_values().collect())
}

pub(crate) fn require_current_grant<'a>(
    expected: &VerifiedRetryGrant,
    current: &'a [VerifiedRetryGrant],
) -> Result<&'a VerifiedRetryGrant> {
    current
        .iter()
        .find(|grant| {
            grant.record().operation_id == expected.record().operation_id
                && grant.digest() == expected.digest()
                && grant.provenance().record_id == expected.provenance().record_id
                && grant.provenance().writer_account_id == expected.provenance().writer_account_id
        })
        .ok_or_else(|| {
            Error::new(
                "stale_result",
                "retry grant no longer has its exact current native publication",
            )
        })
}
/// Finalize an already published decision even if its exact new budget has
/// started. This observes the original history; it does not select new work or
/// reconcile any successor process/effect.
pub(crate) fn published_retry_target(
    config: &kegari_config::Config,
    expected: &VerifiedRetryGrant,
    current: &[VerifiedRetryGrant],
    receipts: &[Receipt],
) -> Result<RetryTarget> {
    require_current_grant(expected, current)?;
    if current
        .iter()
        .filter(|g| g.work() == expected.work() && g.phase() == expected.phase())
        .count()
        != 1
    {
        return Err(Error::new(
            "ownership_conflict",
            "retry publication has a competing successor grant",
        ));
    }
    let workspace = config
        .workspaces
        .values()
        .find(|w| w.id == expected.record().workspace_id)
        .ok_or_else(|| Error::new("stale_config", "retry workspace disappeared"))?;
    let account = &config.connections[&workspace.codex_connection].account_id;
    let mut actual = BTreeMap::new();
    let mut ordinals = BTreeSet::new();
    for receipt in receipts {
        let Some(run) = crate::control::run_reference_from_receipt(receipt)? else {
            continue;
        };
        if run.phase != *expected.phase() && run.phase != expected.phase_key() {
            continue;
        }
        if receipt.runtime_id != config.runtime.id
            || run.workspace != workspace.id
            || &run.account != account
        {
            return Err(Error::new(
                "identity_mismatch",
                "retry history account/workspace/runtime changed",
            ));
        }
        if run.phase == *expected.phase() {
            if !matches!(
                receipt.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            ) || receipt.observed_result.as_ref().is_none_or(|value| {
                value["local_execution"] != "exited"
                    || value["unresolved_effects"]
                        .as_array()
                        .is_none_or(|effects| !effects.is_empty())
            }) || !ordinals.insert(run.attempt)
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "published retry original history is not completely retired",
                ));
            }
            actual.insert(receipt.operation_id, receipt.payload_digest.clone());
        }
    }
    if actual != expected.record().prior_admissions
        || ordinals.iter().copied().ne(1..=ordinals.len() as u32)
    {
        return Err(Error::new(
            "stale_result",
            "published retry original admission inventory changed",
        ));
    }
    Ok(RetryTarget {
        phase: expected.phase().clone(),
        admissions: actual.into_keys().collect(),
    })
}
struct RuntimeDecisionEvidence {
    runtime: Arc<RuntimeCoordinator>,
    delivery: Arc<dyn RuntimeDeliveryDecisions>,
}
impl DecisionEvidence for RuntimeDecisionEvidence {
    fn current(&self, parent: &IssueKey) -> Result<ParentDelivery> {
        self.runtime.current_delivery(parent)
    }
    fn outcomes(&self) -> Result<DependencyOutcomes> {
        self.runtime.dependency_outcomes()
    }
    fn retry_target(&self, work: &WorkIdentity) -> Result<RetryTarget> {
        self.runtime.retry_target(work)
    }
    fn published_retry_target(&self, expected: &VerifiedRetryGrant) -> Result<RetryTarget> {
        self.runtime.published_retry_target(expected)
    }
    fn record_acceptance(&self, parent: &IssueKey, acceptance: VerifiedAcceptance) -> Result<()> {
        self.runtime.retain_acceptance(parent, acceptance)
    }
    fn record_correction(&self, correction: VerifiedExplicitCorrection) -> Result<()> {
        self.runtime.retain_correction(correction)
    }
    fn retry_committed(&self, grant: VerifiedRetryGrant) -> Result<()> {
        self.runtime.retain_retry_grant(grant)
    }
    fn release(&self, decision: AuthorizedDecision) -> HostFuture<'_, Receipt> {
        self.delivery.release(decision)
    }
    fn recover_release(&self, decision: AuthorizedDecision) -> HostFuture<'_, Receipt> {
        self.delivery.recover_release(decision)
    }
    fn reconcile(&self, decision: AuthorizedOperationRecovery) -> HostFuture<'_, Receipt> {
        self.delivery.reconcile(decision)
    }
}

/// Select an unambiguous unresolved phase from real run receipts. Ordering by
/// last-modified time would let a late recovery of an older phase retarget retry.
/// Superseded budgets require a verified grant, not a caller-supplied phase name.
#[cfg(test)]
pub(crate) fn select_retry_target(
    runtime: Uuid,
    work: &WorkIdentity,
    receipts: &[Receipt],
    grants: &[VerifiedRetryGrant],
) -> Result<RetryTarget> {
    select_retry_target_inner(runtime, work, receipts, grants, &BTreeMap::new())
}

/// Completed protocol outcomes become retry targets only through the genuine
/// retained, currently blocked delivery obligation. A completed receipt alone
/// is still excluded by the ordinary selector.
pub(crate) fn select_retry_target_with_unfinished(
    runtime: Uuid,
    work: &WorkIdentity,
    receipts: &[Receipt],
    grants: &[VerifiedRetryGrant],
    unfinished: &[Arc<crate::delivery::UnfinishedChildWork>],
    limits: &kegari_config::RuntimeConfig,
) -> Result<RetryTarget> {
    let mut completed = BTreeMap::new();
    for proof in unfinished {
        if proof.authority().work() != work
            || proof.predecessor_receipt().runtime_id != runtime
            || limits.id != runtime
        {
            return Err(Error::new(
                "scope_violation",
                "unfinished retry target differs from current work/runtime",
            ));
        }
        if proof.retry_blocker(limits, receipts)?
            && completed
                .insert(proof.phase().clone(), proof.predecessor())
                .is_some()
        {
            return Err(Error::new(
                "ambiguous_retry_target",
                "multiple retained obligations target one completed budget",
            ));
        }
    }
    select_retry_target_inner(runtime, work, receipts, grants, &completed)
}
fn select_retry_target_inner(
    runtime: Uuid,
    work: &WorkIdentity,
    receipts: &[Receipt],
    grants: &[VerifiedRetryGrant],
    completed: &BTreeMap<PhaseKey, Uuid>,
) -> Result<RetryTarget> {
    work.validate()?;
    let key = IssueKey::new(
        &work.linear_endpoint,
        work.linear_workspace,
        work.child.unwrap_or(work.parent),
    )?
    .identity()?;
    let generation = canonical_digest(1, &work.generation)?;
    let mut phases: BTreeMap<PhaseKey, Vec<(&Receipt, RunReference)>> = BTreeMap::new();
    for receipt in receipts {
        let Some(run) = crate::control::run_reference_from_receipt(receipt)? else {
            continue;
        };
        if run.phase.work != key || run.phase.generation != generation {
            continue;
        }
        if receipt.runtime_id != runtime {
            return Err(Error::new(
                "record_corrupt",
                "retry history belongs to another runtime",
            ));
        }
        crate::runtime_schedule::require_provider_clear(runtime, &run.account, &key, receipts)?;
        phases
            .entry(run.phase.clone())
            .or_default()
            .push((receipt, run));
    }
    let mut candidates = Vec::new();
    for (phase, mut records) in phases {
        records.sort_by_key(|(_, run)| run.attempt);
        let workspace = records[0].1.workspace;
        for (index, (receipt, run)) in records.iter().enumerate() {
            if run.attempt as usize != index + 1 || run.workspace != workspace {
                return Err(Error::new(
                    "record_corrupt",
                    "retry history has missing, duplicate or cross-workspace admissions",
                ));
            }
            let result = receipt.observed_result.as_ref().ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "retry admission has no actual result",
                )
            })?;
            if !matches!(
                receipt.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            ) || result["local_execution"] != "exited"
                || result["unresolved_effects"]
                    .as_array()
                    .is_none_or(|effects| !effects.is_empty())
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "retry requires confirmed local exit and reconciled effects",
                ));
            }
        }
        let superseding = grants
            .iter()
            .filter(|g| g.work() == work && g.phase() == &phase)
            .collect::<Vec<_>>();
        if superseding.len() > 1 {
            return Err(Error::new(
                "ownership_conflict",
                "multiple verified grants replace one phase budget",
            ));
        }
        if !superseding.is_empty() {
            continue;
        }
        let last = records.last().unwrap().0;
        let observed = &last.observed_result.as_ref().unwrap()["observation"];
        match observed["outcome"].as_str() {
            Some("completed") if completed.get(&phase) == Some(&last.operation_id) => (),
            Some("completed") => continue,
            Some("failed" | "blocked" | "interrupted") => (),
            _ => {
                return Err(Error::new(
                    "reconciliation_required",
                    "retry phase has no recognized terminal worker outcome",
                ));
            }
        }
        candidates.push(RetryTarget {
            phase,
            admissions: records
                .iter()
                .map(|(receipt, _)| receipt.operation_id)
                .collect::<BTreeSet<_>>(),
        });
    }
    if candidates.len() != 1 {
        return Err(Error::new(
            if candidates.is_empty() {
                "no_retry_target"
            } else {
                "ambiguous_retry_target"
            },
            "retry requires exactly one unresolved current phase without an existing grant",
        ));
    }
    Ok(candidates.remove(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::Generation;
    use chrono::Utc;
    fn digest(value: &str) -> String {
        canonical_digest(1, &value).unwrap()
    }
    fn work() -> WorkIdentity {
        WorkIdentity {
            linear_endpoint: "https://api.linear.app/graphql".into(),
            linear_workspace: Uuid::from_u128(2),
            parent: Uuid::from_u128(3),
            child: Some(Uuid::from_u128(4)),
            generation: Generation {
                authorization: digest("authorization"),
                definition: digest("definition"),
                correction: digest("correction"),
                scope: digest("scope"),
            },
        }
    }
    fn receipt(phase: &str, attempt: u32, outcome: &str) -> Receipt {
        let work = work();
        let key = IssueKey::new(
            &work.linear_endpoint,
            work.linear_workspace,
            work.child.unwrap(),
        )
        .unwrap()
        .identity()
        .unwrap();
        let id = Uuid::new_v4();
        let run = RunReference {
            workspace: Uuid::from_u128(5),
            account: "account".into(),
            normalized_state: Some("todo".into()),
            phase: PhaseKey {
                work: key.clone(),
                generation: canonical_digest(1, &work.generation).unwrap(),
                phase: phase.into(),
                retry_grant: None,
            },
            profile_digest: digest("profile"),
            instruction_digest: digest("instructions"),
            max_attempts: 3,
            max_turns: 20,
            max_review_rounds: 3,
            attempt,
        };
        Receipt {
            operation_id: id,
            request_id: id,
            runtime_id: Uuid::from_u128(1),
            owner_generation: 1,
            actor: "runtime".into(),
            kind: "admission".into(),
            canonical_targets: vec![key],
            payload_digest: digest("payload"),
            expected: BTreeMap::from([("run".into(), serde_json::to_string(&run).unwrap())]),
            phase: ReceiptPhase::Reconciled,
            remote_ids: BTreeMap::new(),
            observed_result: Some(
                json!({"local_execution":"exited","unresolved_effects":[],"observation":{"outcome":outcome,"session":{"intervention":null,"input_required":false}}}),
            ),
            cancellation: "none".into(),
            updated_at: Utc::now(),
        }
    }
    #[test]
    fn retry_selects_one_unresolved_exact_phase_and_its_complete_history() {
        let first = receipt("implementation", 1, "failed");
        let second = receipt("implementation", 2, "blocked");
        let history = [
            receipt("review", 1, "completed"),
            second.clone(),
            first.clone(),
        ];
        let target = select_retry_target(Uuid::from_u128(1), &work(), &history, &[]).unwrap();
        assert_eq!(target.phase.phase, "implementation");
        assert_eq!(
            target.admissions,
            BTreeSet::from([first.operation_id, second.operation_id])
        );
        let mut other = work();
        other.generation.correction = digest("later-correction");
        assert_eq!(
            select_retry_target(Uuid::from_u128(1), &other, &history, &[])
                .unwrap_err()
                .code,
            "no_retry_target"
        );
    }
    #[test]
    fn retry_never_guesses_between_phases_or_ignores_unknown_effects() {
        let failed = receipt("implementation", 1, "failed");
        let other = receipt("review", 1, "blocked");
        assert_eq!(
            select_retry_target(Uuid::from_u128(1), &work(), &[failed.clone(), other], &[])
                .unwrap_err()
                .code,
            "ambiguous_retry_target"
        );
        let mut unknown = failed.clone();
        unknown.observed_result.as_mut().unwrap()["unresolved_effects"] = json!([Uuid::new_v4()]);
        assert_eq!(
            select_retry_target(Uuid::from_u128(1), &work(), &[unknown], &[])
                .unwrap_err()
                .code,
            "reconciliation_required"
        );
        let mut intervened = failed;
        intervened.observed_result.as_mut().unwrap()["observation"]["session"]["intervention"] =
            json!({"reason_code":"verification_required"});
        assert_eq!(
            select_retry_target(Uuid::from_u128(1), &work(), &[intervened], &[])
                .unwrap_err()
                .code,
            "provider_intervention"
        );
    }
    #[test]
    fn retry_rejects_missing_and_duplicate_admission_ordinals() {
        for history in [
            vec![receipt("implementation", 2, "failed")],
            vec![
                receipt("implementation", 1, "failed"),
                receipt("implementation", 1, "failed"),
            ],
        ] {
            assert_eq!(
                select_retry_target(Uuid::from_u128(1), &work(), &history, &[])
                    .unwrap_err()
                    .code,
                "record_corrupt"
            );
        }
    }
    #[test]
    fn retry_recovery_requires_exact_native_publication_and_real_prior_admissions() {
        use crate::{
            decisions::{RetryBudget, RetryGrant},
            evidence::encode_signed_publication,
        };
        use sha2::{Digest, Sha256};
        let runtime = Uuid::from_u128(1);
        let workspace = Uuid::from_u128(5);
        let writer = Uuid::from_u128(20);
        let key = AttestationKey::new("retry-recovery-fixture".into(), vec![31; 32]).unwrap();
        let prior = receipt("implementation", 1, "failed");
        let command = Uuid::from_u128(10);
        let effect = crate::control_command::command_effect_id(command);
        let bytes = Sha256::digest(format!("kegari:v0:decision-stage:{effect}:retry").as_bytes());
        let mut id = [0; 16];
        id.copy_from_slice(&bytes[..16]);
        id[6] = (id[6] & 0x0f) | 0x80;
        id[8] = (id[8] & 0x3f) | 0x80;
        let operation = Uuid::from_bytes(id);
        let grant = RetryGrant {
            schema_version: 1,
            runtime_id: runtime,
            workspace_id: workspace,
            command_id: command,
            operation_id: operation,
            work: work(),
            previous_phase: crate::control::run_reference_from_receipt(&prior)
                .unwrap()
                .unwrap()
                .phase,
            prior_admissions: BTreeMap::from([(prior.operation_id, prior.payload_digest.clone())]),
            budget: RetryBudget {
                max_attempts: 3,
                max_turns: 20,
                max_review_rounds: 3,
            },
            control_actor: "operator".into(),
            principal: kegari_control_api::identity::Principal::Unix { uid: 501 },
            linear_actor: writer,
            provider_writer: writer.to_string(),
            reason: "Resolved the recorded blocker".into(),
            granted_at: Utc::now(),
        };
        let signed = key.sign("operator_retry", &grant).unwrap();
        let body = encode_signed_publication("operator_retry", &signed).unwrap();
        let comment = kegari_providers::Comment {
            id: Uuid::from_u128(30),
            issue_id: work().child.unwrap(),
            body_digest: kegari_digest::bytes_digest(body.as_bytes()),
            body,
            user_id: Some(writer),
            delegated: false,
            bot: false,
            created_at: Some(Utc::now()),
            updated_at: None,
            edited_at: None,
            url: "https://linear.app/fixture/comment".into(),
        };
        let mut published = prior.clone();
        published.operation_id = operation;
        published.request_id = operation;
        published.kind = "linear_comment".into();
        published.canonical_targets.push(format!(
            "linear:{}:{}:{}",
            work().linear_endpoint,
            work().linear_workspace,
            work().child.unwrap()
        ));
        published.expected = BTreeMap::from([
            ("parent_command_id".into(), command.to_string()),
            ("provider_body_digest".into(), comment.body_digest.clone()),
            ("provider_writer".into(), writer.to_string()),
        ]);
        published.remote_ids = BTreeMap::from([("comment_id".into(), comment.id.to_string())]);
        let receipts = vec![prior, published];
        let restored = recover_retry_comment(
            runtime,
            workspace,
            &writer.to_string(),
            &work(),
            &comment,
            &receipts,
            &key,
        )
        .unwrap();
        assert_eq!(restored.phase_key().retry_grant, Some(operation));
        let current = recover_retry_comments(
            runtime,
            workspace,
            &writer.to_string(),
            &work(),
            std::slice::from_ref(&comment),
            &receipts,
            &key,
        )
        .unwrap();
        require_current_grant(&restored, &current).unwrap();
        let deleted = recover_retry_comments(
            runtime,
            workspace,
            &writer.to_string(),
            &work(),
            &[],
            &receipts,
            &key,
        )
        .unwrap();
        assert_eq!(
            require_current_grant(&restored, &deleted).unwrap_err().code,
            "stale_result"
        );
        assert!(
            recover_retry_comments(
                runtime,
                workspace,
                &writer.to_string(),
                &work(),
                &[comment.clone(), comment.clone()],
                &receipts,
                &key
            )
            .is_err()
        );
        assert!(require_current_grant(&restored, std::slice::from_ref(&restored)).is_ok());
        assert_eq!(
            require_current_grant(&restored, &[]).unwrap_err().code,
            "stale_result"
        );
        for mode in ["writer", "body", "target", "receipt", "history"] {
            let mut changed = comment.clone();
            let mut records = receipts.clone();
            match mode {
                "writer" => changed.user_id = Some(Uuid::from_u128(99)),
                "body" => {
                    changed.body.push(' ');
                    changed.body_digest = kegari_digest::bytes_digest(changed.body.as_bytes());
                }
                "target" => changed.issue_id = Uuid::from_u128(99),
                "receipt" => {
                    records[1].remote_ids.clear();
                }
                "history" => {
                    records.remove(0);
                }
                _ => unreachable!(),
            }
            assert!(
                recover_retry_comment(
                    runtime,
                    workspace,
                    &writer.to_string(),
                    &work(),
                    &changed,
                    &records,
                    &key
                )
                .is_err(),
                "{mode}"
            );
        }
        let mut newer = work();
        newer.generation.correction = digest("newer");
        assert!(
            recover_retry_comment(
                runtime,
                workspace,
                &writer.to_string(),
                &newer,
                &comment,
                &receipts,
                &key
            )
            .is_err()
        );
    }
}
