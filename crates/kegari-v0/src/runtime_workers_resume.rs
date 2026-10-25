//! Native consumer for a separately recovered same-process terminal session.
//! Selection/metadata observation precede this path; no clone or after_create is
//! repeated, and no cold journal may synthesize the original joined completion.
use super::*;

pub async fn prepare_resumed(
    runtime: Arc<RuntimeCoordinator>,
    request: LaunchRequest,
    predecessor: Uuid,
    recovery: worker_run::VerifiedProviderRecovery,
    source: crate::git::VerifiedReusableSource,
) -> Result<AdmittedWorker> {
    if request.unfinished.is_some()
        || request.source_advanced.is_some()
        || request.retried_unfinished.is_some()
        || request.review_retry.is_some()
    {
        return Err(Error::new(
            "resume_incompatible",
            "session resume cannot replace another retained admission cause",
        ));
    }
    let mut plan = Plan::read(&runtime, request)?;
    let previous = runtime.retained_completion(predecessor)?.ok_or_else(|| {
        Error::new(
            "reconciliation_required",
            "native session resume requires its actual joined predecessor",
        )
    })?;
    let old_contract = previous.contract.as_ref().ok_or_else(|| {
        Error::new(
            "resume_incompatible",
            "original worker contract is unavailable",
        )
    })?;
    let old_execution = previous.execution.as_ref().ok_or_else(|| {
        Error::new(
            "resume_incompatible",
            "original owned execution is unavailable",
        )
    })?;
    let retained = recovery.observation().retained();
    if source.binding() != &old_execution.binding
        || previous.operation_id != predecessor
        || retained.state().binding.operation_id != predecessor.to_string()
        || retained.reference().phase != plan.request.phase
        || plan.source.as_ref().is_none_or(|selected| {
            selected.repository != old_execution.binding.repository.id
                || selected.revision != old_execution.binding.revision
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
    let prepared = worker_run::prepare(Preparation {
        config: plan.config.clone(), workflow: plan.workflow.clone(), authority: plan.request.authority.clone(),
        operation_id: plan.request.operation_id, role: plan.role, phase: plan.request.phase.phase.clone(), context: plan.context.clone(),
        isolation: boundary.plan().clone(), repository: Some(RepositoryContext { root: repository, revision: old_execution.binding.revision.commit.clone(), scope_paths }),
        repository_id: plan.repository_id.clone(), read_resources: grants.resources, review: plan.review.clone(), required_context: vec![], project_doc_max_bytes: 32_768,
        provider_discovery: ProviderDiscovery::Unavailable { reason: "Pinned app-server does not expose an authenticated project-instruction discovery inventory; supplied accessible files are separately hashed".into() }, known_secrets: vec![],
    })?;
    let directory = runtime.services.housekeeping.prepare_directory_reuse(
        &source,
        boundary,
        &old_execution.ownership,
        &plan.workflow,
    )?;
    let bridge = Arc::new(worker_run::VerifiedSessionResume::prepare(
        previous.clone(),
        recovery,
        &prepared,
    )?);
    let contract = prepared.contract().clone();
    let schedule = runtime.next_attempt_for_session_resume(&contract, &bridge, &directory)?;
    if plan.request.max_turns > schedule.max_turns {
        return Err(Error::new(
            "turns_exhausted",
            "resumed session exceeds original turn cap",
        ));
    }
    let permit = runtime.admit_resumed_worker(
        &plan.request.lease,
        RunAdmission {
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
        },
        &contract,
        &bridge,
        &directory,
    )?;
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
        records.write("contract.json", &json!({"scope":contract.scope_digest(),"instructions":contract.instructions().manifest,"tool_contract":contract.tools().digest(),"isolation":contract.isolation().plan()}))?;
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
            boundary: Arc::new(JournaledSandboxBoundary::new(
                processes.clone(),
                permit.clone(),
            )),
            ownership: old_execution.ownership.clone(),
            binding,
            limits: old_execution.limits.clone(),
            check_outputs: BTreeMap::new(),
        });
        let hooks: Arc<dyn RunHooks> = if contract
            .instructions()
            .manifest
            .permissions
            .repository_code_execution
        {
            let hook_records = records.child("hooks")?;
            let hooks = ExecutorRunHooks::new_reused(
                ExecutorHookServices {
                    contract: contract.clone(),
                    control: runtime.control.clone(),
                    host: runtime.clone(),
                    execution: execution.clone(),
                    registry: runtime.executions.clone(),
                    records: hook_records,
                    session_id: permit.operation_id,
                },
                &enrollment,
            )?;
            runtime
                .services
                .housekeeping
                .register_hook_ledger_location(permit.operation_id, &records.child("hooks")?)?;
            Arc::new(hooks)
        } else {
            Arc::new(NonExecutingRunHooks::new(
                contract.clone(),
                runtime.control.clone(),
                runtime.clone(),
            )?)
        };
        let admitted = bridge.admitted(&permit, &runtime.control, enrollment)?;
        runtime.register_session_resume(&permit, &admitted)?;
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
