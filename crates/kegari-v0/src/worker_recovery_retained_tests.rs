use super::*;
use crate::control::ReceiptPhase;
use kegari_types::run::RunReference;
use serde_json::{Value, json};
use std::collections::BTreeMap;

// Fixed original implementation corpus supplies ordinary serialized history,
// never a live worker, owned process or joined recovery capability.
fn retained_third_attempt() -> RetainedResumeSession {
    let golden: Value = serde_json::from_str(include_str!(
        "../../kegari-codex/tests/fixtures/codex-final-sessions.json"
    ))
    .unwrap();
    let mut state = SessionState::restore(
        golden["cases"]["two_prefix"]["serialized"]
            .as_str()
            .unwrap()
            .as_bytes(),
    )
    .unwrap();
    let input = json!([{"type":"text","text":"golden input 3"}]);
    let params = json!({"threadId":"golden-thread","input":input});
    let turn = json!({"id":"turn-3","status":"failed","items":[{"id":"input-3","type":"userMessage","content":input}],"error":{"message":"synthetic failure","codexErrorInfo":"serverOverloaded"}});
    state.turns_started = 1;
    state
        .notification(
            "turn/started",
            &json!({"threadId":"golden-thread","turn":{"id":"turn-3","status":"inProgress"}}),
        )
        .unwrap();
    state
        .notification(
            "turn/completed",
            &json!({"threadId":"golden-thread","turn":turn}),
        )
        .unwrap();
    state.submissions = Some(serde_json::from_value(json!({"records":[{
        "rpc_id":3,"method":"turn/start","thread":"golden-thread","expected_turn":null,
        "input_digest":canonical_digest(1,&json!([{"type":"text","text":"golden input 3","text_elements":[]}])).unwrap(),
        "frame_digest":canonical_digest(1,&json!({"id":3,"method":"turn/start","params":params})).unwrap(),
        "outcome":{"state":"acknowledged","turn":"turn-3"}
    }]})).unwrap());
    state
        .usage
        .observe("golden-account", "golden-thread", 60, true);
    let mut reference: RunReference =
        serde_json::from_str(golden["reference"].as_str().unwrap()).unwrap();
    reference.instruction_digest = state.binding.instruction_digest.clone();
    reference.attempt = 3;
    RetainedResumeSession::from_journal(
        state,
        canonical_digest(1, &"synthetic third admission").unwrap(),
        reference,
        canonical_digest(1, &"synthetic third journal").unwrap(),
    )
    .unwrap()
}

#[test]
fn retained_resume_verifies_actual_receipts_across_multiple_generations() {
    let observed = retained_third_attempt();
    let runtime = Uuid::from_u128(200);
    let mut state = serde_json::to_value(observed.state()).unwrap();
    let prefix = &mut state["resume_origin"]["prefix"];
    let segments = prefix["segments"].as_array_mut().unwrap();
    let mut receipts = Vec::<Receipt>::new();
    for index in 0..=segments.len() {
        let (operation, run): (Uuid, RunReference) = if let Some(segment) = segments.get(index) {
            (
                segment["binding"]["operation_id"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap(),
                serde_json::from_value(segment["reference"].clone()).unwrap(),
            )
        } else {
            (
                observed.state().binding.operation_id.parse().unwrap(),
                observed.reference().clone(),
            )
        };
        let mut expected = BTreeMap::from([("run".into(), serde_json::to_string(&run).unwrap())]);
        if index > 0 {
            let prior = &segments[index - 1];
            let resume = RunResumeReference {
                predecessor: receipts[index - 1].operation_id,
                predecessor_receipt_digest: canonical_digest(1, &receipts[index - 1]).unwrap(),
                recovery_digest: prior["transition"]["recovery_digest"]
                    .as_str()
                    .unwrap()
                    .into(),
                instruction_compatibility_digest: prior["transition"]["instruction_digest"]
                    .as_str()
                    .unwrap()
                    .into(),
                directory_reuse_digest: canonical_digest(1, &"synthetic receipt only").unwrap(),
                review_rounds_consumed: 0,
            };
            expected.insert(
                "session_resume".into(),
                serde_json::to_string(&resume).unwrap(),
            );
        }
        let receipt = Receipt {
            operation_id: operation,
            request_id: operation,
            runtime_id: runtime,
            owner_generation: 1,
            actor: "synthetic receipt observation".into(),
            kind: "admission".into(),
            canonical_targets: vec![run.phase.work.clone()],
            payload_digest: canonical_digest(1, &expected).unwrap(),
            expected,
            phase: ReceiptPhase::Reconciled,
            remote_ids: BTreeMap::new(),
            observed_result: Some(json!({"local_execution":"exited","unresolved_effects":[]})),
            cancellation: "none".into(),
            updated_at: chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        };
        if let Some(segment) = segments.get_mut(index) {
            segment["admission_digest"] = json!(canonical_digest(1, &receipt).unwrap());
        }
        receipts.push(receipt);
    }
    prefix["digest"] = json!(
        canonical_digest(
            1,
            &json!({"schema_version":prefix["schema_version"],"segments":prefix["segments"]})
        )
        .unwrap()
    );
    let state = SessionState::restore(&serde_json::to_vec(&state).unwrap()).unwrap();
    let candidate = RetainedResumeSession::from_journal(
        state,
        canonical_digest(1, receipts.last().unwrap()).unwrap(),
        observed.reference().clone(),
        observed.journal_digest().into(),
    )
    .unwrap();
    candidate.verify_prior_receipts(&receipts).unwrap();
    for index in 0..receipts.len() {
        let mut missing = receipts.clone();
        missing.remove(index);
        assert!(
            candidate.verify_prior_receipts(&missing).is_err(),
            "missing {index}"
        );
        let mut changed = receipts.clone();
        changed[index].owner_generation += 1;
        assert!(
            candidate.verify_prior_receipts(&changed).is_err(),
            "changed whole receipt {index}"
        );
    }
    for field in [
        "missing",
        "predecessor",
        "predecessor_receipt_digest",
        "recovery_digest",
        "instruction_compatibility_digest",
    ] {
        let mut changed = receipts.clone();
        let next = changed.last_mut().unwrap();
        if field == "missing" {
            next.expected.remove("session_resume");
        } else {
            let mut reference: Value =
                serde_json::from_str(&next.expected["session_resume"]).unwrap();
            reference[field] = if field == "predecessor" {
                json!(Uuid::from_u128(999))
            } else {
                json!(canonical_digest(1, &field).unwrap())
            };
            next.expected
                .insert("session_resume".into(), reference.to_string());
        }
        // Permit the changed final envelope at this data-only seam to isolate
        // typed-link checking from the already-tested whole-receipt hash check.
        let candidate = RetainedResumeSession::from_journal(
            candidate.state().clone(),
            canonical_digest(1, next).unwrap(),
            candidate.reference().clone(),
            candidate.journal_digest().into(),
        )
        .unwrap();
        assert!(
            candidate.verify_prior_receipts(&changed).is_err(),
            "typed link {field}"
        );
    }
}
