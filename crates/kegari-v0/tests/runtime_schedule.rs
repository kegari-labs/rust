use chrono::{Duration, Utc};
use kegari_config::RuntimeConfig;
use kegari_digest::canonical_digest;
use kegari_scheduler::Trigger;
use kegari_types::error::Error;
use kegari_types::run::PhaseKey;
use kegari_types::run::RunReference;
use kegari_v0::{
    control::{Receipt, ReceiptPhase},
    runtime_schedule::{occupying_runs, plan, require_provider_clear},
};
use serde_json::json;
use std::collections::BTreeMap;
use uuid::Uuid;
fn digest(value: &str) -> String {
    canonical_digest(1, &value).unwrap()
}
fn config() -> RuntimeConfig {
    serde_json::from_value(json!({"id":Uuid::from_u128(1),"host_id":"fixture","max_agents":3}))
        .unwrap()
}
fn phase() -> PhaseKey {
    PhaseKey {
        work: digest("canonical-child"),
        generation: digest("generation"),
        phase: "implementation".into(),
        retry_grant: None,
    }
}
fn receipt(ordinal: u32, outcome: &str, code: &str) -> Receipt {
    let run = RunReference {
        normalized_state: Some("todo".into()),
        workspace: Uuid::from_u128(2),
        account: "actual-account".into(),
        phase: phase(),
        profile_digest: digest("profile"),
        instruction_digest: digest("instructions"),
        max_attempts: 3,
        max_turns: 20,
        max_review_rounds: 3,
        attempt: ordinal,
    };
    let id = Uuid::from_u128(100 + u128::from(ordinal));
    Receipt {
        operation_id: id,
        request_id: id,
        runtime_id: config().id,
        owner_generation: 1,
        actor: "runtime".into(),
        kind: "admission".into(),
        canonical_targets: vec![phase().work],
        payload_digest: digest("payload"),
        expected: BTreeMap::from([("run".into(), serde_json::to_string(&run).unwrap())]),
        phase: ReceiptPhase::Reconciled,
        remote_ids: BTreeMap::new(),
        observed_result: Some(
            json!({"local_execution":"exited","unresolved_effects":[],"observation":{"outcome":outcome,"error":Error::new(code,"fixture failure"),"session":{"intervention":null,"input_required":false}}}),
        ),
        cancellation: "none".into(),
        updated_at: Utc::now() - Duration::seconds(60),
    }
}
#[test]
fn recovered_unknown_admissions_keep_capacity_without_live_permit_handles() {
    let mut retained = Vec::new();
    for (index, phase) in [
        ReceiptPhase::Prepared,
        ReceiptPhase::Sent,
        ReceiptPhase::Acknowledged,
        ReceiptPhase::Reconciled,
    ]
    .into_iter()
    .enumerate()
    {
        let mut old = receipt(1, "completed", "");
        old.operation_id = Uuid::from_u128(200 + index as u128);
        old.request_id = old.operation_id;
        let mut run = kegari_v0::control::run_reference_from_receipt(&old)
            .unwrap()
            .unwrap();
        run.phase.work = digest(&format!("held-child-{index}"));
        old.canonical_targets = vec![run.phase.work.clone()];
        old.expected
            .insert("run".into(), serde_json::to_string(&run).unwrap());
        // Fixture represents protected records after losing every in-memory
        // handle. Protocol completion has never established local tree exit.
        old.phase = phase;
        old.observed_result =
            Some(json!({"observation":{"outcome":"completed"},"local_exit_confirmed":false}));
        retained.push(old);
    }
    assert_eq!(occupying_runs(config().id, &retained).unwrap().len(), 4);
    retained[3].observed_result =
        Some(json!({"local_execution":"exited","unresolved_effects":[Uuid::from_u128(44)]}));
    // External uncertainty continues holding work independently, but an actual
    // host-recorded local exit frees exactly this one model-session slot.
    assert_eq!(occupying_runs(config().id, &retained).unwrap().len(), 3);
    retained[0].observed_result = Some(json!({"local_execution":"exited"}));
    assert_eq!(occupying_runs(config().id, &retained).unwrap().len(), 3);
    retained[0].runtime_id = Uuid::from_u128(999);
    assert_eq!(
        occupying_runs(config().id, &retained).unwrap_err().code,
        "record_corrupt"
    );
}
#[test]
fn retries_are_timed_from_actual_exit_and_honor_longer_provider_wait() {
    let now = Utc::now();
    let mut prior = receipt(1, "failed", "provider_transport");
    prior.updated_at = now;
    assert_eq!(
        plan(
            &config(),
            &phase(),
            Uuid::from_u128(2),
            &[prior.clone()],
            now + Duration::seconds(9)
        )
        .unwrap_err()
        .code,
        "retry_wait"
    );
    let next = plan(
        &config(),
        &phase(),
        Uuid::from_u128(2),
        &[prior.clone()],
        now + Duration::seconds(10),
    )
    .unwrap();
    assert_eq!(next.attempt, 2);
    assert_eq!(next.trigger, Trigger::ScheduledRetry);
    prior.observed_result.as_mut().unwrap()["observation"]["error"]["details"] =
        json!({"retry_after_ms":60000});
    assert_eq!(
        plan(
            &config(),
            &phase(),
            Uuid::from_u128(2),
            &[prior],
            now + Duration::seconds(59)
        )
        .unwrap_err()
        .code,
        "retry_wait"
    );
}
#[test]
fn polling_completed_blocked_and_interrupted_work_is_not_a_trigger() {
    for outcome in ["completed", "blocked", "interrupted", "cancelled"] {
        assert_eq!(
            plan(
                &config(),
                &phase(),
                Uuid::from_u128(2),
                &[receipt(1, outcome, "timeout")],
                Utc::now()
            )
            .unwrap_err()
            .code,
            "no_work_trigger"
        );
    }
    for code in [
        "access_denied",
        "unsupported_codex_version",
        "invalid_config",
        "scope_conflict",
        "provider_intervention",
        "no_progress",
    ] {
        assert_eq!(
            plan(
                &config(),
                &phase(),
                Uuid::from_u128(2),
                &[receipt(1, "failed", code)],
                Utc::now()
            )
            .unwrap_err()
            .code,
            "persistent_blocker"
        );
    }
}
#[test]
fn unknown_effects_missing_exit_and_manual_intervention_never_retry() {
    let mut prior = receipt(1, "failed", "timeout");
    prior.phase = ReceiptPhase::Sent;
    assert_eq!(
        plan(
            &config(),
            &phase(),
            Uuid::from_u128(2),
            &[prior.clone()],
            Utc::now()
        )
        .unwrap_err()
        .code,
        "reconciliation_required"
    );
    prior.phase = ReceiptPhase::Reconciled;
    prior.observed_result.as_mut().unwrap()["unresolved_effects"] = json!([Uuid::from_u128(500)]);
    assert_eq!(
        plan(
            &config(),
            &phase(),
            Uuid::from_u128(2),
            &[prior.clone()],
            Utc::now()
        )
        .unwrap_err()
        .code,
        "reconciliation_required"
    );
    prior.observed_result.as_mut().unwrap()["unresolved_effects"] = json!([]);
    prior.observed_result.as_mut().unwrap()["local_execution"] = json!("unknown");
    assert_eq!(
        plan(
            &config(),
            &phase(),
            Uuid::from_u128(2),
            &[prior.clone()],
            Utc::now()
        )
        .unwrap_err()
        .code,
        "reconciliation_required"
    );
    prior.observed_result.as_mut().unwrap()["local_execution"] = json!("exited");
    prior.observed_result.as_mut().unwrap()["observation"]["session"]["intervention"] =
        json!({"kind":"verification"});
    assert_eq!(
        plan(
            &config(),
            &phase(),
            Uuid::from_u128(2),
            &[prior],
            Utc::now()
        )
        .unwrap_err()
        .code,
        "provider_intervention"
    );
}
#[test]
fn consumed_phase_budget_survives_new_settings_and_rejects_history_gaps() {
    let history = vec![
        receipt(1, "failed", "timeout"),
        receipt(2, "failed", "process_exit"),
        receipt(3, "failed", "provider_transport"),
    ];
    let mut changed = config();
    changed.max_attempts_per_phase = 100;
    changed.max_turns = 100;
    assert_eq!(
        plan(&changed, &phase(), Uuid::from_u128(2), &history, Utc::now())
            .unwrap_err()
            .code,
        "attempts_exhausted"
    );
    let next = plan(
        &changed,
        &phase(),
        Uuid::from_u128(2),
        &history[..1],
        Utc::now(),
    )
    .unwrap();
    assert_eq!(next.max_attempts, 3);
    assert_eq!(next.max_turns, 20);
    assert_eq!(
        plan(
            &changed,
            &phase(),
            Uuid::from_u128(2),
            &history[1..],
            Utc::now()
        )
        .unwrap_err()
        .code,
        "record_corrupt"
    );
    let mut permitted_new_generation = phase();
    permitted_new_generation.generation = digest("material-correction");
    assert_eq!(
        plan(
            &changed,
            &permitted_new_generation,
            Uuid::from_u128(2),
            &history,
            Utc::now()
        )
        .unwrap()
        .attempt,
        1
    );
    // Planning is not authority: the caller still needs the current sealed gate
    // and atomic host admission for this material generation.
}

#[test]
fn provider_intervention_survives_generation_retry_and_later_success() {
    let mut blocked = receipt(1, "blocked", "provider_intervention");
    blocked.observed_result.as_mut().unwrap()["observation"]["session"]["intervention"] = json!({"account_id":"actual-account","source_method":"turn/completed","source_digest":digest("provider-signal"),"reason_code":"verification_required"});
    let mut later = receipt(1, "completed", "unused");
    later.operation_id = Uuid::from_u128(999);
    let mut changed = kegari_v0::control::run_reference_from_receipt(&later)
        .unwrap()
        .unwrap();
    changed.phase.generation = digest("new-correction");
    changed.phase.retry_grant = Some(Uuid::from_u128(42));
    changed.profile_digest = digest("new-profile");
    later
        .expected
        .insert("run".into(), serde_json::to_string(&changed).unwrap());
    let receipts = [blocked, later];
    for (account, work) in [
        ("actual-account", digest("unrelated-child")),
        ("replacement-account", phase().work),
    ] {
        assert_eq!(
            require_provider_clear(config().id, account, &work, &receipts)
                .unwrap_err()
                .code,
            "provider_intervention"
        );
    }
    assert!(
        require_provider_clear(
            config().id,
            "unrelated-account",
            &digest("unrelated-work"),
            &receipts
        )
        .is_ok()
    );
}

#[test]
fn delayed_exit_preserves_provider_hold_and_input_wait_is_work_scoped() {
    let mut prior = receipt(1, "blocked", "provider_intervention");
    let observation = prior.observed_result.as_mut().unwrap()["observation"].clone();
    prior.observed_result.as_mut().unwrap()["observation"] =
        json!({"owned_handles":"observed_exited", "prior":observation});
    prior.observed_result.as_mut().unwrap()["observation"]["prior"]["session"]["input_required"] =
        json!(true);
    assert_eq!(
        require_provider_clear(config().id, "new-account", &phase().work, &[prior.clone()])
            .unwrap_err()
            .code,
        "provider_input_required"
    );
    assert!(
        require_provider_clear(
            config().id,
            "actual-account",
            &digest("different-work"),
            &[prior.clone()]
        )
        .is_ok()
    );
    prior.observed_result.as_mut().unwrap()["observation"]["prior"]["session"]["intervention"] =
        json!({"reason_code":"policy_review"});
    assert_eq!(
        require_provider_clear(
            config().id,
            "actual-account",
            &digest("different-work"),
            &[prior]
        )
        .unwrap_err()
        .code,
        "provider_intervention"
    );
}
