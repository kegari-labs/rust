//! Real admission, worker and result acceptance with synthetic Codex JSON-RPC.
//! No installed Codex, repository command or live provider is invoked.
#[path = "support/pinned_git.rs"]
mod pinned_git;
include!("support/worker_fixture.rs");

async fn enroll_native_check(f: &mut Fixture) -> String {
    let read = f.call("context_read", json!({"key":"issue"}));
    assert_eq!(f.worker.call(&read).await.unwrap()["body"], "fixture");
    let evidence = f.call("context_read", json!({"key":"validation_evidence"}));
    let references = f.worker.call(&evidence).await.unwrap();
    let reference = references["reads"][0]["id"].as_str().unwrap();
    let proposal = f.call(
        "validation_propose",
        json!({
            "previous":null,"reason":"Check the actual assigned behavior",
            "supporting_reads":[reference],"requirements":[{
                "id":"behavior","statement":"The assigned behavior is valid",
                "rationale":"The admitted task requires this behavior",
                "argv":["/usr/bin/true"],"cwd":".","services":[],"artifacts":[]
            }]
        }),
    );
    f.worker.call(&proposal).await.unwrap()["plan_digest"]
        .as_str()
        .unwrap()
        .into()
}

fn native_frame(f: &Fixture, item_id: &str, completed: bool, exit: Option<i32>) -> Value {
    json!({"method":if completed {"item/completed"} else {"item/started"},
    "params":{"threadId":"thread","turnId":"turn","item":{
        "type":"commandExecution","id":item_id,"command":"/usr/bin/true",
        "cwd":f.input.isolation.plan().cwd,
        "status":if completed {"completed"} else {"inProgress"},
        "commandActions":[],"exitCode":exit,
        "aggregatedOutput":if completed {Some("actual synthetic Codex output")} else {None},
        "durationMs":if completed {Some(4)} else {None},
        "processId":null,"source":"agent"
    }}})
}

async fn pump_frames(f: &mut Fixture, frames: &[Value]) -> Result<()> {
    let mut bytes = Vec::new();
    for frame in frames {
        bytes.extend(serde_json::to_vec(frame).unwrap());
        bytes.push(b'\n');
    }
    let mut rpc = RpcClient::from_transport(
        std::io::Cursor::new(bytes),
        tokio::io::sink(),
        StdDuration::from_secs(2),
    )
    .unwrap();
    for _ in frames {
        rpc.pump(
            &mut f.state,
            &mut f.worker,
            StdDuration::from_secs(2),
            StdDuration::from_secs(2),
        )
        .await?;
    }
    Ok(())
}

fn record_call(f: &Fixture, plan: &str, item: &str) -> ToolCall {
    f.call(
        "validation_record",
        json!({"plan_digest":plan,"obligation_id":"behavior","item_id":item}),
    )
}

#[tokio::test]
async fn admitted_child_accepts_actual_codex_result_and_records_completed_outcome() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let repository = &f.input.isolation.plan().repository;
    std::fs::write(repository.join(".gitignore"), "generated-output/\n").unwrap();
    let plan = enroll_native_check(&mut f).await;
    let started = native_frame(&f, "native-check", false, None);
    pump_frames(&mut f, &[started]).await.unwrap();
    // Ordinary ignored build products are not changed repository source. The
    // companion stale-result case still refuses an actual tracked-file edit.
    let generated = f.input.isolation.plan().repository.join("generated-output");
    std::fs::create_dir(&generated).unwrap();
    std::fs::write(generated.join("result.bin"), "synthetic generated output").unwrap();
    let completed = native_frame(&f, "native-check", true, Some(0));
    pump_frames(&mut f, &[completed]).await.unwrap();
    let results = f.call("context_read", json!({"key":"execution_results"}));
    let observed = f.worker.call(&results).await.unwrap();
    assert_eq!(observed["execution_layer"], "codex_app_server");
    assert_eq!(observed["entries"][0]["item_id"], "native-check");
    assert_eq!(observed["entries"][0]["exit_code"], 0);
    assert_eq!(observed["entries"][0]["source_unchanged"], true);
    let record = record_call(&f, &plan, "native-check");
    let result = f.worker.call(&record).await.unwrap();
    assert_eq!(result["successful"], true);
    assert_eq!(f.worker.call(&record).await.unwrap(), result);
    let outcome = f.call(
        "outcome_submit",
        json!({"claim":{
        "schema_version":1,"summary":"Actual observed native check passed",
        "fulfilled_criteria":["The assigned behavior is valid"],
        "disposition":{"kind":"no_change","reason":"The exact source satisfies the task"}
    },"artifact_refs":[]}),
    );
    assert_eq!(
        f.worker.call(&outcome).await.unwrap()["state"],
        "pending_host_verification"
    );
    assert!(f.sink.0.lock().unwrap().is_empty());
    let pending = f.worker.observe().unwrap();
    assert!(pending.state.get("evaluation_child_proposal").is_none());
    assert!(pending.summary.is_none());
    pump_frames(
        &mut f,
        &[
            json!({"method":"turn/completed","params":{"threadId":"thread","turn":{
                "id":"turn","status":"completed","items":[],"error":null
            }}}),
        ],
    )
    .await
    .unwrap();
    let completed = f.worker.observe().unwrap();
    let proposal = &completed.state["evaluation_child_proposal"];
    assert_eq!(proposal["kind"], "untrusted_completed_child_proposal");
    assert_eq!(proposal["thread"], "thread");
    assert_eq!(proposal["completed_turn"], "turn");
    assert_eq!(proposal["source_revision"], completed.revision);
    assert_eq!(proposal["candidate_digest"], completed.candidate_digest);
    assert_eq!(
        proposal["claim"]["summary"],
        "Actual observed native check passed"
    );
    assert!(completed.summary.is_none());
    let checks = f.worker.take_verified_checks();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].record().schema_version, 5);
    let provenance = checks[0].record().codex.as_ref().unwrap();
    assert_eq!(provenance.thread_id, "thread");
    assert_eq!(provenance.turn_id, "turn");
    assert_eq!(provenance.item_id, "native-check");
    assert_eq!(
        checks[0].record().command.stdout,
        "actual synthetic Codex output"
    );
    assert_eq!(f.sink.0.lock().unwrap().len(), 1);
    assert!(f.executions.active_operations().unwrap().is_empty());
}

#[tokio::test]
async fn missing_unknown_and_stale_native_results_cannot_satisfy_checks() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let plan = enroll_native_check(&mut f).await;
    let missing = record_call(&f, &plan, "never-observed");
    assert_eq!(
        f.worker.call(&missing).await.unwrap_err().code,
        "execution_result_missing"
    );
    let frames = [
        native_frame(&f, "unknown-exit", false, None),
        native_frame(&f, "unknown-exit", true, None),
    ];
    pump_frames(&mut f, &frames).await.unwrap();
    let unknown = record_call(&f, &plan, "unknown-exit");
    assert_eq!(
        f.worker.call(&unknown).await.unwrap_err().code,
        "codex_result_incomplete"
    );
    let frames = [
        native_frame(&f, "stale-source", false, None),
        native_frame(&f, "stale-source", true, Some(0)),
    ];
    pump_frames(&mut f, &frames).await.unwrap();
    std::fs::write(
        f.input.isolation.plan().repository.join("a.txt"),
        "new source after check\n",
    )
    .unwrap();
    let stale = record_call(&f, &plan, "stale-source");
    assert_eq!(
        f.worker.call(&stale).await.unwrap_err().code,
        "stale_execution_result"
    );
    assert!(f.worker.take_verified_checks().is_empty());
    assert!(f.sink.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn configured_native_artifacts_require_fresh_bytes_and_preserve_failed_outputs() {
    let mut f = Fixture::new(AgentRole::Implementation);
    let output = f.input.isolation.plan().scratch.join("check.json");
    std::fs::write(&output, b"same successful report").unwrap();
    let frames = [
        native_frame(&f, "stale-artifact", false, None),
        native_frame(&f, "stale-artifact", true, Some(0)),
    ];
    pump_frames(&mut f, &frames).await.unwrap();
    let stale = f.call(
        "validation_record",
        json!({"name":"suite","kind":"local","item_id":"stale-artifact"}),
    );
    assert_eq!(
        f.worker.call(&stale).await.unwrap_err().code,
        "artifact_stale"
    );

    let start = native_frame(&f, "fresh-artifact", false, None);
    pump_frames(&mut f, &[start]).await.unwrap();
    // Same-content atomic replacement supplies an actual new file identity;
    // freshness does not require different test output or a timing sleep.
    let replacement = output.with_extension("replacement");
    std::fs::write(&replacement, b"same successful report").unwrap();
    std::fs::rename(&replacement, &output).unwrap();
    let completed = native_frame(&f, "fresh-artifact", true, Some(0));
    pump_frames(&mut f, &[completed]).await.unwrap();
    let accept = f.call(
        "validation_record",
        json!({"name":"suite","kind":"local","item_id":"fresh-artifact"}),
    );
    assert_eq!(f.worker.call(&accept).await.unwrap()["successful"], true);
    let successful = f.worker.take_verified_checks();
    assert_eq!(successful.len(), 1);
    let proof = successful[0].record().codex.as_ref().unwrap();
    assert_eq!(proof.schema_version, 2);
    assert_eq!(proof.artifact_freshness.len(), 1);
    assert_eq!(proof.artifact_freshness[0].path, output);
    assert_eq!(
        f.worker.observe().unwrap().checks["suite"].status,
        kegari_v0::evaluation::CheckStatus::Passed
    );

    let start = native_frame(&f, "failed-artifact", false, None);
    pump_frames(&mut f, &[start]).await.unwrap();
    std::fs::write(&output, b"actual failed report").unwrap();
    let completed = native_frame(&f, "failed-artifact", true, Some(7));
    pump_frames(&mut f, &[completed]).await.unwrap();
    let accept = f.call(
        "validation_record",
        json!({"name":"suite","kind":"local","item_id":"failed-artifact"}),
    );
    assert_eq!(f.worker.call(&accept).await.unwrap()["successful"], false);
    let failed = f.worker.take_failed_checks();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].record().command.exit_code, Some(7));
    for artifact in successful[0]
        .record()
        .artifacts
        .iter()
        .chain(&failed[0].record().artifacts)
    {
        if artifact.path.exists() {
            std::fs::remove_file(&artifact.path).unwrap();
        }
    }
    // Original successful and failed artifacts are retained outside worker
    // roots; observation needs no fallback to overwritten/deleted scratch.
    let observed = f.worker.observe().unwrap();
    assert_eq!(
        observed.checks["suite"].status,
        kegari_v0::evaluation::CheckStatus::Failed
    );
    assert_eq!(
        observed.artifacts[&output.to_string_lossy().into_owned()].digest,
        kegari_digest::bytes_digest(b"actual failed report")
    );
}

#[tokio::test]
async fn native_result_pages_preserve_complete_output_and_exact_observation_binding() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let long_output = format!(
        "{}{}",
        "ordinary diagnostic\n".repeat(4000),
        "a€\u{0001}".repeat(3000)
    );
    for index in 0..5 {
        let item = format!("paged-{index}");
        let start = native_frame(&f, &item, false, None);
        let mut completed = native_frame(&f, &item, true, Some(7));
        completed["params"]["item"]["aggregatedOutput"] = if index == 4 {
            json!(long_output)
        } else {
            json!("bounded diagnostic\n".repeat(1100))
        };
        pump_frames(&mut f, &[start, completed]).await.unwrap();
    }
    let mut offset = 0;
    let mut results_digest = Value::Null;
    let mut names = BTreeSet::new();
    let mut output_cursor = Value::Null;
    loop {
        let mut arguments = json!({"key":"execution_results","offset":offset});
        if !results_digest.is_null() {
            arguments["results_digest"] = results_digest.clone();
        }
        let call = f.call("context_read", arguments);
        let page = f.worker.call(&call).await.unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() <= 65_536);
        assert_eq!(page["total"], 5);
        results_digest = page["results_digest"].clone();
        for entry in page["entries"].as_array().unwrap() {
            assert!(names.insert(entry["item_id"].as_str().unwrap().to_owned()));
            if entry["item_id"] == "paged-4" {
                output_cursor = entry["output_cursor"].clone();
                assert!(entry["output"].is_object());
            }
        }
        let Some(next) = page["next_offset"].as_u64() else {
            break;
        };
        assert!(next > offset);
        offset = next;
    }
    assert_eq!(names.len(), 5);
    assert!(offset > 0, "the result inventory itself must span pages");
    let mut offset = 0;
    let mut reconstructed = String::new();
    loop {
        let call = f.call(
            "context_read",
            json!({"key":"execution_results",
            "output_cursor":output_cursor,"offset":offset}),
        );
        let page = f.worker.call(&call).await.unwrap();
        assert!(serde_json::to_vec(&page).unwrap().len() <= 65_536);
        assert_eq!(page["output_cursor"], output_cursor);
        assert_eq!(page["total_bytes"], long_output.len());
        let text = page["page"]["data"].as_str().unwrap();
        assert_eq!(
            page["page_digest"],
            kegari_digest::bytes_digest(text.as_bytes())
        );
        reconstructed.push_str(text);
        let Some(next) = page["next_offset"].as_u64() else {
            break;
        };
        assert_eq!(next as usize, reconstructed.len());
        offset = next;
    }
    assert_eq!(reconstructed, long_output);
    assert_eq!(
        output_cursor["output_digest"],
        kegari_digest::bytes_digest(long_output.as_bytes())
    );
    for field in ["source_digest", "output_digest"] {
        let mut substituted = output_cursor.clone();
        substituted[field] =
            json!("sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff");
        let call = f.call(
            "context_read",
            json!({"key":"execution_results",
            "output_cursor":substituted,"offset":0}),
        );
        assert_eq!(
            f.worker.call(&call).await.unwrap_err().code,
            "stale_context"
        );
    }
    let mut foreign = output_cursor.clone();
    foreign["turn_id"] = json!("another-turn");
    let call = f.call(
        "context_read",
        json!({"key":"execution_results","output_cursor":foreign,"offset":0}),
    );
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "execution_result_missing"
    );
    let inside_scalar = long_output.find('€').unwrap() + 1;
    let call = f.call(
        "context_read",
        json!({"key":"execution_results",
        "output_cursor":output_cursor,"offset":inside_scalar}),
    );
    assert_eq!(f.worker.call(&call).await.unwrap_err().code, "invalid_page");
    let frames = [
        native_frame(&f, "new-result", false, None),
        native_frame(&f, "new-result", true, Some(0)),
    ];
    pump_frames(&mut f, &frames).await.unwrap();
    let call = f.call(
        "context_read",
        json!({"key":"execution_results","offset":1,"results_digest":results_digest}),
    );
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "stale_context"
    );
    // Reading a failed command's bytes never creates successful evidence.
    assert!(f.worker.take_verified_checks().is_empty());
    assert!(f.worker.take_failed_checks().is_empty());
    assert!(f.sink.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn completed_child_blocker_retains_unrun_obligations_without_claiming_completion() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    enroll_native_check(&mut f).await;
    let outcome = f.call(
        "outcome_submit",
        json!({"claim":{
        "schema_version":1,"summary":"The required fixture capability is unavailable",
        "fulfilled_criteria":[],"disposition":{"kind":"blocked",
            "reason":"Required boundary auditing capability is absent",
            "next_action":"The permitted fixture owner must supply that capability"}
    },"artifact_refs":[]}),
    );
    f.worker.call(&outcome).await.unwrap();
    assert!(
        f.worker
            .observe()
            .unwrap()
            .state
            .get("evaluation_child_blocker")
            .is_none()
    );
    pump_frames(
        &mut f,
        &[
            json!({"method":"turn/completed","params":{"threadId":"thread","turn":{
                "id":"turn","status":"completed","items":[],"error":null
            }}}),
        ],
    )
    .await
    .unwrap();
    let observed = f.worker.observe().unwrap();
    assert!(observed.state.get("evaluation_child_proposal").is_none());
    assert_eq!(observed.state["evaluation_child_blocker"]["reported"], true);
    assert_eq!(
        observed.state["evaluation_child_blocker"]["proposed"],
        false
    );
    assert_eq!(
        observed.state["evaluation_child_blocker"]["claim"]["fulfilled_criteria"],
        json!([])
    );
    assert_eq!(
        observed.state["evaluation_child_blocker"]["completed_turn"],
        "turn"
    );
    assert_eq!(
        observed.checks["behavior"].status,
        kegari_v0::evaluation::CheckStatus::NotRun
    );
    assert!(observed.checks["behavior"].receipt_digest.is_none());
    assert!(f.worker.take_verified_checks().is_empty());
    assert_eq!(f.sink.0.lock().unwrap().len(), 1);
}
