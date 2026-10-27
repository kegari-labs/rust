//! Actual ScopedWorker/Control/read/journal components. Execution uses the
//! shared injected boundary; none of these tests qualifies a native profile.
use super::*;
use crate::validation::{DiscoveryReadReference, ValidationPlan};

fn events(f: &Fixture) -> Vec<(String, Value)> {
    f.worker
        .journal
        .dir
        .records()
        .unwrap()
        .into_iter()
        .filter(|leaf| leaf.starts_with("event-"))
        .map(|leaf| {
            let value = f.worker.journal.dir.read(&leaf).unwrap();
            (leaf, value)
        })
        .collect()
}
async fn read(f: &mut Fixture, tool: &str, arguments: Value) -> String {
    let call = f.call(tool, arguments);
    f.worker.call(&call).await.unwrap();
    f.worker.validation_evidence().unwrap()["reads"][0]["id"]
        .as_str()
        .unwrap()
        .into()
}
fn requirement(id: &str) -> Value {
    json!({"id":id,"statement":"the requested behavior remains valid","rationale":"observed task/source specifies this behavior",
        "argv":["/usr/bin/true"],"cwd":".","services":[],"artifacts":[]})
}
fn proposal(
    f: &Fixture,
    reads: &[String],
    previous: Option<&str>,
    requirements: Vec<Value>,
) -> ToolCall {
    f.call("validation_propose", json!({"previous":previous,"reason":"initial discovery or explicit revision from current observed inputs",
        "supporting_reads":reads,"requirements":requirements}))
}
async fn enroll(f: &mut Fixture) -> String {
    let reference = read(f, "source_read", json!({"path":"a.txt"})).await;
    let call = proposal(f, &[reference], None, vec![requirement("behavior")]);
    f.worker.call(&call).await.unwrap()["plan_digest"]
        .as_str()
        .unwrap()
        .into()
}
async fn run(f: &mut Fixture, plan: &str, obligation: &str) -> Result<Value> {
    let call = f.call(
        "validation_run",
        json!({"plan_digest":plan,"obligation_id":obligation}),
    );
    tokio::time::timeout(StdDuration::from_secs(15), f.worker.call(&call))
        .await
        .expect("bounded injected validation")
}
fn validation_receipts(f: &Fixture) -> Vec<Receipt> {
    f.control
        .operations()
        .unwrap()
        .into_iter()
        .filter(|r| r.kind == "worker_validation")
        .collect()
}
fn retained_plan(f: &mut Fixture) -> ValidationPlan {
    f.worker.take_validation_plan().unwrap()
}

#[tokio::test]
async fn successful_original_reads_have_exact_finished_references_and_replay_once() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let call = f.call("source_read", json!({"path":"a.txt"}));
    let original = f.worker.call(&call).await.unwrap();
    assert_eq!(f.worker.call(&call).await.unwrap(), original);
    let index = f.worker.validation_evidence().unwrap();
    assert_eq!(index["total_reads"], 1);
    let reference: DiscoveryReadReference =
        serde_json::from_value(index["reads"][0]["reference"].clone()).unwrap();
    assert_eq!(reference.digest().unwrap(), index["reads"][0]["id"]);
    let finished: Value = f.worker.journal.dir.read(&reference.journal.leaf).unwrap();
    assert_eq!(finished["kind"], "tool_finished");
    assert_eq!(finished["record"]["request_id"], json!(reference.call_id));
    assert_eq!(
        canonical_digest(1, &finished).unwrap(),
        reference.journal.digest
    );
    assert_eq!(
        reference.result_digest,
        canonical_digest(1, &original).unwrap()
    );
    let context = read(&mut f, "context_read", json!({"key":"issue"})).await;
    let proposed = proposal(
        &f,
        &[reference.digest().unwrap(), context],
        None,
        vec![requirement("behavior")],
    );
    f.worker.call(&proposed).await.unwrap();
    assert_eq!(retained_plan(&mut f).record().supporting_reads.len(), 2);
}

#[tokio::test]
async fn read_reply_persistence_failure_never_publishes_supporting_reference() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let next = *f.worker.journal.next.lock().unwrap();
    // tool_started writes next; real source read succeeds; tool_finished cannot
    // replace this directory. Its pending callback is not a usable read fact.
    std::fs::create_dir(
        f.worker
            .journal
            .dir
            .path()
            .join(format!("event-{:08}.json", next + 1)),
    )
    .unwrap();
    let call = f.call("source_read", json!({"path":"a.txt"}));
    assert!(f.worker.call(&call).await.is_err());
    assert_eq!(f.worker.validation_evidence().unwrap()["total_reads"], 0);
    assert!(f.worker.take_validation_plan().is_none());
    assert!(f.worker.call(&call).await.is_err());
    assert!(f.boundary.last_cancel.lock().unwrap().is_none());
}

#[tokio::test]
async fn forged_read_changed_source_and_replaced_finished_record_refuse_enrollment() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let forged = proposal(
        &f,
        &[bytes_digest(b"agent supplied ref")],
        None,
        vec![requirement("behavior")],
    );
    assert_eq!(
        f.worker.call(&forged).await.unwrap_err().code,
        "validation_read"
    );
    let id = read(&mut f, "source_read", json!({"path":"a.txt"})).await;
    std::fs::write(
        f.input.isolation.plan().repository.join("a.txt"),
        b"changed after actual read\n",
    )
    .unwrap();
    let changed = proposal(
        &f,
        std::slice::from_ref(&id),
        None,
        vec![requirement("behavior")],
    );
    assert_eq!(
        f.worker.call(&changed).await.unwrap_err().code,
        "validation_read_changed"
    );
    std::fs::write(
        f.input.isolation.plan().repository.join("a.txt"),
        b"original\n",
    )
    .unwrap();
    let index = f.worker.validation_evidence().unwrap();
    let reference: DiscoveryReadReference =
        serde_json::from_value(index["reads"][0]["reference"].clone()).unwrap();
    let mut event: Value = f.worker.journal.dir.read(&reference.journal.leaf).unwrap();
    event["record"]["result"] = json!({"Ok":{"path":"a.txt","content":"rewritten"}});
    f.worker
        .journal
        .dir
        .write(&reference.journal.leaf, &event)
        .unwrap();
    let replaced = proposal(&f, &[id], None, vec![requirement("behavior")]);
    assert_eq!(
        f.worker.call(&replaced).await.unwrap_err().code,
        "validation_read"
    );
    assert!(f.worker.take_validation_plan().is_none());
    assert!(validation_receipts(&f).is_empty());
}

#[tokio::test]
async fn empty_configured_checks_use_actual_enrollment_execution_chunks_and_ack() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let plan = enroll(&mut f).await;
    let call = f.call(
        "validation_run",
        json!({"plan_digest":plan,"obligation_id":"behavior"}),
    );
    let result = f.worker.call(&call).await.unwrap();
    assert_eq!(result["successful"], true);
    assert_eq!(result["plan_digest"], plan);
    assert_eq!(result["outcome"]["stdout_preview"], "deterministic fixture");
    assert_eq!(
        result["outcome"]["stdout_digest"],
        bytes_digest(b"deterministic fixture")
    );
    assert!(result.get("before").is_none());
    assert!(serde_json::to_vec(&result).unwrap().len() < 65_536);
    assert_eq!(f.worker.call(&call).await.unwrap(), result);
    assert_eq!(validation_receipts(&f).len(), 1);
    assert_eq!(validation_receipts(&f)[0].phase, ReceiptPhase::Acknowledged);
    assert!(f.executions.active_operations().unwrap().is_empty());
    assert!(f.worker.validation_history_error().is_none());
    let records = f.worker.take_validation_attempts();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].schema_version, 4);
    assert_eq!(records[0].discovered.as_ref().unwrap().plan_digest, plan);
    assert_eq!(records[0].command.stdout, "deterministic fixture");
    assert_eq!(
        records[0].evidence_class,
        ExecutionEvidenceClass::DeterministicFixture
    );
    let all = events(&f);
    let (_, event) = all
        .iter()
        .rev()
        .find(|(_, event)| event["kind"] == "validation_check_entry")
        .unwrap();
    let reference: crate::worker::command::CommandEntryReference =
        serde_json::from_value(event["record"].clone()).unwrap();
    let bytes: Vec<u8> = reference
        .chunks
        .iter()
        .flat_map(|leaf| {
            let chunk: Value = f.worker.journal.dir.read(leaf).unwrap();
            assert_eq!(chunk["kind"], "validation_check_chunk");
            hex::decode(chunk["record"]["payload_hex"].as_str().unwrap()).unwrap()
        })
        .collect();
    assert_eq!(bytes_digest(&bytes), reference.digest);
    let finished: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(finished["state"], "finished");
    assert_eq!(finished["command"]["stdout"], "deterministic fixture");
    // Pure rendering oracle on a descriptive copy: escaping cannot make the
    // two previews exceed the public tool budget. No capability is constructed.
    let mut escaped = finished.clone();
    escaped["command"]["stdout"] = json!("\u{1}".repeat(6000));
    escaped["command"]["stderr"] = json!("\u{2}".repeat(6000));
    let preview = crate::worker::validation::result_summary(&escaped, &reference).unwrap();
    assert_eq!(preview["outcome"]["stdout_bytes"], 6000);
    assert_eq!(
        preview["outcome"]["stdout_preview"].as_str().unwrap().len(),
        4096
    );
    assert!(serde_json::to_vec(&preview).unwrap().len() < 65_536);
    assert_eq!(escaped["command"]["stdout"].as_str().unwrap().len(), 6000);
    assert!(
        all.iter()
            .any(|(_, event)| event["kind"] == "validation_attempt_ack")
    );
    let checks = f.worker.take_verified_checks();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].validation_plan().unwrap().digest(), plan);
    assert!(checks[0].production_evidence().is_err());
}

#[tokio::test]
async fn revisions_keep_obligation_history_old_attempts_and_reject_old_plan_execution() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let old = enroll(&mut f).await;
    run(&mut f, &old, "behavior").await.unwrap();
    let read_id = read(&mut f, "context_read", json!({"key":"issue"})).await;
    let dropping = proposal(
        &f,
        std::slice::from_ref(&read_id),
        Some(&old),
        vec![requirement("replacement")],
    );
    assert_eq!(
        f.worker.call(&dropping).await.unwrap_err().code,
        "validation_plan"
    );
    let mut changed = requirement("behavior");
    changed["argv"] = json!(["/usr/bin/true", "revised literal"]);
    let revised = proposal(
        &f,
        &[read_id],
        Some(&old),
        vec![changed, requirement("additional")],
    );
    let current = f.worker.call(&revised).await.unwrap()["plan_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(current, old);
    assert_eq!(
        run(&mut f, &old, "behavior").await.unwrap_err().code,
        "validation_plan_changed"
    );
    assert_eq!(validation_receipts(&f).len(), 1);
    assert_eq!(f.worker.take_validation_attempts().len(), 1);
    assert_eq!(
        f.worker.take_verified_checks()[0]
            .validation_plan()
            .unwrap()
            .digest(),
        old
    );
    let plan = retained_plan(&mut f);
    assert_eq!(plan.digest(), current);
    assert_eq!(plan.history().len(), 2);
    assert_eq!(plan.record().predecessor.as_deref(), Some(old.as_str()));
}

#[tokio::test]
async fn newer_failed_and_unknown_attempts_cannot_hide_behind_an_earlier_pass() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let plan = enroll(&mut f).await;
    run(&mut f, &plan, "behavior").await.unwrap();
    f.boundary.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        run(&mut f, &plan, "behavior").await.unwrap()["successful"],
        false
    );
    let records = f.worker.take_validation_attempts();
    assert_eq!(records.len(), 2);
    assert_eq!(records.iter().filter(|r| !r.successful).count(), 1);
    assert!(f.worker.validation_history_error().is_none());
    f.boundary.unknown.store(true, Ordering::SeqCst);
    assert_eq!(
        run(&mut f, &plan, "behavior").await.unwrap_err().code,
        "operation_uncertain"
    );
    assert!(f.worker.validation_history_error().is_some());
    assert_eq!(
        f.executions.observe_exited(f.permit.operation_id).unwrap(),
        0
    );
    assert!(!f.executions.active_operations().unwrap().is_empty());
    let read_id = read(&mut f, "context_read", json!({"key":"issue"})).await;
    let revised = proposal(&f, &[read_id], Some(&plan), vec![requirement("behavior")]);
    assert!(f.worker.call(&revised).await.is_err());
    assert_eq!(retained_plan(&mut f).digest(), plan);
}

#[tokio::test]
async fn configured_obligations_are_preserved_and_parent_discovery_is_not_invented() {
    let mut f = Fixture::new(AgentRole::Implementation);
    let read_id = read(&mut f, "context_read", json!({"key":"issue"})).await;
    // A workflow catalog alone is not a selection requirement. The actual
    // delivery assignment selects names; exercise that same advisory guard.
    let missing = proposal(
        &f,
        std::slice::from_ref(&read_id),
        None,
        vec![requirement("behavior")],
    );
    let parsed = serde_json::from_value(missing.arguments.clone()).unwrap();
    assert_eq!(
        crate::worker::validation::validate_configured_requirements(
            &f.input.workflow,
            &f.input.isolation,
            &["suite".into()],
            &parsed
        )
        .unwrap_err()
        .code,
        "validation_plan"
    );
    crate::worker::validation::validate_configured_requirements(
        &f.input.workflow,
        &f.input.isolation,
        &[],
        &parsed,
    )
    .unwrap();
    let good = proposal(&f, &[read_id], None, vec![requirement("suite")]);
    let parsed = serde_json::from_value(good.arguments.clone()).unwrap();
    crate::worker::validation::validate_configured_requirements(
        &f.input.workflow,
        &f.input.isolation,
        &["suite".into()],
        &parsed,
    )
    .unwrap();
    f.worker.call(&good).await.unwrap();
    let mut parent = Fixture::without_checks(AgentRole::Validation);
    let proposed = proposal(
        &parent,
        &[bytes_digest(b"no child read")],
        None,
        vec![requirement("behavior")],
    );
    assert_eq!(
        parent.worker.call(&proposed).await.unwrap_err().code,
        "validation_scope"
    );
    let malformed = parent.call("validation_run", json!({"name":"suite","kind":"local","plan_digest":bytes_digest(b"data"),"obligation_id":"suite"}));
    assert!(parent.worker.call(&malformed).await.is_err());
    assert!(validation_receipts(&parent).is_empty());
}

#[tokio::test]
async fn enrolled_history_refuses_cold_resume_and_large_plan_reply_stays_bounded() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let mut refs = Vec::new();
    for _ in 0..40 {
        refs.push(read(&mut f, "context_read", json!({"key":"issue"})).await);
    }
    let requirements: Vec<_> = (0..32)
        .map(|n| {
            let mut requirement = requirement(&format!("obligation-{n}"));
            requirement["statement"] = json!("observed behavior ".repeat(70));
            requirement
        })
        .collect();
    let proposed = proposal(&f, &refs, None, requirements);
    assert!(serde_json::to_vec(&proposed.arguments).unwrap().len() < 65_536);
    let reply = f.worker.call(&proposed).await.unwrap();
    assert!(serde_json::to_vec(&reply).unwrap().len() < 65_536);
    let plan = reply["plan_digest"].as_str().unwrap();
    run(&mut f, plan, "obligation-0").await.unwrap();
    let record = f.worker.take_validation_attempts().pop().unwrap();
    assert!(
        serde_json::to_vec(&record.discovered.as_ref().unwrap().plan)
            .unwrap()
            .len()
            > 65_536
    );
    // This shared fixture uses the actual ordinary admission, not a fabricated
    // run-resume capability. Feed its genuine enrollment event to the exact
    // refusal used by recover_session; full native resume is not claimed.
    let (_, event) = events(&f)
        .into_iter()
        .find(|(_, event)| event["kind"] == "validation_plan")
        .unwrap();
    assert_eq!(
        crate::worker::validation::refuse_cold_history(&event)
            .unwrap_err()
            .code,
        "validation_recovery_pending"
    );
}

#[tokio::test]
async fn known_service_failure_is_sticky_even_after_real_cleanup_ack() {
    let mut f = Fixture::with_bundle(
        worker_bundle::Mode::ReadinessFailure,
        WorkerContract::new_configured,
    );
    let read_id = read(&mut f, "context_read", json!({"key":"issue"})).await;
    let configured = &f.input.workflow.front_matter.checks.local["suite"];
    let mut requirement = requirement("suite");
    requirement["argv"] = json!(configured.argv);
    requirement["services"] = json!(configured.services);
    let proposed = proposal(&f, &[read_id], None, vec![requirement]);
    let plan = f.worker.call(&proposed).await.unwrap()["plan_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(run(&mut f, &plan, "suite").await.is_err());
    assert_eq!(validation_receipts(&f)[0].phase, ReceiptPhase::Acknowledged);
    assert!(f.executions.active_operations().unwrap().is_empty());
    assert!(f.worker.validation_history_error().is_some());
    assert!(f.worker.take_validation_attempts().is_empty());
    assert!(f.worker.take_verified_checks().is_empty());
    assert!(f.worker.take_failed_checks().is_empty());
}

#[tokio::test]
async fn revised_plan_settlement_write_failure_keeps_original_pending_owner_and_old_evidence() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let old = enroll(&mut f).await;
    run(&mut f, &old, "behavior").await.unwrap();
    let reference = read(&mut f, "context_read", json!({"key":"issue"})).await;
    let proposed = proposal(
        &f,
        &[reference],
        Some(&old),
        vec![requirement("behavior"), requirement("additional")],
    );
    let next = *f.worker.journal.next.lock().unwrap();
    // Actual sequence: tool_started, validation_plan, tool_finished, then
    // validation_plan_settled. Fail only the last write, after real enrollment.
    std::fs::create_dir(
        f.worker
            .journal
            .dir
            .path()
            .join(format!("event-{:08}.json", next + 3)),
    )
    .unwrap();
    assert!(f.worker.call(&proposed).await.is_err());
    assert!(f.worker.validation_history_error().is_some());
    assert_eq!(retained_plan(&mut f).digest(), old);
    assert_eq!(f.worker.take_validation_attempts().len(), 1);
    assert_eq!(f.worker.take_verified_checks().len(), 1);
    assert!(f.worker.validation_history_error().is_some());
}

#[tokio::test]
async fn original_worker_seal_refuses_omitted_failure_and_changed_plan_or_uncertainty() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let plan_digest = enroll(&mut f).await;
    run(&mut f, &plan_digest, "behavior").await.unwrap();
    f.boundary.fail.store(true, Ordering::SeqCst);
    run(&mut f, &plan_digest, "behavior").await.unwrap();
    let seal = f.worker.seal_validation_history();
    let error = f.worker.validation_history_error();
    let plan = retained_plan(&mut f);
    let records = f.worker.take_validation_attempts();
    seal.verify(f.permit.operation_id, Some(&plan), &records, error.as_ref())
        .unwrap();
    let passes: Vec<_> = records.iter().filter(|r| r.successful).cloned().collect();
    assert_eq!(passes.len(), 1);
    assert_eq!(
        seal.verify(f.permit.operation_id, Some(&plan), &passes, error.as_ref())
            .unwrap_err()
            .code,
        "validation_history_binding"
    );
    assert!(
        seal.verify(f.permit.operation_id, None, &records, error.as_ref())
            .is_err()
    );
    assert!(
        seal.verify(Uuid::new_v4(), Some(&plan), &records, error.as_ref())
            .is_err()
    );
    assert!(
        seal.verify(
            f.permit.operation_id,
            Some(&plan),
            &records,
            Some(&Error::new("changed", "caller supplied uncertainty"))
        )
        .is_err()
    );
}

#[tokio::test]
async fn actual_assignment_envelope_selects_only_known_required_catalog_names() {
    let f = Fixture::new(AgentRole::Implementation);
    let proposal: crate::validation::ValidationProposal = serde_json::from_value(
        proposal(
            &f,
            &[bytes_digest(b"shape only")],
            None,
            vec![requirement("other")],
        )
        .arguments,
    )
    .unwrap();
    let context = json!({"evidence":{"delivery_assignment":{"validation_assignment":{"configured_checks":["suite"]}}}});
    assert_eq!(
        crate::worker::validation::validate_assignment_requirements(
            &f.input.workflow,
            &f.input.isolation,
            &context,
            &proposal
        )
        .unwrap_err()
        .code,
        "validation_plan"
    );
    let empty = json!({"evidence":{"delivery_assignment":{"validation_assignment":{"configured_checks":[]}}},"validation_assignment":{"configured_checks":["suite"]}});
    crate::worker::validation::validate_assignment_requirements(
        &f.input.workflow,
        &f.input.isolation,
        &empty,
        &proposal,
    )
    .unwrap();
}

#[tokio::test]
async fn protected_outcome_binds_actual_history_and_old_absence_serializes_unchanged() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let plan = enroll(&mut f).await;
    run(&mut f, &plan, "behavior").await.unwrap();
    let expected = f.worker.seal_validation_history().digest().unwrap();
    let call = f.call(
        "outcome_submit",
        json!({"claim":{"summary":"component result"},"artifact_refs":[]}),
    );
    f.worker.call(&call).await.unwrap();
    f.state.notification("turn/completed", &json!({"threadId":"thread","turn":{"id":"turn","status":"completed","items":[],"error":null}})).unwrap();
    f.worker.checkpoint(&f.state, None).unwrap();
    let actual = f.sink.0.lock().unwrap()[0].clone();
    assert_eq!(actual["validation_history_digest"], expected);
    assert_eq!(
        f.worker.seal_validation_history().digest().unwrap(),
        expected
    );
    let mut historical = actual;
    historical
        .as_object_mut()
        .unwrap()
        .remove("validation_history_digest");
    let decoded: WorkerOutcome = serde_json::from_value(historical.clone()).unwrap();
    assert!(decoded.validation_history_digest.is_none());
    assert_eq!(serde_json::to_value(decoded).unwrap(), historical);
}

#[tokio::test]
async fn read_reference_page_obeys_byte_bound_with_long_actual_paths() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let mut relative = PathBuf::new();
    for index in 0..4 {
        relative.push(format!("{index}{}", "p".repeat(179)));
    }
    let parent = f.input.isolation.plan().repository.join(&relative);
    std::fs::create_dir_all(&parent).unwrap();
    std::fs::write(parent.join("read.txt"), b"actual read bytes").unwrap();
    relative.push("read.txt");
    for _ in 0..20 {
        read(&mut f, "source_read", json!({"path":relative})).await;
    }
    let index = f.worker.validation_evidence().unwrap();
    let shown = index["reads"].as_array().unwrap().len();
    assert!(shown > 0 && shown < 16);
    assert_eq!(index["total_reads"], 20);
    assert_eq!(index["omitted_reads"], 20 - shown);
    assert!(serde_json::to_vec(&index).unwrap().len() <= 65_536);
}

#[tokio::test]
async fn discovered_service_bundle_joins_all_writers_before_artifact_and_transcript_capture() {
    let mut f = Fixture::with_bundle(worker_bundle::Mode::Success, WorkerContract::new_configured);
    let reference = read(&mut f, "source_read", json!({"path":"a.txt"})).await;
    let configured = &f.input.workflow.front_matter.checks.local["suite"];
    let mut obligation = requirement("suite");
    obligation["argv"] = json!(configured.argv);
    obligation["services"] = json!(configured.services);
    obligation["artifacts"] = json!([configured.result_artifact]);
    let proposed = proposal(&f, &[reference], None, vec![obligation]);
    let plan = f.worker.call(&proposed).await.unwrap()["plan_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    let result = run(&mut f, &plan, "suite").await.unwrap();
    assert_eq!(result["successful"], true);
    assert_eq!(
        f.boundary.bundle.as_ref().unwrap().events(),
        vec![
            "service:start",
            "readiness:start",
            "readiness:joined",
            "check:start",
            "check:joined",
            "service:stop",
            "service:joined",
        ]
    );
    assert_eq!(validation_receipts(&f)[0].phase, ReceiptPhase::Acknowledged);
    assert!(f.executions.active_operations().unwrap().is_empty());
    assert!(f.worker.validation_history_error().is_none());
    let records = f.worker.take_validation_attempts();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    record.validate_shape().unwrap();
    assert_eq!(record.schema_version, 4);
    let provenance = record.discovered.as_ref().unwrap();
    assert_eq!(provenance.plan_digest, plan);
    assert_eq!(provenance.service_observations.len(), 1);
    assert_eq!(record.bundle.as_ref().unwrap().services.len(), 1);
    let output = f.input.isolation.plan().scratch.join("check.json");
    let bytes = std::fs::read(&output).unwrap();
    assert_eq!(bytes, b"written-during-service-stop\n");
    assert!(
        record
            .artifacts
            .iter()
            .any(|artifact| artifact.path == output && artifact.digest == bytes_digest(&bytes))
    );
    let transcript = std::fs::read(&provenance.transcript).unwrap();
    assert!(!transcript.is_empty());
    assert!(
        record
            .artifacts
            .iter()
            .any(|artifact| artifact.path == provenance.transcript
                && artifact.digest == bytes_digest(&transcript))
    );
    assert!(
        events(&f)
            .iter()
            .any(|(_, event)| event["kind"] == "validation_attempt_ack")
    );
    let checks = f.worker.take_verified_checks();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].validation_plan().unwrap().digest(), plan);
    assert!(checks[0].production_evidence().is_err());
}

#[test]
fn integrated_scope_uses_retained_assignment_vector_and_cannot_be_requested_as_a_kind() {
    use crate::worker::validation::classify_discovery_scope;
    // Pure classification of host-bound data. This does not construct a native
    // assignment, validation plan, source snapshot or execution capability.
    let vector = bytes_digest(b"component candidate vector identity");
    let context = json!({"evidence":{"delivery_assignment":{"validation_assignment":{"kind":"integrated","vector":vector,"configured_checks":[]}}}});
    assert_eq!(
        classify_discovery_scope(
            AgentRole::Validation,
            "parent_integrated_validation",
            ExecutionRole::Validation,
            Some(&vector),
            &context
        )
        .unwrap(),
        CheckKind::Integrated
    );
    for (role, phase, execution, supplied) in [
        (
            AgentRole::Implementation,
            "parent_integrated_validation",
            ExecutionRole::Validation,
            Some(vector.as_str()),
        ),
        (
            AgentRole::Validation,
            "work",
            ExecutionRole::Validation,
            Some(vector.as_str()),
        ),
        (
            AgentRole::Validation,
            "parent_integrated_validation",
            ExecutionRole::Child,
            Some(vector.as_str()),
        ),
        (
            AgentRole::Validation,
            "parent_integrated_validation",
            ExecutionRole::Validation,
            None,
        ),
    ] {
        assert!(classify_discovery_scope(role, phase, execution, supplied, &context).is_err());
    }
    let changed = bytes_digest(b"different actual vector");
    assert!(
        classify_discovery_scope(
            AgentRole::Validation,
            "parent_integrated_validation",
            ExecutionRole::Validation,
            Some(&changed),
            &context
        )
        .is_err()
    );
    assert!(classify_discovery_scope(AgentRole::Validation, "parent_integrated_validation", ExecutionRole::Validation, Some(&vector), &json!({"validation_assignment":context["evidence"]["delivery_assignment"]["validation_assignment"]})).is_err());
}

#[tokio::test]
async fn additional_source_read_uses_actual_host_repository_and_settled_snapshot_reference() {
    let mut f = Fixture::with_read_repository(AgentRole::Implementation);
    let sources = f.call("context_read", json!({"key":"read_sources"}));
    let listed = f.worker.call(&sources).await.unwrap();
    assert!(
        listed["repositories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|source| source["repository_id"] == "43" && source["primary"] == false)
    );
    let call = f.call(
        "source_read",
        json!({"repository_id":"43","path":"notes.txt"}),
    );
    let actual = f.worker.call(&call).await.unwrap();
    assert_eq!(actual["repository_id"], "43");
    assert_eq!(actual["content"], "actual additional repository input\n");
    assert_eq!(
        actual["digest"],
        bytes_digest(b"actual additional repository input\n")
    );
    let index = f.worker.validation_evidence().unwrap();
    let reference: DiscoveryReadReference =
        serde_json::from_value(index["reads"][0]["reference"].clone()).unwrap();
    let crate::validation::DiscoveryReadContent::Source { path, .. } = &reference.content else {
        panic!("source callback must retain actual bytes");
    };
    assert_eq!(
        path,
        &f.input
            .read_resources
            .keys()
            .next()
            .unwrap()
            .join("notes.txt")
    );
    let proposed = proposal(
        &f,
        &[reference.digest().unwrap()],
        None,
        vec![requirement("cross-repository-obligation")],
    );
    let result = f.worker.call(&proposed).await.unwrap();
    let plan = retained_plan(&mut f);
    assert_eq!(plan.digest(), result["plan_digest"]);
    assert_eq!(plan.record().kind, CheckKind::Local); // no invented integrated capability
    assert_eq!(
        plan.record().supporting_reads[0].digest().unwrap(),
        reference.digest().unwrap()
    );
    assert_eq!(
        plan.record().original_binding,
        f.execution.as_ref().unwrap().binding
    );
}

#[tokio::test]
async fn additional_read_mutation_and_caller_chosen_roots_cannot_enroll() {
    let mut f = Fixture::with_read_repository(AgentRole::Implementation);
    let reference = read(
        &mut f,
        "source_read",
        json!({"repository_id":"43","path":"notes.txt"}),
    )
    .await;
    let root = f.input.read_resources.keys().next().unwrap().clone();
    let outside = f.call(
        "source_read",
        json!({"repository_id":root,"path":"notes.txt"}),
    );
    assert_eq!(
        f.worker.call(&outside).await.unwrap_err().code,
        "scope_violation"
    );
    let traversal = f.call(
        "source_read",
        json!({"repository_id":"43","path":"../operator/a.txt"}),
    );
    assert_eq!(
        f.worker.call(&traversal).await.unwrap_err().code,
        "scope_violation"
    );
    std::fs::write(root.join("notes.txt"), b"changed host input\n").unwrap();
    let proposed = proposal(&f, &[reference], None, vec![requirement("behavior")]);
    assert!(f.worker.call(&proposed).await.is_err());
    assert!(f.worker.take_validation_plan().is_none());
    assert!(validation_receipts(&f).is_empty());
    assert!(f.boundary.last_cancel.lock().unwrap().is_none());
}

#[tokio::test]
async fn readonly_review_can_read_an_original_additional_repository_without_execution() {
    let mut f = Fixture::with_read_repository(AgentRole::Review);
    let call = f.call(
        "source_read",
        json!({"repository_id":"43","path":"notes.txt"}),
    );
    assert_eq!(
        f.worker.call(&call).await.unwrap()["content"],
        "actual additional repository input\n"
    );
    let command = f.call("command_run", json!({"argv":["/usr/bin/true"],"cwd":"."}));
    assert_eq!(
        f.worker.call(&command).await.unwrap_err().code,
        "unsupported_tool"
    );
    let write = f.call("source_write", json!({"path":"notes.txt","expected_digest":null,"content":"must not write","executable":false}));
    assert_eq!(
        f.worker.call(&write).await.unwrap_err().code,
        "unsupported_tool"
    );
    assert_eq!(f.worker.validation_evidence().unwrap()["total_reads"], 0);
    assert!(f.execution.is_none());
}

#[tokio::test]
async fn integrated_selected_constraints_use_integrated_table_and_keep_primary_cwd() {
    let f = Fixture::with_bundle(worker_bundle::Mode::Success, WorkerContract::new_configured);
    let mut obligation = requirement("e2e");
    obligation["argv"] = json!(f.input.workflow.front_matter.checks.integrated["e2e"].argv);
    let proposed: crate::validation::ValidationProposal = serde_json::from_value(
        proposal(
            &f,
            &[bytes_digest(b"schema-only reference")],
            None,
            vec![obligation],
        )
        .arguments,
    )
    .unwrap();
    let context = json!({"evidence":{"delivery_assignment":{"validation_assignment":{"kind":"integrated","configured_checks":["e2e"]}}}});
    crate::worker::validation::validate_assignment_requirements(
        &f.input.workflow,
        &f.input.isolation,
        &context,
        &proposed,
    )
    .unwrap();
    let mut changed = proposed.clone();
    changed.requirements[0].argv.push("changed".into());
    assert!(
        crate::worker::validation::validate_assignment_requirements(
            &f.input.workflow,
            &f.input.isolation,
            &context,
            &changed
        )
        .is_err()
    );
    changed = proposed;
    let cwd = f.input.isolation.plan().repository.join("another-cwd");
    std::fs::create_dir(&cwd).unwrap();
    changed.requirements[0].cwd = cwd.to_string_lossy().into_owned();
    assert!(
        crate::worker::validation::validate_assignment_requirements(
            &f.input.workflow,
            &f.input.isolation,
            &context,
            &changed
        )
        .is_err()
    );
}

// This helper moves actual worker-owned capabilities into the same descriptive
// completion fields as worker_run. It does not assert a native or joined run.
fn take_artifact_completion(f: &mut Fixture) -> crate::worker_run::RunCompletion {
    let validation_history = Some(f.worker.seal_validation_history());
    let validation_error = f.worker.validation_history_error();
    crate::worker_run::RunCompletion {
        operation_id: f.permit.operation_id,
        state: f.state.clone(),
        outcome: crate::worker_run::RunOutcome::Completed,
        error: None,
        elapsed_ms: 0,
        hook_error: None,
        checks: f.worker.take_verified_checks(),
        failed_checks: f.worker.take_failed_checks(),
        validation_plan: f.worker.take_validation_plan(),
        validation_attempts: f.worker.take_validation_attempts(),
        validation_artifacts: f.worker.take_validation_artifacts(),
        validation_error,
        validation_history,
        execution: f.execution.clone(),
        contract: Some(f.contract.clone()),
        local_exit_confirmed: false,
    }
}
async fn enroll_artifact(f: &mut Fixture, missing: bool) -> String {
    let reference = read(f, "source_read", json!({"path":"a.txt"})).await;
    let mut obligation = requirement("behavior");
    let mut artifacts = vec![f.input.isolation.plan().scratch.join("check.json")];
    if missing {
        artifacts.push(f.input.isolation.plan().scratch.join("absent.json"));
    }
    obligation["artifacts"] = json!(artifacts);
    let proposed = proposal(f, &[reference], None, vec![obligation]);
    f.worker.call(&proposed).await.unwrap()["plan_digest"]
        .as_str()
        .unwrap()
        .into()
}

#[tokio::test]
async fn retained_failed_and_superseded_output_bytes_survive_overwrite_and_scratch_removal() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let plan = enroll_artifact(&mut f, false).await;
    f.boundary.fail.store(true, Ordering::SeqCst);
    run(&mut f, &plan, "behavior").await.unwrap();
    let path = f.input.isolation.plan().scratch.join("check.json");
    let failed_bytes = std::fs::read(&path).unwrap();
    let reference = read(&mut f, "context_read", json!({"key":"issue"})).await;
    let mut obligation = requirement("behavior");
    obligation["artifacts"] = json!([path]);
    let revised = proposal(&f, &[reference], Some(&plan), vec![obligation]);
    let current = f.worker.call(&revised).await.unwrap()["plan_digest"]
        .as_str()
        .unwrap()
        .to_owned();
    f.boundary.fail.store(false, Ordering::SeqCst);
    run(&mut f, &current, "behavior").await.unwrap();
    let passed_bytes = std::fs::read(&path).unwrap();
    assert_ne!(failed_bytes, passed_bytes);
    let mut completion = take_artifact_completion(&mut f);
    assert_eq!(
        completion.validation_plan.as_ref().unwrap().history().len(),
        2
    );
    assert_eq!(completion.validation_attempts.len(), 2);
    let failed = completion
        .validation_attempts
        .iter()
        .find(|a| !a.successful)
        .unwrap();
    let failed_proof = failed
        .artifacts
        .iter()
        .find(|a| a.path == path)
        .unwrap()
        .clone();
    let passed = completion
        .validation_attempts
        .iter()
        .find(|a| a.successful)
        .unwrap();
    let passed_proof = passed
        .artifacts
        .iter()
        .find(|a| a.path == path)
        .unwrap()
        .clone();
    std::fs::remove_dir_all(f.input.isolation.plan().scratch.clone()).unwrap();
    assert_eq!(
        completion
            .validation_artifact_reader()
            .unwrap()
            .read(&failed_proof)
            .unwrap(),
        failed_bytes
    );
    assert_eq!(
        completion
            .validation_artifact_reader()
            .unwrap()
            .read(&passed_proof)
            .unwrap(),
        passed_bytes
    );
    assert_eq!(
        completion.checks[0].production_evidence().unwrap_err().code,
        "unqualified_fixture"
    );
    let actual = completion.validation_artifacts.pop().unwrap();
    assert_eq!(
        completion.validation_artifact_reader().err().unwrap().code,
        "validation_history_binding"
    );
    completion.validation_artifacts.push(actual);
    let mut foreign = failed_proof.clone();
    foreign.validation_operation = Uuid::new_v4();
    assert_eq!(
        completion
            .validation_artifact_reader()
            .unwrap()
            .read(&foreign)
            .unwrap_err()
            .code,
        "artifact_conflict"
    );
    completion.validation_attempts.retain(|a| a.successful);
    assert_eq!(
        completion.validation_artifact_reader().err().unwrap().code,
        "validation_history_binding"
    );
}

#[tokio::test]
async fn retained_nonrepair_artifacts_require_original_bytes_not_current_scratch_fallback() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let plan = enroll_artifact(&mut f, true).await;
    assert_eq!(
        run(&mut f, &plan, "behavior").await.unwrap()["successful"],
        false
    );
    let completion = take_artifact_completion(&mut f);
    let record = &completion.validation_attempts[0];
    assert_eq!(record.command.exit_code, Some(0));
    assert!(!record.successful); // Missing artifact cannot become a repair trigger.
    let proof = record
        .artifacts
        .iter()
        .find(|a| a.path.ends_with("check.json"))
        .unwrap();
    let expected = std::fs::read(&proof.path).unwrap();
    assert_eq!(
        completion
            .validation_artifact_reader()
            .unwrap()
            .read(proof)
            .unwrap(),
        expected
    );
    let retained = completion
        .validation_artifacts
        .iter()
        .find(|a| a.matches_reference(proof))
        .unwrap();
    let chunk = &retained.retained_paths()[0];
    std::fs::write(chunk, b"different retained bytes").unwrap();
    assert_eq!(
        completion
            .validation_artifact_reader()
            .unwrap()
            .read(proof)
            .unwrap_err()
            .code,
        "artifact_conflict"
    );
    std::fs::remove_file(chunk).unwrap();
    assert!(
        completion
            .validation_artifact_reader()
            .unwrap()
            .read(proof)
            .is_err()
    );
    assert_eq!(
        std::fs::read(&proof.path).unwrap(),
        expected,
        "intact scratch is not a fallback capability"
    );
}

#[tokio::test]
async fn retained_artifacts_survive_ack_persistence_failure_with_sticky_history_error() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let plan = enroll_artifact(&mut f, false).await;
    let first = *f.worker.journal.next.lock().unwrap();
    run(&mut f, &plan, "behavior").await.unwrap();
    // Use the actual preceding identical invocation's chunk sequence to locate
    // the ACK, and assert the second invocation reaches this exact boundary.
    let ack_leaf = events(&f)
        .into_iter()
        .find(|(_, e)| e["kind"] == "validation_attempt_ack")
        .unwrap()
        .0;
    let ack_index: u64 = ack_leaf
        .strip_prefix("event-")
        .unwrap()
        .strip_suffix(".json")
        .unwrap()
        .parse()
        .unwrap();
    let offset = ack_index - first;
    let next = *f.worker.journal.next.lock().unwrap();
    std::fs::create_dir(
        f.worker
            .journal
            .dir
            .path()
            .join(format!("event-{:08}.json", next + offset)),
    )
    .unwrap();
    assert!(run(&mut f, &plan, "behavior").await.is_err());
    assert!(f.worker.validation_history_error().is_some());
    assert_eq!(*f.worker.journal.next.lock().unwrap(), next + offset);
    assert_eq!(validation_receipts(&f).len(), 2);
    assert!(
        validation_receipts(&f)
            .iter()
            .all(|r| r.phase == ReceiptPhase::Acknowledged)
    );
    let completion = take_artifact_completion(&mut f);
    assert_eq!(completion.validation_attempts.len(), 2);
    assert!(completion.validation_error.is_some());
    for proof in completion
        .validation_attempts
        .iter()
        .flat_map(|a| &a.artifacts)
    {
        assert_eq!(
            bytes_digest(
                &completion
                    .validation_artifact_reader()
                    .unwrap()
                    .read(proof)
                    .unwrap()
            ),
            proof.digest
        );
    }
}
