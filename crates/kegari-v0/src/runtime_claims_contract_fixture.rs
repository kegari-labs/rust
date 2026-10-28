//! Deterministic protected-journal fixture over a genuinely assembled contract.
//! This records synthetic completed protocol input, never native process or
//! command evidence. Its caller must retain actual execution capabilities.
use super::*;
use crate::{
    codex::{CodexProfile, SessionBinding, SessionState},
    worker::{WorkerAuthority, WorkerContract},
};
use kegari_config::Speed;
use serde_json::json;

pub(crate) fn test_completed_contract_fixture(
    contract: &WorkerContract,
    admission: Receipt,
    claim: Value,
) -> Result<(tempfile::TempDir, VerifiedAuthorizedOutcome, SessionState)> {
    let WorkerAuthority::Authorized(authority) = contract.authority() else {
        return Err(Error::new(
            "worker_binding",
            "fixture requires authorized work",
        ));
    };
    let run = RunReference::from_receipt(&admission)?
        .ok_or_else(|| Error::new("worker_binding", "fixture requires actual run admission"))?;
    let manifest = &contract.instructions().manifest;
    if manifest.operation_id != admission.operation_id
        || contract.account_id() != run.account
        || contract.scope_digest()
            != admission
                .expected
                .get("scope")
                .map(String::as_str)
                .unwrap_or("")
        || contract.instructions().manifest_digest != run.instruction_digest
    {
        return Err(Error::new(
            "worker_binding",
            "fixture contract differs from actual admission",
        ));
    }
    let temp = tempfile::tempdir().map_err(|error| Error::io("create fixture records", error))?;
    let base = temp
        .path()
        .canonicalize()
        .map_err(|error| Error::io("canonical fixture records", error))?;
    let claims = PrivateDir::create(&base.join("claims"))?;
    let records = PrivateDir::create(&base.join("records"))?;
    let key = AttestationKey::new("contract-fixture".into(), vec![73; 32])?;
    let mut session = SessionState::new(
        SessionBinding {
            schema_version: 1,
            account_id: run.account.clone(),
            work_identity: run.phase.work.clone(),
            operation_id: admission.operation_id.to_string(),
            generation: manifest.work_generation.clone(),
            instruction_digest: contract.instructions().manifest_digest.clone(),
            isolation_digest: manifest.permissions.enforcement_profile.clone(),
            profile: CodexProfile {
                model: manifest.profile.model.clone(),
                reasoning: manifest.profile.reasoning.clone(),
                speed: match manifest.profile.speed {
                    Speed::Standard => "standard",
                    Speed::Fast => "fast",
                }
                .into(),
            },
            cwd: manifest.cwd.clone(),
        },
        run.max_turns,
    )?;
    let turn = format!("fixture-turn-{}", admission.operation_id);
    session.thread_id = Some(format!("fixture-thread-{}", admission.operation_id));
    // This protected-claims fixture supplies a synthetic terminal summary, not
    // actual full provider history that could qualify a thread for resumption.
    session.provider_history = None;
    session.turns_started = 1;
    session.terminal = Some("completed".into());
    session
        .completed_turns
        .insert(turn.clone(), "completed".into());
    session.last_completed_turn = Some(turn.clone());
    let outcome = WorkerOutcome {
        validation_history_digest: None,
        schema_version: 1,
        runtime_id: admission.runtime_id,
        operation_id: admission.operation_id,
        proposal_id: Uuid::new_v4(),
        processed_correction: None,
        completed_turn: turn,
        work: Some(authority.work().clone()),
        independent_review: None,
        session: session.clone(),
        instructions: manifest.clone(),
        instruction_digest: contract.instructions().manifest_digest.clone(),
        tool_contract_digest: contract.tools().digest().into(),
        fresh_gate_digest: authority.digest()?,
        claim,
        artifact_refs: vec![],
        checks: vec![],
    };
    records.write(
        "contract.json",
        &json!({"scope":contract.scope_digest(),"instructions":manifest,
        "tool_contract":contract.tools().digest(),"isolation":contract.isolation().plan()}),
    )?;
    let journal = records.child("journal")?;
    journal.write("binding.json", &json!({"schema_version":1,"scope_digest":contract.scope_digest(),
        "instruction_digest":contract.instructions().manifest_digest,"tool_contract_digest":contract.tools().digest()}))?;
    journal.write(
        "event-00000000.json",
        &json!({"kind":"session","record":{"state":session,"pending_method":null}}),
    )?;
    journal.write(
        "event-00000001.json",
        &json!({"kind":"outcome_pending","record":outcome}),
    )?;
    let signed = key.sign(
        "pending_worker_claim",
        &json!({"schema_version":1,"outcome":outcome,"authority":authority}),
    )?;
    claims.write_bytes(
        &format!("pending-worker-{}.json", outcome.proposal_id),
        &serde_json::to_vec(&signed).map_err(|_| Error::invalid("fixture claim encoding"))?,
    )?;
    let verified = recover_authorized(
        &key,
        &claims,
        &records,
        admission.runtime_id,
        admission,
        outcome.proposal_id,
    )?;
    Ok((temp, verified, session))
}
