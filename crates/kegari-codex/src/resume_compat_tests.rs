use super::*;
use crate::CodexProfile;

fn binding(operation: u128, instruction: &str) -> SessionBinding {
    SessionBinding {
        schema_version: 1,
        account_id: "golden-account".into(),
        work_identity: "golden-work".into(),
        operation_id: uuid::Uuid::from_u128(operation).to_string(),
        generation: canonical_digest(1, &"golden-generation").unwrap(),
        instruction_digest: canonical_digest(1, &instruction).unwrap(),
        isolation_digest: canonical_digest(1, &"golden-isolation").unwrap(),
        profile: CodexProfile {
            model: "golden-model".into(),
            reasoning: "high".into(),
            speed: "standard".into(),
        },
        cwd: PathBuf::from("/fixture/golden/repository"),
    }
}
fn reference(state: &SessionState, attempt: u32) -> kegari_types::run::RunReference {
    kegari_types::run::RunReference {
        workspace: uuid::Uuid::from_u128(100),
        account: state.binding.account_id.clone(),
        normalized_state: None,
        phase: kegari_types::run::PhaseKey {
            work: state.binding.work_identity.clone(),
            generation: state.binding.generation.clone(),
            phase: "implementation".into(),
            retry_grant: None,
        },
        profile_digest: canonical_digest(1, &"golden-profile").unwrap(),
        instruction_digest: state.binding.instruction_digest.clone(),
        max_attempts: 3,
        max_turns: 3,
        max_review_rounds: 3,
        attempt,
    }
}
fn failed(
    mut state: SessionState,
    attempt: u32,
    all_turns: &mut Vec<Value>,
) -> VerifiedThreadRecovery {
    let turn_id = format!("turn-{attempt}");
    let input = json!([{"type":"text","text":format!("golden input {attempt}")}]);
    let turn = json!({"id":turn_id,"status":"failed","items":[{"id":format!("input-{attempt}"),"type":"userMessage","content":input}],"error":{"message":"synthetic failure","codexErrorInfo":"serverOverloaded"}});
    let params = json!({"threadId":"golden-thread","input":input});
    state.thread_id = Some("golden-thread".into());
    state.turns_started = 1;
    state
        .submissions
        .as_mut()
        .unwrap()
        .prepare(3, "turn/start", &params)
        .unwrap();
    state
        .notification(
            "turn/started",
            &json!({"threadId":"golden-thread","turn":{"id":turn_id,"status":"inProgress"}}),
        )
        .unwrap();
    state
        .notification(
            "turn/completed",
            &json!({"threadId":"golden-thread","turn":turn}),
        )
        .unwrap();
    state
        .submissions
        .as_mut()
        .unwrap()
        .observe_response(3, &Ok(json!({"turn":turn})))
        .unwrap();
    state.usage.observe(
        "golden-account",
        "golden-thread",
        u64::from(attempt) * 20,
        true,
    );
    all_turns.push(turn);
    let history_digest = canonical_digest(1, all_turns).unwrap();
    let prefix = state
        .resume_origin
        .as_ref()
        .and_then(|origin| origin.prefix.as_ref())
        .map(|prefix| prefix.segments.as_slice())
        .unwrap_or_default();
    let submission_digest = submission_digest(prefix, state.submissions.as_ref().unwrap()).unwrap();
    let reference = reference(&state, attempt);
    let retained = ResumeCandidate::from_retained_parts(
        state,
        canonical_digest(1, &format!("golden admission {attempt}")).unwrap(),
        reference,
        canonical_digest(1, &format!("golden journal {attempt}")).unwrap(),
    )
    .unwrap();
    let response_digest = canonical_digest(1, &json!({"turns":all_turns})).unwrap();
    let transient_failure = TransientTurnFailure::ServerOverloaded;
    let digest = canonical_digest(1, &json!({"retained":retained.digest,"response":response_digest,"history":history_digest,"submissions":submission_digest,"failure":transient_failure})).unwrap();
    VerifiedThreadRecovery {
        retained,
        response_digest,
        history_digest,
        submission_digest,
        transient_failure,
        digest,
    }
}
fn successor(proof: &VerifiedThreadRecovery, operation: u128, instruction: &str) -> SessionState {
    let mut state = SessionState::new(binding(operation, instruction), 3).unwrap();
    state.thread_id = Some("golden-thread".into());
    state.resume_origin = Some(
        SessionResumeOrigin::from_recovery(
            proof,
            &canonical_digest(1, &format!("golden recovery {operation}")).unwrap(),
            &canonical_digest(1, &format!("golden bridge {operation}")).unwrap(),
            &state.binding,
        )
        .unwrap(),
    );
    state.resume_origin.as_mut().unwrap().readback_digest = Some(proof.digest().into());
    state
}
fn corpus() -> Value {
    let fresh = SessionState::new(binding(1, "first instructions"), 3).unwrap();
    let mut turns = Vec::new();
    let first = failed(fresh.clone(), 1, &mut turns);
    let second = successor(&first, 2, "second instructions");
    let second_failed = failed(second.clone(), 2, &mut turns);
    let third = successor(&second_failed, 3, "third instructions");
    let mut legacy = second.clone();
    legacy.resume_origin.as_mut().unwrap().prefix = None;
    let mut cases = BTreeMap::new();
    for (name, state) in [
        ("fresh", fresh),
        ("failed", first.retained().state().clone()),
        ("one_prefix", second),
        ("two_prefix", third),
        ("legacy_without_prefix", legacy),
    ] {
        let serialized = serde_json::to_string(&state).unwrap();
        let restored = SessionState::restore(serialized.as_bytes()).unwrap();
        assert_eq!(serde_json::to_string(&restored).unwrap(), serialized);
        cases.insert(
            name,
            json!({"serialized":serialized,"digest":canonical_digest(1,&state).unwrap()}),
        );
    }
    json!({"cases":cases,"first_candidate_digest":first.retained().digest(),"second_candidate_digest":second_failed.retained().digest(),"first_recovery_digest":first.digest(),"second_recovery_digest":second_failed.digest(),"reference":serde_json::to_string(first.retained().reference()).unwrap()})
}
#[test]
fn final_extraction_fixed_session_history_bytes_and_digests() {
    let actual = corpus();
    let expected: Value =
        serde_json::from_str(include_str!("../tests/fixtures/codex-final-sessions.json")).unwrap();
    assert_eq!(actual, expected);
}
