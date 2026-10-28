//! Production composition with deterministic provider/process observations only.
//! Does not qualify OS confinement or launch any model/provider action.
use kegari_types::run::RunReference;
#[path = "support/pinned_git.rs"]
mod pinned_git;
include!("support/worker_fixture.rs");
use kegari_digest::{bytes_digest, canonical_digest};

#[test]
fn roles_expose_no_implementation_or_execution_to_parent_or_review() {
    for role in [
        AgentRole::Coordination,
        AgentRole::Review,
        AgentRole::IndependentReview,
    ] {
        let tools = tool_definitions(role, true);
        assert!(
            !tools
                .iter()
                .any(|(e, _)| e.capability == ToolCapability::SourceWrite
                    || e.capability == ToolCapability::ValidationResult
                    || e.capability == ToolCapability::ValidationProposal)
        );
        assert!(!role.permits(ToolCapability::SourceWrite));
    }
    assert!(AgentRole::Testing.permits(ToolCapability::SourceWrite));
}
#[tokio::test]
async fn actual_context_is_bounded_and_tool_schema_is_closed() {
    let mut f = Fixture::new(AgentRole::Coordination);
    let call = f.call("context_read", json!({"key":"issue"}));
    assert_eq!(f.worker.call(&call).await.unwrap()["body"], "fixture");
    let call = f.call("context_read", json!({"key":"issue","shell":"whoami"}));
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "invalid_input"
    );
    let call = f.call("source_write", json!({}));
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "unsupported_tool"
    );
}
#[tokio::test]
async fn source_reader_rejects_control_traversal_links_and_git() {
    let mut f = Fixture::new(AgentRole::Coordination);
    let call = f.call("source_read", json!({"path":"a.txt"}));
    assert_eq!(f.worker.call(&call).await.unwrap()["content"], "original\n");
    for path in ["../control/intent.json", ".git/config", "/etc/passwd"] {
        let call = f.call("source_read", json!({"path":path}));
        assert!(f.worker.call(&call).await.is_err());
    }
    let repo = f
        .contract
        .instructions()
        .manifest
        .repository_root
        .as_ref()
        .unwrap();
    std::os::unix::fs::symlink("/etc/passwd", repo.join("linked")).unwrap();
    let call = f.call("source_read", json!({"path":"linked"}));
    assert!(f.worker.call(&call).await.is_err());
}
#[tokio::test]
async fn revoked_owner_fenced_permit_and_stale_turn_prevent_effects() {
    let mut f = Fixture::new(AgentRole::Implementation);
    let call=f.call("source_write",json!({"path":"a.txt","expected_digest":bytes_digest(b"original\n"),"content":"changed","executable":false}));
    f.host.revoked.store(true, Ordering::SeqCst);
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "access_denied"
    );
    f.host.revoked.store(false, Ordering::SeqCst);
    f.host.owner.store(false, Ordering::SeqCst);
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "ownership_conflict"
    );
    f.host.owner.store(true, Ordering::SeqCst);
    let mut stale = call.clone();
    stale.turn_id = "old".into();
    assert_eq!(
        f.worker.call(&stale).await.unwrap_err().code,
        "stale_result"
    );
    f.control.fence(id(4)).unwrap();
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "access_denied"
    );
}
#[tokio::test]
async fn actual_source_write_is_scoped_journaled_and_idempotent() {
    let mut f = Fixture::new(AgentRole::Implementation);
    let mut call=f.call("source_write",json!({"path":"a.txt","expected_digest":bytes_digest(b"original\n"),"content":"changed\n","executable":true}));
    let first = f.worker.call(&call).await.unwrap();
    assert_eq!(first["completed"], true);
    assert_eq!(f.worker.call(&call).await.unwrap(), first);
    let repo = f
        .contract
        .instructions()
        .manifest
        .repository_root
        .as_ref()
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.join("a.txt")).unwrap(),
        "changed\n"
    );
    call.arguments["content"] = json!("another");
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "request_conflict"
    );
}
#[tokio::test]
async fn out_of_scope_source_and_operator_tool_names_are_rejected() {
    let mut f = Fixture::new(AgentRole::Implementation);
    for name in ["approve", "release", "provider_mutate", "shell"] {
        let call = f.call(name, json!({"roles":["release"]}));
        assert_eq!(
            f.worker.call(&call).await.unwrap_err().code,
            "unsupported_tool"
        );
    }
    let call = f.call(
        "source_write",
        json!({"path":"WORKFLOW.md","expected_digest":null,"content":"attack","executable":false}),
    );
    assert!(f.worker.call(&call).await.is_err());
}
#[tokio::test]
async fn proposals_are_not_completion_and_require_current_completed_turn() {
    let mut f = Fixture::new(AgentRole::Coordination);
    let call = f.call(
        "outcome_submit",
        json!({"claim":{"all_passed":true},"artifact_refs":[]}),
    );
    let result = f.worker.call(&call).await.unwrap();
    assert_eq!(result["state"], "pending_host_verification");
    assert!(f.sink.0.lock().unwrap().is_empty());
    assert_eq!(f.worker.observe().unwrap().state["delivery"], "pending");
    f.state.notification("turn/completed",&json!({"threadId":"thread","turn":{"id":"turn","status":"completed","items":[],"error":null}})).unwrap();
    f.worker.checkpoint(&f.state, None).unwrap();
    assert_eq!(f.sink.0.lock().unwrap().len(), 1);
    f.worker.checkpoint(&f.state, None).unwrap();
    assert_eq!(f.sink.0.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn unknown_recovery_and_pending_requests_do_not_become_clean() {
    let mut f = Fixture::new(AgentRole::Coordination);
    assert_eq!(
        f.worker
            .reconcile_prior(Some(&json!({"status":"unknown"})))
            .unwrap_err()
            .code,
        "reconciliation_required"
    );
    f.worker
        .checkpoint(&f.state, Some("interrupted_unknown"))
        .unwrap();
    let call = f.call("context_read", json!({"key":"issue"}));
    assert_eq!(f.worker.call(&call).await.unwrap_err().code, "stale_result");
    let mut state = f.state.clone();
    state.binding.account_id = "other-account".into();
    assert_eq!(
        f.worker.checkpoint(&state, None).unwrap_err().code,
        "worker_binding"
    );
}
#[tokio::test]
async fn correction_proposal_cannot_expand_children_or_claim_consumption() {
    let mut f = Fixture::new(AgentRole::Coordination);
    let call=f.call("correction_propose",json!({"source_comment":id(100),"source_revision":"edited-1","children":[id(12)],"message":"fix it"}));
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "scope_violation"
    );
    let call=f.call("correction_propose",json!({"source_comment":id(100),"source_revision":"edited-1","children":[id(11)],"message":"fix it"}));
    assert_eq!(
        f.worker.call(&call).await.unwrap()["state"],
        "pending_host_verification"
    );
    assert_eq!(f.state.correction_consumed, 0);
}
#[tokio::test]
async fn configured_check_uses_executor_and_never_mints_fixture_production_proof() {
    let mut f = Fixture::new(AgentRole::Implementation);
    let call = f.call("validation_run", json!({"name":"suite","kind":"local"}));
    let result = f.worker.call(&call).await.unwrap();
    assert_eq!(result["successful"], true);
    let checks = f.worker.take_verified_checks();
    assert_eq!(checks.len(), 1);
    assert_eq!(
        checks[0].production_evidence().unwrap_err().code,
        "unqualified_fixture"
    );
    assert_eq!(
        f.worker.observe().unwrap().checks["suite"].status,
        kegari_v0::evaluation::CheckStatus::Passed
    );
    let call = f.call("validation_run", json!({"name":"suite","kind":"local"}));
    assert!(
        f.worker.call(&call).await.unwrap()["successful"]
            .as_bool()
            .unwrap()
    );
}

#[tokio::test]
async fn evaluation_observation_cannot_relabel_old_checks_after_source_changes() {
    let mut f = Fixture::new(AgentRole::Implementation);
    let run = f.call("validation_run", json!({"name":"suite","kind":"local"}));
    f.worker.call(&run).await.unwrap();
    let before = f.worker.observe().unwrap();
    assert_eq!(
        before.checks["suite"].status,
        kegari_v0::evaluation::CheckStatus::Passed
    );
    assert_eq!(
        before.checks["suite"].candidate_digest,
        before.candidate_digest
    );
    let edit = f.call("source_write", json!({"path":"a.txt","expected_digest":bytes_digest(b"original\n"),"content":"changed after validation\n","executable":false}));
    f.worker.call(&edit).await.unwrap();
    let after = f.worker.observe().unwrap();
    assert_ne!(before.candidate_digest, after.candidate_digest);
    assert_eq!(
        after.checks["suite"].candidate_digest,
        before.candidate_digest
    );
    assert_eq!(
        after.checks["suite"].status,
        kegari_v0::evaluation::CheckStatus::Unknown
    );
    let rerun = f.call("validation_run", json!({"name":"suite","kind":"local"}));
    // Changed source requires the next host-sealed exact revision. A retry in
    // this original binding must not invent passing validation for dirty files.
    assert_eq!(
        f.worker.call(&rerun).await.unwrap_err().code,
        "dirty_execution"
    );
    let still_unverified = f.worker.observe().unwrap();
    assert_eq!(
        still_unverified.checks["suite"].status,
        kegari_v0::evaluation::CheckStatus::Unknown
    );
    assert_eq!(
        still_unverified.checks["suite"].candidate_digest,
        before.candidate_digest
    );
    assert_eq!(still_unverified.candidate_digest, after.candidate_digest);
}

#[tokio::test]
async fn evaluation_artifacts_require_intact_private_retained_bytes() {
    let mut f = Fixture::new(AgentRole::Implementation);
    let run = f.call("validation_run", json!({"name":"suite","kind":"local"}));
    f.worker.call(&run).await.unwrap();
    let observed = f.worker.observe().unwrap();
    assert!(!observed.artifacts.is_empty());
    let private = f._temp.path().join("control/worker");
    let archived = std::fs::read_dir(&private)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("artifact-")
        })
        .unwrap();
    std::fs::write(archived, b"altered retained fixture").unwrap();
    assert_eq!(f.worker.observe().unwrap_err().code, "artifact_conflict");
}

#[tokio::test]
async fn failed_check_is_retained_as_failed_and_corrected_rerun_keeps_old_bytes() {
    let mut f = Fixture::new(AgentRole::Implementation);
    f.boundary.fail.store(true, Ordering::SeqCst);
    let call = f.call("validation_run", json!({"name":"suite","kind":"local"}));
    assert_eq!(f.worker.call(&call).await.unwrap()["successful"], false);
    assert!(f.worker.take_verified_checks().is_empty());
    assert_eq!(
        f.worker.observe().unwrap().checks["suite"].status,
        kegari_v0::evaluation::CheckStatus::Failed
    );
    assert!(
        f.control
            .snapshot()
            .unwrap()
            .unresolved_operations
            .is_empty()
    );
    f.boundary.fail.store(false, Ordering::SeqCst);
    let call = f.call("validation_run", json!({"name":"suite","kind":"local"}));
    assert_eq!(f.worker.call(&call).await.unwrap()["successful"], true);
    assert_eq!(f.worker.take_verified_checks().len(), 1);
    let failures = f.worker.take_failed_checks();
    assert_eq!(failures.len(), 1);
    assert!(!failures[0].record().successful);
    assert_eq!(
        failures[0].record().binding.work,
        f.contract.authority().work().unwrap().clone()
    );
    assert!(f.worker.take_failed_checks().is_empty());
}
#[tokio::test]
async fn independent_review_has_no_linear_authority_or_execution_and_pins_pr() {
    let mut f = Fixture::new(AgentRole::IndependentReview);
    assert!(f.contract.authority().work().is_none());
    for name in [
        "source_write",
        "validation_run",
        "correction_propose",
        "git_inspect",
    ] {
        let call = f.call(name, json!({}));
        assert_eq!(
            f.worker.call(&call).await.unwrap_err().code,
            "unsupported_tool"
        );
    }
    let mut input = f.input.clone();
    input.review.as_mut().unwrap().head = "1".repeat(40);
    assert!(WorkerContract::new(input).is_err());
    let call = f.call(
        "outcome_submit",
        json!({"claim":{"clean":true},"artifact_refs":[]}),
    );
    f.worker.call(&call).await.unwrap();
    f.state.notification("turn/completed",&json!({"threadId":"thread","turn":{"id":"turn","status":"completed","items":[],"error":null}})).unwrap();
    f.worker.checkpoint(&f.state, None).unwrap();
    let records = f.sink.0.lock().unwrap();
    assert!(records[0]["work"].is_null());
    assert_eq!(records[0]["independent_review"]["pull_request"], 1);
}
#[test]
fn changed_tools_profiles_or_authorized_write_paths_cannot_rebind_contract() {
    let f = Fixture::new(AgentRole::Implementation);
    let mut input = f.input.clone();
    input.assembled.manifest.tools[0].name = "shell".into();
    input.assembled.manifest_digest = canonical_digest(1, &input.assembled.manifest).unwrap();
    assert!(WorkerContract::new(input).is_err());
    let mut input = f.input.clone();
    input.assembled.manifest.profile.model = "other".into();
    input.assembled.manifest_digest = canonical_digest(1, &input.assembled.manifest).unwrap();
    assert!(WorkerContract::new(input).is_err());
    let mut input = f.input.clone();
    let mut plan = input.isolation.plan().clone();
    plan.source_writes = vec![PathGrant {
        path: plan.repository.clone(),
        directory: true,
    }];
    input.isolation = plan.validate().unwrap();
    input.assembled.manifest.permissions.source_write_paths =
        vec![input.isolation.plan().repository.clone()];
    input.assembled.manifest.permissions.enforcement_profile = input.isolation.digest().into();
    input.assembled.manifest_digest = canonical_digest(1, &input.assembled.manifest).unwrap();
    assert_eq!(
        WorkerContract::new(input).unwrap_err().code,
        "scope_violation"
    );
    assert_eq!(
        f.worker
            .validate_live_operation(
                f.contract.instructions(),
                &f.input.isolation,
                f.contract.tools(),
                &f.contract.work_key().unwrap()
            )
            .unwrap_err()
            .code,
        "unqualified_fixture"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn force_is_durable_and_status_remains_responsive_during_long_check() {
    let mut f = Fixture::new(AgentRole::Implementation);
    f.boundary.block.store(true, Ordering::SeqCst);
    let call = f.call("validation_run", json!({"name":"suite","kind":"local"}));
    let control = f.control.clone();
    let executions = f.executions.clone();
    let task = tokio::spawn(async move {
        let result = f.worker.call(&call).await;
        (f, result)
    });
    tokio::time::timeout(StdDuration::from_secs(10), async {
        while executions.active_operations().unwrap().is_empty() {
            tokio::time::sleep(StdDuration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let status_control = control.clone();
    let status = tokio::time::timeout(
        StdDuration::from_millis(250),
        tokio::task::spawn_blocking(move || status_control.snapshot()),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    assert_eq!(
        executions.cancel_fenced(&control, id(4)).unwrap_err().code,
        "access_denied"
    );
    control
        .lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: id(1),
                actor: Actor {
                    principal: None,
                    identity: "fixture-operator".into(),
                    roles: BTreeSet::from(["lifecycle".into()]),
                },
                kind: "force_stop".into(),
                canonical_targets: vec![id(1).to_string()],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            Lifecycle::ForceStop,
            status.intent.generation,
        )
        .unwrap();
    assert_eq!(control.snapshot().unwrap().intent.desired, Intent::Stopped);
    assert_eq!(executions.cancel_fenced(&control, id(4)).unwrap(), 1);
    let (mut f, result) = tokio::time::timeout(StdDuration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
    assert!(f.worker.take_verified_checks().is_empty());
    assert!(executions.active_operations().unwrap().is_empty());
    assert!(
        f.control
            .snapshot()
            .unwrap()
            .unresolved_operations
            .is_empty()
    );
}

#[tokio::test]
async fn adapter_observation_binds_interleaved_tool_before_start_response() {
    let mut f = Fixture::new(AgentRole::Coordination);
    f.state.turn_id = None;
    f.worker.checkpoint(&f.state, Some("turn/start")).unwrap();
    let mut call = f.call("context_read", json!({"key":"issue"}));
    call.turn_id = "interleaved-turn".into();
    assert_eq!(f.worker.call(&call).await.unwrap_err().code, "stale_result");
    f.state.turn_id = Some("interleaved-turn".into());
    f.worker.observe_session(&f.state).unwrap();
    assert_eq!(f.worker.call(&call).await.unwrap()["body"], "fixture");
    f.worker
        .checkpoint(&f.state, Some("interrupted_unknown"))
        .unwrap();
    assert_eq!(f.worker.call(&call).await.unwrap_err().code, "stale_result");
}

fn run_preparation(f: &Fixture) -> kegari_v0::worker_run::Preparation {
    let proof = json!({"workflow_ref":"fixture/repo/.github/workflows/caller.yml@refs/heads/trusted","workflow_sha":"0123456789012345678901234567890123456789","job_workflow_ref":"fixture/trusted/.github/workflows/proof.yml@0123456789012345678901234567890123456789","job_workflow_sha":"0123456789012345678901234567890123456789","event_name":"workflow_dispatch"});
    let auth = f._temp.path().canonicalize().unwrap().join("auth");
    let config: kegari_config::Config = serde_json::from_value(json!({
        "schema_version":1,"runtime":{"id":id(1),"host_id":"fixture-host","max_agents":2},
        "connections":{
            "linear":{"kind":"linear","account_id":"linear-fixture","endpoint":"https://linear.invalid/graphql","credential_ref":"keychain:fixture-linear"},
            "github":{"kind":"github","account_id":"github-fixture","endpoint":"https://github.invalid/api","credential_ref":"keychain:fixture-github"},
            "codex":{"kind":"codex","account_id":"fixture-account","auth_home":auth,"max_agents":2,"argv":["/usr/bin/true","app-server","--listen","stdio://"]}
        },
        "workspaces":{"fixture":{
            "id":id(3),"linear_workspace_id":id(2),"linear_connection":"linear","github_connection":"github","codex_connection":"codex","workflow":f.input.workflow.path,
            "linear":{"teams":"all","projects":"all","users":"all","authorization_label_id":id(20),"states":{"team":{"prepared":id(30),"running":id(31),"blocked":id(32),"child_done":id(33),"parent_accepted":id(34),"cancelled":id(35)}}},
            "github":{"repositories":["42"]},"actors":{"authorizers":[id(5)],"correctors":[id(5)],"acceptors":[id(5)]},
            "profiles":{"coordination":profile()},"routing_labels":{"tool":{id(40).to_string():"codex"},"model":{id(41).to_string():"fixture-model"},"reasoning":{id(42).to_string():"high"},"speed":{id(43).to_string():"standard"}},
            "max_agents":2,"release":{"mode":"manual","spacing_seconds":0},
            "repositories":{"42":{"source":f._temp.path().join("operator"),"local_checks":["suite"],"integrated_checks":["e2e"],"ci":[{"workflow_id":1,"required_checks":["suite"],"proof":proof}],"production_ci":[{"workflow_id":2,"required_checks":["suite"],"proof":proof}],"no_change_validation":{"workflow_id":1,"workflow_revision":"0123456789012345678901234567890123456789","source_input":"source_sha","operation_input":"operation_id","observed_source_output":"tested_sha","proof":proof},"deployment":"none"}}
        }},"control":{"local_users":{"501":{"id":"fixture-operator","linear_user_id":id(5),"roles":["inspect","configure","lifecycle","correct","accept","release","recover"]}}}
    })).unwrap();
    config.validate().unwrap();
    let mut parent = f.parent.clone();
    parent.config_revision = kegari_v0::config::config_revision(&config).unwrap();
    let child = f.input.assembled.manifest.role.permits_source_writes();
    let gate = parent
        .fresh_gate(
            child.then_some(id(11)),
            if child {
                GatePurpose::NewOperation
            } else {
                GatePurpose::Coordination
            },
            &DependencyOutcomes::default(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    let mut isolation = f.input.isolation.plan().clone();
    if f.input
        .assembled
        .manifest
        .role
        .permits_repository_execution()
    {
        isolation.managed_execution = Some(BoundManagedExecution::automatic(&isolation).unwrap());
    }
    kegari_v0::worker_run::Preparation {
        config: Arc::new(config),
        workflow: f.input.workflow.clone(),
        authority: WorkerAuthority::Authorized(gate),
        operation_id: id(4),
        role: f.input.assembled.manifest.role,
        phase: "work".into(),
        context: f.input.context.clone(),
        isolation,
        repository: Some(RepositoryContext {
            root: f.input.isolation.plan().repository.clone(),
            revision: f
                .input
                .assembled
                .manifest
                .repository_revision
                .clone()
                .unwrap(),
            scope_paths: vec!["a.txt".into()],
        }),
        repository_id: f.input.repository.clone(),
        read_resources: BTreeMap::new(),
        review: None,
        required_context: vec![],
        project_doc_max_bytes: 32768,
        provider_discovery: ProviderDiscovery::Unavailable {
            reason: "deterministic fixture; no actual Codex discovery".into(),
        },
        known_secrets: vec![],
    }
}

#[test]
fn worker_run_preparation_assembles_exact_current_profile_and_tools_without_launch() {
    let f = Fixture::new(AgentRole::Implementation);
    let prepared = kegari_v0::worker_run::prepare(run_preparation(&f)).unwrap();
    assert_eq!(prepared.contract().account_id(), "fixture-account");
    assert_eq!(
        prepared.contract().instructions().manifest.profile,
        profile()
    );
    prepared
        .contract()
        .tools()
        .verify(
            prepared.contract().instructions(),
            prepared.contract().tools().definitions(),
        )
        .unwrap();
    assert_eq!(
        prepared.permissions().isolation_digest,
        prepared.isolation().digest()
    );
    assert_eq!(
        prepared.contract().instructions().manifest.runtime_build,
        kegari_v0::BUILD_ID
    );
}

#[test]
fn worker_run_preparation_rejects_changed_configuration_workflow_and_scope() {
    let f = Fixture::new(AgentRole::Implementation);
    let mut input = run_preparation(&f);
    Arc::make_mut(&mut input.config).runtime.max_turns += 1;
    assert_eq!(
        kegari_v0::worker_run::prepare(input).err().unwrap().code,
        "stale_config"
    );
    let mut input = run_preparation(&f);
    input.isolation.source_writes.push(PathGrant {
        path: f._temp.path().canonicalize().unwrap().join("operator"),
        directory: true,
    });
    assert!(kegari_v0::worker_run::prepare(input).is_err());
    let input = run_preparation(&f);
    std::fs::write(
        input.workflow.path.as_deref().unwrap(),
        "---\nschema_version: 1\n---\nChanged\n",
    )
    .unwrap();
    assert_eq!(
        kegari_v0::worker_run::prepare(input).err().unwrap().code,
        "stale_config"
    );
}

#[tokio::test]
async fn nonexecuting_hooks_reject_execution_role_and_revoked_owner() {
    use kegari_v0::worker_run::{NonExecutingRunHooks, RunHooks};
    let child = Fixture::new(AgentRole::Implementation);
    assert_eq!(
        NonExecutingRunHooks::new(
            child.contract.clone(),
            child.control.clone(),
            child.host.clone()
        )
        .err()
        .unwrap()
        .code,
        "scope_violation"
    );
    let coordination = Fixture::new(AgentRole::Coordination);
    let hooks = NonExecutingRunHooks::new(
        coordination.contract.clone(),
        coordination.control.clone(),
        coordination.host.clone(),
    )
    .unwrap();
    hooks.before(&coordination.permit).await.unwrap();
    coordination.host.owner.store(false, Ordering::SeqCst);
    assert_eq!(
        hooks.launched(&coordination.permit).await.unwrap_err().code,
        "ownership_conflict"
    );
}

#[tokio::test]
async fn fenced_nonexecuting_after_hook_records_inapplicability_without_new_authority() {
    use kegari_v0::worker_run::{AfterHookObservation, NonExecutingRunHooks, RunHooks};
    let f = Fixture::new(AgentRole::Coordination);
    let hooks =
        NonExecutingRunHooks::new(f.contract.clone(), f.control.clone(), f.host.clone()).unwrap();
    hooks.before(&f.permit).await.unwrap();
    hooks.launched(&f.permit).await.unwrap();
    f.control.fence(f.permit.operation_id).unwrap();
    f.host.owner.store(false, Ordering::SeqCst);
    let receipts = f.control.operations().unwrap().len();
    assert_eq!(
        hooks.after(&f.permit, &f.state).await.unwrap(),
        AfterHookObservation::NotApplicable
    );
    assert_eq!(f.control.operations().unwrap().len(), receipts);
    assert!(f.control.validate_permit(&f.permit, || Ok(())).is_err());
    let mut other = f.state.clone();
    other.binding.operation_id = Uuid::new_v4().to_string();
    assert_eq!(
        hooks.after(&f.permit, &other).await.unwrap_err().code,
        "worker_binding"
    );
}

async fn initialized_hook_records(f: &Fixture) -> PrivateDir {
    let dir = PrivateDir::open(&f._temp.path().canonicalize().unwrap().join("control"))
        .unwrap()
        .child("hooks")
        .unwrap();
    let execution = f.execution.as_ref().unwrap();
    let executor = LocalExecutor {
        isolation: &f.input.isolation,
        git: &execution.git,
        boundary: execution.boundary.as_ref(),
        limits: execution.limits.clone(),
        ownership: &execution.ownership,
    };
    let mut ledger = HookLedger::new(&execution.binding).unwrap();
    executor
        .run_hook(
            &mut ledger,
            HookInvocation {
                binding: &execution.binding,
                workflow: &f.input.workflow,
                phase: HookPhase::AfterCreate,
                session: None,
                eligible_cleanup_plan: None,
            },
            |ledger| dir.write("hooks.json", ledger),
        )
        .await
        .unwrap();
    assert!(ledger.initialized);
    dir
}
fn executor_hooks(f: &Fixture, records: PrivateDir) -> kegari_v0::worker_run::ExecutorRunHooks {
    kegari_v0::worker_run::ExecutorRunHooks::new(kegari_v0::worker_run::ExecutorHookServices {
        contract: f.contract.clone(),
        control: f.control.clone(),
        host: f.host.clone(),
        execution: f.execution.as_ref().unwrap().clone(),
        registry: f.executions.clone(),
        records,
        session_id: id(99),
    })
    .unwrap()
}
#[tokio::test]
async fn worker_hooks_require_actual_creation_and_record_actual_session_lifecycle() {
    use kegari_v0::worker_run::RunHooks;
    let f = Fixture::with_hooks(
        AgentRole::Implementation,
        "hooks:\n  before_run: 'true'\n  after_run: 'true'\n",
    );
    let records = initialized_hook_records(&f).await;
    let path = records.path().to_owned();
    let hooks = executor_hooks(&f, records);
    assert_eq!(
        hooks.launched(&f.permit).await.unwrap_err().code,
        "hook_gate"
    );
    hooks.before(&f.permit).await.unwrap();
    hooks.launched(&f.permit).await.unwrap();
    hooks.after(&f.permit, &f.state).await.unwrap();
    let ledger: HookLedger = PrivateDir::open(&path).unwrap().read("hooks.json").unwrap();
    assert_eq!(ledger.sessions[&id(99)], SessionLifecycle::Ended);
    assert_eq!(
        ledger.records[&format!("BeforeRun:{}", id(99))].state,
        HookState::Succeeded
    );
    assert_eq!(
        ledger.records[&format!("AfterRun:{}", id(99))].state,
        HookState::Succeeded
    );
    assert_eq!(
        hooks.before(&f.permit).await.unwrap_err().code,
        "reconciliation_required"
    );
    assert!(f.executions.active_operations().unwrap().is_empty());
}

#[tokio::test]
async fn failed_worker_before_hook_never_records_launched_session() {
    use kegari_v0::worker_run::RunHooks;
    let f = Fixture::with_hooks(AgentRole::Implementation, "hooks:\n  before_run: 'false'\n");
    let records = initialized_hook_records(&f).await;
    let path = records.path().to_owned();
    let hooks = executor_hooks(&f, records);
    f.boundary.fail.store(true, Ordering::SeqCst);
    assert_eq!(
        hooks.before(&f.permit).await.unwrap_err().code,
        "hook_failed"
    );
    assert_eq!(
        hooks.launched(&f.permit).await.unwrap_err().code,
        "hook_gate"
    );
    hooks.after(&f.permit, &f.state).await.unwrap();
    let ledger: HookLedger = PrivateDir::open(&path).unwrap().read("hooks.json").unwrap();
    assert!(ledger.sessions.is_empty());
    assert!(!ledger.records.keys().any(|key| key.starts_with("AfterRun")));
}

#[tokio::test]
async fn fenced_after_hook_preserves_absent_and_configured_distinctions() {
    use kegari_v0::worker_run::{AfterHookObservation, AfterHookSkip, RunHooks};
    for configured in [false, true] {
        let f = Fixture::with_hooks(
            AgentRole::Implementation,
            if configured {
                "hooks:\n  after_run: 'true'\n"
            } else {
                ""
            },
        );
        let records = initialized_hook_records(&f).await;
        let path = records.path().to_owned();
        let hooks = executor_hooks(&f, records);
        hooks.before(&f.permit).await.unwrap();
        hooks.launched(&f.permit).await.unwrap();
        f.control.fence(f.permit.operation_id).unwrap();
        let receipts = f.control.operations().unwrap().len();
        let outcome = hooks.after(&f.permit, &f.state).await;
        let ledger: HookLedger = PrivateDir::open(&path).unwrap().read("hooks.json").unwrap();
        assert_eq!(ledger.sessions[&id(99)], SessionLifecycle::Ended);
        let key = format!("AfterRun:{}", id(99));
        if configured {
            assert_eq!(outcome.unwrap_err().code, "hook_not_run_fenced");
            assert!(!ledger.records.contains_key(&key));
        } else {
            assert_eq!(
                outcome.unwrap(),
                AfterHookObservation::Skipped {
                    reason: AfterHookSkip::HookNotConfigured
                }
            );
            assert_eq!(ledger.records[&key].state, HookState::Skipped);
            assert!(ledger.records[&key].script_digest.is_none());
            assert!(ledger.records[&key].exit_code.is_none());
        }
        assert_eq!(f.control.operations().unwrap().len(), receipts);
        assert!(f.control.validate_permit(&f.permit, || Ok(())).is_err());
        assert!(f.executions.active_operations().unwrap().is_empty());
    }
}

#[tokio::test]
async fn unconfirmed_hook_exit_keeps_owned_handle_and_sent_receipt() {
    use kegari_v0::worker_run::RunHooks;
    let f = Fixture::with_hooks(AgentRole::Implementation, "hooks:\n  before_run: 'true'\n");
    let records = initialized_hook_records(&f).await;
    let path = records.path().to_owned();
    let hooks = executor_hooks(&f, records);
    f.boundary.unknown.store(true, Ordering::SeqCst);
    assert_eq!(
        hooks.before(&f.permit).await.unwrap_err().code,
        "operation_uncertain"
    );
    assert!(f.executions.active_operations().unwrap().contains(&id(4)));
    assert_eq!(f.executions.observe_exited(id(4)).unwrap(), 0);
    let operations = f.control.operations().unwrap();
    let receipt = operations
        .iter()
        .find(|receipt| receipt.kind == "worker_hook")
        .unwrap();
    assert_eq!(receipt.phase, ReceiptPhase::Sent);
    let ledger: HookLedger = PrivateDir::open(&path).unwrap().read("hooks.json").unwrap();
    assert_eq!(
        ledger.records[&format!("BeforeRun:{}", id(99))].state,
        HookState::StartedUnknown
    );
    f.boundary
        .last_cancel
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .exited
        .store(true, Ordering::SeqCst);
    assert_eq!(f.executions.observe_exited(id(4)).unwrap(), 1);
    assert!(f.executions.active_operations().unwrap().is_empty());
    assert_eq!(
        f.control
            .operations()
            .unwrap()
            .iter()
            .find(|item| item.operation_id == receipt.operation_id)
            .unwrap()
            .phase,
        ReceiptPhase::Sent
    );
    assert_eq!(
        hooks.before(&f.permit).await.unwrap_err().code,
        "reconciliation_required"
    );
}

// Explicit fixture metadata: no native process identity qualification is claimed.
struct FixtureProviderProbe {
    marker: PathBuf,
    boot: Uuid,
}
impl kegari_platform::ProcessProbe for FixtureProviderProbe {
    fn current(&self) -> Result<kegari_platform::ProcessIdentity> {
        Ok(kegari_platform::ProcessIdentity {
            pid: std::process::id(),
            parent_pid: 1,
            process_group: std::process::id(),
            uid: unsafe { libc::geteuid() },
            boot_id: self.boot,
            started_seconds: 1,
            started_microseconds: 1,
            executable: "/fixture/host".into(),
        })
    }
    fn observe(
        &self,
        _: &kegari_platform::ProcessIdentity,
    ) -> Result<kegari_platform::ProcessObservation> {
        Ok(kegari_platform::ProcessObservation::Gone)
    }
    fn matching_executable(&self, path: &Path) -> Result<Vec<kegari_platform::ProcessIdentity>> {
        let deadline = std::time::Instant::now() + StdDuration::from_millis(100);
        loop {
            if let Some(pid) = std::fs::read_to_string(&self.marker)
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
            {
                return Ok(vec![kegari_platform::ProcessIdentity {
                    pid,
                    parent_pid: std::process::id(),
                    process_group: pid,
                    uid: unsafe { libc::geteuid() },
                    boot_id: self.boot,
                    started_seconds: 2,
                    started_microseconds: 1,
                    executable: path.into(),
                }]);
            }
            if std::time::Instant::now() >= deadline {
                return Ok(vec![]);
            }
            std::thread::sleep(StdDuration::from_millis(2));
        }
    }
}
fn native_fixture_run(
    f: &Fixture,
) -> (
    kegari_v0::worker_run::PreparedRun,
    Permit,
    kegari_v0::worker_run::RunServices,
    PathBuf,
) {
    use kegari_v0::worker_run::{NonExecutingRunHooks, RunServices};
    use std::os::unix::fs::PermissionsExt;
    let base = f._temp.path().canonicalize().unwrap();
    let executable = base.join("deterministic-app-server");
    let marker = base.join("fixture-provider-started");
    // A disposable protocol/process fixture, never the installed Codex binary.
    // It returns only version metadata and then waits without starting any turn.
    std::fs::write(&executable,format!("#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' '{}'; exit 0; fi\nprintf '%s' $$ > '{}'\nexec /bin/sleep 30\n",kegari_codex::SUPPORTED_CODEX_VERSION,marker.display())).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut input = run_preparation(f);
    Arc::make_mut(&mut input.config)
        .connections
        .get_mut("codex")
        .unwrap()
        .argv[0] = executable.to_string_lossy().into_owned();
    let mut parent = f.parent.clone();
    parent.config_revision = kegari_v0::config::config_revision(&input.config).unwrap();
    input.authority = WorkerAuthority::Authorized(
        parent
            .fresh_gate(
                None,
                GatePurpose::Coordination,
                &DependencyOutcomes::default(),
                Utc::now(),
                Duration::seconds(30),
            )
            .unwrap(),
    );
    input.operation_id = id(100);
    input.context["operation"]["id"] = json!(id(100));
    let prepared = kegari_v0::worker_run::prepare(input).unwrap();
    *f.host.authority.lock().unwrap() = prepared.contract().authority().clone();
    f.control
        .retire_local(id(4), json!({"fixture":"no process launched"}))
        .unwrap();
    let permit = f
        .control
        .admit_run(
            &f.permit.lease,
            RunAdmission {
                request_id: id(100),
                work_key: prepared.contract().authority().work_key().unwrap(),
                scope_digest: prepared.contract().scope_digest().into(),
                reference: RunReference {
                    normalized_state: Some("in progress".into()),
                    workspace: id(3),
                    account: "fixture-account".into(),
                    phase: kegari_types::run::PhaseKey {
                        work: prepared.contract().work_key().unwrap(),
                        generation: canonical_digest(
                            1,
                            &prepared.contract().authority().work().unwrap().generation,
                        )
                        .unwrap(),
                        phase: "work".into(),
                        retry_grant: None,
                    },
                    profile_digest: canonical_digest(1, &profile()).unwrap(),
                    instruction_digest: prepared.contract().instructions().manifest_digest.clone(),
                    max_attempts: 2,
                    max_turns: 1,
                    max_review_rounds: 2,
                    attempt: 1,
                },
            },
            || Ok(()),
        )
        .unwrap();
    let services = RunServices {
        processes: Arc::new(
            kegari_v0::process_records::ProcessJournal::with_probe(
                PrivateDir::open(&base.join("control"))
                    .unwrap()
                    .child("worker-processes")
                    .unwrap(),
                f.permit.lease.runtime_id,
                Arc::new(FixtureProviderProbe {
                    marker: marker.clone(),
                    boot: id(987),
                }),
            )
            .unwrap(),
        ),
        hooks: Arc::new(
            NonExecutingRunHooks::new(
                prepared.contract().clone(),
                f.control.clone(),
                f.host.clone(),
            )
            .unwrap(),
        ),
        worker: WorkerServices {
            control: f.control.clone(),
            providers: ProviderBroker::default(),
            host: f.host.clone(),
            sink: f.sink.clone(),
            execution: None,
            journal: PrivateDir::open(&base.join("control"))
                .unwrap()
                .child("native-fixture-run")
                .unwrap(),
            executions: f.executions.clone(),
        },
    };
    (prepared, permit, services, marker)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_provider_launch_checks_exact_binding_and_closed_or_fenced_scope_before_spawn() {
    for close in [false, true] {
        let f = Fixture::new(AgentRole::Coordination);
        let (prepared, permit, services, marker) = native_fixture_run(&f);
        let processes = services.processes.clone();
        let broker =
            ScopedWorker::new(prepared.contract().clone(), permit.clone(), services.worker)
                .unwrap();
        let base = f._temp.path().canonicalize().unwrap();
        let executable = base.join("deterministic-app-server");
        let auth = base.join("auth");
        let mut wrong = prepared.permissions().clone();
        wrong.profile_id = "wrong".into();
        assert_eq!(
            broker
                .provider_launch(
                    processes.clone(),
                    &executable,
                    &auth,
                    &wrong,
                    StdDuration::from_secs(2)
                )
                .err()
                .unwrap()
                .code,
            "worker_binding"
        );
        let launch = broker
            .provider_launch(
                processes.clone(),
                &executable,
                &auth,
                prepared.permissions(),
                StdDuration::from_secs(2),
            )
            .unwrap();
        launch
            .verify_binding(
                &permit.work_key,
                prepared.contract().instructions(),
                prepared.isolation(),
                prepared.permissions(),
                &executable,
                &auth,
            )
            .unwrap();
        assert_eq!(
            launch
                .verify_binding(
                    "another-work",
                    prepared.contract().instructions(),
                    prepared.isolation(),
                    prepared.permissions(),
                    &executable,
                    &auth
                )
                .unwrap_err()
                .code,
            "worker_binding"
        );
        if close {
            launch.finish().unwrap();
        } else {
            f.control.fence(permit.operation_id).unwrap();
        }
        let result = launch.launch().await.err().unwrap();
        assert_eq!(
            result.code,
            if close {
                "admission_closed"
            } else {
                "access_denied"
            }
        );
        launch.finish().unwrap();
        assert!(processes.observe(permit.operation_id).unwrap().is_empty());
        assert!(f.executions.active_operations().unwrap().is_empty());
        assert!(!marker.exists());
        assert_eq!(
            launch.launch().await.err().unwrap().code,
            "operation_uncertain"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fenced_worker_cannot_start_even_the_disposable_provider_process() {
    let f = Fixture::new(AgentRole::Coordination);
    let (prepared, permit, services, marker) = native_fixture_run(&f);
    f.control.fence(permit.operation_id).unwrap();
    let error = kegari_v0::worker_run::run(
        prepared,
        permit,
        services,
        kegari_v0::worker_run::RunOptions {
            initial_input: "deterministic fixture".into(),
            max_seconds: 5,
            max_turns: 1,
            resume: None,
        },
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "access_denied");
    assert!(!marker.exists());
    assert!(f.executions.active_operations().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_cannot_increase_original_admission_turn_budget() {
    let f = Fixture::new(AgentRole::Coordination);
    let (prepared, permit, services, marker) = native_fixture_run(&f);
    let error = kegari_v0::worker_run::run(
        prepared,
        permit,
        services,
        kegari_v0::worker_run::RunOptions {
            initial_input: "fixture".into(),
            max_seconds: 5,
            max_turns: 2,
            resume: None,
        },
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "admission_budget");
    assert!(!marker.exists());
    assert!(f.executions.active_operations().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forced_worker_joins_actual_fixture_provider_before_confirming_local_exit() {
    let f = Fixture::new(AgentRole::Coordination);
    let (prepared, permit, services, marker) = native_fixture_run(&f);
    let processes = services.processes.clone();
    let operation = permit.operation_id;
    let mut run = kegari_v0::worker_run::run(
        prepared,
        permit,
        services,
        kegari_v0::worker_run::RunOptions {
            initial_input: "deterministic fixture".into(),
            max_seconds: 10,
            max_turns: 1,
            resume: None,
        },
    )
    .unwrap();
    tokio::time::timeout(StdDuration::from_secs(3), async {
        while !marker.exists() {
            tokio::time::sleep(StdDuration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let process_records = processes.observe(operation).unwrap();
    assert_eq!(
        process_records.len(),
        2,
        "both metadata probe and provider are durable"
    );
    assert!(
        process_records
            .iter()
            .any(|p| p.state == kegari_v0::process_records::RecoveryState::GuardHeld)
    );
    let snapshot = f.control.snapshot().unwrap();
    f.control
        .lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: id(1),
                actor: Actor {
                    principal: None,
                    identity: "fixture-operator".into(),
                    roles: BTreeSet::from(["lifecycle".into()]),
                },
                kind: "force-stop".into(),
                canonical_targets: vec![id(1).to_string()],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            Lifecycle::ForceStop,
            snapshot.intent.generation,
        )
        .unwrap();
    run.cancel_fenced(&f.control).unwrap();
    let completion = tokio::time::timeout(StdDuration::from_secs(8), run.join())
        .await
        .unwrap()
        .unwrap();
    assert!(completion.local_exit_confirmed);
    assert!(
        completion.contract.is_some(),
        "actual run retains its immutable contract"
    );
    assert!(
        completion.execution.is_none(),
        "coordination never invents source execution ownership"
    );
    let records = processes.observe(operation).unwrap();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|p| p.state
        == kegari_v0::process_records::RecoveryState::OwnedGroupExitRecorded
        && p.owned_group_exited
        && !p.all_descendants_exited));

    assert!(completion.error.is_some());
    assert!(completion.state.thread_id.is_none());
    assert_eq!(completion.state.turns_started, 0);
    assert!(completion.checks.is_empty());
    assert!(f.executions.active_operations().unwrap().is_empty());
    assert_eq!(
        completion.observation()["completion"],
        "worker_claim_requires_host_delivery_verification"
    );
}

#[test]
fn runtime_launch_request_binds_canonical_work_generation_and_exact_source_before_effects() {
    use kegari_types::run::PhaseKey;
    use kegari_v0::runtime_workers::{LaunchRequest, SourceSelection};
    for role in [
        AgentRole::Implementation,
        AgentRole::Coordination,
        AgentRole::IndependentReview,
    ] {
        let f = Fixture::new(role);
        let authority = f.contract.authority().clone();
        let generation = match &authority {
            WorkerAuthority::Authorized(gate) => {
                kegari_digest::canonical_digest(1, &gate.work().generation).unwrap()
            }
            WorkerAuthority::Independent(gate) => gate.review_identity().into(),
        };
        let source = if role == AgentRole::Coordination {
            None
        } else {
            Some(SourceSelection {
                repository: "42".into(),
                revision: f
                    .execution
                    .as_ref()
                    .map(|execution| execution.binding.revision.clone())
                    .unwrap_or_else(|| {
                        let head = f.input.review.as_ref().unwrap().head.clone();
                        kegari_v0::git::Revision {
                            commit: head,
                            tree: "a".repeat(40),
                        }
                    }),
            })
        };
        let mut request = LaunchRequest {
            delivery: None,
            unfinished: None,
            source_advanced: None,
            retried_unfinished: None,
            validation_retry: None,
            review_retry: None,
            review_round: None,
            lease: f.permit.lease.clone(),
            operation_id: id(810),
            authority,
            phase: PhaseKey {
                work: f.contract.work_key().unwrap(),
                generation,
                phase: "work".into(),
                retry_grant: None,
            },
            source,
            max_seconds: 60,
            max_turns: 2,
        };
        request.validate_binding().unwrap();
        request.phase.work = bytes_digest(b"different canonical issue");
        assert_eq!(
            request.validate_binding().unwrap_err().code,
            "worker_binding"
        );
        request.phase.work = f.contract.work_key().unwrap();
        request.phase.generation = bytes_digest(b"stale generation");
        assert_eq!(
            request.validate_binding().unwrap_err().code,
            "worker_binding"
        );
        request.phase.generation = match &request.authority {
            WorkerAuthority::Authorized(gate) => {
                kegari_digest::canonical_digest(1, &gate.work().generation).unwrap()
            }
            WorkerAuthority::Independent(gate) => gate.review_identity().into(),
        };
        request.source = Some(SourceSelection {
            repository: "999".into(),
            revision: kegari_v0::git::Revision {
                commit: "b".repeat(40),
                tree: "c".repeat(40),
            },
        });
        assert_eq!(
            request.validate_binding().unwrap_err().code,
            "scope_violation"
        );
    }
}

#[test]
fn authorized_resource_write_contract_rejects_unconfigured_flags_and_scope_expansion() {
    use kegari_v0::worker::ReadResource;
    let f = Fixture::new(AgentRole::Testing);
    let mut preparation = run_preparation(&f);
    let resource = f._temp.path().canonicalize().unwrap().join("test-data");
    std::fs::create_dir(&resource).unwrap();
    let grant = PathGrant {
        path: resource.clone(),
        directory: true,
    };
    preparation.isolation.read_roots.push(grant.clone());
    preparation.isolation.resource_writes.push(grant);
    preparation.read_resources.insert(
        resource.clone(),
        ReadResource::TestResource {
            name: "database".into(),
            writable: true,
            disposable: true,
        },
    );
    // A caller-supplied path cannot invent a configured authorized resource.
    assert_eq!(
        kegari_v0::worker_run::prepare(preparation)
            .err()
            .unwrap()
            .code,
        "scope_violation"
    );
    let mut preparation = run_preparation(&f);
    Arc::make_mut(&mut preparation.config)
        .workspaces
        .values_mut()
        .next()
        .unwrap()
        .test_resources
        .insert(
            "database".into(),
            kegari_config::TestResource {
                path: resource.to_str().unwrap().into(),
                writable: true,
                disposable: true,
            },
        );
    let mut parent = f.parent.clone();
    parent.config_revision = kegari_v0::config::config_revision(&preparation.config).unwrap();
    parent
        .children
        .get_mut(&id(11))
        .unwrap()
        .scope
        .as_mut()
        .unwrap()
        .test_resources = vec!["database".into()];
    let gate = parent
        .fresh_gate(
            Some(id(11)),
            GatePurpose::NewOperation,
            &DependencyOutcomes::default(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    preparation.authority = WorkerAuthority::Authorized(gate);
    preparation.isolation.read_roots.push(PathGrant {
        path: resource.clone(),
        directory: true,
    });
    preparation.isolation.resource_writes.push(PathGrant {
        path: resource.clone(),
        directory: true,
    });
    preparation.read_resources.insert(
        resource.clone(),
        ReadResource::TestResource {
            name: "database".into(),
            writable: false,
            disposable: true,
        },
    );
    assert_eq!(
        kegari_v0::worker_run::prepare(preparation.clone())
            .err()
            .unwrap()
            .code,
        "scope_violation"
    );
    preparation.read_resources.insert(
        resource.clone(),
        ReadResource::TestResource {
            name: "database".into(),
            writable: true,
            disposable: true,
        },
    );
    // Rebind the host-derived domain after the actual admitted resource grants
    // are final; the domain does not authorize the configured scope itself.
    preparation.isolation.managed_execution =
        Some(BoundManagedExecution::automatic(&preparation.isolation).unwrap());
    let prepared = kegari_v0::worker_run::prepare(preparation).unwrap();
    assert_eq!(
        prepared
            .contract()
            .instructions()
            .manifest
            .permissions
            .resource_write_paths,
        vec![resource]
    );
}

#[test]
fn repository_free_child_keeps_reasoning_role_without_execution_or_fabricated_git() {
    let f = Fixture::new(AgentRole::Documentation);
    let mut input = run_preparation(&f);
    let mut parent = f.parent.clone();
    parent.config_revision = kegari_v0::config::config_revision(&input.config).unwrap();
    let scope = parent
        .children
        .get_mut(&id(11))
        .unwrap()
        .scope
        .as_mut()
        .unwrap();
    scope.write_repository = None;
    scope.write_paths.clear();
    let gate = parent
        .fresh_gate(
            Some(id(11)),
            GatePurpose::NewOperation,
            &DependencyOutcomes::default(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    input.authority = WorkerAuthority::Authorized(gate);
    input.repository = None;
    input.repository_id = None;
    input.isolation.source_writes.clear();
    input.isolation.managed_execution = None;
    let prepared = kegari_v0::worker_run::prepare(input).unwrap();
    let manifest = &prepared.contract().instructions().manifest;
    assert_eq!(manifest.role, AgentRole::Documentation);
    assert!(!manifest.permissions.repository_code_execution);
    assert!(manifest.repository_revision.is_none());
    assert!(!manifest.tools.iter().any(|tool| matches!(
        tool.name.as_str(),
        "validation_run" | "git_inspect" | "source_write"
    )));
    kegari_v0::worker_run::NonExecutingRunHooks::new(
        prepared.contract().clone(),
        f.control.clone(),
        f.host.clone(),
    )
    .unwrap();
}

#[test]
fn recovered_session_retains_earlier_intervention_and_rejects_wrong_account_or_missing_events() {
    use kegari_codex::{ProviderIntervention, SessionBinding};
    let f = Fixture::new(AgentRole::Coordination);
    let (prepared, permit, mut services, _) = native_fixture_run(&f);
    let contract = prepared.contract().clone();
    let records = PrivateDir::open(&f._temp.path().canonicalize().unwrap().join("control"))
        .unwrap()
        .child("session-recovery-fixture")
        .unwrap();
    records.write("contract.json",&json!({"scope":contract.scope_digest(),"instructions":contract.instructions().manifest,"tool_contract":contract.tools().digest(),"isolation":prepared.isolation().plan()})).unwrap();
    services.worker.journal = records.child("journal").unwrap();
    let journal_path = services.worker.journal.path().to_path_buf();
    let mut broker = ScopedWorker::new(contract.clone(), permit.clone(), services.worker).unwrap();
    let manifest = &contract.instructions().manifest;
    let mut state = SessionState::new(
        SessionBinding {
            schema_version: 1,
            account_id: contract.account_id().into(),
            work_identity: contract.work_key().unwrap(),
            operation_id: permit.operation_id.to_string(),
            generation: manifest.work_generation.clone(),
            instruction_digest: contract.instructions().manifest_digest.clone(),
            isolation_digest: prepared.isolation().digest().into(),
            profile: kegari_codex::CodexProfile {
                model: manifest.profile.model.clone(),
                reasoning: manifest.profile.reasoning.clone(),
                speed: "standard".into(),
            },
            cwd: manifest.cwd.clone(),
        },
        1,
    )
    .unwrap();
    state.max_turns = 2;
    assert_eq!(
        broker.checkpoint(&state, None).unwrap_err().code,
        "admission_budget"
    );
    state.max_turns = 1;
    state.turns_started = 2;
    assert_eq!(
        broker.checkpoint(&state, None).unwrap_err().code,
        "admission_budget"
    );
    state.turns_started = 0;
    state.thread_id = Some("actual-fixture-thread".into());
    state.input_required = true;
    state.intervention = Some(ProviderIntervention {
        schema_version: 1,
        account_id: contract.account_id().into(),
        thread_id: "actual-fixture-thread".into(),
        turn_id: "fixture-turn".into(),
        source_method: "thread/status/changed".into(),
        reason_code: "usage_limit".into(),
        required_resolution: "verified provider resolution".into(),
        observed_at: Utc::now().to_rfc3339(),
        source_digest: bytes_digest(b"fixture intervention"),
    });
    broker.checkpoint(&state, Some("turn/start")).unwrap();
    state.intervention = None;
    state.input_required = false;
    broker.checkpoint(&state, None).unwrap();
    let receipt = f.control.operation(permit.operation_id).unwrap();
    let recovered = kegari_v0::worker::recover_session(&records, &receipt)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.session_records, 2);
    assert!(recovered.state.input_required);
    assert_eq!(
        recovered.state.intervention.unwrap().reason_code,
        "usage_limit"
    );
    let journal = PrivateDir::open(&journal_path).unwrap();
    let mut event: Value = journal.read("event-00000001.json").unwrap();
    event["record"]["state"]["binding"]["account_id"] = json!("different-account");
    journal.write("event-00000001.json", &event).unwrap();
    assert_eq!(
        kegari_v0::worker::recover_session(&records, &receipt)
            .unwrap_err()
            .code,
        "record_corrupt"
    );
    std::fs::remove_file(journal_path.join("event-00000000.json")).unwrap();
    assert_eq!(
        kegari_v0::worker::recover_session(&records, &receipt)
            .unwrap_err()
            .code,
        "record_corrupt"
    );
}

#[test]
fn driver_preparation_identity_ignores_refresh_noise_without_resetting_phase_attempt() {
    use kegari_types::run::PhaseKey;
    use kegari_v0::{
        runtime::WorkerAction,
        runtime_schedule::RunSchedule,
        scheduler::{ReadyWork, Trigger, WorkKind},
    };
    let f = Fixture::new(AgentRole::Implementation);
    let gate = f
        .parent
        .fresh_gate(
            Some(id(11)),
            GatePurpose::NewOperation,
            &DependencyOutcomes::default(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    let authority = WorkerAuthority::Authorized(gate);
    let work = authority.work_key().unwrap();
    let generation = canonical_digest(1, &authority.work().unwrap().generation).unwrap();
    let mut action = WorkerAction {
        authority,
        phase: PhaseKey {
            work: work.clone(),
            generation: generation.clone(),
            phase: "implementation".into(),
            retry_grant: None,
        },
        schedule: RunSchedule {
            attempt: 1,
            max_attempts: 2,
            max_turns: 2,
            max_review_rounds: 2,
            trigger: Trigger::InitialAuthorization,
            not_before: Utc::now(),
        },
        ready: ReadyWork {
            canonical_key: work,
            workspace: id(3),
            account: "fixture-account".into(),
            generation,
            state: "in progress".into(),
            priority: None,
            created_at: None,
            kind: WorkKind::Child,
            trigger: Some(Trigger::InitialAuthorization),
            not_before: Utc::now(),
            gate_blockers: vec![],
        },
    };
    let original = kegari_v0::runtime_driver::preparation_identity(&action).unwrap();
    let mut refreshed = f.parent.clone();
    refreshed.source_digest = bytes_digest(b"later provider timestamp and own comment");
    refreshed.observed_at = Utc::now();
    action.authority = WorkerAuthority::Authorized(
        refreshed
            .fresh_gate(
                Some(id(11)),
                GatePurpose::NewOperation,
                &DependencyOutcomes::default(),
                Utc::now(),
                Duration::seconds(30),
            )
            .unwrap(),
    );
    action.ready.not_before = Utc::now();
    assert_eq!(
        kegari_v0::runtime_driver::preparation_identity(&action).unwrap(),
        original
    );
    action.phase.retry_grant = Some(id(991));
    assert_ne!(
        kegari_v0::runtime_driver::preparation_identity(&action).unwrap(),
        original
    );
    action.phase.retry_grant = None;
    action.schedule.attempt = 2;
    assert_ne!(
        kegari_v0::runtime_driver::preparation_identity(&action).unwrap(),
        original
    );
}

#[test]
fn correction_context_contains_exact_native_text_and_rejects_stale_or_missing_revisions() {
    use kegari_providers::{Comment, Issue};
    use kegari_v0::{
        authority::{CommentRevision, CorrectionInvalidations},
        reconcile::IssueObservation,
    };
    let fixture = Fixture::new(AgentRole::Coordination);
    let mut parent = fixture.parent.clone();
    let body = "Please preserve the currently accepted behavior and test the corrected edge case.";
    let comment = Comment {
        id: id(501),
        issue_id: parent.key.issue,
        body: body.into(),
        body_digest: bytes_digest(body.as_bytes()),
        user_id: Some(id(502)),
        delegated: false,
        bot: false,
        created_at: Some(Utc::now()),
        updated_at: None,
        edited_at: None,
        url: "https://linear.invalid/comment/501".into(),
    };
    let issue:Issue=serde_json::from_value(json!({"id":parent.key.issue,"workspace_id":parent.key.workspace,"endpoint":parent.key.endpoint,"identifier":"FIX-10","title":"Parent","description":"Complete current parent specification","team_id":id(503),"project_id":null,"creator_id":id(502),"assignee_id":null,"parent_id":null,"state_id":id(504),"state_name":"In Progress","state_type":"started","labels":[],"priority":null,"created_at":null,"updated_at":null,"url":"https://linear.invalid/FIX-10","diagnostics":[]})).unwrap();
    parent.issue = Some(issue.clone());
    let observation = IssueObservation {
        issue,
        comments: vec![comment.clone()],
        history: vec![],
        dependencies: vec![],
    };
    parent
        .observations
        .insert(parent.key.issue, observation.clone());
    let mut unrelated = observation;
    unrelated.comments[0].body = "unrelated private source".into();
    parent.observations.insert(id(999), unrelated);
    parent.corrections = Corrections::from_invalidations(CorrectionInvalidations {
        pending: BTreeMap::from([(
            comment.id,
            CommentRevision {
                id: comment.id,
                author: comment.user_id,
                body_digest: comment.body_digest.clone(),
                edited_at: None,
                observed_revision: 0,
            },
        )]),
        ..Default::default()
    })
    .unwrap();
    let gate = parent
        .fresh_gate(
            None,
            GatePurpose::Coordination,
            &DependencyOutcomes::default(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    let context = kegari_v0::runtime_workers::native_context(&parent, &gate).unwrap();
    assert_eq!(context["pending_correction_comments"][0]["body"], body);
    assert_eq!(
        context["pending_correction_comments"][0]["user_id"],
        id(502).to_string()
    );
    assert!(!context.to_string().contains("unrelated private source"));
    let bound = kegari_v0::runtime_workers::authorized_context(
        &parent,
        &gate,
        "work",
        id(4),
        &["42".into()],
        json!({"retained_evidence":{"digest":bytes_digest(b"retained")}}),
    )
    .unwrap();
    assert!(bound.get("native_context").is_none());
    assert_eq!(bound["evidence"]["native_context"], context);
    assert!(
        bound["operation"]["work"]["generation"]
            .get("authorization")
            .is_none()
    );
    assert_eq!(
        bound["operation"]["work"]["generation"]["authorization_digest"],
        gate.work().generation.authorization
    );
    assert_eq!(
        bound["evidence"]["retained_evidence"]["digest"],
        bytes_digest(b"retained")
    );
    let manifest = &fixture.contract.instructions().manifest;
    let build = |context: &Value| {
        assemble(AssemblyInput {
            workflow: &fixture.input.workflow,
            role: AgentRole::Coordination,
            phase: "work",
            context,
            workspace_id: gate.workspace_id(),
            operation_id: id(4),
            work_generation: &manifest.work_generation,
            correction_revision: &manifest.correction_revision,
            profile: gate.profile(),
            cwd: &manifest.cwd,
            codex_home: &manifest.codex_home,
            codex_version: &manifest.codex_version,
            repository: manifest
                .repository_root
                .as_ref()
                .map(|root| RepositoryContext {
                    root: root.clone(),
                    revision: manifest.repository_revision.clone().unwrap(),
                    scope_paths: vec!["a.txt".into()],
                }),
            required_context: &[],
            project_doc_max_bytes: 32768,
            permissions: manifest.permissions.clone(),
            tools: manifest.tools.clone(),
            provider_discovery: ProviderDiscovery::Unavailable {
                reason: "deterministic context fixture".into(),
            },
            known_secrets: &[],
        })
    };
    let assembled = build(&bound).unwrap();
    assert!(assembled.prompt.contains(body));
    assert!(!assembled.prompt.contains("unrelated private source"));
    let mut actual = fixture.input.clone();
    actual.context = bound.clone();
    actual.authority = WorkerAuthority::Authorized(gate.clone());
    actual.assembled = assembled;
    WorkerContract::new(actual).unwrap();
    let mut rejected = bound.clone();
    rejected["evidence"]["request_headers"] =
        json!({"authorization":bytes_digest(b"credential-shaped value")});
    assert!(build(&rejected).is_err());
    let mut rejected = bound;
    rejected["native_context"] = context;
    assert!(build(&rejected).is_err());
    parent
        .observations
        .get_mut(&parent.key.issue)
        .unwrap()
        .comments[0]
        .body = "edited after admission".into();
    assert_eq!(
        kegari_v0::runtime_workers::native_context(&parent, &gate)
            .unwrap_err()
            .code,
        "stale_correction"
    );
    parent
        .observations
        .get_mut(&parent.key.issue)
        .unwrap()
        .comments
        .clear();
    assert_eq!(
        kegari_v0::runtime_workers::native_context(&parent, &gate)
            .unwrap_err()
            .code,
        "incompletely_observed"
    );
}

#[tokio::test]
async fn pending_classification_defers_actual_completed_proposal_and_closes_source_gate() {
    let mut fixture = Fixture::new(AgentRole::Implementation);
    let claim = fixture.call(
        "outcome_submit",
        json!({"claim":{"summary":"completed original input"},"artifact_refs":[]}),
    );
    fixture.worker.call(&claim).await.unwrap();
    fixture.host.waiting.store(true, Ordering::SeqCst);
    let write=fixture.call("source_write",json!({"path":"a.txt","expected_digest":bytes_digest(b"original\n"),"content":"unexpected","executable":false}));
    assert_eq!(
        fixture.worker.call(&write).await.unwrap_err().code,
        "correction_classification_pending"
    );
    fixture.state.notification("turn/completed",&json!({"threadId":"thread","turn":{"id":"turn","status":"completed","items":[],"error":null}})).unwrap();
    fixture.worker.checkpoint(&fixture.state, None).unwrap();
    assert!(fixture.sink.0.lock().unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(fixture.input.isolation.plan().repository.join("a.txt")).unwrap(),
        "original\n"
    );
    // Informational resolution does not change the work or consume a correction.
    fixture.host.waiting.store(false, Ordering::SeqCst);
    fixture.worker.checkpoint(&fixture.state, None).unwrap();
    assert_eq!(fixture.sink.0.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn protected_recovery_candidate_requires_actual_input_and_after_hook_and_fences_metadata_spawn()
 {
    use kegari_v0::worker_run::{RecoveryPreparation, RecoveryProviderLaunch};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    for scenario in ["fenced", "timeout"] {
        let f = Fixture::new(AgentRole::Coordination);
        let (prepared, permit, mut services, marker) = native_fixture_run(&f);
        let contract = prepared.contract().clone();
        let base = f._temp.path().canonicalize().unwrap();
        let records = PrivateDir::open(&base.join("control"))
            .unwrap()
            .child("protected-recovery")
            .unwrap();
        records.write("contract.json",&json!({"scope":contract.scope_digest(),"instructions":contract.instructions().manifest,"tool_contract":contract.tools().digest(),"isolation":prepared.isolation().plan()})).unwrap();
        services.worker.journal = records.child("journal").unwrap();
        let journal = PrivateDir::open(services.worker.journal.path()).unwrap();
        let processes = services.processes.clone();
        let hooks = services.hooks.clone();
        let mut broker =
            ScopedWorker::new(contract.clone(), permit.clone(), services.worker).unwrap();
        let m = &contract.instructions().manifest;
        let mut state = SessionState::new(
            SessionBinding {
                schema_version: 1,
                account_id: contract.account_id().into(),
                work_identity: contract.work_key().unwrap(),
                operation_id: permit.operation_id.to_string(),
                generation: m.work_generation.clone(),
                instruction_digest: contract.instructions().manifest_digest.clone(),
                isolation_digest: prepared.isolation().digest().into(),
                profile: CodexProfile {
                    model: m.profile.model.clone(),
                    reasoning: m.profile.reasoning.clone(),
                    speed: "standard".into(),
                },
                cwd: m.cwd.clone(),
            },
            1,
        )
        .unwrap();
        state.thread_id = Some("fixture-original-thread".into());
        state.turns_started = 1;
        let (client, server) = tokio::io::duplex(32_768);
        let (reader, writer) = tokio::io::split(client);
        let (sr, mut sw) = tokio::io::split(server);
        let mut sr = BufReader::new(sr);
        let wire = tokio::spawn(async move {
            let mut line = String::new();
            sr.read_line(&mut line).await.unwrap();
            let init: Value = serde_json::from_str(&line).unwrap();
            sw.write_all(format!("{}\n",json!({"id":init["id"],"result":{"userAgent":"fixture","platformFamily":"unix","platformOs":"macos","codexHome":"/private/tmp/fixture"}})).as_bytes()).await.unwrap();
            line.clear();
            sr.read_line(&mut line).await.unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&line).unwrap()["method"],
                "initialized"
            );
            line.clear();
            sr.read_line(&mut line).await.unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "turn/start");
            let turn = json!({"id":"actual-failed-turn","status":"failed","items":[{"id":"user-one","type":"userMessage","content":request["params"]["input"]}],"error":{"message":"synthetic wire overload","codexErrorInfo":"serverOverloaded"}});
            for frame in [
                json!({"method":"turn/started","params":{"threadId":"fixture-original-thread","turn":{"id":"actual-failed-turn","status":"inProgress"}}}),
                json!({"method":"turn/completed","params":{"threadId":"fixture-original-thread","turn":turn}}),
                json!({"id":request["id"],"result":{"turn":turn}}),
            ] {
                sw.write_all(format!("{frame}\n").as_bytes()).await.unwrap();
            }
        });
        let mut rpc = RpcClient::from_transport(reader, writer, StdDuration::from_secs(2)).unwrap();
        rpc.initialize().await.unwrap();
        rpc.request("turn/start",json!({"threadId":"fixture-original-thread","input":[{"type":"text","text":"actual bounded fixture input"}]}),Some(&mut state),&mut broker).await.unwrap();
        broker.checkpoint(&state, None).unwrap();
        wire.await.unwrap();
        let admission = f.control.operation(permit.operation_id).unwrap();
        assert!(
            kegari_v0::worker::recover_session(&records, &admission)
                .unwrap()
                .unwrap()
                .resume_candidate()
                .is_err()
        );
        let observed = hooks.after(&permit, &state).await.unwrap();
        drop(broker);
        // Actual pure nonexecuting lifecycle observation, written in the protected
        // journal's production event shape. No native process or completion is invented.
        let seq = journal
            .records()
            .unwrap()
            .into_iter()
            .filter(|name| name != "binding.json")
            .count();
        journal.write(&format!("event-{seq:08}.json"),&json!({"kind":"after_hook","record":{"operation":permit.operation_id,"observed":observed},"observed_at":Utc::now()})).unwrap();
        let receipt=f.control.retire_local(permit.operation_id,json!({"outcome":"failed","local_exit_confirmed":true,"hook_error":null,"session":state})).unwrap();
        let recovered = kegari_v0::worker::recover_session(&records, &receipt)
            .unwrap()
            .unwrap();
        let candidate = recovered.resume_candidate().unwrap();
        let mut tampered = recovered.clone();
        tampered.state.turns_started = 0;
        assert!(tampered.resume_candidate().is_err());
        let mut config = run_preparation(&f).config;
        Arc::make_mut(&mut config)
            .connections
            .get_mut("codex")
            .unwrap()
            .argv[0] = base
            .join("deterministic-app-server")
            .to_string_lossy()
            .into_owned();
        tokio::time::sleep(StdDuration::from_millis(1050)).await;
        let make_input = |max_attempts, timeout| RecoveryPreparation {
            config: config.clone(),
            contract: contract.clone(),
            candidate: candidate.clone(),
            records: PrivateDir::open(records.path()).unwrap(),
            lease: permit.lease.clone(),
            control: f.control.clone(),
            host: f.host.clone(),
            processes: processes.clone(),
            executions: f.executions.clone(),
            timeout,
            attempt: 1,
            max_attempts,
        };
        let before = f.control.operations().unwrap().len();
        assert_eq!(
            RecoveryProviderLaunch::admit(make_input(999, StdDuration::from_secs(1)))
                .err()
                .unwrap()
                .code,
            "resume_incompatible"
        );
        assert_eq!(
            RecoveryProviderLaunch::admit(make_input(1, StdDuration::from_secs(301)))
                .err()
                .unwrap()
                .code,
            "resume_incompatible"
        );
        assert_eq!(f.control.operations().unwrap().len(), before);
        let recovery =
            RecoveryProviderLaunch::admit(make_input(1, StdDuration::from_secs(1))).unwrap();
        let id = recovery.operation_id();
        let metadata = f.control.operation(id).unwrap();
        assert!(
            SessionRecoveryPreparationReference::from_receipt(&metadata)
                .unwrap()
                .is_some()
        );
        assert!(
            kegari_v0::control::run_reference_from_receipt(&metadata)
                .unwrap()
                .is_none()
        );
        if scenario == "fenced" {
            f.control.fence(id).unwrap();
            assert_eq!(recovery.recover().await.unwrap_err().code, "access_denied");
            assert!(!marker.exists());
        } else {
            assert_eq!(recovery.recover().await.unwrap_err().code, "timeout");
            assert!(marker.exists());
            let observed = f.control.operation(id).unwrap();
            assert_eq!(observed.phase, ReceiptPhase::Reconciled);
            assert_eq!(
                observed.observed_result.unwrap()["observation"]["retryable"],
                false
            );
        }
        assert!(f.executions.active_operations().unwrap().is_empty());
    }
}

#[tokio::test]
async fn evaluation_review_claim_requires_actual_current_recorded_completion() {
    // Actual ScopedWorker/Control/journal with declared synthetic provider state.
    // This is neither a live model trial nor a native confinement qualification.
    let mut fixture = Fixture::new(AgentRole::IndependentReview);
    let claim =
        json!({"schema_version":1,"verdict":"pass","unresolved_findings":[],"supporting_links":[]});
    let call = fixture.call("outcome_submit", json!({"claim":claim,"artifact_refs":[]}));
    let submitted = fixture.worker.call(&call).await.unwrap();
    assert_eq!(submitted["state"], "pending_host_verification");
    assert!(fixture.worker.observe().unwrap().review_claim.is_none());
    fixture.state.notification("turn/completed", &json!({"threadId":"thread","turn":{"id":"turn","status":"completed","items":[],"error":null}})).unwrap();
    fixture.worker.checkpoint(&fixture.state, None).unwrap();
    let facts = fixture.worker.observe().unwrap();
    let review = facts.review_claim.unwrap();
    assert!(facts.summary.is_none());
    assert_eq!(review.operation, fixture.permit.operation_id);
    assert_eq!(
        review.proposal.to_string(),
        submitted["proposal_id"].as_str().unwrap()
    );
    assert_eq!(
        review.source_revision,
        fixture
            .contract
            .instructions()
            .manifest
            .repository_revision
            .clone()
            .unwrap()
    );
    let repository = review.repository.as_ref().unwrap();
    assert_eq!(
        repository.hostname,
        fixture.input.repository.as_ref().unwrap().hostname
    );
    let target = fixture.input.review.as_ref().unwrap();
    assert_eq!(
        repository.slug.as_deref(),
        Some(format!("{}/{}", target.repository.owner, target.repository.name).as_str())
    );
    assert_eq!(serde_json::to_value(&review.claim).unwrap(), claim);
    assert!(
        facts
            .evidence_references
            .contains(&review.outcome_reference)
    );
    assert_eq!(facts.state["delivery"], "pending"); // The claim granted no product success.
    fixture.worker.checkpoint(&fixture.state, None).unwrap();
    assert_eq!(fixture.sink.0.lock().unwrap().len(), 1);
    fixture.state.notification("turn/started", &json!({"threadId":"thread","turn":{"id":"later","status":"inProgress","items":[],"error":null}})).unwrap();
    fixture.worker.checkpoint(&fixture.state, None).unwrap();
    assert!(fixture.worker.observe().unwrap().review_claim.is_none());
}

#[tokio::test]
async fn evaluation_does_not_fall_back_from_malformed_latest_review_to_earlier_pass() {
    let mut fixture = Fixture::new(AgentRole::IndependentReview);
    for claim in [
        json!({"schema_version":1,"verdict":"pass","unresolved_findings":[],"supporting_links":[]}),
        json!({"schema_version":1,"verdict":"pass","unresolved_findings":[],"supporting_links":[],"summary":{"all_passed":true}}),
    ] {
        let call = fixture.call("outcome_submit", json!({"claim":claim,"artifact_refs":[]}));
        fixture.worker.call(&call).await.unwrap();
    }
    fixture.state.notification("turn/completed", &json!({"threadId":"thread","turn":{"id":"turn","status":"completed","items":[],"error":null}})).unwrap();
    fixture.worker.checkpoint(&fixture.state, None).unwrap();
    assert_eq!(
        fixture.worker.observe().unwrap_err().code,
        "invalid_outcome"
    );
}

#[test]
fn independent_observation_without_actual_runtime_registration_stays_unavailable() {
    // Real ScopedWorker/Control/contract and source observation, but this test
    // host has no RuntimeCoordinator registration. It must not invent a match.
    let f = Fixture::new(AgentRole::IndependentReview);
    let observed = f.worker.observe().unwrap();
    let boundary = observed.independent_boundary.as_ref().unwrap();
    assert_eq!(boundary.source_revision, observed.revision);
    assert_eq!(boundary.candidate_digest, observed.candidate_digest);
    boundary.validate().unwrap();
    match &boundary.check {
        kegari_v0::evaluation::IndependentBoundaryCheck::Unavailable { error } => {
            assert_eq!(error.code, "boundary_observation_unavailable");
        }
        _ => panic!("a component host cannot assert actual runtime registration"),
    }
    assert!(observed.review_claim.is_none());
    assert_eq!(observed.state, json!({"delivery":"pending"}));
    assert!(observed.effects.is_empty());
    assert!(
        Fixture::new(AgentRole::Implementation)
            .worker
            .observe()
            .unwrap()
            .independent_boundary
            .is_none()
    );
}

#[test]
fn ordinary_scoped_worker_does_not_enable_raw_protocol_capture() {
    // Real component Control/contract/journal, no declared evaluation or native
    // run. Receiving the callback alone must not opt this worker into logging.
    let mut f = Fixture::new(AgentRole::IndependentReview);
    let journal = PrivateDir::open(
        &f._temp
            .path()
            .canonicalize()
            .unwrap()
            .join("control/worker"),
    )
    .unwrap();
    let before = journal.records().unwrap();
    let params = json!({"threadId":"thread","turnId":"turn","callId":"outside-trial","tool":"unsupported","arguments":{"text":"must not be logged"}});
    f.worker
        .observe_protocol(kegari_codex::ProtocolObservation::ServerRequest {
            state: Some(&f.state),
            rpc_id: &json!(1),
            sequence: 1,
            method: "item/tool/call",
            params: &params,
        });
    assert_eq!(journal.records().unwrap(), before);
    assert!(f.worker.observe().unwrap().declared_protocol.is_none());
    assert!(f.worker.observe().unwrap().independent_broker.is_none());
}

#[test]
fn changed_workflow_origin_cannot_rebind_an_assembled_worker_contract() {
    let fixture = Fixture::new(AgentRole::Implementation);
    let mut input = fixture.input.clone();
    input.workflow.path = Some(fixture._temp.path().join("another-WORKFLOW.md"));
    assert_eq!(
        WorkerContract::new(input).unwrap_err().code,
        "worker_binding"
    );
    let mut input = fixture.input.clone();
    input.workflow.embedded = Workflow::installed_default().unwrap().embedded;
    assert_eq!(
        WorkerContract::new(input).unwrap_err().code,
        "worker_binding"
    );
}
