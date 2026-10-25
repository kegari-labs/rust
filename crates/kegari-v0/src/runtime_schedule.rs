//! Durable worker-attempt planning. This selects a possible future admission;
//! it never grants a permit, consumes a slot or turns a worker exit into delivery.
use crate::{
    config::RuntimeConfig,
    control::{Receipt, ReceiptPhase, RunReference},
    error::{Error, Result},
    scheduler::{PhaseKey, Trigger, retry_at},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

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
    ) {
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

/// Final capacity check uses actual active admission records under the host lock.
/// Missing historical state names count against every applicable state limit.
pub fn ensure_capacity(
    config: &crate::config::Config,
    active: &[RunReference],
    next: &RunReference,
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
    if config.connections[&workspace.codex_connection].account_id != next.account {
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
        .get(&next.account)
        .copied()
        .ok_or_else(|| Error::invalid("selected account has no configured finite capacity"))?
        as usize;
    let state = next
        .normalized_state
        .as_deref()
        .ok_or_else(|| Error::invalid("new session requires its normalized admission state"))?;
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
