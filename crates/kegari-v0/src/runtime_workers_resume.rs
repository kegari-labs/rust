//! Native consumer for a separately recovered protected terminal session.
//! Selection/metadata observation precede this path; no clone or after_create is
//! repeated, and no cold journal may synthesize the original joined completion.
use super::*;
use kegari_types::run::RunReference;

pub async fn prepare_resumed(
    runtime: Arc<RuntimeCoordinator>,
    request: LaunchRequest,
    predecessor: Uuid,
    recovery: worker_run::VerifiedProviderRecovery,
    source: crate::git::VerifiedReusableSource,
) -> Result<AdmittedWorker> {
    prepare_resumed_inner(runtime, request, predecessor, recovery, source, None).await
}

pub(crate) async fn prepare_paired_resumed(
    runtime: Arc<RuntimeCoordinator>,
    request: LaunchRequest,
    recovery: worker_run::VerifiedProviderRecovery,
    source: crate::git::VerifiedReusableSource,
    candidate: Arc<crate::delivery::VerifiedPairedRecoveryCandidate>,
) -> Result<AdmittedWorker> {
    if request.phase != *candidate.phase()
        || request
            .delivery
            .as_ref()
            .is_none_or(|assignment| candidate.plan().verify_assignment(assignment).is_err())
    {
        return Err(Error::new(
            "paired_recovery_lineage",
            "paired resume changed its actual retained assignment or phase",
        ));
    }
    let predecessor = candidate.failed().operation_id;
    prepare_resumed_inner(
        runtime,
        request,
        predecessor,
        recovery,
        source,
        Some(candidate),
    )
    .await
}

async fn prepare_resumed_inner(
    runtime: Arc<RuntimeCoordinator>,
    request: LaunchRequest,
    predecessor: Uuid,
    recovery: worker_run::VerifiedProviderRecovery,
    source: crate::git::VerifiedReusableSource,
    paired: Option<Arc<crate::delivery::VerifiedPairedRecoveryCandidate>>,
) -> Result<AdmittedWorker> {
    if request.unfinished.is_some()
        || request.source_advanced.is_some()
        || request.retried_unfinished.is_some()
        || request.review_retry.is_some()
        || request.validation_retry.is_some()
        || request.review_round.is_some()
    {
        return Err(Error::new(
            "resume_incompatible",
            "session resume cannot replace another retained admission cause",
        ));
    }
    if request.phase.phase == "child_replacement" {
        let replacement = runtime
            .replacement_resume_assignment(predecessor)?
            .ok_or_else(|| {
                Error::new(
                    "resume_incompatible",
                    "replacement resume lost its actual retained assignment",
                )
            })?;
        if paired.is_some()
            || request
                .delivery
                .as_ref()
                .map(crate::delivery::DeliveryWorkerAssignment::digest)
                .transpose()?
                != Some(replacement.digest()?)
        {
            return Err(Error::new(
                "resume_incompatible",
                "replacement resume changed its original assignment",
            ));
        }
    }
    let mut plan = Plan::read(&runtime, request)?;
    let previous = runtime.resume_predecessor(predecessor)?;
    let old_contract = previous.contract()?;
    let old_execution = previous.execution()?;
    let retained = recovery.observation().retained();
    if source.binding() != &old_execution.binding
        || previous.operation_id() != predecessor
        || retained.state().binding.operation_id != predecessor.to_string()
        || retained.reference().phase != plan.request.phase
        || plan.source.as_ref().is_none_or(|selected| {
            selected.repository() != old_execution.binding.repository.id
                || selected.revision() != Some(&old_execution.binding.revision)
        })
        || plan.repository_id.as_ref() != Some(&old_execution.binding.repository)
    {
        return Err(Error::new(
            "resume_incompatible",
            "native resume selection changed original source or phase",
        ));
    }
    if plan.request.delivery.is_none() {
        plan.context["attempt"] = json!(
            retained
                .reference()
                .attempt
                .checked_add(1)
                .ok_or_else(|| Error::new("attempts_exhausted", "resume attempt overflow"))?
        );
    }
    let boundary = old_contract.isolation();
    if old_contract.instructions().manifest.asset_version
        != kegari_context::instructions::ASSET_VERSION
    {
        return Err(Error::new(
            "resume_incompatible",
            "legacy custom-executor sessions cannot resume as Codex-native execution",
        ));
    }
    let repository = boundary.plan().repository.clone();
    let grants = grants(&plan, &repository)?;
    let scope_paths = match &plan.request.authority {
        WorkerAuthority::Authorized(gate) => gate
            .child_scope()
            .map(|scope| scope.write_paths.clone())
            .unwrap_or_default(),
        WorkerAuthority::Independent(_) => vec![".".into()],
    };
    // The actual assembler must prove compatibility; old prompts/manifests are
    // never relabelled with the new operation or supplied as new assembly.
    let mut prepared = worker_run::prepare(Preparation {
        config: plan.config.clone(), workflow: plan.workflow.clone(), authority: plan.request.authority.clone(),
        operation_id: plan.request.operation_id, role: plan.role, phase: plan.request.phase.phase.clone(), context: plan.context.clone(),
        isolation: boundary.plan().clone(), repository: Some(RepositoryContext { root: repository, revision: Some(old_execution.binding.revision.commit.clone()), scope_paths }),
        repository_id: plan.repository_id.clone(), read_resources: grants.resources, review: plan.review.clone(), required_context: vec![], project_doc_max_bytes: 32_768,
        provider_discovery: ProviderDiscovery::Unavailable { reason: "Pinned app-server does not expose an authenticated project-instruction discovery inventory; supplied accessible files are separately hashed".into() }, known_secrets: vec![],
    })?;
    if let Some(checkpoint) = recovery.retained_session().validation() {
        prepared = prepared.with_recovered_validation(checkpoint)?;
    }
    let directory = runtime.services.housekeeping.prepare_directory_reuse(
        &source,
        boundary,
        &old_execution.ownership,
        &plan.workflow,
    )?;
    let bridge = Arc::new(worker_run::VerifiedSessionResume::prepare_predecessor(
        previous.clone(),
        recovery,
        &prepared,
    )?);
    let contract = prepared.contract().clone();
    let schedule = match &paired {
        Some(candidate) => runtime
            .next_attempt_for_paired_session_resume(&contract, &bridge, &directory, candidate)?,
        None => runtime.next_attempt_for_session_resume(&contract, &bridge, &directory)?,
    };
    if plan.request.max_turns > schedule.max_turns {
        return Err(Error::new(
            "turns_exhausted",
            "resumed session exceeds original turn cap",
        ));
    }
    let admission = RunAdmission {
        request_id: plan.request.operation_id,
        work_key: contract.work_key()?,
        scope_digest: contract.scope_digest().into(),
        reference: RunReference {
            workspace: plan.workspace.id,
            account: plan.account.clone(),
            normalized_state: Some(plan.normalized_state.clone()),
            phase: plan.request.phase.clone(),
            profile_digest: canonical_digest(1, &contract.instructions().manifest.profile)?,
            instruction_digest: contract.instructions().manifest_digest.clone(),
            max_attempts: schedule.max_attempts,
            max_turns: schedule.max_turns,
            max_review_rounds: schedule.max_review_rounds,
            attempt: schedule.attempt,
        },
    };
    let permit = match &paired {
        Some(candidate) => runtime.admit_paired_resumed_worker(
            &plan.request.lease,
            admission,
            &contract,
            &bridge,
            &directory,
            candidate,
        )?,
        None => runtime.admit_resumed_worker(
            &plan.request.lease,
            admission,
            &contract,
            &bridge,
            &directory,
        )?,
    };
    let start: Result<AdmittedWorker> = (|| {
        let mut binding = old_execution.binding.clone();
        binding.operation_id = permit.operation_id;
        let enrollment = runtime.services.housekeeping.register_reused_execution(
            &permit,
            &directory,
            &binding,
            contract.isolation(),
            &old_execution.ownership,
            &plan.workflow,
        )?;
        runtime.register_run(permit.clone(), contract.clone())?;
        let records = runtime
            .registration
            .root()
            .child("worker-records")?
            .child(&permit.operation_id.to_string())?;
        if !records.records_bounded(1)?.is_empty() {
            return Err(Error::new(
                "reconciliation_required",
                "resumed worker already has retained operation records",
            ));
        }
        persist_contract(&runtime, &records, &contract)?;
        let processes = Arc::new(crate::process_records::ProcessJournal::new(
            runtime.registration.root().child("worker-processes")?,
            plan.config.runtime.id,
        )?);
        let execution = Arc::new(WorkerExecution {
            git: Arc::new(old_execution.git.with_execution_scope(GitExecutionScope {
                limits: GitLimits::default(),
                start_gate: scoped_git_gate(
                    runtime.clone(),
                    permit.clone(),
                    plan.request.authority.clone(),
                ),
                processes: processes.clone(),
                permit: permit.clone(),
                executions: runtime.executions.clone(),
            })?),
            git_executable: old_execution.git_executable.clone(),
            ownership: old_execution.ownership.clone(),
            binding,
            limits: old_execution.limits.clone(),
            check_outputs: BTreeMap::new(),
        });
        if contract
            .instructions()
            .manifest
            .permissions
            .repository_code_execution
        {
            let hooks =
                crate::execution::HookLedger::for_reused_execution(&enrollment, &plan.workflow)?;
            records.write("hooks.json", &hooks)?;
            runtime
                .services
                .housekeeping
                .register_hook_ledger_location(permit.operation_id, &records)?;
        }
        let hooks: Arc<dyn RunHooks> = Arc::new(CodexRunHooks::new(
            contract.clone(),
            runtime.control.clone(),
            runtime.clone(),
            Some(&execution),
            PrivateDir::open(records.path())?,
        )?);
        let admitted = bridge.admitted(&permit, &runtime.control, enrollment)?;
        runtime.register_session_resume(&permit, &admitted)?;
        if plan.request.phase.phase == "child_replacement" {
            runtime.retain_replacement_resume_successor(predecessor, permit.operation_id)?;
        }
        if let Some(candidate) = &paired {
            runtime.register_paired_resume_round(&permit, &contract, candidate, &admitted)?;
        }
        let services = RunServices {
            processes,
            worker: WorkerServices {
                control: runtime.control.clone(),
                host: runtime.clone(),
                providers: providers(&runtime, &plan.config, plan.workspace.id)?,
                sink: runtime.clone(),
                execution: Some(execution),
                journal: records.child("journal")?,
                executions: runtime.executions.clone(),
            },
            hooks,
        };
        let options = RunOptions {
            initial_input: format!(
                "Continue the admitted {} phase with the exact recovered task, current instructions and tools. Preserve historical evidence on its original operation and submit an honest scoped outcome.",
                plan.request.phase.phase
            ),
            max_seconds: plan.request.max_seconds,
            max_turns: plan.request.max_turns,
            resume: Some(admitted),
        };
        Ok(AdmittedWorker {
            runtime: runtime.clone(),
            permit: permit.clone(),
            input: Some((prepared, services, options)),
            transferred: false,
        })
    })();
    if let Err(error) = &start {
        runtime.control.fence(permit.operation_id)?;
        runtime
            .executions
            .cancel_fenced(&runtime.control, permit.operation_id)?;
        if !runtime
            .executions
            .active_operations()?
            .contains(&permit.operation_id)
        {
            runtime.control.retire_local(
                permit.operation_id,
                json!({"outcome":"failed","error":error,"phase":"session_resume_start"}),
            )?;
        }
    }
    start
}

pub(super) fn persist_contract(
    runtime: &RuntimeCoordinator,
    records: &PrivateDir,
    contract: &crate::worker::WorkerContract,
) -> Result<()> {
    let authority = match contract.authority() {
        WorkerAuthority::Authorized(gate) => Some(runtime.services.key.sign(
            "worker_contract_authority",
            &json!({"schema_version":1,"runtime":runtime.configuration.snapshot()?.runtime.id,
                "operation":contract.instructions().manifest.operation_id,"scope":contract.scope_digest(),
                "authority_digest":gate.digest()?,"authority":gate}),
        )?),
        WorkerAuthority::Independent(_) => None,
    };
    records.write("contract.json", &json!({"scope":contract.scope_digest(),
        "instructions":contract.instructions().manifest,"tool_contract":contract.tools().digest(),
        "isolation":contract.isolation().plan(),"context":contract.context(),"authority":authority}))
}

/// Rebuild sealed assembly from protected inputs and current authority. This
/// restores no process handle and creates no synthetic joined completion.
pub(crate) fn restore_cold_session(
    runtime: &RuntimeCoordinator,
    lease: &Lease,
    receipt: &crate::control::Receipt,
) -> Result<Arc<worker_run::ColdResumeSession>> {
    worker_run::require_closed_receipt(receipt)?;
    let run = crate::control::run_reference_from_receipt(receipt)?
        .ok_or_else(|| Error::new("resume_incompatible", "cold source has no run admission"))?;
    if !matches!(
        run.phase.phase.as_str(),
        "implementation" | "child_replacement"
    ) || run.phase.retry_grant.is_some()
    {
        return Err(Error::new(
            "resume_incompatible",
            "cold reconstruction is limited to implementation or retained replacement admissions",
        ));
    }
    let replacement = if run.phase.phase == "child_replacement" {
        Some(
            runtime
                .replacement_resume_assignment(receipt.operation_id)?
                .ok_or_else(|| {
                    Error::new(
                        "resume_incompatible",
                        "cold replacement needs its actual retained source and assignment",
                    )
                })?,
        )
    } else {
        None
    };
    let config = runtime.configuration.snapshot()?;
    let (binding, boundary, ownership, workflow) = runtime
        .services
        .housekeeping
        .cold_execution_source(&config, receipt.operation_id)?;
    let child = binding
        .work
        .child
        .ok_or_else(|| Error::new("resume_incompatible", "cold recovery requires a child"))?;
    let projection = runtime.observations.admission_snapshot()?;
    let parent = projection.parent(run.workspace, binding.work.parent)?;
    let authority = WorkerAuthority::Authorized(parent.fresh_gate(
        Some(child),
        crate::reconcile::GatePurpose::NewOperation,
        &runtime.dependency_outcomes()?,
        chrono::Utc::now(),
        chrono::Duration::seconds(30),
    )?);
    if authority.work() != Some(&binding.work) {
        return Err(Error::new("stale_generation", "cold recovery work changed"));
    }
    let request = LaunchRequest {
        lease: lease.clone(),
        operation_id: receipt.operation_id,
        authority,
        phase: run.phase.clone(),
        source: Some(SourceSelection::Committed {
            repository: binding.repository.id.clone(),
            revision: binding.revision.clone(),
        }),
        delivery: replacement,
        unfinished: None,
        source_advanced: None,
        retried_unfinished: None,
        validation_retry: None,
        review_retry: None,
        review_round: None,
        max_seconds: 3600,
        max_turns: run.max_turns,
    };
    let plan = Plan::read(runtime, request)?;
    if plan.workspace.evaluation.is_some() || plan.workflow.digest != workflow.digest {
        return Err(Error::new(
            "resume_incompatible",
            "cold workflow changed or belongs to an evaluation",
        ));
    }
    let records = PrivateDir::open(
        &runtime
            .registration
            .root()
            .path()
            .join("worker-records")
            .join(receipt.operation_id.to_string()),
    )?;
    let saved: Value = records.read("contract.json")?;
    let restored_authority =
        match (&plan.request.authority, saved.get("authority")) {
            (WorkerAuthority::Authorized(current), Some(value)) if !value.is_null() => {
                let signed = serde_json::from_value(value.clone()).map_err(|_| {
                    Error::new(
                        "invalid_attestation",
                        "retained authority envelope is malformed",
                    )
                })?;
                WorkerAuthority::Authorized(current.recover_worker_contract(
                    &runtime.services.key,
                    &signed,
                    receipt.runtime_id,
                    receipt.operation_id,
                    receipt.expected.get("scope").ok_or_else(|| {
                        Error::new("record_corrupt", "cold admission lacks scope")
                    })?,
                )?)
            }
            _ => plan.request.authority.clone(),
        };
    let mut context = saved
        .get("context")
        .cloned()
        .unwrap_or_else(|| plan.context.clone());
    if plan.request.delivery.is_none() {
        context["attempt"] = json!(run.attempt);
    }
    let repository = boundary.plan().repository.clone();
    let scoped = grants(&plan, &repository)?;
    let scope_paths = match &plan.request.authority {
        WorkerAuthority::Authorized(gate) => gate
            .child_scope()
            .map(|scope| scope.write_paths.clone())
            .unwrap_or_default(),
        _ => unreachable!(),
    };
    let prepared = worker_run::prepare(Preparation {
        config: plan.config.clone(), workflow: plan.workflow.clone(), authority: restored_authority,
        operation_id: receipt.operation_id, role: plan.role, phase: run.phase.phase.clone(), context,
        isolation: boundary.plan().clone(), repository: Some(RepositoryContext { root: repository, revision: Some(binding.revision.commit.clone()), scope_paths }),
        repository_id: Some(binding.repository.clone()), read_resources: scoped.resources, review: None, required_context: vec![], project_doc_max_bytes: 32_768,
        provider_discovery: ProviderDiscovery::Unavailable { reason: "Pinned app-server does not expose an authenticated project-instruction discovery inventory; supplied accessible files are separately hashed".into() }, known_secrets: vec![],
    })?;
    let contract = prepared.contract().clone();
    if saved["scope"].as_str() != Some(contract.scope_digest())
        || saved["tool_contract"].as_str() != Some(contract.tools().digest())
        || saved["instructions"]
            != serde_json::to_value(&contract.instructions().manifest)
                .map_err(|_| Error::invalid("encode restored manifest"))?
        || saved["isolation"]
            != serde_json::to_value(contract.isolation().plan())
                .map_err(|_| Error::invalid("encode restored scope"))?
    {
        return Err(Error::new(
            "resume_incompatible",
            "cold assembly does not reproduce its original protected manifest and scope",
        ));
    }
    let checkpoint =
        worker_run::ColdValidationCheckpoint::read(&records, &runtime.services.key, receipt)?;
    let retained =
        crate::worker::recover_session_with_validation(&records, receipt, checkpoint.as_ref())?
            .ok_or_else(|| Error::new("resume_incompatible", "cold protected session is absent"))?
            .resume_candidate()?;
    let git_executable = executable("git")?;
    let execution = Arc::new(WorkerExecution {
        git: Arc::new(Git::from_pinned_executable(&git_executable)?),
        git_executable,
        ownership: Arc::new(ownership),
        binding,
        limits: ExecutionLimits::default(),
        check_outputs: BTreeMap::new(),
    });
    let processes = Arc::new(crate::process_records::ProcessJournal::new(
        PrivateDir::open(&runtime.registration.root().path().join("worker-processes"))?,
        config.runtime.id,
    )?);
    worker_run::ColdResumeSession::observe(
        retained,
        contract,
        execution,
        records.path().to_path_buf(),
        runtime.control.clone(),
        processes,
    )
}
