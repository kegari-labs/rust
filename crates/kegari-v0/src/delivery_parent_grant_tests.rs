//! Actual native retry command/provider-wire and protected Control closure.
//! The retired worker observation is synthetic component data. This test does
//! not create ParentReassemblyWork, a joined completion, acceptance, or checks.
use super::*;
use crate::{
    app::WorkflowControl,
    control::{Actor, RunAdmission},
    reconcile::GatePurpose,
};

#[tokio::test]
async fn parent_grant_capture_uses_actual_retry_command_and_current_native_publication() {
    let (_temp, built, delivery, wire, owner) =
        crate::runtime_delivery::tests::fixture_authorization_with_wire(true).await;
    let (lease, _owner_lock) = owner.start(&built.runtime);
    let projection = built.runtime.observations.refresh().await.unwrap();
    let gate = projection
        .parent(Uuid::from_u128(2), Uuid::from_u128(100))
        .unwrap()
        .fresh_gate(
            None,
            GatePurpose::NewOperation,
            &built.runtime.dependency_outcomes().unwrap(),
            Utc::now(),
            chrono::Duration::seconds(30),
        )
        .unwrap();
    let config = built.runtime.configuration.snapshot().unwrap();
    let workspace = config
        .workspaces
        .values()
        .find(|w| w.id == gate.workspace_id())
        .unwrap();
    let run = RunReference {
        workspace: gate.workspace_id(),
        account: config.connections[&workspace.codex_connection]
            .account_id
            .clone(),
        phase: PhaseKey {
            work: gate.work_key().unwrap(),
            generation: canonical_digest(1, &gate.work().generation).unwrap(),
            phase: "parent_evidence_review".into(),
            retry_grant: None,
        },
        profile_digest: canonical_digest(1, &"component profile").unwrap(),
        instruction_digest: canonical_digest(1, &"component instructions").unwrap(),
        normalized_state: None,
        max_attempts: config.runtime.max_attempts_per_phase,
        max_turns: config.runtime.max_turns,
        max_review_rounds: config.runtime.max_review_rounds,
        attempt: 1,
    };
    let permit = built
        .runtime
        .control
        .admit_run(
            &lease,
            RunAdmission {
                request_id: Uuid::new_v4(),
                work_key: run.phase.work.clone(),
                scope_digest: canonical_digest(1, &"explicit synthetic parent terminal metadata")
                    .unwrap(),
                reference: run.clone(),
            },
            || Ok(()),
        )
        .unwrap();
    built
        .runtime
        .control
        .retire_local(
            permit.operation_id,
            json!({
                "outcome":"failed", "local_exit_confirmed":true,"hook_error":null,
                "error":{"code":"turn_incomplete","message":"synthetic closed parent failure"},
                "session":{"terminal":"failed","input_required":false,"intervention":null}
            }),
        )
        .unwrap();
    built.runtime.observations.refresh().await.unwrap();
    let workflow = crate::runtime_workflow::RuntimeWorkflow::new(
        built.runtime.clone(),
        delivery.clone(),
        delivery,
    );
    let actor = Actor {
        identity: "operator".into(),
        principal: Some(crate::control::Principal::Unix { uid: 501 }),
        roles: ["recover".into(), "inspect".into(), "lifecycle".into()].into(),
    };
    let mut request = crate::api::CommandRequest {
        api_version: 1,
        request_id: Uuid::new_v4(),
        runtime_id: config.runtime.id,
        command: crate::api::CommandName::Retry,
        args: json!({"work":"TEST-100","reason":"The known failed parent task is understood; retry with a bounded budget"}),
        expected_config_revision: Some(config.revision().unwrap()),
        expected_control_generation: Some(
            built.runtime.control.snapshot().unwrap().intent.generation,
        ),
    };
    let mut mutation = crate::app::mutation_request(
        &actor,
        &request,
        workflow.command_targets(&request).unwrap(),
    );
    mutation.kind = "command/Retry".into();
    mutation.payload = json!({"arguments":request.args,"secret_digest":null});
    let (command, new) = built
        .runtime
        .control
        .queue_command(mutation, "recover", || Ok(()))
        .unwrap();
    assert!(new);
    request.request_id = crate::app::command_effect_id(command.operation_id);
    workflow.execute(actor, request, command).await.unwrap();
    built.runtime.observations.refresh().await.unwrap();
    let current = || {
        crate::runtime_workflow::current_retry_grants(
            &config,
            &built.runtime.observations.admission_snapshot().unwrap(),
            gate.work(),
            &built.runtime.control.operations().unwrap(),
            &built.runtime.services.key,
        )
    };
    let grants = current().unwrap();
    assert_eq!(grants.len(), 1);
    let phase = grants[0].phase_key();
    let receipts = built.runtime.control.operations().unwrap();
    let (retained, refs) =
        grant_lineage::capture(&phase, &gate, config.runtime.id, &receipts, &grants).unwrap();
    assert_eq!(retained[0].digest(), grants[0].digest());
    assert_eq!(refs[0].previous_phase, run.phase);
    assert_eq!(refs[0].prior_admissions.len(), 1);
    assert!(refs[0].prior_admissions.contains_key(&permit.operation_id));
    assert_eq!(
        refs[0].publication.receipt_digest,
        canonical_digest(
            1,
            &built
                .runtime
                .control
                .operation(grants[0].record().operation_id)
                .unwrap()
        )
        .unwrap()
    );
    let mut forked = grants.clone();
    forked.push(grants[0].clone());
    assert_eq!(
        grant_lineage::capture(&phase, &gate, config.runtime.id, &receipts, &forked)
            .unwrap_err()
            .code,
        "parent_reassembly"
    );
    let saved = wire
        .lock()
        .unwrap()
        .comments
        .get(&gate.work().parent)
        .cloned()
        .unwrap();
    wire.lock()
        .unwrap()
        .comments
        .get_mut(&gate.work().parent)
        .unwrap()
        .retain(|comment| {
            comment["id"].as_str() != Some(grants[0].provenance().record_id.as_str())
        });
    built.runtime.observations.refresh().await.unwrap();
    assert_eq!(
        grant_lineage::capture(
            &phase,
            &gate,
            config.runtime.id,
            &receipts,
            &current().unwrap()
        )
        .unwrap_err()
        .code,
        "parent_reassembly"
    );
    wire.lock()
        .unwrap()
        .comments
        .insert(gate.work().parent, saved);
    built.runtime.observations.refresh().await.unwrap();
    grant_lineage::capture(
        &phase,
        &gate,
        config.runtime.id,
        &receipts,
        &current().unwrap(),
    )
    .unwrap();
    // The read-only active adapter uses the actual Control permit and retained
    // snapshot. This does not manufacture a reassembly or carry capability.
    let observed = built.runtime.observations.refresh().await.unwrap();
    let active_gate = observed
        .parent(gate.workspace_id(), gate.work().parent)
        .unwrap()
        .fresh_gate(
            None,
            GatePurpose::NewOperation,
            &built.runtime.dependency_outcomes().unwrap(),
            Utc::now(),
            chrono::Duration::seconds(30),
        )
        .unwrap();
    let active_parent = observed
        .parent(gate.workspace_id(), gate.work().parent)
        .unwrap();
    let e2e_id = active_parent.metadata.as_ref().unwrap().e2e_child;
    let e2e_gate = active_parent
        .fresh_gate(
            Some(e2e_id),
            GatePurpose::NewOperation,
            &built.runtime.dependency_outcomes().unwrap(),
            Utc::now(),
            chrono::Duration::seconds(30),
        )
        .unwrap();
    assert_ne!(e2e_gate.work(), active_gate.work());
    assert_eq!(e2e_gate.work().child, Some(e2e_id));
    let _active = built
        .runtime
        .observations
        .capture_active(
            active_gate.workspace_id(),
            active_gate.config_revision(),
            active_gate.workflow_digest(),
        )
        .unwrap();
    let active_permit = built
        .runtime
        .control
        .admit(
            &lease,
            Uuid::new_v4(),
            active_gate.work_key().unwrap(),
            canonical_digest(1, &"read-only active native grant fixture").unwrap(),
            || {
                built.runtime.registration.verify_active_owner(&lease)?;
                built
                    .runtime
                    .observations
                    .active_gate(&active_gate, &built.runtime.dependency_outcomes()?)
                    .map(|_| ())
            },
        )
        .unwrap();
    let active = crate::runtime_delivery::ActiveParentGrantObserver::new(
        built.runtime.clone(),
        active_permit.clone(),
        active_gate.clone(),
        config.clone(),
    )
    .unwrap();
    assert_eq!(
        active.observe(&active_gate).unwrap()[0].digest(),
        grants[0].digest()
    );
    assert!(
        active.observe(&e2e_gate).unwrap().is_empty(),
        "actual integrated child authority is observed through its parent owner"
    );
    // A genuinely observed new grant also supersedes an ungranted budget.
    assert_eq!(
        grant_lineage::capture(
            &run.phase,
            &gate,
            config.runtime.id,
            &receipts,
            &active.observe(&active_gate).unwrap(),
        )
        .unwrap_err()
        .code,
        "parent_reassembly"
    );

    let workflow = config
        .workspaces
        .values()
        .find(|w| w.id == gate.workspace_id())
        .unwrap()
        .workflow
        .clone()
        .unwrap();
    let original_workflow = std::fs::read_to_string(&workflow).unwrap();
    std::fs::write(
        &workflow,
        format!("{original_workflow}\nNew instructions apply to new admissions.\n"),
    )
    .unwrap();
    {
        let mut changed = built.configuration.config.lock().unwrap();
        changed.runtime.max_agents = changed.runtime.max_agents.saturating_add(1);
    }
    let updated = built.runtime.observations.refresh().await.unwrap();
    let new_e2e_gate = updated
        .parent(gate.workspace_id(), gate.work().parent)
        .unwrap()
        .fresh_gate(
            Some(e2e_id),
            GatePurpose::NewOperation,
            &built.runtime.dependency_outcomes().unwrap(),
            Utc::now(),
            chrono::Duration::seconds(30),
        )
        .unwrap();
    assert_ne!(new_e2e_gate.workflow_digest(), e2e_gate.workflow_digest());
    assert_eq!(
        active.observe(&new_e2e_gate).unwrap_err().code,
        "scope_violation",
        "a new current child generation cannot be relabeled as the retained integrated worker"
    );
    assert!(active.observe(&e2e_gate).unwrap().is_empty());
    {
        let mut changed = built.configuration.config.lock().unwrap();
        changed
            .workspaces
            .values_mut()
            .find(|w| w.id == gate.workspace_id())
            .unwrap()
            .linear
            .teams = crate::config::Selector::Ids(vec![Uuid::from_u128(999).to_string()]);
    }
    wire.lock().unwrap().set_selected(false);
    let next = built.runtime.observations.refresh().await.unwrap();
    let next_parent = next
        .parent(gate.workspace_id(), gate.work().parent)
        .unwrap();
    assert!(!next_parent.included);
    assert_eq!(
        next_parent
            .fresh_gate(
                None,
                GatePurpose::NewOperation,
                &built.runtime.dependency_outcomes().unwrap(),
                Utc::now(),
                chrono::Duration::seconds(30),
            )
            .unwrap_err()
            .code,
        "excluded",
        "ordinary new-work admission cannot adopt an excluded parent after reload"
    );
    let retained = active.observe(&active_gate).unwrap();
    assert_eq!(retained[0].digest(), grants[0].digest());
    assert!(
        active.observe(&e2e_gate).unwrap().is_empty(),
        "the exact retained E2E gate survives harmless workflow/filter reload"
    );
    grant_lineage::capture(&phase, &gate, config.runtime.id, &receipts, &retained).unwrap();
    // The in-gate reader is the actual provider/release callback path; it must
    // not reenter Control or switch to ordinary discovery during a reload.
    built
        .runtime
        .control
        .validate_permit(&active_permit, || {
            assert!(active.observe_in_gate(&e2e_gate)?.is_empty());
            grant_lineage::capture(
                &phase,
                &gate,
                config.runtime.id,
                &receipts,
                &active.observe_in_gate(&active_gate)?,
            )
            .map(|_| ())
        })
        .unwrap();

    let original_comments = wire.lock().unwrap().comments[&gate.work().parent].clone();
    wire.lock()
        .unwrap()
        .comments
        .get_mut(&gate.work().parent)
        .unwrap()
        .retain(|comment| {
            comment["id"].as_str() != Some(grants[0].provenance().record_id.as_str())
        });
    built.runtime.observations.refresh().await.unwrap();
    assert_eq!(
        grant_lineage::capture(
            &phase,
            &gate,
            config.runtime.id,
            &receipts,
            &active.observe(&active_gate).unwrap(),
        )
        .unwrap_err()
        .code,
        "parent_reassembly"
    );
    wire.lock()
        .unwrap()
        .comments
        .insert(gate.work().parent, original_comments.clone());
    let duplicate = original_comments
        .iter()
        .find(|comment| comment["id"].as_str() == Some(grants[0].provenance().record_id.as_str()))
        .unwrap()
        .clone();
    wire.lock()
        .unwrap()
        .comments
        .get_mut(&gate.work().parent)
        .unwrap()
        .push(duplicate);
    built.runtime.observations.refresh().await.unwrap();
    assert!(
        active.observe(&active_gate).is_err(),
        "duplicate actual native publication cannot become unique retained authority"
    );
    wire.lock()
        .unwrap()
        .comments
        .insert(gate.work().parent, original_comments);
    built.runtime.observations.refresh().await.unwrap();
    active.observe(&active_gate).unwrap();
    built
        .configuration
        .fences
        .lock()
        .unwrap()
        .workspaces
        .insert(gate.workspace_id());
    assert_eq!(
        active.observe(&e2e_gate).unwrap_err().code,
        "permission_fenced"
    );
    assert_eq!(
        active.observe(&active_gate).unwrap_err().code,
        "permission_fenced"
    );
    built
        .configuration
        .fences
        .lock()
        .unwrap()
        .workspaces
        .clear();
    built
        .runtime
        .control
        .fence(active_permit.operation_id)
        .unwrap();
    assert_eq!(
        active.observe(&active_gate).unwrap_err().code,
        "access_denied"
    );
    built
        .runtime
        .control
        .retire_local(
            active_permit.operation_id,
            json!({"fixture":"read_only_active_grant_observation"}),
        )
        .unwrap();
    assert!(
        built
            .runtime
            .retained_completion(permit.operation_id)
            .unwrap()
            .is_none(),
        "synthetic terminal metadata is never a joined worker capability"
    );
}
