use chrono::Utc;
use kegari_v0::{authority::*, scheduler::*};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[test]
fn latest_label_event_and_complete_history_required() {
    let label = Uuid::new_v4();
    let author = Uuid::new_v4();
    let now = Utc::now();
    let event = LabelTransition {
        event_id: Uuid::new_v4(),
        occurred_at: now,
        actor: Some(author),
        added: BTreeSet::from([label]),
        removed: BTreeSet::new(),
        unambiguous: true,
    };
    let labels = BTreeSet::from([label]);
    let allowed = BTreeSet::from([author]);
    assert!(
        latest_authorization(label, &labels, std::slice::from_ref(&event), true, &allowed).is_ok()
    );
    assert!(
        latest_authorization(
            label,
            &labels,
            std::slice::from_ref(&event),
            false,
            &allowed
        )
        .is_err()
    );
    let mut unauthorized = event.clone();
    unauthorized.actor = None;
    unauthorized.occurred_at += chrono::Duration::seconds(1);
    assert!(latest_authorization(label, &labels, &[event, unauthorized], true, &allowed).is_err());
}

#[test]
fn missing_reparented_child_stays_required_and_zero_never_passes() {
    let e2e = Uuid::new_v4();
    let missing = Uuid::new_v4();
    let parent = ParentMetadata {
        schema_version: 1,
        kind: "parent".into(),
        required_children: vec![e2e],
        e2e_child: e2e,
        visual: false,
        production_targets: vec![],
    };
    assert_eq!(
        required_children(
            &parent,
            &BTreeSet::from([e2e]),
            &BTreeSet::from([missing]),
            &BTreeSet::new()
        )
        .unwrap(),
        BTreeSet::from([e2e, missing])
    );
    let mut zero = parent;
    zero.required_children.clear();
    assert!(
        required_children(&zero, &BTreeSet::new(), &BTreeSet::new(), &BTreeSet::new()).is_err()
    );
}

#[test]
fn strict_scope_metadata_rejects_duplicate_keys_and_path_confusion() {
    let text = "```kegari\n{\"schema_version\":1,\"schema_version\":1}\n```";
    assert!(metadata::<ChildScope>(text).is_err());
    for path in [
        "../secret",
        "/tmp/file",
        "src/../file",
        ".git/config",
        "src/*.rs",
        "src//x",
    ] {
        assert!(validate_relative_boundary(path).is_err(), "{path}");
    }
    let scope = ChildScope {
        schema_version: 1,
        kind: "child".into(),
        responsibility: Responsibility::Implementation,
        write_repository: Some("repo".into()),
        write_paths: vec!["src/".into()],
        read_repositories: vec![],
        test_resources: vec![],
    };
    assert!(scope.permits_path("src/lib.rs"));
    assert!(!scope.permits_path("src-elsewhere/lib.rs"));
    assert!(!scope.permits_path("src/../secret"));
}

#[test]
fn informational_corrections_and_completion_steer_races() {
    let actor = Uuid::new_v4();
    let id = Uuid::new_v4();
    let correctors = BTreeSet::from([actor]);
    let mut book = Corrections::default();
    let revision = CommentRevision {
        id,
        author: Some(actor),
        body_digest: "info".into(),
        edited_at: None,
        observed_revision: 1,
    };
    let original = book.generation().unwrap();
    book.observe(revision.clone(), &correctors, &BTreeSet::new(), None)
        .unwrap();
    assert!(!book.gates_current(0));
    book.classify(id, "info", CommentClass::Informational)
        .unwrap();
    assert!(book.gates_current(0));
    assert_eq!(book.generation().unwrap(), original);
    let mut edited = revision;
    edited.body_digest = "fix".into();
    edited.observed_revision = 2;
    book.observe(edited, &correctors, &BTreeSet::new(), None)
        .unwrap();
    assert!(
        book.classify(id, "info", CommentClass::Informational)
            .is_err()
    );
    book.classify(id, "fix", CommentClass::Corrective).unwrap();
    assert!(!book.gates_current(0));
    assert!(book.gates_current(1));
    book.classify(id, "fix", CommentClass::Ambiguous).unwrap();
    assert!(!book.gates_current(1));
    let mut delivery = CorrectionDelivery {
        current_turn: Some("old".into()),
        ..Default::default()
    };
    delivery.observed(1);
    delivery.steered(SteeringResult::NoActiveTurn);
    assert!(delivery.queued);
    assert!(delivery.completed("old", 0).is_err());
    assert!(delivery.queued);
    delivery.current_turn = Some("new".into());
    assert!(delivery.completed("old", 1).is_err());
    assert_eq!(delivery.current_turn.as_deref(), Some("new"));
    delivery.completed("new", 1).unwrap();
    assert!(!delivery.queued);
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
fn correction_definition_change_is_distinct_from_scope_expansion() {
    let child = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let now = Utc::now();
    let label = Uuid::new_v4();
    let previous = ScopeSnapshot {
        child_scopes: BTreeMap::from([(child, "scope-1".into())]),
        child_definitions: BTreeMap::from([(child, "definition-1".into())]),
        parent_definition: "parent-1".into(),
        production_targets: BTreeSet::from(["target".into()]),
    };
    let event = LabelTransition {
        event_id: Uuid::new_v4(),
        occurred_at: now - chrono::Duration::seconds(1),
        actor: Some(actor),
        added: BTreeSet::from([label]),
        removed: BTreeSet::new(),
        unambiguous: true,
    };
    let mut changed = previous.clone();
    changed
        .child_definitions
        .insert(child, "correction-2".into());
    require_scope_authorization(&previous, &changed, &event, now, true).unwrap();
    assert!(require_scope_authorization(&previous, &changed, &event, now, false).is_err());
    changed.child_scopes.insert(child, "wider-scope".into());
    assert!(require_scope_authorization(&previous, &changed, &event, now, true).is_err());
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
fn pending_phase_and_correction_generation_ignore_local_poll_counters() {
    let author = Uuid::from_u128(901);
    let comment = Uuid::from_u128(902);
    let mut left = Corrections::default();
    let mut right = Corrections::default();
    for (book, counter) in [(&mut left, 1), (&mut right, 999)] {
        book.observe(
            CommentRevision {
                id: comment,
                author: Some(author),
                body_digest: "native-content-digest".into(),
                edited_at: None,
                observed_revision: counter,
            },
            &BTreeSet::from([author]),
            &BTreeSet::new(),
            None,
        )
        .unwrap();
    }
    assert_eq!(
        left.pending_revision_digest().unwrap(),
        right.pending_revision_digest().unwrap()
    );
    left.classify(comment, "native-content-digest", CommentClass::Corrective)
        .unwrap();
    right
        .classify(comment, "native-content-digest", CommentClass::Corrective)
        .unwrap();
    assert_eq!(left.generation().unwrap(), right.generation().unwrap());
    right
        .observe(
            CommentRevision {
                id: comment,
                author: Some(author),
                body_digest: "edited-content".into(),
                edited_at: None,
                observed_revision: 1000,
            },
            &BTreeSet::from([author]),
            &BTreeSet::new(),
            None,
        )
        .unwrap();
    assert_ne!(
        left.pending_revision_digest().unwrap(),
        right.pending_revision_digest().unwrap()
    );
    assert_ne!(left.generation().unwrap(), right.generation().unwrap());
}
