//! Duplex component tests: real submitted frames and terminal observations, no
//! native joined-worker, source-reuse or live admission capability construction.
use super::tests::{Observed, read, write};
use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::BufReader;

pub(super) async fn repeated_resume_sequence(first: &VerifiedThreadRecovery, thread: Value) {
    let original = canonical_digest(1, first.retained().state()).unwrap();
    let (second, second_thread) = successor(first, thread, "turn-two").await;
    let second_digest = canonical_digest(1, second.retained().state()).unwrap();
    let (third, third_thread) = successor(&second, second_thread.clone(), "turn-three").await;
    let state = third.retained().state();
    let prefix = state
        .resume_origin
        .as_ref()
        .unwrap()
        .prefix
        .as_ref()
        .unwrap();
    assert_eq!(prefix.segments.len(), 2);
    assert_eq!(prefix.segments[0].reference.attempt, 1);
    assert_eq!(prefix.segments[1].reference.attempt, 2);
    assert_eq!(third.retained().reference().attempt, 3);
    assert_eq!(third.retained().reference().max_attempts, 3);
    assert_eq!(prefix.segments[0].submissions.records[0].rpc_id, 3);
    // A fresh transport may reuse a numeric RPC ID. Its operation-scoped
    // provenance must not overwrite the earlier submitted frame.
    assert_eq!(prefix.segments[1].submissions.records[0].rpc_id, 5);
    assert_eq!(state.submissions.as_ref().unwrap().records[0].rpc_id, 5);
    assert_ne!(
        prefix.segments[1].binding.operation_id,
        state.binding.operation_id
    );
    assert_eq!(
        prefix.segments[0].usage.observed_tokens["fixture-account"],
        20
    );
    assert_eq!(
        prefix.segments[1].usage.observed_tokens["fixture-account"],
        10
    );
    assert_eq!(state.usage.observed_tokens["fixture-account"], 10);
    let key = "fixture-account\0thread-one";
    assert!(state.usage.earlier_usage_unknown.contains(key));
    assert_eq!(
        prefix.segments[1].usage.prior_unattributed_usage[key].known_increase,
        35
    );
    assert_eq!(state.usage.prior_unattributed_usage[key].known_increase, 35);
    assert_eq!(state.usage.cumulative_baselines[key], 190);
    assert_eq!(
        canonical_digest(1, first.retained().state()).unwrap(),
        original
    );
    assert_eq!(
        canonical_digest(1, second.retained().state()).unwrap(),
        second_digest
    );

    // Full history is checked again on each metadata read. The prior failed
    // boundary is permitted only with the exact verified transient provenance.
    for scenario in [
        "reordered",
        "truncated",
        "changed_input",
        "prior_nontransient",
        "unknown_turn",
    ] {
        let mut changed = third_thread.clone();
        let turns = changed["turns"].as_array_mut().unwrap();
        match scenario {
            "reordered" => turns.swap(0, 1),
            "truncated" => {
                turns.remove(0);
            }
            "changed_input" => {
                turns[0]["items"][0]["content"][0]["text"] = json!("unsubmitted replacement")
            }
            "prior_nontransient" => turns[0]["error"]["codexErrorInfo"] = json!("unauthorized"),
            _ => turns[0]["id"] = json!("unrecorded-turn"),
        }
        assert!(
            observe(third.retained(), changed).await.is_err(),
            "{scenario}"
        );
    }
    for scenario in ["reordered", "truncated", "changed_input", "changed_usage"] {
        let mut changed = state.clone();
        let prefix = changed
            .resume_origin
            .as_mut()
            .unwrap()
            .prefix
            .as_mut()
            .unwrap();
        match scenario {
            "reordered" => prefix.segments.swap(0, 1),
            "truncated" => {
                prefix.segments.remove(0);
            }
            "changed_input" => {
                prefix.segments[0].submissions.records[0].input_digest =
                    Some(canonical_digest(1, &"changed").unwrap())
            }
            _ => {
                prefix.segments[0]
                    .usage
                    .observed_tokens
                    .insert("fixture-account".into(), 0);
            }
        }
        assert!(
            SessionState::restore(&serde_json::to_vec(&changed).unwrap()).is_err(),
            "{scenario}"
        );
        let changed_origin = changed.resume_origin.as_mut().unwrap();
        let prefix = changed_origin.prefix.as_mut().unwrap();
        prefix.digest = prefix.content_digest().unwrap();
        assert!(
            changed_origin
                .preserves(state.resume_origin.as_ref().unwrap())
                .is_err(),
            "an appended checkpoint cannot rewrite prefix: {scenario}"
        );
    }
    let mut legacy = second.retained().state().clone();
    legacy.resume_origin.as_mut().unwrap().prefix = None;
    // Legacy journal data remains inspectable, but cannot establish a repeated
    // recovery's missing ordered/submitted history.
    SessionState::restore(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(
        ResumeCandidate::from_retained_parts(
            legacy,
            second.retained().admission_digest().into(),
            second.retained().reference().clone(),
            second.retained().journal_digest().into()
        )
        .is_err()
    );
    let mut enlarged = third.retained().reference().clone();
    enlarged.max_attempts += 1;
    assert!(
        ResumeCandidate::from_retained_parts(
            state.clone(),
            third.retained().admission_digest().into(),
            enlarged,
            third.retained().journal_digest().into()
        )
        .is_err()
    );
    // An unknown earlier baseline is not healed by another resume or a later
    // cumulative counter; only the new session's independently observed deltas
    // belong to it.
    let mut unknown = UsageLedger::default();
    unknown.observe_resume_baseline("fixture-account", "thread-one", 100, false);
    let next = unknown.resumed_session("fixture-account", "thread-one");
    let next_again = next.resumed_session("fixture-account", "thread-one");
    assert!(next_again.earlier_usage_unknown.contains(key));
    assert!(next_again.observed_tokens.is_empty());
    assert_eq!(next_again.cumulative_baselines[key], 100);
}

async fn successor(
    proof: &VerifiedThreadRecovery,
    mut thread: Value,
    turn_id: &str,
) -> (VerifiedThreadRecovery, Value) {
    let old = proof.retained().state();
    let mut binding = old.binding.clone();
    binding.operation_id = uuid::Uuid::new_v4().to_string();
    binding.instruction_digest = canonical_digest(1, &turn_id).unwrap();
    let mut state = SessionState::new(binding, old.max_turns).unwrap();
    state.thread_id = old.thread_id.clone();
    state.usage = old.usage.resumed_session("fixture-account", "thread-one");
    state.resume_origin = Some(
        SessionResumeOrigin::from_recovery(
            proof,
            proof.digest(),
            &canonical_digest(1, &"component bridge").unwrap(),
            &state.binding,
        )
        .unwrap(),
    );
    let baseline = state.usage.cumulative_baselines["fixture-account\0thread-one"];
    let old_turn = old.last_completed_turn.clone().unwrap();
    let turn_id = turn_id.to_owned();
    let (client, server) = tokio::io::duplex(65_536);
    let (reader, writer) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let server = tokio::spawn(async move {
        let mut sr = BufReader::new(sr);
        handshake(&mut sr, &mut sw).await;
        let readback = read(&mut sr).await;
        assert_eq!(readback["method"], "thread/read");
        write(
            &mut sw,
            json!({"id":readback["id"],"result":{"thread":thread}}),
        )
        .await;
        let resume = read(&mut sr).await;
        assert_eq!(resume["method"], "thread/resume");
        write(&mut sw,json!({"id":resume["id"],"result":{"thread":{"id":"thread-one"},"cwd":"/private/tmp/fixture","model":"fixture-model","modelProvider":"openai","reasoningEffort":"high","approvalPolicy":"never","activePermissionProfile":{"id":"fixture"},"instructionSources":[]}})).await;
        let turn = read(&mut sr).await;
        assert_eq!(turn["method"], "turn/start");
        assert_eq!(turn["id"], 5);
        let terminal = json!({"id":turn_id,"status":"failed","items":[{"id":format!("input-{turn_id}"),"type":"userMessage","content":turn["params"]["input"]}],"error":{"message":"bounded component failure","codexErrorInfo":"serverOverloaded"}});
        write(&mut sw,json!({"method":"turn/started","params":{"threadId":"thread-one","turn":{"id":turn_id,"status":"inProgress"}}})).await;
        // No prior-turn refresh precedes the first new-turn report. Its entire
        // increase remains ambiguous, followed by an old-turn late update.
        for (id, total) in [
            (&turn_id, baseline + 30),
            (&turn_id, baseline + 30),
            (&turn_id, baseline + 25),
            (&old_turn, baseline + 35),
            (&old_turn, baseline + 35),
            (&old_turn, baseline + 34),
            (&turn_id, baseline + 40),
            (&turn_id, baseline + 45),
        ] {
            write(&mut sw,json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread-one","turnId":id,"tokenUsage":{"total":{"totalTokens":total}}}})).await;
        }
        write(
            &mut sw,
            json!({"method":"turn/completed","params":{"threadId":"thread-one","turn":terminal}}),
        )
        .await;
        write(&mut sw, json!({"id":turn["id"],"result":{"turn":terminal}})).await;
        thread["turns"].as_array_mut().unwrap().push(terminal);
        let recovery = read(&mut sr).await;
        assert_eq!(recovery["method"], "thread/read");
        write(
            &mut sw,
            json!({"id":recovery["id"],"result":{"thread":thread}}),
        )
        .await;
        thread
    });
    let mut rpc = RpcClient::from_transport(reader, writer, Duration::from_secs(2)).unwrap();
    rpc.initialize().await.unwrap();
    rpc.verify_account("fixture-account").await.unwrap();
    let mut broker = Observed(Arc::new(Mutex::new(vec![])));
    let permissions = PermissionConfiguration {
        profile_id: "fixture".into(),
        isolation_digest: state.binding.isolation_digest.clone(),
        settings: json!({}),
    };
    rpc.resume_protocol(
        &mut state,
        ResumeRequest {
            recovery: proof,
            prompt: "independently assembled identical task semantics",
            predecessor: old.binding.operation_id.parse().unwrap(),
            bridge_digest: proof.digest(),
        },
        &permissions,
        &mut broker,
    )
    .await
    .unwrap();
    assert!(state.usage.observed_tokens.is_empty());
    rpc.start_turn(
        &mut state,
        &permissions,
        "continue the unchanged admitted task",
        &mut broker,
    )
    .await
    .unwrap();
    let state = SessionState::restore(&serde_json::to_vec(&state).unwrap()).unwrap();
    let mut reference = proof.retained().reference().clone();
    reference.attempt += 1;
    reference.instruction_digest = state.binding.instruction_digest.clone();
    let retained = ResumeCandidate::from_retained_parts(
        state,
        canonical_digest(1, &reference).unwrap(),
        reference,
        canonical_digest(1, &"actual component journal").unwrap(),
    )
    .unwrap();
    let next = rpc
        .recover_terminal_failure(&retained, &mut broker)
        .await
        .unwrap();
    (next, server.await.unwrap())
}

async fn handshake(
    sr: &mut BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
    sw: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>,
) {
    let init = read(sr).await;
    assert_eq!(init["method"], "initialize");
    write(sw,json!({"id":init["id"],"result":{"userAgent":"fixture","platformFamily":"unix","platformOs":"macos","codexHome":"/private/tmp/fixture-home"}})).await;
    assert_eq!(read(sr).await["method"], "initialized");
    let account = read(sr).await;
    assert_eq!(account["method"], "account/read");
    write(sw,json!({"id":account["id"],"result":{"account":{"type":"chatgpt"},"workspaceRouting":{"chatgptAccountId":"fixture-account"}}})).await;
}

async fn observe(candidate: &ResumeCandidate, thread: Value) -> Result<VerifiedThreadRecovery> {
    let (client, server) = tokio::io::duplex(65_536);
    let (reader, writer) = tokio::io::split(client);
    let (sr, mut sw) = tokio::io::split(server);
    let server = tokio::spawn(async move {
        let mut sr = BufReader::new(sr);
        handshake(&mut sr, &mut sw).await;
        let request = read(&mut sr).await;
        assert_eq!(request["method"], "thread/read");
        write(
            &mut sw,
            json!({"id":request["id"],"result":{"thread":thread}}),
        )
        .await;
    });
    let mut rpc = RpcClient::from_transport(reader, writer, Duration::from_secs(2)).unwrap();
    rpc.initialize().await.unwrap();
    rpc.verify_account("fixture-account").await.unwrap();
    let result = rpc
        .recover_terminal_failure(candidate, &mut Observed(Arc::new(Mutex::new(vec![]))))
        .await;
    server.await.unwrap();
    result
}
