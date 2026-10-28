//! Durable worker-attempt planning. This selects a possible future admission;
//! it never grants a permit, consumes a slot or turns a worker exit into delivery.
use crate::{
    control::{Receipt, ReceiptPhase, RunReference},
    scheduler::{PhaseKey, Trigger, retry_at},
};
use chrono::{DateTime, Utc};
use kegari_config::RuntimeConfig;
use kegari_types::error::{Error, Result};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[cfg(test)]
#[path = "runtime_schedule_continuation_tests.rs"]
mod continuation_tests;

/// An admitted run keeps its capacity through restart until the host recorded
/// actual local exit. Losing the in-memory Permit map never frees a session slot.
pub fn occupying_runs(runtime: Uuid, receipts: &[Receipt]) -> Result<Vec<RunReference>> {
    let mut active = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for receipt in receipts {
        let Some(run) = RunReference::from_receipt(receipt)? else {
            continue;
        };
        if receipt.runtime_id != runtime || !seen.insert(receipt.operation_id) {
            return Err(Error::new(
                "record_corrupt",
                "capacity history crosses runtime ownership or duplicates a run",
            ));
        }
        let exited = receipt.phase == ReceiptPhase::Reconciled
            && receipt
                .observed_result
                .as_ref()
                .and_then(|value| value.get("local_execution"))
                .and_then(Value::as_str)
                == Some("exited");
        if !exited {
            active.push(run);
        }
    }
    Ok(active)
}

/// A grant for a completed implementation is meaningful only with the sealed
/// remaining-work capability. It must not fall through to ordinary initial
/// implementation dispatch and lose the retained source/findings contract.
pub(crate) fn grant_requires_unfinished(
    phase: &PhaseKey,
    granted_phase: &PhaseKey,
    receipts: &[Receipt],
) -> Result<bool> {
    if granted_phase.retry_grant.is_none()
        || phase.work != granted_phase.work
        || phase.generation != granted_phase.generation
        || phase.phase != granted_phase.phase
        || phase == granted_phase
    {
        return Err(Error::new(
            "worker_binding",
            "granted retry phase does not replace the original budget",
        ));
    }
    let mut history = Vec::new();
    let mut granted_admitted = false;
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)? {
            if &run.phase == phase {
                history.push((run.attempt, receipt));
            }
            if &run.phase == granted_phase {
                granted_admitted = true;
            }
        }
    }
    history.sort_by_key(|(attempt, _)| *attempt);
    if history.is_empty()
        || history
            .iter()
            .enumerate()
            .any(|(i, (attempt, _))| *attempt as usize != i + 1)
    {
        return Err(Error::new(
            "reconciliation_required",
            "granted phase history is absent or incomplete",
        ));
    }
    let latest = history.last().unwrap().1;
    if !reconciled_exit(latest) {
        return Err(Error::new(
            "reconciliation_required",
            "granted phase has no confirmed local exit",
        ));
    }
    match latest
        .observed_result
        .as_ref()
        .and_then(|r| r.pointer("/observation/outcome"))
        .and_then(Value::as_str)
    {
        Some("completed") => Ok(!granted_admitted),
        Some("failed" | "blocked" | "interrupted") => Ok(false),
        _ => Err(Error::new(
            "reconciliation_required",
            "granted phase outcome is not known",
        )),
    }
}

/// Provider intervention belongs to its observed account/work, not to a phase
/// budget. A correction, retry grant, account alias or later successful receipt
/// cannot clear it. Only a separately verified provider resolution may do so;
/// no such resolution is inferred from receipt text or operator retry.
pub fn require_provider_clear(
    runtime: Uuid,
    account: &str,
    work: &str,
    receipts: &[Receipt],
) -> Result<()> {
    for receipt in receipts {
        let Some(run) = RunReference::from_receipt(receipt)? else {
            continue;
        };
        if receipt.runtime_id != runtime {
            return Err(Error::new(
                "record_corrupt",
                "run belongs to another runtime",
            ));
        }
        let same_work = run.phase.work == work;
        if run.account != account && !same_work {
            continue;
        }
        let Some(result) = &receipt.observed_result else {
            continue;
        };
        // retire_local wraps the actual worker observation. Re-observation of
        // local exit may preserve the earlier observation inside `prior`.
        let mut observed = result;
        for _ in 0..8 {
            if let Some(intervention) = observed.pointer("/session/intervention")
                && !intervention.is_null()
            {
                return Err(Error::new(
                    "provider_intervention",
                    "recorded provider intervention requires verified provider resolution",
                )
                .detail(json!({"operation_id":receipt.operation_id,"account":run.account,"work":run.phase.work,"provider_signal":intervention})));
            }
            if same_work
                && observed
                    .pointer("/session/input_required")
                    .and_then(Value::as_bool)
                    == Some(true)
            {
                return Err(Error::new(
                    "provider_input_required",
                    "prior provider input requirement cannot be bypassed by a replacement session",
                )
                .detail(json!({"operation_id":receipt.operation_id,"work":run.phase.work})));
            }
            match observed
                .get("observation")
                .or_else(|| observed.get("prior"))
            {
                Some(next) => observed = next,
                None => break,
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct RunSchedule {
    pub attempt: u32,
    pub max_attempts: u32,
    pub max_turns: u32,
    pub max_review_rounds: u32,
    pub trigger: Trigger,
    pub not_before: DateTime<Utc>,
}
/// All receipts for a phase are counted, including failed preparation of an
/// admitted model session. A current configuration may lower, never reset or
/// enlarge, that phase's already consumed budget.
pub fn plan(
    runtime: &RuntimeConfig,
    phase: &PhaseKey,
    workspace: Uuid,
    receipts: &[Receipt],
    now: DateTime<Utc>,
) -> Result<RunSchedule> {
    crate::control::reject_unmarked_parent_phase(phase, receipts)?;
    plan_inner(
        runtime,
        phase,
        workspace,
        receipts,
        PlanCause::default(),
        now,
    )
}
/// Only a retained exact-source failed review can use terminal review failure
/// as a retry cause. Ordinary failed runs keep the generic failure taxonomy.
pub(crate) fn plan_review_retry(
    runtime: &RuntimeConfig,
    phase: &PhaseKey,
    workspace: Uuid,
    receipts: &[Receipt],
    review: &crate::delivery::ReviewRetryWork,
    now: DateTime<Utc>,
) -> Result<RunSchedule> {
    plan_review_retry_inner(runtime, phase, workspace, receipts, (review, None), now)
}
fn plan_review_retry_inner(
    runtime: &RuntimeConfig,
    phase: &PhaseKey,
    workspace: Uuid,
    receipts: &[Receipt],
    proofs: (
        &crate::delivery::ReviewRetryWork,
        Option<&crate::delivery::VerifiedReviewRoundPlan>,
    ),
    now: DateTime<Utc>,
) -> Result<RunSchedule> {
    let (review, round) = proofs;
    review.verify_inventory(receipts)?;
    // Exact-phase history below is empty for the first session of a new grant.
    // Its authenticated prior namespaces must still retain their opaque plan.
    if round.is_none() && review.requires_review_round() {
        return Err(Error::new(
            "review_round_unavailable",
            "marked review retry ancestry requires its exact opaque round plan",
        ));
    }
    review.verify_retry_cause()?;
    let failed = receipts
        .iter()
        .find(|r| r.operation_id == review.predecessor())
        .ok_or_else(|| Error::new("reconciliation_required", "failed review disappeared"))?;
    if phase != review.phase()
        || workspace != review.authority().workspace_id()
        || runtime.id != failed.runtime_id
    {
        return Err(Error::new(
            "worker_binding",
            "review schedule changes its verified scope",
        ));
    }
    let mut limits = runtime.clone();
    limits.max_attempts_per_phase = limits
        .max_attempts_per_phase
        .min(review.grant().new_budget().max_attempts);
    limits.max_turns = limits.max_turns.min(review.grant().new_budget().max_turns);
    limits.max_review_rounds = limits
        .max_review_rounds
        .min(review.grant().new_budget().max_review_rounds);
    let mut next = plan_inner(
        &limits,
        phase,
        workspace,
        receipts,
        PlanCause {
            continuation: None,
            review_retry: Some(review),
            validation_retry: None,
            round,
        },
        now,
    )?;
    if next.attempt != review.next_attempt()? {
        return Err(Error::new(
            "stale_result",
            "review retry does not follow its exact predecessor",
        ));
    }
    next.not_before = next.not_before.max(retry_at(
        failed.updated_at,
        next.attempt.saturating_sub(1).max(1),
        runtime.retry_cap_ms,
        failed
            .observed_result
            .as_ref()
            .and_then(|v| v.pointer("/observation/error/details/retry_after_ms"))
            .and_then(Value::as_u64),
    )?);
    if next.not_before > now {
        return Err(Error::new(
            "retry_wait",
            "failed review waits for bounded failure spacing",
        )
        .detail(json!({"not_before":next.not_before,"attempt":next.attempt})));
    }
    Ok(next)
}
/// Exact-source validation retries use a typed cause; this does not broaden
/// generic failed-session policy or allocate review/fix rounds.
pub(crate) fn plan_validation_retry(
    runtime: &RuntimeConfig,
    phase: &PhaseKey,
    workspace: Uuid,
    receipts: &[Receipt],
    validation: &crate::delivery::ValidationRetryWork,
    now: DateTime<Utc>,
) -> Result<RunSchedule> {
    validation.verify_inventory(receipts)?;
    validation.verify_retry_cause()?;
    let failed = receipts
        .iter()
        .find(|r| r.operation_id == validation.predecessor())
        .ok_or_else(|| Error::new("reconciliation_required", "failed validation disappeared"))?;
    if phase != validation.phase()
        || workspace != validation.authority().workspace_id()
        || runtime.id != failed.runtime_id
    {
        return Err(Error::new(
            "worker_binding",
            "validation schedule changes its verified scope",
        ));
    }
    let mut limits = runtime.clone();
    limits.max_attempts_per_phase = limits
        .max_attempts_per_phase
        .min(validation.grant().new_budget().max_attempts);
    limits.max_turns = limits
        .max_turns
        .min(validation.grant().new_budget().max_turns);
    limits.max_review_rounds = limits
        .max_review_rounds
        .min(validation.grant().new_budget().max_review_rounds);
    let mut next = plan_inner(
        &limits,
        phase,
        workspace,
        receipts,
        PlanCause {
            continuation: None,
            review_retry: None,
            validation_retry: Some(validation),
            round: None,
        },
        now,
    )?;
    if next.attempt != validation.next_attempt()? {
        return Err(Error::new(
            "stale_result",
            "validation retry does not follow its exact predecessor",
        ));
    }
    next.not_before = next.not_before.max(retry_at(
        failed.updated_at,
        next.attempt.saturating_sub(1).max(1),
        runtime.retry_cap_ms,
        failed
            .observed_result
            .as_ref()
            .and_then(|v| v.pointer("/observation/error/details/retry_after_ms"))
            .and_then(Value::as_u64),
    )?);
    if next.not_before > now {
        return Err(Error::new(
            "retry_wait",
            "failed validation waits for bounded failure spacing",
        )
        .detail(json!({"not_before":next.not_before,"attempt":next.attempt})));
    }
    Ok(next)
}
/// Planning is diagnostic, not a continuation capability. Only the runtime's
/// opaque unfinished-work proof can reach the continued admission transaction.
pub(crate) fn plan_unfinished(
    runtime: &RuntimeConfig,
    phase: &PhaseKey,
    workspace: Uuid,
    receipts: &[Receipt],
    continuation: &crate::control::RunContinuationReference,
    now: DateTime<Utc>,
) -> Result<RunSchedule> {
    plan_inner(
        runtime,
        phase,
        workspace,
        receipts,
        PlanCause {
            continuation: Some(PlanTrigger::Unfinished(continuation)),
            ..PlanCause::default()
        },
        now,
    )
}
pub(crate) fn plan_source_advanced(
    runtime: &RuntimeConfig,
    phase: &PhaseKey,
    workspace: Uuid,
    receipts: &[Receipt],
    source: &crate::control::SourceAdvancedReference,
    now: DateTime<Utc>,
) -> Result<RunSchedule> {
    if !matches!(
        phase.phase.as_str(),
        "child_local_validation" | "child_codex_review"
    ) {
        return Err(Error::new(
            "worker_binding",
            "source advancement cannot change this worker phase",
        ));
    }
    plan_inner(
        runtime,
        phase,
        workspace,
        receipts,
        PlanCause {
            continuation: Some(PlanTrigger::SourceAdvanced(source)),
            ..PlanCause::default()
        },
        now,
    )
}
/// Plan only from the delivery producer's actual current round capability.
/// Control independently verifies its typed references at atomic admission.
pub(crate) struct RoundScheduleInput<'a> {
    pub runtime: &'a RuntimeConfig,
    pub phase: &'a PhaseKey,
    pub workspace: Uuid,
    pub receipts: &'a [Receipt],
    pub grants: &'a [crate::decisions::VerifiedRetryGrant],
    pub proof: &'a crate::delivery::VerifiedReviewRoundPlan,
    pub cause: &'a crate::control::ReviewRoundRunCause,
    pub review_retry: Option<&'a crate::delivery::ReviewRetryWork>,
    pub now: DateTime<Utc>,
}
pub(crate) fn plan_review_round(input: RoundScheduleInput<'_>) -> Result<RunSchedule> {
    let RoundScheduleInput {
        runtime,
        phase,
        workspace,
        receipts,
        grants,
        proof,
        cause,
        review_retry,
        now,
    } = input;
    use crate::control::{ReviewRoundCause, ReviewRoundRunCause};
    proof.verify_current(receipts, grants)?;
    let reference = proof.reference();
    if proof.subject_phase() != phase
        || reference.workspace != workspace
        || reference.runtime != runtime.id
    {
        return Err(Error::new(
            "worker_binding",
            "round schedule changes exact scope",
        ));
    }
    // This bound belongs to the complete review namespace. Paired implementation
    // retains its own captured attempt/turn limits, including an older review cap.
    if !reference.is_repair()
        && reference.ordinal > runtime.max_review_rounds.min(reference.round_limit)
    {
        return Err(Error::new(
            "review_rounds_exhausted",
            "the current review namespace has no remaining round",
        ));
    }
    let continuation = match (&reference.cause, cause) {
        (
            ReviewRoundCause::Initial {} | ReviewRoundCause::TransientRetry { .. },
            ReviewRoundRunCause::Review,
        ) => None,
        (ReviewRoundCause::PairedRepair { findings }, ReviewRoundRunCause::PairedRepair(link))
            if link.predecessor == reference.source.implementation.operation
                && link.predecessor_receipt_digest
                    == reference.source.implementation.receipt_digest
                && link.failure_operation == findings.review.operation =>
        {
            Some(PlanTrigger::Unfinished(link))
        }
        (
            ReviewRoundCause::RepairContinuation {
                current_source,
                validation,
                ..
            },
            ReviewRoundRunCause::PairedRepair(link),
        ) if link.predecessor == current_source.implementation.operation
            && link.predecessor_receipt_digest == current_source.implementation.receipt_digest
            && link.failure_operation == validation.validation.operation =>
        {
            Some(PlanTrigger::Unfinished(link))
        }
        (
            ReviewRoundCause::AfterRepair {
                findings, repair, ..
            },
            ReviewRoundRunCause::AfterRepair(link),
        ) if link.predecessor == findings.review.operation
            && link.predecessor_receipt_digest == findings.review.receipt_digest
            && link.source_operation == repair.operation
            && link.source_receipt_digest == repair.receipt_digest =>
        {
            Some(PlanTrigger::SourceAdvanced(link))
        }
        _ => {
            return Err(Error::new(
                "worker_binding",
                "round cause differs from actual source",
            ));
        }
    };
    let mut limits = runtime.clone();
    if !reference.is_repair() {
        limits.max_review_rounds = limits.max_review_rounds.min(reference.round_limit);
    }
    if let Some(retry) = review_retry {
        if !matches!(cause, ReviewRoundRunCause::Review) {
            return Err(Error::new(
                "worker_binding",
                "review retry cannot replace a paired cause",
            ));
        }
        return plan_review_retry_inner(
            &limits,
            phase,
            workspace,
            receipts,
            (retry, Some(proof)),
            now,
        );
    }
    plan_inner(
        &limits,
        phase,
        workspace,
        receipts,
        PlanCause {
            continuation,
            review_retry: None,
            validation_retry: None,
            round: Some(proof),
        },
        now,
    )
}

#[derive(Default)]
struct PlanCause<'a> {
    continuation: Option<PlanTrigger<'a>>,
    review_retry: Option<&'a crate::delivery::ReviewRetryWork>,
    validation_retry: Option<&'a crate::delivery::ValidationRetryWork>,
    round: Option<&'a crate::delivery::VerifiedReviewRoundPlan>,
}

#[derive(Clone, Copy)]
enum PlanTrigger<'a> {
    Unfinished(&'a crate::control::RunContinuationReference),
    SourceAdvanced(&'a crate::control::SourceAdvancedReference),
}
impl<'a> PlanTrigger<'a> {
    fn predecessor(self) -> Uuid {
        match self {
            Self::Unfinished(r) => r.predecessor,
            Self::SourceAdvanced(r) => r.predecessor,
        }
    }
    fn predecessor_digest(self) -> &'a str {
        match self {
            Self::Unfinished(r) => &r.predecessor_receipt_digest,
            Self::SourceAdvanced(r) => &r.predecessor_receipt_digest,
        }
    }
    fn cause(self) -> Uuid {
        match self {
            Self::Unfinished(r) => r.failure_operation,
            Self::SourceAdvanced(r) => r.source_operation,
        }
    }
    fn trigger_digest(self) -> &'a str {
        match self {
            Self::Unfinished(r) => &r.trigger_digest,
            Self::SourceAdvanced(r) => &r.trigger_digest,
        }
    }
    fn progress_digest(self) -> &'a str {
        match self {
            Self::Unfinished(r) => &r.progress_digest,
            Self::SourceAdvanced(r) => &r.progress_digest,
        }
    }
    fn review_rounds(self) -> u32 {
        match self {
            Self::Unfinished(r) => r.review_rounds_consumed,
            Self::SourceAdvanced(r) => r.review_rounds_consumed,
        }
    }
}

fn plan_inner(
    runtime: &RuntimeConfig,
    phase: &PhaseKey,
    workspace: Uuid,
    receipts: &[Receipt],
    cause: PlanCause<'_>,
    now: DateTime<Utc>,
) -> Result<RunSchedule> {
    let PlanCause {
        continuation,
        review_retry,
        validation_retry,
        round,
    } = cause;
    crate::evidence::validate_digest(&phase.generation)?;
    if phase.work.is_empty() || phase.phase.is_empty() || workspace.is_nil() {
        return Err(Error::invalid(
            "worker schedule requires canonical phase identity",
        ));
    }
    let mut history = Vec::new();
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)?
            && &run.phase == phase
        {
            if receipt.runtime_id != runtime.id || run.workspace != workspace {
                return Err(Error::new(
                    "record_corrupt",
                    "phase history crosses runtime or workspace ownership",
                ));
            }
            history.push((receipt, run));
        }
    }
    if round.is_none()
        && history.iter().any(|(receipt, _)| {
            receipt.expected.contains_key("review_round")
                || receipt.expected.contains_key("review_round_preparation")
        })
    {
        return Err(Error::new(
            "review_round_unavailable",
            "retained round history requires its exact opaque current plan",
        ));
    }
    history.sort_by_key(|(_, run)| run.attempt);
    for (index, (_, run)) in history.iter().enumerate() {
        if run.attempt as usize != index + 1 {
            return Err(Error::new(
                "record_corrupt",
                "phase attempt history has duplicate or missing ordinals",
            ));
        }
    }
    let mut next = RunSchedule {
        attempt: u32::try_from(history.len())
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| {
                Error::new(
                    "attempts_exhausted",
                    "phase history exceeds finite ordinal space",
                )
            })?,
        max_attempts: runtime.max_attempts_per_phase,
        max_turns: runtime.max_turns,
        max_review_rounds: runtime.max_review_rounds,
        trigger: if phase.retry_grant.is_some() {
            Trigger::OperatorRetry
        } else {
            Trigger::InitialAuthorization
        },
        not_before: now,
    };
    for (_, run) in &history {
        next.max_attempts = next.max_attempts.min(run.max_attempts);
        next.max_turns = next.max_turns.min(run.max_turns);
        next.max_review_rounds = next.max_review_rounds.min(run.max_review_rounds);
    }
    if [next.max_attempts, next.max_turns, next.max_review_rounds].contains(&0)
        || next.attempt > next.max_attempts
    {
        return Err(
            Error::new("attempts_exhausted", "finite phase budget is exhausted")
                .detail(json!({"attempts":history.len(),"bound":next.max_attempts})),
        );
    }
    let Some((last, _)) = history.last() else {
        if continuation.is_some() {
            return Err(Error::new(
                "no_work_trigger",
                "unfinished work has no original phase admission",
            ));
        }
        return Ok(next);
    };
    // Earlier uncertainty cannot be erased by a later unrelated receipt.
    if history.iter().any(|(receipt, _)| {
        !matches!(
            receipt.phase,
            ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
        )
    }) {
        return Err(Error::new(
            "reconciliation_required",
            "an earlier phase admission remains unresolved",
        ));
    }
    let result = last.observed_result.as_ref().ok_or_else(|| {
        Error::new(
            "reconciliation_required",
            "last phase receipt has no actual observation",
        )
    })?;
    if result.get("local_execution").and_then(Value::as_str) != Some("exited")
        || result
            .get("unresolved_effects")
            .and_then(Value::as_array)
            .is_none_or(|effects| !effects.is_empty())
    {
        return Err(Error::new(
            "reconciliation_required",
            "actual local exit and complete effect reconciliation are required",
        ));
    }
    let observed = &result["observation"];
    if observed
        .pointer("/session/intervention")
        .is_some_and(|value| !value.is_null())
        || observed
            .pointer("/session/input_required")
            .and_then(Value::as_bool)
            == Some(true)
    {
        return Err(Error::new(
            "provider_intervention",
            "provider intervention cannot become an automatic retry",
        ));
    }
    if let Some(continuation) = continuation {
        if history.iter().any(|(receipt, _)| !reconciled_exit(receipt)) {
            return Err(Error::new(
                "reconciliation_required",
                "every earlier implementation attempt must have a confirmed exit and reconciled effects",
            ));
        }
        if last.operation_id != continuation.predecessor()
            || kegari_digest::canonical_digest(1, *last)? != continuation.predecessor_digest()
            || observed.get("outcome").and_then(Value::as_str) != Some("completed")
            || observed
                .get("hook_error")
                .is_some_and(|error| !error.is_null())
            || observed
                .pointer("/session/terminal")
                .and_then(Value::as_str)
                != Some("completed")
            || observed
                .get("local_exit_confirmed")
                .and_then(Value::as_bool)
                != Some(true)
        {
            return Err(Error::new(
                "stale_result",
                "unfinished work differs from latest actual completed implementation and confirmed exit",
            ));
        }
        for (receipt, _) in &history {
            let unfinished = crate::control::RunContinuationReference::from_receipt(receipt)?;
            let advanced = crate::control::SourceAdvancedReference::from_receipt(receipt)?;
            for previous in unfinished
                .as_ref()
                .map(PlanTrigger::Unfinished)
                .into_iter()
                .chain(advanced.as_ref().map(PlanTrigger::SourceAdvanced))
            {
                if previous.trigger_digest() == continuation.trigger_digest()
                    || previous.progress_digest() == continuation.progress_digest()
                {
                    return Err(Error::new(
                        "no_progress",
                        "this source and obligation already consumed an admission",
                    ));
                }
                if round.is_none() && previous.review_rounds() > continuation.review_rounds() {
                    return Err(Error::new(
                        "record_corrupt",
                        "admission cannot reset consumed review rounds",
                    ));
                }
            }
        }
        if round.is_none()
            && !(phase.phase == "child_local_validation"
                && matches!(continuation, PlanTrigger::SourceAdvanced(_)))
            && continuation.review_rounds() >= next.max_review_rounds
        {
            return Err(Error::new(
                "review_rounds_exhausted",
                "unfinished repair cannot exceed the original review-round bound",
            ));
        }
        let failure = receipts
            .iter()
            .find(|receipt| receipt.operation_id == continuation.cause())
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "unfinished obligation has no actual failure admission",
                )
            })?;
        let failure_run = RunReference::from_receipt(failure)?.ok_or_else(|| {
            Error::new(
                "no_work_trigger",
                "unfinished obligation lacks a worker run",
            )
        })?;
        if failure.runtime_id != runtime.id
            || failure_run.workspace != workspace
            || failure_run.phase.work != phase.work
            || failure_run.phase.generation != phase.generation
        {
            return Err(Error::new(
                "scope_violation",
                "unfinished obligation belongs to another runtime, workspace or work generation",
            ));
        }
        if !reconciled_exit(failure) {
            return Err(Error::new(
                "reconciliation_required",
                "unfinished failure session must have a confirmed exit and reconciled effects",
            ));
        }
        if let PlanTrigger::SourceAdvanced(reference) = continuation {
            if failure_run.phase.phase != "implementation"
                || kegari_digest::canonical_digest(1, failure)? != reference.source_receipt_digest
                || !completed_session(failure)
            {
                return Err(Error::new(
                    "stale_result",
                    "new source must bind its exact completed implementation",
                ));
            }
            for receipt in receipts {
                if let Some(source_run) = RunReference::from_receipt(receipt)?
                    && source_run.phase == failure_run.phase
                    && (source_run.attempt > failure_run.attempt
                        || source_run.attempt == failure_run.attempt
                            && receipt.operation_id != failure.operation_id)
                {
                    return Err(Error::new(
                        "stale_result",
                        "a later or conflicting implementation source admission exists",
                    ));
                }
            }
        }
        next.trigger = match continuation {
            PlanTrigger::Unfinished(_) => Trigger::VerifiedUnfinished,
            PlanTrigger::SourceAdvanced(_) => Trigger::VerifiedSourceAdvanced,
        };
        next.not_before =
            last.updated_at.max(failure.updated_at) + chrono::Duration::milliseconds(1000);
        if next.not_before > now {
            return Err(Error::new(
                "retry_wait",
                "verified unfinished work awaits one-second continuation spacing",
            )
            .detail(json!({"not_before":next.not_before,"attempt":next.attempt})));
        }
        return Ok(next);
    }
    if observed.get("outcome").and_then(Value::as_str) != Some("failed") {
        return Err(Error::new(
            "no_work_trigger",
            "completed, blocked or interrupted work needs a verified new trigger",
        ));
    }
    let error: Error =
        serde_json::from_value(observed.get("error").cloned().unwrap_or(Value::Null))
            .map_err(|_| Error::new("record_corrupt", "failed run lacks its typed failure"))?;
    if !matches!(
        error.code.as_str(),
        "process_exit" | "provider_transport" | "timeout" | "stream_timeout" | "stall_timeout"
    ) && !((review_retry.is_some()
        || validation_retry.is_some()
        || round.is_some_and(|proof| {
            matches!(
                proof.reference().cause,
                crate::control::ReviewRoundCause::TransientRetry { .. }
            )
        }))
        && error.code == "turn_incomplete")
    {
        return Err(Error::new(
            "persistent_blocker",
            "this failure requires explicit resolution before another phase budget",
        )
        .detail(json!({"cause":error.code})));
    }
    next.trigger = Trigger::ScheduledRetry;
    next.not_before = retry_at(
        last.updated_at,
        next.attempt - 1,
        runtime.retry_cap_ms,
        error.details.get("retry_after_ms").and_then(Value::as_u64),
    )?;
    if next.not_before > now {
        return Err(Error::new("retry_wait", "bounded retry is not due yet")
            .detail(json!({"not_before":next.not_before,"attempt":next.attempt})));
    }
    Ok(next)
}

fn completed_session(receipt: &Receipt) -> bool {
    receipt.observed_result.as_ref().is_some_and(|result| {
        let observed = &result["observation"];
        observed["outcome"].as_str() == Some("completed")
            && observed["local_exit_confirmed"].as_bool() == Some(true)
            && observed["hook_error"].is_null()
            && observed["session"]["terminal"].as_str() == Some("completed")
            && observed["session"]["intervention"].is_null()
            && observed["session"]["input_required"].as_bool() != Some(true)
    })
}

fn reconciled_exit(receipt: &Receipt) -> bool {
    receipt.phase == ReceiptPhase::Reconciled
        && receipt.observed_result.as_ref().is_some_and(|result| {
            result.get("local_execution").and_then(Value::as_str) == Some("exited")
                && result
                    .get("unresolved_effects")
                    .and_then(Value::as_array)
                    .is_some_and(|effects| effects.is_empty())
        })
}

/// Final capacity check uses actual active admission records under the host lock.
/// Missing historical state names count against every applicable state limit.
pub fn ensure_capacity(
    config: &kegari_config::Config,
    active: &[RunReference],
    next: &RunReference,
    quota: Option<&crate::scheduler::Quota>,
) -> Result<()> {
    let state = next
        .normalized_state
        .as_deref()
        .ok_or_else(|| Error::invalid("new session requires its normalized admission state"))?;
    check_capacity(
        config,
        active,
        CapacityRequest {
            workspace: next.workspace,
            account: &next.account,
            normalized_state: state,
        },
        quota,
    )
}

/// Prospective capacity facts do not contain or fabricate a worker admission.
/// The same calculation is repeated against actual receipts at final admission.
pub(crate) struct CapacityRequest<'a> {
    pub workspace: Uuid,
    pub account: &'a str,
    pub normalized_state: &'a str,
}
pub(crate) fn check_capacity(
    config: &kegari_config::Config,
    active: &[RunReference],
    next: CapacityRequest<'_>,
    quota: Option<&crate::scheduler::Quota>,
) -> Result<()> {
    let workspace = config
        .workspaces
        .values()
        .find(|ws| ws.id == next.workspace)
        .ok_or_else(|| {
            Error::new(
                "scope_violation",
                "capacity workspace is no longer configured",
            )
        })?;
    if config
        .connections
        .get(&workspace.codex_connection)
        .is_none_or(|connection| connection.account_id != next.account)
    {
        return Err(Error::new(
            "identity_mismatch",
            "session capacity names another configured account",
        ));
    }
    if matches!(quota, Some(crate::scheduler::Quota::Exhausted { .. })) {
        return Err(Error::new(
            "quota_wait",
            "actual account exhaustion blocks new sessions until refreshed",
        ));
    }
    let account_limit = config
        .account_limits()
        .get(next.account)
        .copied()
        .ok_or_else(|| Error::invalid("selected account has no configured finite capacity"))?
        as usize;
    let state = next.normalized_state;
    if state.is_empty() || state != state.trim().to_lowercase() {
        return Err(Error::invalid(
            "capacity requires a normalized nonempty admission state",
        ));
    }
    if active.len() >= config.runtime.max_agents as usize
        || active
            .iter()
            .filter(|run| run.account == next.account)
            .count()
            >= account_limit
        || active
            .iter()
            .filter(|run| run.workspace == next.workspace)
            .count()
            >= workspace.max_agents as usize
        || workspace
            .max_agents_by_state
            .get(state)
            .is_some_and(|limit| {
                active
                    .iter()
                    .filter(|run| {
                        run.workspace == next.workspace
                            && run
                                .normalized_state
                                .as_deref()
                                .is_none_or(|old| old == state)
                    })
                    .count()
                    >= *limit as usize
            })
    {
        return Err(Error::new(
            "capacity_wait",
            "global, account, workspace or normalized state capacity is full",
        ));
    }
    Ok(())
}

/// A completed parent may be reassembled only through the distinct opaque
/// baseline-change producer and Control reference; this preserves its finite
/// phase history without classifying completed work as a transport retry.
pub(crate) fn plan_parent_reassembly(
    runtime: &RuntimeConfig,
    proof: &crate::control::ParentReassemblyReference,
    receipts: &[Receipt],
    now: DateTime<Utc>,
) -> Result<RunSchedule> {
    proof.validate()?;
    if crate::control::parent_reassembly_inventory(receipts, proof)? != proof.inventory_digest {
        return Err(Error::new(
            "stale_result",
            "parent phase inventory changed before scheduling",
        ));
    }
    let max_attempts = runtime.max_attempts_per_phase.min(proof.max_attempts);
    let max_turns = runtime.max_turns.min(proof.max_turns);
    let max_review_rounds = runtime.max_review_rounds.min(proof.max_review_rounds);
    if proof.attempt > max_attempts || [max_attempts, max_turns, max_review_rounds].contains(&0) {
        return Err(Error::new(
            "attempts_exhausted",
            "parent reassembly exhausted the original finite phase budget",
        ));
    }
    let retired = receipts
        .iter()
        .filter(|r| {
            r.operation_id == proof.predecessor.operation
                || r.operation_id == proof.source.operation
        })
        .collect::<Vec<_>>();
    if retired.len() != 2 {
        return Err(Error::new(
            "reconciliation_required",
            "parent source or predecessor receipt is unavailable",
        ));
    }
    let mut not_before =
        retired.iter().map(|r| r.updated_at).max().unwrap() + chrono::Duration::seconds(1);
    if proof.transient_of.is_some() {
        let failed = retired
            .iter()
            .find(|r| r.operation_id == proof.predecessor.operation)
            .unwrap();
        let retry_after = crate::control::parent_transient_failure(failed)?;
        let mut attempts = std::collections::BTreeMap::new();
        for receipt in receipts {
            if let Some(run) = RunReference::from_receipt(receipt)?
                && run.phase == proof.phase
            {
                attempts.insert(run.attempt, receipt);
            }
        }
        let mut failure_index = 0_u32;
        for receipt in attempts.values().rev() {
            if receipt
                .observed_result
                .as_ref()
                .and_then(|r| r.pointer("/observation/outcome"))
                .and_then(Value::as_str)
                != Some("failed")
            {
                break;
            }
            failure_index = failure_index
                .checked_add(1)
                .ok_or_else(|| Error::new("attempts_exhausted", "parent failure index overflow"))?;
        }
        not_before = not_before.max(retry_at(
            failed.updated_at,
            failure_index,
            runtime.retry_cap_ms,
            retry_after,
        )?);
    }
    if now < not_before {
        return Err(Error::new(
            "continuation_wait",
            "parent reassembly awaits its bounded retirement/retry delay",
        ));
    }
    Ok(RunSchedule {
        attempt: proof.attempt,
        max_attempts,
        max_turns,
        max_review_rounds,
        trigger: if proof.transient_of.is_some() {
            Trigger::ScheduledRetry
        } else {
            Trigger::VerifiedParentReassembly
        },
        not_before,
    })
}
