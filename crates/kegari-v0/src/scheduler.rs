//! Pure scheduling projection. Admission and durable run recording are injected
//! host transactions; polling and profile changes are deliberately not triggers.
use crate::error::{Error, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    InitialAuthorization,
    Correction,
    DependencySatisfied,
    ScheduledRetry,
    OperatorRetry,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    Child,
    Coordination,
    IndependentReview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyWork {
    pub canonical_key: String,
    pub workspace: Uuid,
    pub account: String,
    pub generation: String,
    pub state: String,
    pub priority: Option<i32>,
    pub created_at: Option<DateTime<Utc>>,
    pub kind: WorkKind,
    pub trigger: Option<Trigger>,
    pub not_before: DateTime<Utc>,
    pub gate_blockers: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Limits {
    pub global: usize,
    pub accounts: BTreeMap<String, usize>,
    pub workspaces: BTreeMap<Uuid, usize>,
    pub states: BTreeMap<(Uuid, String), usize>,
}
impl Limits {
    pub fn validate(&self) -> Result<()> {
        if self.global == 0
            || self
                .accounts
                .values()
                .chain(self.workspaces.values())
                .chain(self.states.values())
                .any(|n| *n == 0)
        {
            return Err(Error::invalid("all execution capacities must be positive"));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Allocation {
    pub operation: Uuid,
    pub work: ReadyWork,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Quota {
    Unknown,
    Available {
        observed_at: DateTime<Utc>,
    },
    Exhausted {
        observed_at: DateTime<Utc>,
        reset_at: Option<DateTime<Utc>>,
    },
}

pub struct Scheduler {
    pub limits: Limits,
    pub quotas: BTreeMap<String, Quota>,
    pending: BTreeMap<String, ReadyWork>,
    running: BTreeMap<Uuid, Allocation>,
    last_workspace: Option<Uuid>,
}
impl Scheduler {
    pub fn new(limits: Limits) -> Result<Self> {
        limits.validate()?;
        Ok(Self {
            limits,
            quotas: BTreeMap::new(),
            pending: BTreeMap::new(),
            running: BTreeMap::new(),
            last_workspace: None,
        })
    }
    pub fn enqueue(&mut self, mut work: ReadyWork) -> Result<()> {
        work.state = work.state.trim().to_lowercase();
        if work.canonical_key.is_empty()
            || work.account.is_empty()
            || work.state.is_empty()
            || work.generation.is_empty()
        {
            return Err(Error::invalid(
                "work needs canonical identity, account, state and generation",
            ));
        }
        if self
            .running
            .values()
            .any(|a| a.work.canonical_key == work.canonical_key)
        {
            return Err(Error::new("ownership_conflict", "work already executing"));
        }
        if let Some(previous) = self.pending.get(&work.canonical_key)
            && previous.workspace != work.workspace
        {
            return Err(Error::new(
                "ownership_conflict",
                "canonical work selected by multiple workspaces",
            ));
        }
        // Refresh replaces a pending snapshot, never creates a second claim.
        self.pending.insert(work.canonical_key.clone(), work);
        Ok(())
    }
    fn capacity(&self, work: &ReadyWork) -> bool {
        if self.running.len() >= self.limits.global {
            return false;
        }
        if matches!(
            self.quotas.get(&work.account),
            Some(Quota::Exhausted { .. })
        ) {
            return false;
        }
        let account_used = self
            .running
            .values()
            .filter(|a| a.work.account == work.account)
            .count();
        let workspace_used = self
            .running
            .values()
            .filter(|a| a.work.workspace == work.workspace)
            .count();
        if self
            .limits
            .accounts
            .get(&work.account)
            .is_none_or(|cap| account_used >= *cap)
            || self
                .limits
                .workspaces
                .get(&work.workspace)
                .is_none_or(|cap| workspace_used >= *cap)
        {
            return false;
        }
        if let Some(limit) = self
            .limits
            .states
            .get(&(work.workspace, work.state.clone()))
            && self
                .running
                .values()
                .filter(|a| a.work.workspace == work.workspace && a.work.state == work.state)
                .count()
                >= *limit
        {
            return false;
        }
        true
    }
    /// Host admission callback must revalidate current provider facts, reserve its
    /// permit, and persist the bounded run receipt before returning an operation ID.
    pub fn admit_next(
        &mut self,
        now: DateTime<Utc>,
        mut admit: impl FnMut(&ReadyWork) -> Result<Uuid>,
    ) -> Result<Option<Allocation>> {
        self.limits.validate()?;
        let mut workspaces: Vec<_> = self
            .pending
            .values()
            .map(|w| w.workspace)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if let Some(last) = self.last_workspace {
            let offset = workspaces.partition_point(|id| *id <= last);
            workspaces.rotate_left(offset);
        }
        // Authorized work always gets the first opportunity before optional reviews.
        for independent in [false, true] {
            for workspace in &workspaces {
                let mut candidates: Vec<_> = self
                    .pending
                    .values()
                    .filter(|w| {
                        w.workspace == *workspace
                            && (w.kind == WorkKind::IndependentReview) == independent
                            && w.trigger.is_some()
                            && w.not_before <= now
                            && w.gate_blockers.is_empty()
                            && self.capacity(w)
                    })
                    .cloned()
                    .collect();
                candidates.sort_by(|a, b| {
                    let priority = |p: Option<i32>| p.filter(|n| *n > 0).unwrap_or(i32::MAX);
                    (
                        priority(a.priority),
                        a.created_at.is_none(),
                        a.created_at,
                        &a.canonical_key,
                    )
                        .cmp(&(
                            priority(b.priority),
                            b.created_at.is_none(),
                            b.created_at,
                            &b.canonical_key,
                        ))
                });
                for work in candidates {
                    match admit(&work) {
                        Ok(operation) => {
                            if self.running.contains_key(&operation) {
                                return Err(Error::new(
                                    "ownership_conflict",
                                    "host returned a duplicate active operation",
                                ));
                            }
                            let allocation = Allocation {
                                operation,
                                work: work.clone(),
                                started_at: now,
                            };
                            self.pending.remove(&work.canonical_key);
                            self.running.insert(operation, allocation.clone());
                            self.last_workspace = Some(*workspace);
                            return Ok(Some(allocation));
                        }
                        Err(error) => {
                            // One workspace's failed preflight must not starve independent work.
                            if let Some(pending) = self.pending.get_mut(&work.canonical_key) {
                                pending.gate_blockers.push(error.code);
                            }
                        }
                    }
                }
            }
        }
        Ok(None)
    }
    pub fn finish(&mut self, operation: Uuid) -> Result<Allocation> {
        self.running
            .remove(&operation)
            .ok_or_else(|| Error::new("ownership_conflict", "unknown active allocation"))
    }
    pub fn running(&self) -> Vec<Allocation> {
        self.running.values().cloned().collect()
    }
    pub fn queued(&self) -> Vec<ReadyWork> {
        self.pending.values().cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PhaseKey {
    pub work: String,
    pub generation: String,
    pub phase: String,
    pub retry_grant: Option<Uuid>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptRecord {
    pub operation_id: Uuid,
    pub key: PhaseKey,
    pub ordinal: u32,
    pub turns: u32,
    pub review_round: u32,
    pub progress_digest: Option<String>,
    pub outcome: String,
}
pub struct Attempts {
    records: BTreeMap<Uuid, AttemptRecord>,
    pub max_attempts: u32,
    pub max_turns: u32,
    pub max_review_rounds: u32,
}
impl Attempts {
    pub fn restore(
        records: Vec<AttemptRecord>,
        max_attempts: u32,
        max_turns: u32,
        max_review_rounds: u32,
    ) -> Result<Self> {
        if [max_attempts, max_turns, max_review_rounds].contains(&0) {
            return Err(Error::invalid(
                "attempt/turn/review bounds must be positive",
            ));
        }
        let mut output = Self {
            records: BTreeMap::new(),
            max_attempts,
            max_turns,
            max_review_rounds,
        };
        let mut ordinals = BTreeSet::new();
        for record in records {
            if record.ordinal == 0
                || record.turns > max_turns
                || record.review_round > max_review_rounds
                || record.ordinal > max_attempts
                || !ordinals.insert((record.key.clone(), record.ordinal))
                || output.records.contains_key(&record.operation_id)
            {
                return Err(Error::new(
                    "record_corrupt",
                    "run receipts have conflicting or out-of-bound budget accounting",
                ));
            }
            output.records.insert(record.operation_id, record);
        }
        let keys: BTreeSet<_> = output
            .records
            .values()
            .map(|record| record.key.clone())
            .collect();
        for key in keys {
            let observed: BTreeSet<_> = output
                .records
                .values()
                .filter(|record| record.key == key)
                .map(|record| record.ordinal)
                .collect();
            if observed.iter().copied().ne(1..=observed.len() as u32) {
                return Err(Error::new(
                    "record_corrupt",
                    "attempt history is incomplete; missing attempts cannot restore budget",
                ));
            }
        }
        Ok(output)
    }
    pub fn next(&self, key: &PhaseKey, operation_id: Uuid) -> Result<AttemptRecord> {
        if self.records.contains_key(&operation_id) {
            return Err(Error::new(
                "request_conflict",
                "attempt operation ID already exists",
            ));
        }
        let previous: Vec<_> = self.records.values().filter(|r| &r.key == key).collect();
        let ordinal = previous.iter().map(|r| r.ordinal).max().unwrap_or(0) + 1;
        if ordinal > self.max_attempts {
            return Err(Error::new(
                "attempts_exhausted",
                "bounded phase attempts exhausted, including initial session",
            ));
        }
        if previous.iter().any(|r| {
            matches!(
                r.outcome.as_str(),
                "running" | "uncertain" | "provider_intervention"
            )
        }) {
            return Err(Error::new(
                "reconciliation_required",
                "prior run must be reconciled before another attempt",
            ));
        }
        Ok(AttemptRecord {
            operation_id,
            key: key.clone(),
            ordinal,
            turns: 0,
            review_round: 0,
            progress_digest: None,
            outcome: "running".into(),
        })
    }
    pub fn record(&mut self, record: AttemptRecord) -> Result<()> {
        if record.turns > self.max_turns
            || record.review_round > self.max_review_rounds
            || record.ordinal > self.max_attempts
        {
            return Err(Error::new(
                "attempts_exhausted",
                "turn/review/attempt limit exceeded",
            ));
        }
        if record.outcome == "unfinished-with-progress"
            && (record.progress_digest.as_ref().is_none_or(|d| d.is_empty())
                || self.records.values().any(|r| {
                    r.key == record.key
                        && r.operation_id != record.operation_id
                        && r.progress_digest == record.progress_digest
                }))
        {
            return Err(Error::new(
                "no_progress",
                "continuation requires new verified material progress",
            ));
        }
        if let Some(old) = self.records.get(&record.operation_id)
            && (old.key != record.key || old.ordinal != record.ordinal || record.turns < old.turns)
        {
            return Err(Error::new(
                "record_corrupt",
                "attempt identity or counters changed",
            ));
        }
        self.records.insert(record.operation_id, record);
        Ok(())
    }
    pub fn records(&self) -> Vec<AttemptRecord> {
        self.records.values().cloned().collect()
    }
}

pub fn retry_at(
    now: DateTime<Utc>,
    failure_index: u32,
    cap_ms: u64,
    retry_after_ms: Option<u64>,
) -> Result<DateTime<Utc>> {
    if failure_index == 0 || cap_ms == 0 {
        return Err(Error::invalid("retry index/cap must be positive"));
    }
    let exponential =
        10_000_u64.saturating_mul(1_u64.checked_shl(failure_index - 1).unwrap_or(u64::MAX));
    let delay = exponential.min(cap_ms).max(retry_after_ms.unwrap_or(0));
    now.checked_add_signed(Duration::milliseconds(
        i64::try_from(delay).map_err(|_| Error::invalid("retry delay overflow"))?,
    ))
    .ok_or_else(|| Error::invalid("retry instant overflow"))
}
