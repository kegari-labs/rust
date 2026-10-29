//! Explicit local one-trial execution on the existing runtime. Neither a report
//! nor configuration can manufacture this in-memory command authorization.
use super::*;
use crate::control::Receipt;
use crate::control::ReceiptPhase;
use crate::evaluation::LiveAuthority;
use crate::evaluation::LiveLease;
use crate::evaluation::LiveRequest;
use crate::evaluation::TrialKey;
use crate::runtime_workers;
use crate::runtime_workers::LaunchRequest;
use kegari_control_api::identity::Actor;
use kegari_control_api::identity::Principal;
use kegari_types::api::{CommandName, CommandRequest};
use kegari_types::run::RunReference;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
#[path = "runtime_evaluation_owned_return.rs"]
mod owned_return;
use owned_return::{CleanupOwner, CleanupState, EvaluationTerminal};
#[path = "runtime_evaluation_context.rs"]
mod context_operand;
#[path = "runtime_evaluation_deadline.rs"]
mod deadline;
#[path = "runtime_evaluation_report.rs"]
mod settled_report;
#[path = "runtime_evaluation_source.rs"]
mod source;
#[path = "runtime_evaluation_subject.rs"]
mod subject;
#[path = "runtime_evaluation_tasks.rs"]
mod tasks;
pub(crate) use source::VerifiedEvaluationSource;

#[derive(Default)]
pub(super) struct Evaluations {
    tasks: tasks::EvaluationTasks,
    requests: Mutex<BTreeMap<Uuid, Arc<VerifiedEvaluationRequest>>>,
}
struct VerifiedEvaluationRequest {
    actor: Actor,
    command: Receipt,
    request: CommandRequest,
    lease: Lease,
    workspace: Uuid,
    configuration: Arc<kegari_config::Config>,
    binding: kegari_config::EvaluationBinding,
    suite: crate::evaluation::SuiteManifest,
    variant: crate::evaluation::Variant,
    trial: TrialKey,
    identity: String,
    subject: subject::TrialSubject,
    deadline: deadline::EvaluationDeadline,
    prepared: Mutex<Option<PreparedEvaluation>>,
    source: Mutex<Option<Arc<VerifiedEvaluationSource>>>,
    preparation_started: AtomicBool,
    report_started: AtomicBool,
    failure_recorded: Mutex<bool>,
    cleanup: Mutex<CleanupState>,
    canary: Mutex<Option<Arc<crate::evaluation::CanaryMaterialization>>>,
    context_operand: Mutex<Option<Arc<crate::evaluation::OwnedCanaryPathOperand>>>,
}
struct PreparedEvaluation {
    canary_context_required: bool,
    child_evidence_digest: Option<String>,
    source: Arc<VerifiedEvaluationSource>,
    authority: WorkerAuthority,
}
// Private transient collection, not a deserializable terminal authority.
// Report durability follows the settled deadline decision, like run_suite.
struct PendingEvaluation {
    store: crate::evaluation::ReportStore,
    report_root: PathBuf,
    observed: Result<crate::evaluation::TrialReport>,
    terminal: Result<EvaluationTerminal>,
}
fn fresh_child_responsibility(
    variant: &crate::evaluation::Variant,
) -> Option<crate::authority::Responsibility> {
    use crate::authority::Responsibility;
    use kegari_context::instructions::AgentRole;
    if variant.phase != "implementation" {
        return None;
    }
    match (variant.case_id.as_str(), variant.id.as_str(), variant.role) {
        ("V0-E01", "implementation", AgentRole::Implementation)
        | ("V0-E02", "no_change", AgentRole::Implementation)
        | (
            "V0-E09",
            "unfamiliar_tool" | "stale_guidance" | "unavailable_capability" | "unborn_repository",
            AgentRole::Implementation,
        ) => Some(Responsibility::Implementation),
        ("V0-E01", "testing", AgentRole::Testing)
        | ("V0-E09", "missing_tests", AgentRole::Testing) => Some(Responsibility::Testing),
        ("V0-E01", "documentation", AgentRole::Documentation) => {
            Some(Responsibility::Documentation)
        }
        _ => None,
    }
}
impl VerifiedEvaluationRequest {
    fn current(
        &self,
        runtime: &RuntimeCoordinator,
        snapshot: &crate::control::ControlSnapshot,
    ) -> Result<()> {
        self.deadline.remaining()?;
        runtime.registration.verify_active_owner(&self.lease)?;
        let config = runtime.configuration.snapshot()?;
        crate::control_command::verify_current_actor(&config, &self.actor, "evaluate")?;
        runtime
            .configuration
            .authority_fences()?
            .check_actor(&self.actor, "evaluate")?;
        if crate::config::config_revision(&config)?
            != crate::config::config_revision(&self.configuration)?
            || self.request.expected_control_generation != Some(snapshot.intent.generation)
            || !snapshot.admission_open
            || !snapshot.reconciled
            || snapshot.draining
            || snapshot.service_stopping
            || snapshot
                .fenced_operations
                .contains(&self.request.request_id)
            || snapshot.intent.runtime_id != self.request.runtime_id
        {
            return Err(Error::new(
                "stale_result",
                "explicit evaluation configuration, ownership or current admission changed",
            ));
        }
        // Queue state changes only after the task returns. Whole Prepared command
        // equality is appropriate here; it is never used as a model request digest.
        let actual = runtime.control.operation(self.command.operation_id)?;
        if canonical_digest(1, &actual)? != canonical_digest(1, &self.command)?
            || actual.phase != ReceiptPhase::Prepared
        {
            return Err(Error::new(
                "request_conflict",
                "evaluation command is no longer the actual pending local request",
            ));
        }
        Ok(())
    }
    // Existing bounded model work uses its real permit and pinned contract.
    // Pause/ordinary reload closes only future admissions; revocation still wins.
    fn active(&self, runtime: &RuntimeCoordinator) -> Result<()> {
        self.deadline.remaining()?;
        runtime.registration.verify_active_owner(&self.lease)?;
        let config = runtime.configuration.snapshot()?;
        crate::control_command::verify_current_actor(&config, &self.actor, "evaluate")?;
        let fences = runtime.configuration.authority_fences()?;
        fences.check_actor(&self.actor, "evaluate")?;
        fences.check_workspace(self.workspace)?;
        let actual = runtime.control.operation(self.command.operation_id)?;
        if canonical_digest(1, &actual)? != canonical_digest(1, &self.command)?
            || actual.phase != ReceiptPhase::Prepared
        {
            return Err(Error::new(
                "request_conflict",
                "evaluation no longer has its original pending command",
            ));
        }
        let source = self
            .source
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "evaluation source lock failed"))?;
        let source = source.as_ref().ok_or_else(|| {
            Error::new(
                "evaluation_unavailable",
                "active evaluation has no retained actual source",
            )
        })?;
        runtime
            .services
            .inputs
            .evaluation_providers()
            .ok_or_else(|| {
                Error::new(
                    "evaluation_unavailable",
                    "evaluation namespace registry is absent",
                )
            })?
            .verify_active(self.workspace, self.request.request_id, source.digest())
    }
    fn current_worker(
        &self,
        runtime: &RuntimeCoordinator,
        snapshot: &crate::control::ControlSnapshot,
        operation: Uuid,
        authority: &WorkerAuthority,
        phase: &kegari_types::run::PhaseKey,
        max_turns: u32,
    ) -> Result<()> {
        self.current(runtime, snapshot)?;
        let prepared = self.prepared.lock().map_err(|_| {
            Error::new(
                "runtime_unavailable",
                "evaluation source binding lock failed",
            )
        })?;
        let prepared = prepared.as_ref().ok_or_else(|| {
            Error::new(
                "evaluation_unavailable",
                "explicit evaluation has no actual verified source",
            )
        })?;
        if operation != self.request.request_id
            || phase.phase != self.variant.phase
            || phase.retry_grant.is_some()
            || max_turns > self.variant.bounds.max_turns
            || phase.work != authority.work_key()?
            || (self.binding.child.is_none()
                && !matches!(authority, WorkerAuthority::Independent(_)))
            || (self.binding.child.is_some()
                && !matches!(authority, WorkerAuthority::Authorized(gate) if Some(gate.work().parent)==self.binding.child.as_ref().map(|c|c.parent_id) && gate.work().child==self.binding.child.as_ref().map(|c|c.child_id)))
        {
            return Err(Error::new(
                "evaluation_fidelity",
                "model admission differs from the one declared fresh trial",
            ));
        }
        prepared.source.validate(runtime)?;
        runtime
            .services
            .inputs
            .evaluation_providers()
            .ok_or_else(|| {
                Error::new(
                    "evaluation_unavailable",
                    "evaluation namespace registry is absent",
                )
            })?
            .verify_active(self.workspace, operation, prepared.source.digest())?;
        if prepared.authority.digest()? != authority.digest()? {
            return Err(Error::new(
                "evaluation_fidelity",
                "original evaluation authority changed",
            ));
        }
        authority.compare(&runtime.current_admission_authority(authority)?)?;
        Ok(())
    }
}
impl RuntimeCoordinator {
    pub(crate) fn evaluation_targets(&self, request: &CommandRequest) -> Result<Vec<String>> {
        let workspace = evaluation_workspace(request)?;
        let config = self.configuration.snapshot()?;
        if config
            .workspaces
            .values()
            .find(|w| w.id == workspace)
            .is_none_or(|w| w.evaluation.is_none())
        {
            return Err(Error::new(
                "scope_violation",
                "evaluation workspace is not explicitly configured",
            ));
        }
        Ok(vec![format!(
            "evaluation:{}:{workspace}",
            config.runtime.id
        )])
    }
    pub(crate) async fn execute_evaluation(
        self: &Arc<Self>,
        actor: Actor,
        request: CommandRequest,
        parent: Receipt,
    ) -> Result<Receipt> {
        let proof = self.observe_evaluation_request(actor, request, parent)?;
        let operation = proof.request.request_id;
        let id = proof.identity.clone();
        let task_runtime = self.clone();
        let task_proof = proof.clone();
        self.control.serialize_observation(|snapshot| {
            proof.current(self, snapshot)?;
            proof.subject.register(self)?;
            let mut requests = self.evaluations.requests.lock().map_err(|_| {
                Error::new("runtime_unavailable", "evaluation command map lock failed")
            })?;
            if let Some(old) = requests.get(&operation) {
                if old.identity != id {
                    return Err(Error::new(
                        "request_conflict",
                        "evaluation operation identity changed",
                    ));
                }
            } else {
                if requests.len() >= 65_536 {
                    return Err(Error::new(
                        "record_limit",
                        "evaluation command inventory exceeds protected history bound",
                    ));
                }
                requests.insert(operation, proof.clone());
            }
            drop(requests);
            self.evaluations.tasks.start(operation, id, async move {
                task_runtime.run_evaluation(task_proof).await
            })
        })?;
        self.evaluations.tasks.wait(operation).await
    }
    fn observe_evaluation_request(
        &self,
        actor: Actor,
        request: CommandRequest,
        parent: Receipt,
    ) -> Result<Arc<VerifiedEvaluationRequest>> {
        kegari_control_api::api::validate_command_request(&request, self.runtime_id)?;
        if request.command != CommandName::EvaluationRun
            || !matches!(actor.principal, Some(Principal::Unix { .. }))
            || request.request_id != crate::control_command::command_effect_id(parent.operation_id)
            || parent.runtime_id != self.runtime_id
            || parent.actor != actor.identity
            || parent.kind != "command/EvaluationRun"
        {
            return Err(Error::new(
                "access_denied",
                "evaluation requires its actual authenticated local command",
            ));
        }
        let mut original = request.clone();
        original.request_id = parent.request_id;
        let mut mutation = crate::control_command::mutation_request(
            &actor,
            &original,
            self.evaluation_targets(&request)?,
        );
        mutation.kind = "command/EvaluationRun".into();
        mutation.expected.insert(
            "evaluation_trial".into(),
            kegari_control_api::api::evaluation_trial_identity(&request)?,
        );
        mutation.payload = json!({"arguments":request.args,"secret_digest":Value::Null});
        let expected = canonical_digest(
            1,
            &json!({"kind":mutation.kind,"targets":mutation.canonical_targets,"expected":mutation.expected,"payload":mutation.payload,"principal":actor.principal}),
        )?;
        if parent.payload_digest != expected
            || parent.expected != mutation.expected
            || parent.canonical_targets != mutation.canonical_targets
        {
            return Err(Error::new(
                "request_conflict",
                "evaluation arguments do not match the protected command request",
            ));
        }
        let configuration = Arc::new(self.configuration.snapshot()?);
        self.services.inputs.configure_evaluations(&configuration)?;
        let workspace = evaluation_workspace(&request)?;
        let configured = configuration
            .workspaces
            .values()
            .find(|w| w.id == workspace)
            .ok_or_else(|| Error::new("scope_violation", "evaluation workspace is absent"))?;
        let binding = configured
            .evaluation
            .clone()
            .ok_or_else(|| Error::new("scope_violation", "workspace is not evaluation-only"))?;
        binding.validate(&configuration, configured)?;
        binding.validate_resolved_paths(&configuration, configured)?;
        let suite = crate::evaluation::SuiteManifest::load(Path::new(&binding.suite_manifest))?;
        let trial = TrialKey {
            case_id: evaluation_text(&request, "case")?.into(),
            variant: evaluation_text(&request, "variant")?.into(),
            profile: evaluation_text(&request, "profile")?.into(),
            repetition: u32::try_from(
                request.args["repetition"]
                    .as_u64()
                    .ok_or_else(|| Error::invalid("evaluation repetition missing"))?,
            )
            .map_err(|_| Error::invalid("evaluation repetition exceeds u32"))?,
        };
        if evaluation_text(&request, "suite_digest")? != binding.suite_digest
            || suite.digest()? != binding.suite_digest
            || trial.case_id != binding.case
            || trial.variant != binding.variant
            || trial.profile != binding.profile
            || !suite.plan()?.contains(&trial)
            || request.expected_config_revision.as_deref()
                != Some(crate::config::config_revision(&configuration)?.as_str())
        {
            return Err(Error::new(
                "evaluation_fidelity",
                "evaluation command differs from the configured exact suite/trial",
            ));
        }
        let variant = suite
            .variants
            .iter()
            .find(|v| v.id == trial.variant && v.case_id == trial.case_id)
            .cloned()
            .ok_or_else(|| Error::invalid("declared trial missing"))?;
        let profile = suite
            .profiles
            .iter()
            .find(|p| p.name == trial.profile)
            .and_then(|p| p.binding.as_ref())
            .ok_or_else(|| {
                Error::new(
                    "evaluation_unavailable",
                    "declared model/account profile remains unresolved",
                )
            })?;
        let role_supported = if binding.child.is_some() {
            trial.repetition == 1
                && fresh_child_responsibility(&variant).is_some()
                && ((variant.id == "unborn_repository") == (binding.schema_version == 3))
        } else {
            variant.phase == "independent_review"
                && variant.role == kegari_context::instructions::AgentRole::IndependentReview
                && configured.profiles.independent_review.as_ref() == Some(&profile.profile)
        };
        if variant.bounds.max_turns > configuration.runtime.max_turns
            || !role_supported
            || !matches!(variant.seed, crate::evaluation::Seed::Fresh)
            || configuration.connections[&configured.codex_connection].account_id
                != profile.account_id
        {
            return Err(Error::new(
                "evaluation_fidelity",
                "native evaluator requires an exact migrated fresh review or first child-trial contract",
            ));
        }
        let lease = self.control.active_lease()?;
        if parent.owner_generation != lease.owner_generation {
            return Err(Error::new(
                "ownership_conflict",
                "evaluation command belongs to an earlier host owner",
            ));
        }
        let subject = subject::TrialSubject::derive(&suite, &binding, &trial, &request, &parent)?;
        let identity = canonical_digest(
            1,
            &json!({"command":parent,"request":request,"suite":suite.digest()?,"trial":trial,"lease":lease,"subject":subject.reference()}),
        )?;
        let deadline = deadline::EvaluationDeadline::new(std::time::Duration::from_secs(
            variant.bounds.max_seconds.min(suite.max_seconds),
        ))?;
        let proof = Arc::new(VerifiedEvaluationRequest {
            actor,
            command: parent,
            request,
            lease,
            workspace,
            configuration,
            binding,
            suite,
            variant,
            trial,
            identity,
            subject,
            deadline,
            prepared: Mutex::new(None),
            source: Mutex::new(None),
            preparation_started: AtomicBool::new(false),
            report_started: AtomicBool::new(false),
            canary: Mutex::new(None),
            context_operand: Mutex::new(None),
            failure_recorded: Mutex::new(false),
            cleanup: Mutex::new(CleanupState::Empty),
        });
        self.control
            .serialize_observation(|snapshot| proof.current(self, snapshot))?;
        Ok(proof)
    }
    async fn run_evaluation(
        self: &Arc<Self>,
        proof: Arc<VerifiedEvaluationRequest>,
    ) -> Result<Receipt> {
        let settled = proof
            .deadline
            .settle(self.run_evaluation_inner(proof.clone()), || {
                self.cancel_expired_evaluation(&proof)
            })
            .await;
        match settled.result {
            Ok(pending) => self.publish_settled_evaluation(&proof, pending, settled.deadline),
            Err(error) => {
                let error = settled.deadline.unwrap_or(error);
                if let Ok(owner) = proof.canary.lock()
                    && let Some(owner) = owner.as_ref()
                {
                    owner.note_failure(&error);
                }
                Err(error)
            }
        }
    }

    fn cancel_expired_evaluation(&self, proof: &VerifiedEvaluationRequest) -> Result<()> {
        let operations = [
            source::source_operation(proof.request.request_id)?,
            runtime_workers::preparation_operation(proof.request.request_id)?,
            proof.request.request_id,
        ];
        let mut first = None;
        for operation in operations {
            let result = self.control.fence(operation).and_then(|()| {
                self.executions
                    .cancel_fenced(&self.control, operation)
                    .map(|_| ())
            });
            if let Err(error) = result {
                first.get_or_insert(error);
            }
        }
        first.map_or(Ok(()), Err)
    }
    async fn run_evaluation_inner(
        self: &Arc<Self>,
        proof: Arc<VerifiedEvaluationRequest>,
    ) -> Result<PendingEvaluation> {
        self.control
            .serialize_observation(|snapshot| proof.current(self, snapshot))?;
        proof.subject.verify(self)?;
        let report_parent = PrivateDir::create(Path::new(&proof.binding.report_root))?;
        PrivateDir::create(Path::new(&proof.binding.fixture_root))?;
        let report_root = report_parent
            .path()
            .join(proof.request.request_id.to_string());
        let report = crate::evaluation::ReportStore::create(&report_root, &proof.suite)?;
        report.started(&proof.trial)?;
        proof
            .report_started
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let destination =
            Path::new(&proof.binding.fixture_root).join(proof.request.request_id.to_string());
        let suite_root = Path::new(&proof.binding.suite_manifest)
            .parent()
            .ok_or_else(|| Error::invalid("suite requires parent"))?;
        let grader = crate::evaluation::prepare_private_scorer(
            suite_root,
            &proof.variant,
            &PrivateDir::open(&report_root)?,
            proof.request.request_id,
        )?;
        let canary =
            if proof.variant.canary_diagnostic.is_some() {
                let owner = crate::evaluation::CanaryMaterialization::pending(
                    crate::evaluation::CanaryStageInput {
                        runtime: self.runtime_id,
                        workspace: proof.workspace,
                        command: proof.command.operation_id,
                        operation: proof.request.request_id,
                        suite: &proof.suite,
                        variant: &proof.variant,
                        trial: &proof.trial,
                        deadline: proof.deadline.instant(),
                    },
                )?;
                // Retain before any fallible policy/randomness/private-file work.
                *proof.canary.lock().map_err(|_| {
                    Error::new("runtime_unavailable", "canary retention lock failed")
                })? = Some(owner.clone());
                owner.stage(suite_root, &grader)?;
                Some(owner)
            } else {
                None
            };
        proof.deadline.remaining()?;
        let mut fixture = crate::evaluation::prepare_worker_fixture(
            suite_root,
            &proof.variant,
            &destination,
            grader.path(),
            Path::new(&proof.configuration.runtime.execution_root),
            proof.request.request_id,
        )?;
        if let Some(owner) = canary {
            fixture.attach_canary(owner)?;
        }
        proof.deadline.remaining()?;
        let source = Arc::new(source::observe(self.clone(), proof.clone(), &fixture).await?);
        let providers = self.services.inputs.evaluation_providers().ok_or_else(|| {
            Error::new(
                "evaluation_unavailable",
                "concrete synthetic provider namespace is missing",
            )
        })?;
        self.control.serialize_observation(|snapshot| {
            proof.current(self, snapshot)?;
            source.validate(self)?;
            providers.activate(&source)?;
            *proof.source.lock().map_err(|_| {
                Error::new(
                    "runtime_unavailable",
                    "evaluation source retention lock failed",
                )
            })? = Some(source.clone());
            Ok(())
        })?;
        let mut projection = self
            .observations
            .refresh_bounded(
                proof
                    .deadline
                    .remaining()?
                    .min(std::time::Duration::from_secs(60)),
            )
            .await?;
        let authority = if let Some(child) = &proof.binding.child {
            let parent = projection.parent(proof.workspace, child.parent_id)?;
            if !parent.authorization_published {
                let gate = parent.authorization_gate(Utc::now(), Duration::seconds(30))?;
                let owner = self.clone();
                let checked = proof.clone();
                let checked_source = source.digest().to_owned();
                let publisher = Arc::new(crate::preparation::AuthorizationPublisher::with_inputs(
                    self.configuration.clone(),
                    self.control.clone(),
                    self.observations.clone(),
                    self.services.secrets.clone(),
                    self.services.key.clone(),
                    self.services.inputs.clone(),
                    Arc::new(move |lease| {
                        // This callback also runs under ControlHost serialization.
                        // It must not reenter control or observe a replacement command.
                        owner.verify_owner(lease)?;
                        checked.deadline.remaining()?;
                        let config = owner.configuration.snapshot()?;
                        crate::control_command::verify_current_actor(
                            &config,
                            &checked.actor,
                            "evaluate",
                        )?;
                        let fences = owner.configuration.authority_fences()?;
                        fences.check_actor(&checked.actor, "evaluate")?;
                        fences.check_workspace(checked.workspace)?;
                        owner
                            .services
                            .inputs
                            .evaluation_providers()
                            .ok_or_else(|| {
                                Error::new(
                                    "evaluation_unavailable",
                                    "evaluation registry disappeared",
                                )
                            })?
                            .verify_active(
                                checked.workspace,
                                checked.request.request_id,
                                &checked_source,
                            )
                    }),
                ));
                // Keep polling this owned publication through its normal bounded
                // provider requests and retirement. The outer deadline fences;
                // it never drops a Sent publication future and invents its exit.
                publisher.publish(&proof.lease, gate).await?;
                proof.deadline.remaining()?;
                projection = self
                    .observations
                    .refresh_bounded(
                        proof
                            .deadline
                            .remaining()?
                            .min(std::time::Duration::from_secs(60)),
                    )
                    .await?;
            }
            let parent = projection.parent(proof.workspace, child.parent_id)?;
            let gate = parent.fresh_gate(
                Some(child.child_id),
                crate::reconcile::GatePurpose::NewOperation,
                &self.dependency_outcomes()?,
                Utc::now(),
                Duration::seconds(30),
            )?;
            if gate.child_scope().is_none_or(|scope| {
                Some(scope.responsibility.clone()) != fresh_child_responsibility(&proof.variant)
                    || scope.write_repository.as_deref()
                        != Some(proof.binding.repository_id.to_string().as_str())
            }) {
                return Err(Error::new(
                    "evaluation_fidelity",
                    "fresh child responsibility or repository differs from the declared trial",
                ));
            }
            let declared_profile = proof
                .suite
                .profiles
                .iter()
                .find(|p| p.name == proof.trial.profile)
                .and_then(|p| p.binding.as_ref())
                .ok_or_else(|| Error::invalid("evaluation profile missing"))?;
            if gate.profile() != &declared_profile.profile {
                return Err(Error::new(
                    "evaluation_fidelity",
                    "actual child routing differs from declared profile",
                ));
            }
            WorkerAuthority::Authorized(gate)
        } else {
            let gate = projection.independent_gate(
                proof.workspace,
                proof.binding.repository_id,
                proof.subject.number(),
                Utc::now(),
                Duration::seconds(30),
            )?;
            if Some(gate.pull_request().base_sha.as_str())
                != source.base().map(|r| r.commit.as_str())
                || Some(gate.pull_request().head_sha.as_str())
                    != source.head().map(|r| r.commit.as_str())
            {
                return Err(Error::new(
                    "evaluation_fidelity",
                    "native independent gate differs from actual supplied Git objects",
                ));
            }
            WorkerAuthority::Independent(gate)
        };
        if let WorkerAuthority::Authorized(gate) = &authority {
            fixture.attach_child_context(crate::evaluation::HostChildContext::from_parent(
                projection.parent(proof.workspace, gate.work().parent)?,
                gate,
                &self.services.key,
            )?)?;
        }
        let phase = kegari_types::run::PhaseKey {
            work: authority.work_key()?,
            generation: match &authority {
                WorkerAuthority::Independent(g) => g.review_identity().into(),
                WorkerAuthority::Authorized(g) => canonical_digest(1, &g.work().generation)?,
            },
            phase: proof.variant.phase.clone(),
            retry_grant: None,
        };
        let child_evidence_digest = if let WorkerAuthority::Authorized(gate) = &authority {
            let parent = projection.parent(proof.workspace, gate.work().parent)?;
            let context = runtime_workers::authorized_context(
                parent,
                gate,
                &phase.phase,
                proof.request.request_id,
                &[proof.binding.repository_id.to_string()],
                runtime_workers::scoped_evidence(self, proof.workspace, &parent.key)?,
            )?;
            Some(canonical_digest(
                1,
                context.get("evidence").ok_or_else(|| {
                    Error::new("evaluation_fidelity", "original child evidence is absent")
                })?,
            )?)
        } else {
            None
        };
        *proof.prepared.lock().map_err(|_| {
            Error::new(
                "runtime_unavailable",
                "evaluation source binding lock failed",
            )
        })? = Some(PreparedEvaluation {
            canary_context_required: fixture.requires_canary_context()?,
            child_evidence_digest,
            source,
            authority: authority.clone(),
        });
        let source = runtime_workers::resolve_source(self, &authority).await?;
        let request = LaunchRequest {
            lease: proof.lease.clone(),
            operation_id: proof.request.request_id,
            authority,
            phase,
            source,
            delivery: None,
            unfinished: None,
            source_advanced: None,
            retried_unfinished: None,
            review_retry: None,
            validation_retry: None,
            review_round: None,
            max_seconds: proof.variant.bounds.max_seconds,
            max_turns: proof.variant.bounds.max_turns,
        };
        let operand = self.make_evaluation_context_operand(&proof, &request, &mut fixture)?;
        proof
            .preparation_started
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let worker =
            runtime_workers::prepare_evaluation_admitted(self.clone(), request, operand).await;
        let worker = match worker {
            runtime_workers::EvaluationPreparationReturn::Ready(worker) => worker,
            runtime_workers::EvaluationPreparationReturn::Returned { error, owner } => {
                proof.retain_cleanup(CleanupOwner::Preparation(owner))?;
                return Err(error);
            }
            runtime_workers::EvaluationPreparationReturn::Rejected(error) => return Err(error),
        };
        let mut admitted = match (*worker).into_trial_observed() {
            Ok(trial) => trial,
            Err(runtime_workers::TrialTransferFailure::Owned { owner, error }) => {
                proof.retain_cleanup(CleanupOwner::Transfer(owner))?;
                return Err(error);
            }
            Err(runtime_workers::TrialTransferFailure::Unused { owner, error }) => {
                proof.retain_cleanup(CleanupOwner::Unused(owner))?;
                return Err(error);
            }
        };
        let fixture = match admitted.bind_fixture(&proof.variant, fixture, &report_root) {
            Ok(fixture) => fixture,
            Err(error) => {
                proof.retain_cleanup(CleanupOwner::Trial(Box::new(admitted)))?;
                return Err(error);
            }
        };
        let remaining = match proof.deadline.remaining() {
            Ok(remaining) => remaining,
            Err(error) => {
                proof.retain_cleanup(CleanupOwner::Trial(Box::new(admitted)))?;
                return Err(error);
            }
        };
        let mut live = RuntimeLiveAuthority {
            runtime: self.clone(),
            proof: proof.clone(),
            max_seconds: remaining.as_secs().max(1),
        };
        // End the operation's mutable borrow before moving the real trial into
        // the cleanup slot on an operation-construction failure.
        let observed = match admitted.operation(&fixture, &report_root) {
            Ok(operation) => Ok(crate::evaluation::run_live_trial_until(
                &proof.suite,
                &proof.variant,
                proof.trial.clone(),
                operation,
                &mut live,
                remaining,
                proof.deadline.instant(),
            )
            .await),
            Err(error) => Err(error),
        };
        // The operation borrow has ended. Retain the real owner before any
        // additional fallible diagnostic lock, readback, or report work.
        proof.retain_cleanup(CleanupOwner::Trial(Box::new(admitted)))?;
        let mut observed = observed?;
        // Early live-path returns still retain actual diagnostic files. This
        // readback remains inside work settlement, before native finalization
        // and immutable publication; no extra deadline is created.
        let canary = proof
            .canary
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "canary retention lock failed"))?
            .clone();
        if let Some(owner) = canary {
            match &mut observed {
                Ok(report) => {
                    report.canary_diagnostic = Some(owner.finalize());
                    if report.canary_output.is_none() {
                        report.canary_output = owner.finalize_output();
                    }
                }
                Err(error) => {
                    owner.note_failure(error);
                    let _ = owner.finalize();
                    let _ = owner.finalize_output();
                }
            }
        }
        if let Ok(report) = &mut observed {
            report.omissions.push(format!("GitHub and Linear observations/feedback used the explicitly declared isolated synthetic namespace for workspace {}; no real-provider qualification is claimed. Codex, account, process and sandbox execution remain native.",proof.workspace));
        }
        // Grading/report errors never skip actual native finalization.
        let terminal = self.settle_evaluation_owner(&proof).and_then(|closed| {
            closed.ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "evaluation retains its actual cleanup owner",
                )
            })
        });
        Ok(PendingEvaluation {
            store: report,
            report_root,
            observed,
            terminal,
        })
    }
    fn publish_settled_evaluation(
        &self,
        proof: &VerifiedEvaluationRequest,
        mut pending: PendingEvaluation,
        deadline: Option<Error>,
    ) -> Result<Receipt> {
        if let Ok(report) = &mut pending.observed {
            settled_report::classify(report, deadline.as_ref(), pending.terminal.as_ref().err());
        }
        // The decision is settled: durability below is owned cleanup outside
        // the execution deadline. Never reclassify after immutable publication.
        let report_write = match &pending.observed {
            Ok(report) => pending.store.finish(report),
            Err(_) => Ok(()),
        };
        let linkage = json!({"schema_version":1,"runtime":self.runtime_id,"command":proof.command.operation_id,"command_digest":canonical_digest(1,&proof.command)?,"worker":proof.request.request_id,"suite":proof.binding.suite_digest,"trial":proof.trial,"report_root":pending.report_root,"report_digest":pending.observed.as_ref().ok().map(|r|canonical_digest(1,r)).transpose()?,"report_error":pending.observed.as_ref().err(),"report_write_error":report_write.as_ref().err(),"deadline_error":deadline,"terminal_error":pending.terminal.as_ref().err(),"terminal_operation":pending.terminal.as_ref().ok().map(EvaluationTerminal::operation),"synthetic_provider_namespace":proof.workspace,"source_observation":proof.prepared.lock().map_err(|_|Error::new("runtime_unavailable","evaluation source linkage lock failed"))?.as_ref().map(|p|p.source.digest().to_owned())});
        self.registration.root().child("evaluation-trials")?.write(
            &format!("{}.json", proof.request.request_id),
            &self
                .services
                .key
                .sign("native_evaluation_trial", &linkage)?,
        )?;
        report_write?;
        if let Some(error) = deadline {
            return Err(error);
        }
        let terminal = pending.terminal?;
        pending.observed?;
        self.control.operation(terminal.receipt_operation())
    }
    pub(crate) fn check_evaluation_worker(
        &self,
        snapshot: &crate::control::ControlSnapshot,
        operation: Uuid,
        authority: &WorkerAuthority,
        phase: &kegari_types::run::PhaseKey,
        max_turns: u32,
    ) -> Result<()> {
        let workspace = authority_settings(authority).0;
        let config = self.configuration.snapshot()?;
        let evaluation = config
            .workspaces
            .values()
            .find(|w| w.id == workspace)
            .is_some_and(|w| w.evaluation.is_some());
        let proof = self
            .evaluations
            .requests
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "evaluation command map lock failed"))?
            .get(&operation)
            .cloned();
        match (evaluation, proof) {
            (false, None) => Ok(()),
            (true, Some(proof)) => {
                proof.current_worker(self, snapshot, operation, authority, phase, max_turns)
            }
            _ => Err(Error::new(
                "evaluation_unavailable",
                "evaluation-only admission lacks its exact live explicit command",
            )),
        }
    }
    pub(crate) fn ordinary_worker_transfer(&self, operation: Uuid, workspace: Uuid) -> Result<()> {
        let config = self.configuration.snapshot()?;
        if config
            .workspaces
            .values()
            .find(|w| w.id == workspace)
            .is_some_and(|w| w.evaluation.is_some())
            || self
                .evaluations
                .requests
                .lock()
                .map_err(|_| {
                    Error::new("runtime_unavailable", "evaluation command map lock failed")
                })?
                .contains_key(&operation)
        {
            return Err(Error::new(
                "evaluation_fidelity",
                "explicit trial admission cannot be consumed by the ordinary worker runner",
            ));
        }
        Ok(())
    }
    pub(crate) fn evaluation_trial_transfer(
        &self,
        permit: &Permit,
        contract: &Arc<WorkerContract>,
        options: &crate::worker_run::RunOptions,
    ) -> Result<()> {
        let proof = self
            .evaluations
            .requests
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "evaluation command map lock failed"))?
            .get(&permit.operation_id)
            .cloned()
            .ok_or_else(|| {
                Error::new(
                    "evaluation_unavailable",
                    "trial transfer requires its actual explicit local command",
                )
            })?;
        self.control.serialize_observation(|snapshot| {
            if !snapshot.active_operations.iter().any(|p| {
                p.operation_id == permit.operation_id
                    && p.lease == permit.lease
                    && p.scope_digest == permit.scope_digest
                    && p.work_key == permit.work_key
            }) || options.resume.is_some()
                || options.max_seconds != proof.variant.bounds.max_seconds
                || options.max_turns != proof.variant.bounds.max_turns
            {
                return Err(Error::new(
                    "evaluation_fidelity",
                    "trial transfer changes the actual original permit or finite session bounds",
                ));
            }
            let budget = {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| Error::new("runtime_unavailable", "evaluation run lock failed"))?;
                let run = state.runs.get(&permit.operation_id).ok_or_else(|| {
                    Error::new("worker_binding", "evaluation original run missing")
                })?;
                if run.joined || !Arc::ptr_eq(&run.contract, contract) {
                    return Err(Error::new(
                        "worker_binding",
                        "trial transfer substitutes its actual registered contract",
                    ));
                }
                run.budget.clone()
            };
            proof.current_worker(
                self,
                snapshot,
                permit.operation_id,
                contract.authority(),
                &budget.phase,
                options.max_turns,
            )
        })
    }
    pub(crate) async fn drain_evaluations(&self) -> Result<()> {
        self.evaluations.tasks.drain_finished().await?;
        drain_joined_owners(self.evaluations.tasks.joined_operations()?, |operation| {
            self.drain_joined_evaluation(operation)
        })
    }
    fn drain_joined_evaluation(&self, operation: Uuid) -> Result<()> {
        let proof = self
            .evaluations
            .requests
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "evaluation request lock failed"))?
            .get(&operation)
            .cloned();
        let Some(proof) = proof else {
            return Err(Error::new(
                "ownership_conflict",
                "joined evaluation lost original command",
            ));
        };
        let cleanup = self.settle_evaluation_owner(&proof);
        if let Some(Err(error)) = self.evaluations.tasks.joined_result(operation)? {
            let closure = self.close_returned_evaluation(&proof).and_then(|closed| {
                if closed {
                    Ok(true)
                } else {
                    self.close_never_admitted_evaluation(&proof)
                }
            });
            let never_admitted = closure.as_ref().is_ok_and(|closed| *closed);
            self.record_evaluation_failure(&proof, &error, never_admitted)?;
            closure?;
        }
        cleanup?;
        self.close_returned_evaluation(&proof)?;
        self.control.serialize_observation(|snapshot| {
            if snapshot
                .active_operations
                .iter()
                .any(|p| p.operation_id == operation)
                || snapshot.unresolved_operations.contains_key(&operation)
            {
                return Ok(());
            }
            let (completed, registered) = {
                let state = self.state.lock().map_err(|_| {
                    Error::new("runtime_unavailable", "evaluation completion lock failed")
                })?;
                let Some(completed) = state.completions.get(&operation).cloned() else {
                    return Ok(());
                };
                let registered = state.runs.get(&operation).cloned().ok_or_else(|| {
                    Error::new(
                        "worker_binding",
                        "joined evaluation lost original registered admission",
                    )
                })?;
                (completed, registered)
            };
            if !registered.joined
                || completed
                    .contract
                    .as_ref()
                    .is_none_or(|contract| !Arc::ptr_eq(contract, &registered.contract))
            {
                return Err(Error::new(
                    "worker_binding",
                    "evaluation completion differs from its actual registered worker",
                ));
            }
            if !completed.local_exit_confirmed {
                return Ok(());
            }
            let actual = self.control.operation(operation)?;
            if !closed_model_observation(
                &actual,
                &registered.permit,
                &registered.budget,
                &completed.observation(),
            )? {
                return Ok(());
            }
            let prepared = proof.prepared.lock().map_err(|_| {
                Error::new("runtime_unavailable", "evaluation prepared lock failed")
            })?;
            let Some(prepared) = prepared.as_ref() else {
                return Ok(());
            };
            let exit = VerifiedEvaluationExit {
                workspace: proof.workspace,
                operation,
                source: prepared.source.digest().into(),
            };
            self.services
                .inputs
                .evaluation_providers()
                .ok_or_else(|| Error::new("evaluation_unavailable", "evaluation registry missing"))?
                .close(&exit)
        })?;
        Ok(())
    }
    /// Only an actually joined same-process task that never entered native
    /// preparation may produce this separate namespace exit. A missing model
    /// receipt alone never proves a model/worker joined.
    fn close_never_admitted_evaluation(&self, proof: &VerifiedEvaluationRequest) -> Result<bool> {
        let operation = proof.request.request_id;
        if proof.preparation_started.load(Ordering::SeqCst)
            || self.evaluations.tasks.joined_result(operation)?.is_none()
        {
            return Ok(false);
        }
        let source = proof
            .source
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "evaluation source lock failed"))?
            .clone();
        let Some(source) = source else {
            // No successfully activated namespace belongs to this command.
            return Ok(false);
        };
        let preparation = crate::control::preparation_operation(operation)?;
        let operations = [operation, preparation, source.metadata_operation()];
        for id in operations {
            self.executions.observe_exited(id)?;
        }
        self.control.serialize_observation(|snapshot| {
            self.registration.verify_control_owner()?;
            if proof.preparation_started.load(Ordering::SeqCst)
                || self.evaluations.tasks.joined_result(operation)?.is_none()
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "evaluation never-admitted task identity changed",
                ));
            }
            let command = self.control.operation(proof.command.operation_id)?;
            if command.runtime_id != proof.command.runtime_id
                || command.owner_generation != proof.command.owner_generation
                || command.request_id != proof.command.request_id
                || command.payload_digest != proof.command.payload_digest
                || command.expected != proof.command.expected
                || command.canonical_targets != proof.command.canonical_targets
                || command.actor != proof.command.actor
                || command.kind != proof.command.kind
            {
                return Err(Error::new(
                    "record_corrupt",
                    "original evaluation command identity changed",
                ));
            }
            // Command completion is asynchronous and may already have changed
            // its phase; immutable request identity, task join and stage are exact.
            for id in [operation, preparation] {
                match self.control.operation(id) {
                    Err(error) if error.code == "record_missing" => (),
                    Ok(_) => {
                        return Err(Error::new(
                            "reconciliation_required",
                            "never-admitted evaluation has a worker/preparation receipt",
                        ));
                    }
                    Err(error) => return Err(error),
                }
            }
            if self
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "evaluation run lock failed"))?
                .runs
                .contains_key(&operation)
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "never-admitted evaluation has a registered model",
                ));
            }
            source.verify_closed_metadata(self)?;
            let active = self.executions.active_operations()?;
            let scope = format!(
                "evaluation:{}:{}",
                proof.configuration.runtime.id, proof.workspace
            );
            if operations.iter().any(|id| active.contains(id))
                || snapshot
                    .active_operations
                    .iter()
                    .any(|permit| operations.contains(&permit.operation_id))
                || snapshot
                    .unresolved_operations
                    .iter()
                    .any(|(id, targets)| operations.contains(id) || targets.contains(&scope))
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "evaluation retains an actual local or remote effect",
                ));
            }
            let exit = VerifiedNeverAdmittedEvaluationExit {
                namespace: VerifiedEvaluationExit {
                    workspace: proof.workspace,
                    operation,
                    source: source.digest().into(),
                },
            };
            self.services
                .inputs
                .evaluation_providers()
                .ok_or_else(|| Error::new("evaluation_unavailable", "evaluation registry missing"))?
                .close(&exit.namespace)?;
            Ok(true)
        })
    }
    fn record_evaluation_failure(
        &self,
        proof: &VerifiedEvaluationRequest,
        error: &Error,
        never_admitted: bool,
    ) -> Result<()> {
        if !proof.report_started.load(Ordering::SeqCst) {
            // The durable command still records failure if the report store
            // itself could not be created; never create a replacement attempt.
            return Ok(());
        }
        let mut recorded = proof.failure_recorded.lock().map_err(|_| {
            Error::new(
                "runtime_unavailable",
                "evaluation failure recording lock failed",
            )
        })?;
        if *recorded {
            return Ok(());
        }
        let root = Path::new(&proof.binding.report_root).join(proof.request.request_id.to_string());
        let interrupted = !never_admitted
            || matches!(
                error.code.as_str(),
                "evaluation_deadline" | "timeout" | "operation_uncertain" | "interrupted"
            );
        let canary_diagnostic = proof
            .canary
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "canary retention lock failed"))?
            .as_ref()
            .map(|owner| owner.diagnostic())
            .transpose()?;
        crate::evaluation::ReportStore::finish_owned_failure_with_canary(
            &root,
            &proof.suite,
            &proof.variant,
            &proof.trial,
            error,
            interrupted,
            crate::evaluation::CanaryFailureObservations {
                inputs: canary_diagnostic,
                outputs: proof
                    .canary
                    .lock()
                    .map_err(|_| Error::new("runtime_unavailable", "canary retention lock failed"))?
                    .as_ref()
                    .and_then(|owner| owner.output_snapshot()),
            },
        )?;
        let linkage = json!({"schema_version":1,"runtime":self.runtime_id,"command":proof.command.operation_id,"command_digest":canonical_digest(1,&proof.command)?,"worker":proof.request.request_id,"suite":proof.binding.suite_digest,"trial":proof.trial,"report_root":root,"error":error,"task_joined":true,"model_result_inferred":false,"synthetic_provider_namespace":proof.workspace});
        let records = self.registration.root().child("evaluation-trials")?;
        let name = format!("{}-failure.json", proof.request.request_id);
        let signed = self
            .services
            .key
            .sign("native_evaluation_trial_failure", &linkage)?;
        let value = serde_json::to_value(&signed)
            .map_err(|_| Error::invalid("failure linkage encoding failed"))?;
        let stored = match records.read::<Value>(&name) {
            Ok(old) if old == value => Ok(()),
            Ok(_) => Err(Error::new(
                "record_corrupt",
                "retained evaluation failure linkage changed",
            )),
            Err(error) if error.code == "record_missing" => records.write(&name, &value),
            Err(error) => Err(error),
        };
        stored?;
        *recorded = true;
        Ok(())
    }
    pub(crate) fn pending_evaluations(&self) -> Result<usize> {
        let mut pending = self.evaluations.tasks.pending()?;
        let requests: Vec<_> = self
            .evaluations
            .requests
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "evaluation requests lock failed"))?
            .values()
            .cloned()
            .collect();
        for proof in requests {
            if self
                .evaluations
                .tasks
                .joined_result(proof.request.request_id)?
                .is_some()
                && proof.cleanup_pending()?
            {
                pending = pending.checked_add(1).ok_or_else(|| {
                    Error::new("bounded_input", "evaluation pending count overflow")
                })?;
            }
        }
        Ok(pending)
    }
}
// Receipt-data validation only; caller additionally requires the actual joined
// task, cached completion Arc and original registered contract identity.
pub(super) fn closed_model_observation(
    receipt: &Receipt,
    permit: &Permit,
    budget: &RunReference,
    observed: &Value,
) -> Result<bool> {
    let reference = crate::control::run_reference_from_receipt(receipt)?
        .ok_or_else(|| Error::new("record_corrupt", "evaluation model has no typed admission"))?;
    if receipt.operation_id != permit.operation_id
        || receipt.request_id != permit.operation_id
        || receipt.runtime_id != permit.lease.runtime_id
        || receipt.owner_generation != permit.lease.owner_generation
        || receipt.expected.get("scope") != Some(&permit.scope_digest)
        || receipt.canonical_targets != [permit.work_key.clone()]
        || canonical_digest(1, &reference)? != canonical_digest(1, budget)?
    {
        return Err(Error::new(
            "record_corrupt",
            "evaluation model retirement differs from original admission",
        ));
    }
    let Some(result) = &receipt.observed_result else {
        return Ok(false);
    };
    Ok(receipt.phase == ReceiptPhase::Reconciled
        && result["local_execution"] == json!("exited")
        && result["unresolved_effects"]
            .as_array()
            .is_some_and(Vec::is_empty)
        && result.get("observation") == Some(observed))
}
struct RuntimeLiveAuthority {
    runtime: Arc<RuntimeCoordinator>,
    proof: Arc<VerifiedEvaluationRequest>,
    max_seconds: u64,
}
impl LiveAuthority for RuntimeLiveAuthority {
    fn acquire(&mut self, request: &LiveRequest) -> Result<Box<dyn LiveLease>> {
        let (permit, contract) = {
            let state = self
                .runtime
                .state
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "evaluation run lock failed"))?;
            let run = state
                .runs
                .get(&self.proof.request.request_id)
                .ok_or_else(|| {
                    Error::new(
                        "worker_binding",
                        "evaluation model admission is not registered",
                    )
                })?;
            if run.joined {
                return Err(Error::new(
                    "worker_binding",
                    "evaluation cannot reuse a completed model contract",
                ));
            }
            (run.permit.clone(), run.contract.clone())
        };
        let manifest = &contract.instructions().manifest;
        if request.suite_digest != self.proof.binding.suite_digest
            || request.trial != self.proof.trial
            || request.profile.profile != manifest.profile
            || request.profile.account_id != contract.account_id()
            || request.max_seconds != self.max_seconds
            || request.max_seconds == 0
            || request.max_seconds > self.proof.variant.bounds.max_seconds
            || request.max_turns != self.proof.variant.bounds.max_turns
            || request.execution_root != contract.isolation().plan().execution_root
            || request.isolation_digest != contract.isolation().digest()
            || request.tool_contract_digest != contract.tools().digest()
            || !request.independent_review
        {
            return Err(Error::new(
                "evaluation_fidelity",
                "live request differs from the registered original worker contract",
            ));
        }
        let lease = RuntimeLiveLease {
            runtime: self.runtime.clone(),
            proof: self.proof.clone(),
            permit,
            reference: self.proof.identity.clone(),
        };
        lease.check()?;
        Ok(Box::new(lease))
    }
}
struct RuntimeLiveLease {
    runtime: Arc<RuntimeCoordinator>,
    proof: Arc<VerifiedEvaluationRequest>,
    permit: Permit,
    reference: String,
}
impl LiveLease for RuntimeLiveLease {
    fn check(&self) -> Result<()> {
        self.runtime
            .control
            .validate_permit(&self.permit, || Ok(()))?;
        self.runtime.control.serialize_observation(|snapshot| {
            self.proof.active(&self.runtime)?;
            require_active_evaluation_permit(snapshot, &self.permit)?;
            let state = self.runtime.state.lock().map_err(|_| {
                Error::new("runtime_unavailable", "evaluation current run lock failed")
            })?;
            let run = state.runs.get(&self.permit.operation_id).ok_or_else(|| {
                Error::new("worker_binding", "original evaluation run disappeared")
            })?;
            if run.permit.scope_digest != self.permit.scope_digest {
                return Err(Error::new(
                    "worker_binding",
                    "evaluation original permit changed",
                ));
            }
            let authority = run.contract.authority().clone();
            drop(state);
            authority.compare(&self.runtime.retained_authority(&authority)?)
        })
    }
    fn authority_reference(&self) -> &str {
        &self.reference
    }
}
fn require_active_evaluation_permit(
    snapshot: &crate::control::ControlSnapshot,
    permit: &Permit,
) -> Result<()> {
    if snapshot.fenced_operations.contains(&permit.operation_id)
        || !snapshot.active_operations.iter().any(|active| {
            active.operation_id == permit.operation_id
                && active.lease == permit.lease
                && active.control_generation == permit.control_generation
                && active.scope_digest == permit.scope_digest
                && active.work_key == permit.work_key
                && active.registered_at == permit.registered_at
        })
    {
        return Err(Error::new(
            "access_denied",
            "evaluation original model permit is absent or fenced",
        ));
    }
    Ok(())
}
fn evaluation_workspace(request: &CommandRequest) -> Result<Uuid> {
    Uuid::parse_str(evaluation_text(request, "workspace")?)
        .map_err(|_| Error::invalid("evaluation workspace must be UUID"))
}
fn evaluation_text<'a>(request: &'a CommandRequest, key: &str) -> Result<&'a str> {
    request
        .args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::invalid(format!("evaluation {key} is missing")))
}

// No model result/check capability is manufactured by this separate exit.
struct VerifiedNeverAdmittedEvaluationExit {
    namespace: VerifiedEvaluationExit,
}

pub(crate) struct VerifiedEvaluationExit {
    workspace: Uuid,
    operation: Uuid,
    source: String,
}
impl VerifiedEvaluationExit {
    pub(crate) fn workspace(&self) -> Uuid {
        self.workspace
    }
    pub(crate) fn operation(&self) -> Uuid {
        self.operation
    }
    pub(crate) fn source(&self) -> &str {
        &self.source
    }
}

// Each joined task retains its own cleanup owner. A failure for one operation
// must not prevent independently returned owners from being settled in this pass.
// Preserve the first exact error after every operation has been visited.
fn drain_joined_owners(
    operations: impl IntoIterator<Item = Uuid>,
    mut drain: impl FnMut(Uuid) -> Result<()>,
) -> Result<()> {
    let mut first_error = None;
    for operation in operations {
        if let Err(error) = drain(operation) {
            first_error.get_or_insert(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(test)]
mod command_tests {
    use super::*;
    fn request() -> CommandRequest {
        CommandRequest {
            api_version: 1,
            runtime_id: Uuid::new_v4(),
            request_id: Uuid::new_v4(),
            command: CommandName::EvaluationRun,
            args: json!({"workspace":Uuid::new_v4(),"suite_digest":bytes_digest(b"suite"),"case":"declared","variant":"independent","profile":"bound","repetition":1}),
            expected_config_revision: Some(bytes_digest(b"config")),
            expected_control_generation: Some(1),
        }
    }
    #[test]
    fn trial_command_cannot_supply_authority_or_omit_observed_revisions() {
        let original = request();
        kegari_control_api::api::validate_command_request(&original, original.runtime_id).unwrap();
        for field in [
            "actor",
            "lease",
            "permit",
            "manifest",
            "report",
            "source",
            "max_turns",
        ] {
            let mut changed = original.clone();
            changed.args[field] = json!({"pretend":"authority"});
            assert_eq!(
                kegari_control_api::api::validate_command_request(&changed, changed.runtime_id)
                    .unwrap_err()
                    .code,
                "invalid_input"
            );
        }
        let mut changed = original.clone();
        changed.expected_config_revision = None;
        assert!(
            kegari_control_api::api::validate_command_request(&changed, changed.runtime_id)
                .is_err()
        );
        let mut changed = original.clone();
        changed.expected_control_generation = None;
        assert!(
            kegari_control_api::api::validate_command_request(&changed, changed.runtime_id)
                .is_err()
        );
        let mut changed = original;
        changed.args["repetition"] = json!(0);
        assert!(
            kegari_control_api::api::validate_command_request(&changed, changed.runtime_id)
                .is_err()
        );
    }
}

#[cfg(test)]
mod model_retirement_observation_tests {
    use super::*;
    #[test]
    fn actual_control_retirement_uses_the_closed_outer_observation() {
        // Real Control journal and typed admission; worker observation is plain
        // fixture data. This creates no native run/check/exit capability.
        let root = tempfile::TempDir::new().unwrap();
        let runtime = Uuid::new_v4();
        let control = crate::control::ControlHost::setup(
            PrivateDir::create(&root.path().canonicalize().unwrap().join("control")).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        control
            .lifecycle(
                crate::control::MutationRequest {
                    request_id: Uuid::new_v4(),
                    runtime_id: runtime,
                    actor: Actor {
                        identity: "fixture-operator".into(),
                        principal: None,
                        roles: std::collections::BTreeSet::from(["lifecycle".into()]),
                    },
                    kind: "lifecycle".into(),
                    canonical_targets: vec![runtime.to_string()],
                    expected: BTreeMap::new(),
                    payload: Value::Null,
                },
                crate::control::Lifecycle::Start,
                0,
            )
            .unwrap();
        let lease = Lease {
            runtime_id: runtime,
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        control.attach_reconciled(lease.clone()).unwrap();
        let id = Uuid::new_v4();
        let scope = bytes_digest(b"scope");
        let budget = RunReference {
            workspace: Uuid::new_v4(),
            account: "fixture-account".into(),
            normalized_state: None,
            phase: kegari_types::run::PhaseKey {
                work: "evaluation-fixture".into(),
                generation: bytes_digest(b"generation"),
                phase: "independent_review".into(),
                retry_grant: None,
            },
            profile_digest: bytes_digest(b"profile"),
            instruction_digest: bytes_digest(b"instructions"),
            max_attempts: 1,
            max_turns: 1,
            max_review_rounds: 1,
            attempt: 1,
        };
        let permit = control
            .admit_run(
                &lease,
                crate::control::RunAdmission {
                    request_id: id,
                    work_key: budget.phase.work.clone(),
                    scope_digest: scope,
                    reference: budget.clone(),
                },
                || Ok(()),
            )
            .unwrap();
        control.validate_permit(&permit, || Ok(())).unwrap();
        control
            .serialize_observation(|snapshot| require_active_evaluation_permit(snapshot, &permit))
            .unwrap();
        control
            .lifecycle(
                crate::control::MutationRequest {
                    request_id: Uuid::new_v4(),
                    runtime_id: runtime,
                    actor: Actor {
                        identity: "fixture-operator".into(),
                        principal: None,
                        roles: std::collections::BTreeSet::from(["lifecycle".into()]),
                    },
                    kind: "lifecycle".into(),
                    canonical_targets: vec![runtime.to_string()],
                    expected: BTreeMap::new(),
                    payload: Value::Null,
                },
                crate::control::Lifecycle::Pause,
                control.snapshot().unwrap().intent.generation,
            )
            .unwrap();
        assert!(!control.snapshot().unwrap().admission_open);
        control
            .serialize_observation(|snapshot| require_active_evaluation_permit(snapshot, &permit))
            .unwrap();
        // Fence after the first validation: permits remain registered, so the
        // second serialized observation must inspect the actual fence set.
        control.validate_permit(&permit, || Ok(())).unwrap();
        control.fence(permit.operation_id).unwrap();
        assert!(
            control
                .snapshot()
                .unwrap()
                .active_operations
                .iter()
                .any(|active| active.operation_id == permit.operation_id)
        );
        assert_eq!(
            control
                .serialize_observation(|snapshot| require_active_evaluation_permit(
                    snapshot, &permit
                ))
                .unwrap_err()
                .code,
            "access_denied"
        );
        let observed = json!({"outcome":"completed","component_only":true});
        let actual = control.retire_local(id, observed.clone()).unwrap();
        assert!(closed_model_observation(&actual, &permit, &budget, &observed).unwrap());
        let mut changed = actual.clone();
        changed.observed_result = Some(observed.clone());
        assert!(!closed_model_observation(&changed, &permit, &budget, &observed).unwrap());
        changed = actual.clone();
        changed.observed_result.as_mut().unwrap()["unresolved_effects"] = json!([Uuid::new_v4()]);
        assert!(!closed_model_observation(&changed, &permit, &budget, &observed).unwrap());
        let mut changed_budget = budget.clone();
        changed_budget.max_turns += 1;
        assert!(closed_model_observation(&actual, &permit, &changed_budget, &observed).is_err());
    }
}

#[cfg(test)]
mod cleanup_iteration_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct ComponentOwner {
        operation: Uuid,
        attempts: usize,
        closed: bool,
        error: Option<Error>,
        drops: Arc<AtomicUsize>,
    }
    impl Drop for ComponentOwner {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn earlier_cleanup_error_does_not_skip_later_retained_owners() {
        // Ownership/iteration plumbing only. These values are not actual worker,
        // exit, Control or namespace capabilities and never enter runtime caches.
        let drops = Arc::new(AtomicUsize::new(0));
        let mut first = Error::new("fixture_first", "first retained cleanup is unavailable");
        first.details = json!({"original": ["unchanged", 7]});
        let later = Error::new("fixture_later", "another retained cleanup is unavailable");
        let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let mut owners: Vec<_> = ids
            .into_iter()
            .zip([Some(first.clone()), None, Some(later)])
            .map(|(operation, error)| ComponentOwner {
                operation,
                attempts: 0,
                closed: false,
                error,
                drops: drops.clone(),
            })
            .collect();
        let mut visited = Vec::new();
        let error = drain_joined_owners(ids, |operation| {
            visited.push(operation);
            let owner = owners
                .iter_mut()
                .find(|owner| owner.operation == operation)
                .unwrap();
            owner.attempts += 1;
            if let Some(error) = &owner.error {
                return Err(error.clone());
            }
            owner.closed = true;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(visited, ids);
        assert_eq!(
            canonical_digest(1, &error).unwrap(),
            canonical_digest(1, &first).unwrap()
        );
        assert!(owners.iter().all(|owner| owner.attempts == 1));
        assert!(!owners[0].closed);
        assert!(owners[1].closed);
        assert!(!owners[2].closed);
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        // A later cleanup-only pass reuses all original owners, with no replay or
        // new admission; a recovered first owner no longer masks a later error.
        owners[0].error = None;
        let error = drain_joined_owners(ids, |operation| {
            let owner = owners
                .iter_mut()
                .find(|owner| owner.operation == operation)
                .unwrap();
            owner.attempts += 1;
            if let Some(error) = &owner.error {
                return Err(error.clone());
            }
            owner.closed = true;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.code, "fixture_later");
        assert!(owners[0].closed && owners[1].closed);
        assert!(owners.iter().all(|owner| owner.attempts == 2));
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        owners[2].error = None;
        drain_joined_owners(ids, |operation| {
            let owner = owners
                .iter_mut()
                .find(|owner| owner.operation == operation)
                .unwrap();
            owner.closed = true;
            Ok(())
        })
        .unwrap();
        assert!(owners.iter().all(|owner| owner.closed));
        drop(owners);
        assert_eq!(drops.load(Ordering::SeqCst), 3);
        drain_joined_owners([], |_| panic!("empty inventory must not invoke cleanup")).unwrap();
    }
}
