use chrono::Utc;
use kegari_digest::bytes_digest;
use kegari_scheduler::*;
use kegari_types::run::PhaseKey;
use std::collections::BTreeMap;
use uuid::Uuid;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn work(ws: Uuid, key: &str, account: &str) -> ReadyWork {
    ReadyWork {
        canonical_key: key.into(),
        workspace: ws,
        account: account.into(),
        generation: "g1".into(),
        state: "Ready".into(),
        priority: Some(1),
        created_at: Some(Utc::now()),
        kind: WorkKind::Child,
        trigger: Some(Trigger::InitialAuthorization),
        not_before: Utc::now(),
        gate_blockers: vec![],
    }
}
#[test]
fn scheduler_fairness_shared_account_quota_and_nontriggers() {
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let mut scheduler = Scheduler::new(Limits {
        global: 3,
        accounts: BTreeMap::from([("shared".into(), 1), ("other".into(), 1)]),
        workspaces: BTreeMap::from([(a, 2), (b, 2)]),
        states: BTreeMap::new(),
    })
    .unwrap();
    scheduler.enqueue(work(a, "a1", "shared")).unwrap();
    scheduler.enqueue(work(b, "b1", "shared")).unwrap();
    let first = scheduler
        .admit_next(Utc::now(), |_| Ok(Uuid::new_v4()))
        .unwrap()
        .unwrap();
    assert!(
        scheduler
            .admit_next(Utc::now(), |_| Ok(Uuid::new_v4()))
            .unwrap()
            .is_none()
    );
    scheduler.finish(first.operation).unwrap();
    let next = scheduler
        .admit_next(Utc::now(), |_| Ok(Uuid::new_v4()))
        .unwrap()
        .unwrap();
    assert_ne!(first.work.workspace, next.work.workspace);
    scheduler.quotas.insert(
        "shared".into(),
        Quota::Exhausted {
            observed_at: Utc::now(),
            reset_at: None,
        },
    );
    scheduler.enqueue(work(a, "a2", "shared")).unwrap();
    scheduler.enqueue(work(b, "b2", "other")).unwrap();
    assert_eq!(
        scheduler
            .admit_next(Utc::now(), |_| Ok(Uuid::new_v4()))
            .unwrap()
            .unwrap()
            .work
            .canonical_key,
        "b2"
    );
    let mut no_trigger = work(a, "no-trigger", "shared");
    no_trigger.trigger = None;
    scheduler.enqueue(no_trigger).unwrap();
    assert!(scheduler.queued().iter().any(|w| w.trigger.is_none()));
}

#[test]
fn budget_includes_initial_and_survives_reconstruction() {
    let key = PhaseKey {
        work: "child".into(),
        generation: "g1".into(),
        phase: "implementation".into(),
        retry_grant: None,
    };
    let mut attempts = Attempts::restore(vec![], 3, 20, 3).unwrap();
    for i in 1..=3 {
        let mut record = attempts.next(&key, Uuid::new_v4()).unwrap();
        assert_eq!(record.ordinal, i);
        record.outcome = "failed-retryable".into();
        attempts.record(record).unwrap();
    }
    let restored = Attempts::restore(attempts.records(), 3, 20, 3).unwrap();
    assert_eq!(
        restored.next(&key, Uuid::new_v4()).unwrap_err().code,
        "attempts_exhausted"
    );
    let now = Utc::now();
    assert_eq!(
        (retry_at(now, 1, 300_000, None).unwrap() - now).num_milliseconds(),
        10_000
    );
    assert_eq!(
        (retry_at(now, 3, 300_000, Some(90_000)).unwrap() - now).num_milliseconds(),
        90_000
    );
}

#[test]
fn incomplete_attempt_history_cannot_restore_a_fresh_budget() {
    let key = PhaseKey {
        work: "child".into(),
        generation: "g1".into(),
        phase: "implementation".into(),
        retry_grant: None,
    };
    let record = AttemptRecord {
        operation_id: Uuid::new_v4(),
        key,
        ordinal: 2,
        turns: 1,
        review_round: 0,
        progress_digest: None,
        outcome: "failed-retryable".into(),
    };
    assert!(Attempts::restore(vec![record], 3, 20, 3).is_err());
}

#[test]
fn scheduler_reload_keeps_running_reservation_and_fair_workspace_rotation() {
    use kegari_scheduler::{ReadyWork, Trigger, WorkKind};
    let limits = Limits {
        global: 2,
        accounts: BTreeMap::from([("account".into(), 2)]),
        workspaces: BTreeMap::from([(id(1), 2), (id(2), 2)]),
        states: BTreeMap::new(),
    };
    let mut scheduler = Scheduler::new(limits.clone()).unwrap();
    for (work, workspace) in [("a", id(1)), ("b", id(1)), ("c", id(2))] {
        scheduler
            .enqueue(ReadyWork {
                canonical_key: work.into(),
                workspace,
                account: "account".into(),
                generation: bytes_digest(work.as_bytes()),
                state: "ready".into(),
                priority: None,
                created_at: None,
                kind: WorkKind::Child,
                trigger: Some(Trigger::InitialAuthorization),
                not_before: Utc::now(),
                gate_blockers: vec![],
            })
            .unwrap();
    }
    let first = scheduler
        .admit_next(Utc::now(), |_| Ok(id(10)))
        .unwrap()
        .unwrap();
    assert_eq!(first.work.workspace, id(1));
    let mut reduced = limits.clone();
    reduced.global = 1;
    scheduler.replace_limits(reduced).unwrap();
    assert_eq!(scheduler.running().len(), 1);
    assert!(
        scheduler
            .admit_next(Utc::now(), |_| panic!(
                "reduced cap must preserve active reservation"
            ))
            .unwrap()
            .is_none()
    );
    scheduler.finish(id(10)).unwrap();
    scheduler.replace_limits(limits).unwrap();
    assert_eq!(
        scheduler
            .admit_next(Utc::now(), |_| Ok(id(11)))
            .unwrap()
            .unwrap()
            .work
            .workspace,
        id(2)
    );
}
