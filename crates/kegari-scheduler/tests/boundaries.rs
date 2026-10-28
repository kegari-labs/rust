use chrono::{DateTime, Duration, Utc};
use kegari_scheduler::{
    AttemptRecord, Attempts, Limits, Quota, ReadyWork, Scheduler, Trigger, WorkKind, retry_at,
};
use kegari_types::{error::Error, run::PhaseKey};
use serde_json::to_value;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}
fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}
fn limits() -> Limits {
    Limits {
        global: 8,
        accounts: BTreeMap::from([("shared".into(), 8)]),
        workspaces: BTreeMap::from([(id(1), 8), (id(2), 8)]),
        states: BTreeMap::new(),
    }
}
fn work(workspace: u128, key: &str) -> ReadyWork {
    ReadyWork {
        canonical_key: key.into(),
        workspace: id(workspace),
        account: "shared".into(),
        generation: "generation".into(),
        state: " Ready ".into(),
        priority: Some(1),
        created_at: Some(now()),
        kind: WorkKind::Child,
        trigger: Some(Trigger::InitialAuthorization),
        not_before: now(),
        gate_blockers: vec![],
    }
}
fn record(operation: u128, ordinal: u32) -> AttemptRecord {
    AttemptRecord {
        operation_id: id(operation),
        key: PhaseKey {
            work: "child".into(),
            generation: "generation".into(),
            phase: "implementation".into(),
            retry_grant: None,
        },
        ordinal,
        turns: 1,
        review_round: 0,
        progress_digest: None,
        outcome: "failed-retryable".into(),
    }
}

#[test]
fn priority_dates_and_workspace_rotation_precede_optional_reviews() {
    let mut scheduler = Scheduler::new(limits()).unwrap();
    let mut older = work(1, "dated-z");
    older.created_at = Some(now() - Duration::seconds(1));
    let mut undated = work(1, "undated");
    undated.created_at = None;
    let mut unprioritized = work(1, "unprioritized");
    unprioritized.priority = Some(0);
    let mut other = work(2, "other");
    other.priority = Some(5);
    let mut review = work(2, "review");
    review.kind = WorkKind::IndependentReview;
    for item in [
        unprioritized,
        undated,
        work(1, "dated-a"),
        older,
        review,
        other,
    ] {
        scheduler.enqueue(item).unwrap();
    }
    let mut observed = Vec::new();
    for operation in 10..16 {
        let allocation = scheduler
            .admit_next(now(), |_| Ok(id(operation)))
            .unwrap()
            .unwrap();
        observed.push(allocation.work.canonical_key);
        scheduler.finish(allocation.operation).unwrap();
    }
    assert_eq!(
        observed,
        [
            "dated-z",
            "other",
            "dated-a",
            "undated",
            "unprioritized",
            "review"
        ]
    );
    assert!(scheduler.queued().is_empty());
    assert!(scheduler.running().is_empty());
}

#[test]
fn capacity_and_quota_refusals_do_not_call_admission() {
    for dimension in ["global", "account", "workspace", "state"] {
        let mut configured = limits();
        match dimension {
            "global" => configured.global = 1,
            "account" => *configured.accounts.get_mut("shared").unwrap() = 1,
            "workspace" => *configured.workspaces.get_mut(&id(1)).unwrap() = 1,
            "state" => {
                configured.states.insert((id(1), "ready".into()), 1);
            }
            _ => unreachable!(),
        }
        let mut scheduler = Scheduler::new(configured).unwrap();
        scheduler.enqueue(work(1, "active")).unwrap();
        scheduler
            .admit_next(now(), |_| Ok(id(10)))
            .unwrap()
            .unwrap();
        scheduler.enqueue(work(1, "pending")).unwrap();
        assert!(
            scheduler
                .admit_next(now(), |_| panic!("{dimension} capacity exceeded"))
                .unwrap()
                .is_none()
        );
        assert_eq!(scheduler.running().len(), 1);
        assert_eq!(scheduler.queued().len(), 1);
    }
    for quota in [
        None,
        Some(Quota::Unknown),
        Some(Quota::Available { observed_at: now() }),
    ] {
        let mut scheduler = Scheduler::new(limits()).unwrap();
        if let Some(quota) = quota {
            scheduler.quotas.insert("shared".into(), quota);
        }
        scheduler.enqueue(work(1, "eligible")).unwrap();
        assert!(
            scheduler
                .admit_next(now(), |_| Ok(id(10)))
                .unwrap()
                .is_some()
        );
    }
    for reason in ["account", "workspace", "exhausted"] {
        let mut scheduler = Scheduler::new(limits()).unwrap();
        match reason {
            "account" => {
                scheduler.limits.accounts.clear();
            }
            "workspace" => {
                scheduler.limits.workspaces.clear();
            }
            "exhausted" => {
                scheduler.quotas.insert(
                    "shared".into(),
                    Quota::Exhausted {
                        observed_at: now() - Duration::hours(1),
                        reset_at: Some(now() - Duration::minutes(1)),
                    },
                );
            }
            _ => unreachable!(),
        }
        scheduler.enqueue(work(1, "blocked")).unwrap();
        assert!(
            scheduler
                .admit_next(now(), |_| panic!("{reason} must block"))
                .unwrap()
                .is_none()
        );
        assert!(scheduler.running().is_empty());
    }
}

#[test]
fn eligibility_and_pending_refresh_preserve_running_reservations() {
    let mut scheduler = Scheduler::new(limits()).unwrap();
    scheduler.enqueue(work(1, "active")).unwrap();
    scheduler
        .admit_next(now(), |_| Ok(id(10)))
        .unwrap()
        .unwrap();
    let running = to_value(scheduler.running()).unwrap();
    let mut no_trigger = work(1, "no-trigger");
    no_trigger.trigger = None;
    let mut future = work(1, "future");
    future.not_before += Duration::seconds(1);
    let mut blocked = work(1, "blocked");
    blocked.gate_blockers.push("awaiting_review".into());
    for item in [no_trigger, future, blocked] {
        scheduler.enqueue(item).unwrap();
    }
    assert!(
        scheduler
            .admit_next(now(), |_| panic!("ineligible work reached admission"))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        scheduler.enqueue(work(2, "blocked")).unwrap_err().code,
        "ownership_conflict"
    );
    let mut refreshed = work(1, "blocked");
    refreshed.generation = "new-generation".into();
    scheduler.enqueue(refreshed).unwrap();
    scheduler.retain_pending(&BTreeSet::from(["blocked".into()]));
    assert_eq!(to_value(scheduler.running()).unwrap(), running);
    assert_eq!(scheduler.queued().len(), 1);
    let admitted = scheduler
        .admit_next(now(), |_| Ok(id(11)))
        .unwrap()
        .unwrap();
    assert_eq!(admitted.work.generation, "new-generation");
    assert_eq!(admitted.work.state, "ready");
    assert_eq!(scheduler.running().len(), 2);
}

#[test]
fn rejected_admission_retains_blocker_and_allows_other_work() {
    let mut scheduler = Scheduler::new(limits()).unwrap();
    scheduler.enqueue(work(1, "denied")).unwrap();
    scheduler.enqueue(work(2, "allowed")).unwrap();
    let mut called = Vec::new();
    let allocation = scheduler
        .admit_next(now(), |work| {
            called.push(work.canonical_key.clone());
            if work.canonical_key == "denied" {
                Err(Error::new("scope_revoked", "fixture"))
            } else {
                Ok(id(10))
            }
        })
        .unwrap()
        .unwrap();
    assert_eq!(called, ["denied", "allowed"]);
    assert_eq!(allocation.work.canonical_key, "allowed");
    assert_eq!(scheduler.running().len(), 1);
    let queued = scheduler.queued();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].canonical_key, "denied");
    assert_eq!(queued[0].gate_blockers, ["scope_revoked"]);
    scheduler.finish(id(10)).unwrap();
    assert!(
        scheduler
            .admit_next(now(), |_| panic!("failed work was retried"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn duplicate_active_operation_preserves_pending_capacity_and_rotation() {
    let mut scheduler = Scheduler::new(limits()).unwrap();
    scheduler.enqueue(work(1, "active")).unwrap();
    scheduler
        .admit_next(now(), |_| Ok(id(10)))
        .unwrap()
        .unwrap();
    scheduler.enqueue(work(1, "first-workspace")).unwrap();
    scheduler.enqueue(work(2, "second-workspace")).unwrap();
    let pending = to_value(scheduler.queued()).unwrap();
    let running = to_value(scheduler.running()).unwrap();
    assert_eq!(
        scheduler
            .admit_next(now(), |work| {
                assert_eq!(work.workspace, id(2));
                Ok(id(10))
            })
            .unwrap_err()
            .code,
        "ownership_conflict"
    );
    assert_eq!(to_value(scheduler.queued()).unwrap(), pending);
    assert_eq!(to_value(scheduler.running()).unwrap(), running);
    let next = scheduler
        .admit_next(now(), |_| Ok(id(11)))
        .unwrap()
        .unwrap();
    assert_eq!(next.work.workspace, id(2));
    assert_eq!(scheduler.running().len(), 2);
}

#[test]
fn attempt_restore_rejects_conflicting_identity_and_exceeded_bounds() {
    let first = record(10, 1);
    let valid = Attempts::restore(vec![first.clone(), record(11, 2)], 3, 2, 1).unwrap();
    assert_eq!(valid.next(&first.key, id(12)).unwrap().ordinal, 3);
    for records in [
        vec![first.clone(), record(10, 2)],
        vec![first.clone(), record(11, 1)],
        vec![record(10, 0)],
        vec![AttemptRecord {
            turns: 3,
            ..first.clone()
        }],
        vec![AttemptRecord {
            review_round: 2,
            ..first.clone()
        }],
        vec![AttemptRecord {
            ordinal: 4,
            ..first.clone()
        }],
    ] {
        assert_eq!(
            Attempts::restore(records, 3, 2, 1).err().unwrap().code,
            "record_corrupt"
        );
    }
    assert_eq!(
        Attempts::restore(vec![], 0, 2, 1).err().unwrap(),
        Error::invalid("attempt/turn/review bounds must be positive")
    );
}

#[test]
fn attempts_preserve_reconciliation_progress_and_counter_guards() {
    for outcome in ["running", "uncertain", "provider_intervention"] {
        let prior = AttemptRecord {
            outcome: outcome.into(),
            ..record(10, 1)
        };
        let restored = Attempts::restore(vec![prior.clone()], 3, 2, 1).unwrap();
        assert_eq!(
            restored.next(&prior.key, id(11)).unwrap_err().code,
            "reconciliation_required"
        );
        assert_eq!(
            restored.next(&prior.key, id(10)).unwrap_err().code,
            "request_conflict"
        );
    }
    let mut attempts = Attempts::restore(vec![], 3, 2, 1).unwrap();
    let mut first = record(10, 1);
    first.outcome = "unfinished-with-progress".into();
    assert_eq!(
        attempts.record(first.clone()).unwrap_err().code,
        "no_progress"
    );
    first.progress_digest = Some("progress-one".into());
    attempts.record(first.clone()).unwrap();
    let mut second = AttemptRecord {
        operation_id: id(11),
        ordinal: 2,
        ..first.clone()
    };
    assert_eq!(
        attempts.record(second.clone()).unwrap_err().code,
        "no_progress"
    );
    second.progress_digest = Some("progress-two".into());
    attempts.record(second.clone()).unwrap();
    let before = to_value(attempts.records()).unwrap();
    first.turns = 0;
    assert_eq!(attempts.record(first).unwrap_err().code, "record_corrupt");
    second.turns = 3;
    assert_eq!(
        attempts.record(second).unwrap_err().code,
        "attempts_exhausted"
    );
    assert_eq!(to_value(attempts.records()).unwrap(), before);
}

#[test]
fn retry_backoff_preserves_cap_provider_precedence_and_overflow_refusal() {
    for (index, cap, provider, expected_ms) in [
        (1, 300_000, None, 10_000),
        (3, 300_000, Some(90_000), 90_000),
        (u32::MAX, 300_000, None, 300_000),
        (1, 300_000, Some(450_000), 450_000),
        (1, 1, None, 1),
    ] {
        assert_eq!(
            (retry_at(now(), index, cap, provider).unwrap() - now()).num_milliseconds(),
            expected_ms
        );
    }
    for (index, cap) in [(0, 1), (1, 0)] {
        assert_eq!(
            retry_at(now(), index, cap, None).unwrap_err(),
            Error::invalid("retry index/cap must be positive")
        );
    }
    assert_eq!(
        retry_at(now(), 1, u64::MAX, Some(u64::MAX)).unwrap_err(),
        Error::invalid("retry delay overflow")
    );
    assert_eq!(
        retry_at(DateTime::<Utc>::MAX_UTC, 1, 10_000, None).unwrap_err(),
        Error::invalid("retry instant overflow")
    );
}
