//! Explicit host delivery phases. Model output remains a claim until actual
//! execution/provider observations have established the corresponding gate.
use crate::config::{CiSuite, Repository as RepositoryConfig};
use crate::control::{Actor, ControlHost, MutationRequest, Permit, Receipt};
use crate::digest::{bytes_digest, canonical_digest};
use crate::error::{Error, Result};
use crate::evidence::{
    self, ArtifactReference, AttestationKey, CheckKind, CheckRequirement, CheckRun, CheckSnapshot,
    Conclusion, Publication, RevisionBinding, Runner, Signed, WorkIdentity,
};
use crate::execution::VerifiedCheckExecution;
use crate::git::{Git, RepositoryId};
use crate::providers::{
    GithubClient, HostedProofExpectation, HostedProofVerifier, ProviderBroker, ProviderMutation,
    RepositoryIdentity,
};
use crate::reconcile::FreshGate;
use crate::storage::PrivateDir;
use crate::workflow::Workflow;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

/// Mandatory concrete delivery service used by the authenticated runtime command
/// adapter. Implementations must execute/reconcile retained effects; this trait
/// deliberately provides no default or unsupported-operation implementation.
pub trait RuntimeDeliveryDecisions: Send + Sync {
    /// Read-only canonical parent history, including retained completion errors.
    /// This projection supplies no release eligibility or execution authority.
    fn release_inspection(&self) -> Result<Vec<crate::inspection::ReleaseInspectionRow>>;
    fn no_release_inspection(&self) -> Result<Vec<crate::inspection::NoReleaseInspectionRow>>;
    fn release(
        &self,
        decision: crate::decisions::AuthorizedDecision,
    ) -> crate::app::HostFuture<'_, Receipt>;
    fn recover_release(
        &self,
        decision: crate::decisions::AuthorizedDecision,
    ) -> crate::app::HostFuture<'_, Receipt>;
    fn reconcile(
        &self,
        decision: crate::decisions::AuthorizedOperationRecovery,
    ) -> crate::app::HostFuture<'_, Receipt>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryPhase {
    SourceSealing,
    LocalValidation,
    PublishChild,
    HostedValidation,
    ReviewPublication,
    Staging,
    NoChange,
    CandidateAssembly,
    ParentEvidence,
    Release,
    Recovery,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryScope {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub phase: DeliveryPhase,
    pub work: WorkIdentity,
    pub repository: RepositoryId,
    pub revision: RevisionBinding,
    pub fresh_gate_digest: String,
    pub contract_digest: String,
    pub candidate_vector_digest: Option<String>,
}
impl DeliveryScope {
    pub fn digest(&self) -> Result<String> {
        self.work.validate()?;
        self.revision.validate()?;
        if self.schema_version != 1
            || self.runtime_id.is_nil()
            || self.repository != self.revision.repository
        {
            return Err(Error::invalid(
                "delivery scope has inconsistent runtime/repository identity",
            ));
        }
        evidence::validate_digest(&self.fresh_gate_digest)?;
        evidence::validate_digest(&self.contract_digest)?;
        if let Some(digest) = &self.candidate_vector_digest {
            evidence::validate_digest(digest)?;
        }
        if matches!(
            self.phase,
            DeliveryPhase::ParentEvidence | DeliveryPhase::Release
        ) && self.candidate_vector_digest.is_none()
        {
            return Err(Error::invalid(
                "parent evidence/release scope needs the immutable complete candidate vector",
            ));
        }
        canonical_digest(1, self)
    }
}

/// Created only from the reconciler's private gate and a live host permit.
/// A public serialized Permit alone is never dispatch authority.
#[derive(Clone, Debug)]
pub struct DeliveryAdmission {
    permit: Permit,
    observation: DeliveryObservation,
}
/// Read-only current evidence context. It never grants dispatch authority and
/// may be used while the coordinator is stopped for human acceptance/recovery.
#[derive(Clone, Debug)]
pub struct DeliveryObservation {
    gate: FreshGate,
    scope: DeliveryScope,
    admitted: Option<Permit>,
}
impl DeliveryObservation {
    pub fn fresh_gate(&self) -> &FreshGate {
        &self.gate
    }
    pub fn scope(&self) -> &DeliveryScope {
        &self.scope
    }
}
impl std::ops::Deref for DeliveryAdmission {
    type Target = DeliveryObservation;
    fn deref(&self) -> &Self::Target {
        &self.observation
    }
}
impl DeliveryAdmission {
    pub fn bind(permit: Permit, gate: FreshGate, scope: DeliveryScope) -> Result<Self> {
        let digest = scope.digest()?;
        gate.bind_permit(&permit, &digest)?;
        if gate.work() != &scope.work
            || gate.digest()? != scope.fresh_gate_digest
            || permit.lease.runtime_id != scope.runtime_id
        {
            return Err(Error::new(
                "stale_result",
                "delivery scope differs from current work admission",
            ));
        }
        Ok(Self {
            observation: DeliveryObservation {
                gate,
                scope,
                admitted: Some(permit.clone()),
            },
            permit,
        })
    }
    pub fn scope(&self) -> &DeliveryScope {
        &self.scope
    }
    pub fn operation_id(&self) -> Uuid {
        self.permit.operation_id
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryContract {
    pub configuration_name: String,
    pub repository: RepositoryId,
    pub provider: RepositoryIdentity,
    pub configuration: RepositoryConfig,
}
impl RepositoryContract {
    pub fn digest(&self) -> Result<String> {
        self.repository.validate()?;
        if self.configuration_name.trim().is_empty()
            || self.provider.id == 0
            || self.provider.owner_id == 0
            || self.repository.id != self.provider.id.to_string()
            || self.repository.hostname != self.configuration.hostname
            || self.configuration.main != "main"
            || self.configuration.staging != "staging"
            || self.configuration.review_context != evidence::REVIEW_CONTEXT
        {
            return Err(Error::invalid(
                "delivery contract requires canonical repository and branch identities",
            ));
        }
        for suite in self
            .configuration
            .ci
            .iter()
            .chain(&self.configuration.production_ci)
        {
            if suite.workflow_id == 0 {
                return Err(Error::invalid("workflow identity missing"));
            }
            suite.proof.validate()?;
        }
        self.configuration.no_change_validation.proof.validate()?;
        canonical_digest(1, self)
    }
}

pub type GateCheck = Arc<dyn Fn(&FreshGate) -> Result<()> + Send + Sync>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectRecord {
    schema_version: u32,
    admission_digest: String,
    action: String,
    payload_digest: String,
    request_id: Uuid,
    command: Signed<ProviderMutation>,
    receipt: Option<Receipt>,
}

pub struct DeliveryCoordinator {
    host: Arc<ControlHost>,
    broker: ProviderBroker,
    records: Arc<PrivateDir>,
    key: Arc<AttestationKey>,
    git: Arc<Git>,
    retained_git_sources: BTreeMap<RepositoryId, PathBuf>,
    contracts: BTreeMap<RepositoryId, RepositoryContract>,
    current_gate: GateCheck,
    executions: Arc<crate::worker::ExecutionRegistry>,
    processes: Arc<crate::process_records::ProcessJournal>,
    workers: std::sync::Mutex<BTreeMap<Uuid, WorkerBinding>>,
}
#[derive(Clone)]
struct WorkerBinding {
    permit: Permit,
    gate: FreshGate,
    instruction_digest: String,
    tool_digest: String,
}
pub struct DeliveryServices {
    pub host: Arc<ControlHost>,
    pub broker: ProviderBroker,
    pub records: Arc<PrivateDir>,
    pub key: Arc<AttestationKey>,
    pub git: Arc<Git>,
    /// Host-supplied object cache locations for preserved isolated clones. This
    /// map selects local storage only and never adds repository authority.
    pub retained_git_sources: BTreeMap<RepositoryId, PathBuf>,
    pub current_gate: GateCheck,
    pub executions: Arc<crate::worker::ExecutionRegistry>,
    pub processes: Arc<crate::process_records::ProcessJournal>,
}
impl DeliveryCoordinator {
    pub fn new(services: DeliveryServices, contracts: Vec<RepositoryContract>) -> Result<Self> {
        let mut selected = BTreeMap::new();
        for contract in contracts {
            contract.digest()?;
            if selected
                .insert(contract.repository.clone(), contract)
                .is_some()
            {
                return Err(Error::invalid("duplicate delivery repository contract"));
            }
        }
        if selected.is_empty() {
            return Err(Error::invalid(
                "delivery needs an explicit repository contract",
            ));
        }
        let mut retained_git_sources = BTreeMap::new();
        for (repository, path) in services.retained_git_sources {
            if !selected.contains_key(&repository) {
                return Err(Error::new(
                    "scope_violation",
                    "retained object source is outside selected repositories",
                ));
            }
            let canonical = path
                .canonicalize()
                .map_err(|e| Error::io("resolve retained Git object source", e))?;
            if !canonical.is_dir() {
                return Err(Error::new(
                    "prerequisite",
                    "retained Git source must be a directory",
                ));
            }
            retained_git_sources.insert(repository, canonical);
        }
        Ok(Self {
            host: services.host,
            broker: services.broker,
            records: services.records,
            key: services.key,
            git: services.git,
            retained_git_sources,
            contracts: selected,
            current_gate: services.current_gate,
            executions: services.executions,
            processes: services.processes,
            workers: std::sync::Mutex::new(BTreeMap::new()),
        })
    }
    fn github(&self) -> Result<&GithubClient> {
        self.broker
            .github
            .as_ref()
            .ok_or_else(|| Error::new("prerequisite", "selected GitHub connection is unavailable"))
    }
    fn contract(&self, admission: &DeliveryObservation) -> Result<&RepositoryContract> {
        let contract = self
            .contracts
            .get(&admission.scope.repository)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "repository is outside admitted delivery selection",
                )
            })?;
        if contract.digest()? != admission.scope.contract_digest {
            return Err(Error::new(
                "stale_result",
                "delivery repository contract changed after admission",
            ));
        }
        Ok(contract)
    }
    fn current(&self, admission: &DeliveryObservation, phases: &[DeliveryPhase]) -> Result<()> {
        if !phases.contains(&admission.scope.phase) {
            return Err(Error::new(
                "scope_violation",
                "delivery action is outside admitted phase",
            ));
        }
        self.contract(admission)?;
        match &admission.admitted {
            Some(permit) => self
                .host
                .validate_permit(permit, || (self.current_gate)(&admission.gate)),
            None => (self.current_gate)(&admission.gate),
        }
    }
    pub fn observation(
        &self,
        gate: FreshGate,
        revision: RevisionBinding,
    ) -> Result<DeliveryObservation> {
        if gate.purpose() == crate::reconcile::GatePurpose::Coordination {
            return Err(Error::new(
                "scope_violation",
                "coordination gates cannot promote delivery evidence while correction classification is pending",
            ));
        }
        (self.current_gate)(&gate)?;
        let contract = self
            .contracts
            .get(&revision.repository)
            .ok_or_else(|| Error::new("scope_violation", "evidence repository is not selected"))?;
        let scope = DeliveryScope {
            schema_version: 1,
            runtime_id: self.host.snapshot()?.intent.runtime_id,
            phase: DeliveryPhase::Recovery,
            work: gate.work().clone(),
            repository: revision.repository.clone(),
            revision,
            fresh_gate_digest: gate.digest()?,
            contract_digest: contract.digest()?,
            candidate_vector_digest: None,
        };
        scope.digest()?;
        Ok(DeliveryObservation {
            gate,
            scope,
            admitted: None,
        })
    }
    /// Every action gets a durable stable request ID before provider transmission.
    /// Existing unknown effects flow back through the broker's reconciliation
    /// refusal; this method never allocates a new ID to retry the same action.
    async fn effect(
        &self,
        admission: &DeliveryAdmission,
        action: &str,
        command: ProviderMutation,
    ) -> Result<Receipt> {
        self.effect_guarded(admission, action, command, None).await
    }
    async fn effect_guarded(
        &self,
        admission: &DeliveryAdmission,
        action: &str,
        command: ProviderMutation,
        extra: Option<GateCheck>,
    ) -> Result<Receipt> {
        if let Some(check) = &extra {
            check(admission.fresh_gate())?;
        }
        if action.is_empty() || action.len() > 128 || action.chars().any(char::is_control) {
            return Err(Error::invalid("invalid bounded delivery action identity"));
        }
        self.current(admission, &[admission.scope.phase])?;
        let digest = admission.scope.digest()?;
        let payload_digest = canonical_digest(1, &command)?;
        let leaf = format!(
            "delivery-effect-{}.json",
            &bytes_digest(format!("{}:{action}", admission.operation_id()).as_bytes())[7..]
        );
        let mut record = {
            let _lock = self.records.section_lock("delivery-effects.lock")?;
            match self.records.read::<EffectRecord>(&leaf) {
                Ok(record) => {
                    self.key.verify("delivery_mutation", &record.command)?;
                    if canonical_digest(1, &record.command.payload)? != record.payload_digest {
                        return Err(Error::new(
                            "record_corrupt",
                            "retained mutation command differs",
                        ));
                    }
                    if record.schema_version != 1
                        || record.admission_digest != digest
                        || record.action != action
                        || record.payload_digest != payload_digest
                    {
                        return Err(Error::new(
                            "operation_conflict",
                            "delivery action identity was reused for different authority or payload",
                        ));
                    }
                    record
                }
                Err(error) if error.code == "record_missing" => {
                    let record = EffectRecord {
                        schema_version: 1,
                        admission_digest: digest.clone(),
                        action: action.into(),
                        payload_digest,
                        request_id: Uuid::new_v4(),
                        command: self.key.sign("delivery_mutation", &command)?,
                        receipt: None,
                    };
                    self.records.write(&leaf, &record)?;
                    record
                }
                Err(error) => return Err(error),
            }
        };
        let gate = admission.gate.clone();
        let check = self.current_gate.clone();
        let receipt = self
            .broker
            .mutate(
                self.host.clone(),
                MutationRequest {
                    request_id: record.request_id,
                    runtime_id: admission.scope.runtime_id,
                    actor: Actor {
                        principal: None,
                        identity: format!("owner:{}", admission.permit.lease.owner_id),
                        roles: BTreeSet::new(),
                    },
                    kind: String::new(),
                    canonical_targets: vec![],
                    expected: BTreeMap::from([
                        ("delivery_scope".into(), digest),
                        ("work".into(), admission.scope.work.digest()?),
                        (
                            "control_generation".into(),
                            admission.permit.control_generation.to_string(),
                        ),
                    ]),
                    payload: Value::Null,
                },
                Some(admission.permit.clone()),
                None,
                command,
                move || {
                    check(&gate)?;
                    if let Some(extra) = &extra {
                        extra(&gate)?;
                    }
                    Ok(())
                },
            )
            .await?;
        record.receipt = Some(receipt.clone());
        self.records.write(&leaf, &record)?;
        Ok(receipt)
    }
    pub fn verify_local_checks(
        &self,
        admission: &DeliveryAdmission,
        workflow: &Workflow,
        executions: &[&VerifiedCheckExecution],
    ) -> Result<LocalPassed> {
        self.current(
            admission,
            &[
                DeliveryPhase::LocalValidation,
                DeliveryPhase::PublishChild,
                DeliveryPhase::NoChange,
                DeliveryPhase::HostedValidation,
                DeliveryPhase::Staging,
            ],
        )?;
        if workflow.digest != admission.gate.workflow_digest() {
            return Err(Error::new(
                "stale_result",
                "local checks use another workflow snapshot",
            ));
        }
        let contract = self.contract(admission)?;
        let required = local_requirements(contract, workflow)?;
        let mut runs = Vec::new();
        for execution in executions {
            let record = execution.production_evidence()?;
            if record.discovered.is_some()
                || record.binding.work != admission.scope.work
                || record.workflow_digest != workflow.digest
                || record.binding.repository != admission.scope.repository
                || record.binding.revision.commit != admission.scope.revision.tested_commit
                || record.binding.revision.tree != admission.scope.revision.tested_tree
                || record.kind != CheckKind::Local
                || !record.successful
                || record.before != record.after
            {
                return Err(Error::new(
                    "validation_gate",
                    "local execution is not current admitted source evidence",
                ));
            }
            runs.push(CheckRun {
                id: record.check_name.clone(),
                kind: CheckKind::Local,
                operation_id: record.binding.operation_id,
                work_digest: record.binding.work.digest()?,
                revision: admission.scope.revision.clone(),
                workflow_id: None,
                run_id: None,
                attempt: 1,
                runner: Runner::LocalSandbox,
                command: record.command.argv.clone(),
                environment_identity: record.binding.isolation_digest.clone(),
                started_at: Some(record.started_at),
                completed_at: Some(record.completed_at),
                conclusion: Conclusion::Success,
                artifacts: record.artifacts.iter().map(|a| a.reference()).collect(),
            });
        }
        let snapshot = CheckSnapshot {
            pagination_complete: true,
            observed_at: Utc::now(),
            runs,
        };
        let checks = evidence::verify_checks(
            &required,
            &admission.scope.work,
            &admission.scope.revision,
            &snapshot,
        )?;
        Ok(LocalPassed {
            work: admission.scope.work.clone(),
            revision: admission.scope.revision.clone(),
            workflow_digest: workflow.digest.clone(),
            checks,
            validation: None,
            validation_archive: None,
        })
    }
}

impl DeliveryCoordinator {
    /// Host-only registration ties the pending-outcome sink to the exact admitted
    /// worker contract. Independent PR review uses its separate review job sink.
    pub fn register_worker(
        &self,
        permit: &Permit,
        contract: &crate::worker::WorkerContract,
    ) -> Result<()> {
        let crate::worker::WorkerAuthority::Authorized(gate) = contract.authority() else {
            return Err(Error::new(
                "scope_violation",
                "independent review cannot become parent/child delivery",
            ));
        };
        gate.bind_permit(permit, contract.scope_digest())?;
        self.host
            .validate_permit(permit, || (self.current_gate)(gate))?;
        let binding = WorkerBinding {
            permit: permit.clone(),
            gate: gate.clone(),
            instruction_digest: contract.instructions().manifest_digest.clone(),
            tool_digest: contract.tools().digest().into(),
        };
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| Error::new("delivery_state", "worker binding registry failed"))?;
        if let Some(prior) = workers.get(&permit.operation_id) {
            if prior.permit.scope_digest != permit.scope_digest
                || prior.instruction_digest != binding.instruction_digest
                || prior.tool_digest != binding.tool_digest
                || prior.gate.digest()? != binding.gate.digest()?
            {
                return Err(Error::new(
                    "operation_conflict",
                    "registered worker contract changed",
                ));
            }
        } else {
            workers.insert(permit.operation_id, binding);
        }
        Ok(())
    }
    fn worker_binding(&self, operation: Uuid) -> Result<WorkerBinding> {
        self.workers
            .lock()
            .map_err(|_| Error::new("delivery_state", "worker binding registry failed"))?
            .get(&operation)
            .cloned()
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "worker outcome has no host-admitted binding",
                )
            })
    }
}
impl crate::worker::OutcomeSink for DeliveryCoordinator {
    fn record(&self, outcome: &crate::worker::WorkerOutcome) -> Result<String> {
        let binding = self.worker_binding(outcome.operation_id)?;
        self.host
            .validate_permit(&binding.permit, || (self.current_gate)(&binding.gate))?;
        let correction_matches = match outcome.schema_version {
            1 => outcome.processed_correction.is_none(),
            2 => outcome
                .processed_correction
                .as_ref()
                .is_some_and(|processed| {
                    processed.transition_digest.is_none()
                        && processed.watermark == binding.gate.correction_watermark()
                        && processed.input_digest == outcome.instructions.context_digest
                }),
            _ => false,
        };
        // Corrected claims require RuntimeCoordinator's verified transition
        // reader; this direct sink has only the original immutable worker gate.
        if !correction_matches
            || outcome.proposal_id.is_nil()
            || outcome.runtime_id != binding.permit.lease.runtime_id
            || outcome.work.as_ref() != Some(binding.gate.work())
            || outcome.independent_review.is_some()
            || outcome.fresh_gate_digest != binding.gate.digest()?
            || outcome.instruction_digest != binding.instruction_digest
            || outcome.tool_contract_digest != binding.tool_digest
            || canonical_digest(crate::instructions::ASSEMBLY_SCHEMA, &outcome.instructions)?
                != binding.instruction_digest
            || outcome.instructions.operation_id != outcome.operation_id
            || outcome.session.binding.operation_id != outcome.operation_id.to_string()
            || outcome.session.binding.instruction_digest != binding.instruction_digest
            || outcome.session.binding.work_identity != binding.gate.work_key()?
            || !outcome.session.completion_is_current()
            || outcome.session.terminal.as_deref() != Some("completed")
            || outcome
                .session
                .completed_turns
                .get(&outcome.completed_turn)
                .map(String::as_str)
                != Some("completed")
            || outcome.session.intervention.is_some()
            || outcome.session.input_required
            || outcome.session.turn_id.is_some()
        {
            return Err(Error::new(
                "worker_binding",
                "outcome is not the exact completed admitted session and instruction snapshot",
            ));
        }
        let payload = serde_json::to_value(outcome)
            .map_err(|_| Error::invalid("cannot encode pending worker claim"))?;
        let signed = self.key.sign("pending_worker_claim", &payload)?;
        let bytes = serde_json::to_vec(&signed)
            .map_err(|_| Error::invalid("cannot encode pending claim attestation"))?;
        let leaf = format!("pending-worker-{}.json", outcome.proposal_id);
        let _lock = self.records.section_lock("pending-worker.lock")?;
        match self.records.read_bytes(&leaf, 8 * 1024 * 1024) {
            Ok(prior) if prior == bytes => {}
            Ok(_) => {
                return Err(Error::new(
                    "operation_conflict",
                    "worker reused proposal identity with different content",
                ));
            }
            Err(error) if error.code == "record_missing" => {
                self.records.write_bytes(&leaf, &bytes)?
            }
            Err(error) => return Err(error),
        }
        // The reference names a pending claim, never a verified completion.
        Ok(format!(
            "pending-worker:{}:{}",
            outcome.operation_id, outcome.proposal_id
        ))
    }
}

#[derive(Clone, Debug)]
pub struct LocalPassed {
    work: WorkIdentity,
    revision: RevisionBinding,
    workflow_digest: String,
    checks: Vec<CheckRun>,
    validation: Option<Box<evidence::ValidationEvidence>>,
    validation_archive: Option<crate::validation_archive::ResolvedValidationArchive>,
}
impl LocalPassed {
    pub fn checks(&self) -> &[CheckRun] {
        &self.checks
    }
    fn matches(&self, admission: &DeliveryObservation) -> Result<()> {
        if self.work != admission.scope.work
            || self.revision != admission.scope.revision
            || self.workflow_digest != admission.gate.workflow_digest()
        {
            return Err(Error::new(
                "stale_result",
                "local check capability is stale",
            ));
        }
        Ok(())
    }
}
fn local_requirements(
    contract: &RepositoryContract,
    workflow: &Workflow,
) -> Result<Vec<CheckRequirement>> {
    if contract.configuration.local_checks.is_empty() {
        return Err(Error::new(
            "validation_obligations_pending",
            "local validation obligations have not been established for this repository",
        ));
    }
    contract
        .configuration
        .local_checks
        .iter()
        .map(|name| {
            let configured = workflow
                .front_matter
                .checks
                .local
                .get(name)
                .ok_or_else(|| {
                    Error::new(
                        "incompletely_prepared",
                        "configured local check is absent from selected workflow",
                    )
                })?;
            Ok(CheckRequirement {
                id: name.clone(),
                kind: CheckKind::Local,
                workflow_id: None,
                command: configured.argv.clone(),
            })
        })
        .collect()
}

/// Bearer JWT contents are intentionally neither Debug nor Serialize.
pub struct HostedToken {
    pub expectation: HostedProofExpectation,
    pub token: String,
}
#[derive(Clone, Debug)]
pub struct HostedPassed {
    work: WorkIdentity,
    revision: RevisionBinding,
    kind: CheckKind,
    checks: Vec<CheckRun>,
    proofs: Vec<crate::providers::VerifiedHostedJob>,
}
impl HostedPassed {
    pub fn checks(&self) -> &[CheckRun] {
        &self.checks
    }
    fn matches(&self, admission: &DeliveryObservation, kind: CheckKind) -> Result<()> {
        if self.work != admission.scope.work
            || self.revision != admission.scope.revision
            || self.kind != kind
        {
            return Err(Error::new(
                "stale_result",
                "hosted check capability is stale or belongs to another gate",
            ));
        }
        Ok(())
    }
}
fn hosted_requirements(suites: &[CiSuite], kind: CheckKind) -> Result<Vec<CheckRequirement>> {
    let mut required = Vec::new();
    for suite in suites {
        // A predicate cannot be waived from a worker's claim. Until host
        // changed-path evaluation is supplied, conservatively run the suite.
        for check in &suite.required_checks {
            required.push(CheckRequirement {
                id: check.clone(),
                kind: kind.clone(),
                workflow_id: Some(suite.workflow_id),
                command: vec![],
            });
        }
    }
    if required.is_empty() {
        return Err(Error::new(
            "incompletely_prepared",
            "complete hosted suite is absent",
        ));
    }
    Ok(required)
}
fn timestamp(value: &Value, field: &str) -> Result<DateTime<Utc>> {
    value
        .get(field)
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .ok_or_else(|| {
            Error::new(
                "provider_response",
                format!("required provider timestamp {field} missing"),
            )
        })
}
struct HostedObservation<'a> {
    kind: CheckKind,
    suites: &'a [CiSuite],
    tokens: &'a [HostedToken],
    verifier: &'a HostedProofVerifier,
    owned_run: Option<u64>,
}
impl DeliveryCoordinator {
    pub async fn observe_hosted_checks(
        &self,
        admission: &DeliveryAdmission,
        kind: CheckKind,
        tokens: &[HostedToken],
        verifier: &HostedProofVerifier,
    ) -> Result<HostedPassed> {
        self.current(
            admission,
            &[
                DeliveryPhase::HostedValidation,
                DeliveryPhase::Staging,
                DeliveryPhase::NoChange,
                DeliveryPhase::Release,
            ],
        )?;
        if !matches!(kind, CheckKind::HostedCi | CheckKind::ProductionCi) {
            return Err(Error::invalid(
                "hosted observation cannot replace a local gate",
            ));
        }
        let contract = self.contract(admission)?;
        let suites = if kind == CheckKind::ProductionCi {
            &contract.configuration.production_ci
        } else {
            &contract.configuration.ci
        };
        self.observe_hosted_suite(
            admission,
            HostedObservation {
                kind,
                suites,
                tokens,
                verifier,
                owned_run: None,
            },
        )
        .await
    }
    async fn observe_hosted_suite(
        &self,
        admission: &DeliveryAdmission,
        observations: HostedObservation<'_>,
    ) -> Result<HostedPassed> {
        let HostedObservation {
            kind,
            suites,
            tokens,
            verifier,
            owned_run,
        } = observations;
        let contract = self.contract(admission)?;
        let challenge = self.hosted_challenge(admission)?;
        let required = hosted_requirements(suites, kind.clone())?;
        let github = self.github()?;
        let mut runs = Vec::new();
        let mut proofs = Vec::new();
        let mut identities = BTreeSet::new();
        for token in tokens {
            let expected = &token.expectation;
            let run = github
                .workflow_run(
                    &contract.provider,
                    expected.run_id,
                    Some(expected.run_attempt),
                )
                .await?;
            let current_run = github
                .workflow_run(&contract.provider, expected.run_id, None)
                .await?;
            let pin = suites
                .iter()
                .find(|s| s.workflow_id == run.workflow_id)
                .map(|s| &s.proof)
                .ok_or_else(|| {
                    Error::new(
                        "hosted_proof",
                        "workflow lacks its immutable host-approved proof route",
                    )
                })?;
            if current_run.attempt != run.attempt
                || run.status != "completed"
                || run.conclusion.as_deref() != Some("success")
                || expected.runtime_id != admission.scope.runtime_id
                || expected.operation_id != admission.operation_id()
                || expected.challenge != challenge.challenge
                || owned_run.is_some_and(|id| id != run.id)
                || expected.repository_id != contract.provider.id
                || expected.repository_owner_id != contract.provider.owner_id
                || expected.tested_source_sha != admission.scope.revision.tested_commit
                || expected.actor_id != run.actor_id
                || expected.trigger_sha != run.head_sha
                || expected.event_name != run.event
                || expected.event_name != pin.event_name
                || expected.workflow_ref != pin.workflow_ref
                || expected.workflow_sha != pin.workflow_sha
                || expected.job_workflow_ref != pin.job_workflow_ref
                || expected.job_workflow_sha != pin.job_workflow_sha
                || (run.event == "workflow_dispatch" && run.head_sha != pin.workflow_sha)
                || (run.event != "workflow_dispatch"
                    && run.head_sha != admission.scope.revision.head
                    && run.head_sha != admission.scope.revision.tested_commit)
            {
                return Err(Error::new(
                    "hosted_proof",
                    "workflow/run/source identity is stale or differs from admitted route",
                ));
            }
            let jobs = github.jobs(&contract.provider, run.id, run.attempt).await?;
            let job = jobs
                .iter()
                .find(|j| j.check_run_id == expected.check_run_id)
                .ok_or_else(|| {
                    Error::new(
                        "hosted_proof",
                        "signed job is absent from the complete workflow attempt",
                    )
                })?;
            if job.head_sha != run.head_sha
                || job.status != "completed"
                || job.conclusion.as_deref() != Some("success")
                || !required
                    .iter()
                    .any(|r| r.id == job.name && r.workflow_id == Some(run.workflow_id))
                || !identities.insert((run.id, run.attempt, job.check_run_id))
            {
                return Err(Error::new(
                    "validation_gate",
                    "required hosted job is failed, duplicated or outside configured suite",
                ));
            }
            let raw_checks = github.checks(&contract.provider, &run.head_sha).await?;
            let check = raw_checks
                .iter()
                .find(|c| c.id == job.check_run_id)
                .ok_or_else(|| {
                    Error::new(
                        "validation_gate",
                        "current exact-revision check run is missing",
                    )
                })?;
            if check.data.get("head_sha").and_then(Value::as_str) != Some(run.head_sha.as_str())
                || check.data.get("status").and_then(Value::as_str) != Some("completed")
                || check.data.get("conclusion").and_then(Value::as_str) != Some("success")
            {
                return Err(Error::new(
                    "validation_gate",
                    "check record is not successful exact-source evidence",
                ));
            }
            let now = Utc::now();
            let proof = verifier.verify_and_record(&token.token, expected, now, &self.records)?;
            runs.push(CheckRun {
                id: job.name.clone(),
                kind: kind.clone(),
                operation_id: admission.operation_id(),
                work_digest: admission.scope.work.digest()?,
                revision: admission.scope.revision.clone(),
                workflow_id: Some(run.workflow_id),
                run_id: Some(run.id),
                attempt: u32::try_from(run.attempt)
                    .map_err(|_| Error::invalid("run attempt exceeds supported bound"))?,
                runner: Runner::GitHubHosted,
                command: vec![],
                environment_identity: canonical_digest(1, &proof)?,
                started_at: Some(timestamp(&check.data, "started_at")?),
                completed_at: Some(timestamp(&check.data, "completed_at")?),
                conclusion: Conclusion::Success,
                artifacts: vec![ArtifactReference {
                    reference: run.url.clone(),
                    digest: canonical_digest(1, &proof)?,
                }],
            });
            proofs.push(proof);
        }
        self.current(admission, &[admission.scope.phase])?;
        let checks = evidence::verify_checks(
            &required,
            &admission.scope.work,
            &admission.scope.revision,
            &CheckSnapshot {
                pagination_complete: true,
                observed_at: Utc::now(),
                runs,
            },
        )?;
        Ok(HostedPassed {
            work: admission.scope.work.clone(),
            revision: admission.scope.revision.clone(),
            kind,
            checks,
            proofs,
        })
    }
}

#[derive(Clone, Debug)]
pub struct PublishedReview {
    signed: Signed<evidence::ReviewAttestation>,
    publication: Publication,
    status: Option<evidence::ReviewStatus>,
    verified: evidence::VerifiedReview,
}
impl PublishedReview {
    pub fn verified(&self) -> &evidence::VerifiedReview {
        &self.verified
    }
    pub fn publication(&self) -> &Publication {
        &self.publication
    }
}
#[derive(Clone, Debug)]
pub struct ChildPullRequest {
    work: WorkIdentity,
    revision: RevisionBinding,
    value: crate::providers::PullRequest,
}
impl ChildPullRequest {
    pub(crate) fn provider(&self) -> &crate::providers::PullRequest {
        &self.value
    }
    pub fn number(&self) -> u64 {
        self.value.number
    }
    pub fn url(&self) -> &str {
        &self.value.url
    }
    pub(crate) fn matches(&self, admission: &DeliveryObservation) -> Result<()> {
        if self.work != admission.scope.work || self.revision != admission.scope.revision {
            return Err(Error::new(
                "stale_result",
                "PR belongs to another delivery candidate",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct StagedDelta {
    delta: crate::git::DeltaArtifact,
    pull_request: u64,
    merge_receipt: Uuid,
    protection: evidence::RepositoryProtection,
}
#[derive(Clone, Debug)]
pub struct PendingStaging {
    receipt: Receipt,
    work: WorkIdentity,
    revision: RevisionBinding,
    protection: evidence::RepositoryProtection,
}
impl PendingStaging {
    pub(crate) fn matches(&self, admission: &DeliveryObservation) -> Result<()> {
        if self.work != admission.scope.work || self.revision != admission.scope.revision {
            return Err(Error::new(
                "stale_result",
                "staging receipt belongs to another work generation",
            ));
        }
        Ok(())
    }
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }
}
impl StagedDelta {
    pub fn delta(&self) -> &crate::git::DeltaArtifact {
        &self.delta
    }
    pub fn pull_request(&self) -> u64 {
        self.pull_request
    }
    pub fn merge_receipt(&self) -> Uuid {
        self.merge_receipt
    }
}
pub struct StagingGates<'a> {
    pub proposal: &'a ScopedChange,
    pub pull_request: &'a ChildPullRequest,
    pub local: &'a LocalPassed,
    pub hosted: &'a HostedPassed,
    pub review: &'a PublishedReview,
}
#[derive(Clone, Debug)]
pub struct ScopedChange {
    proposal: crate::git::ProposedDelta,
    child_scope_digest: String,
}

pub struct ChildObjectPublication<'a> {
    pub change: &'a ScopedChange,
    pub committed: &'a crate::git::CommittedSource,
    pub local: &'a LocalPassed,
    pub branch: &'a crate::git::BranchName,
    pub previous: Option<&'a PublishedChildBranch>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChildBranchRecord {
    schema_version: u32,
    runtime_id: Uuid,
    repository: RepositoryId,
    work: WorkIdentity,
    revision: RevisionBinding,
    source_parent: String,
    branch: crate::git::BranchName,
    receipt_id: Uuid,
}
pub struct PublishedChildBranch {
    record: Signed<ChildBranchRecord>,
}
impl PublishedChildBranch {
    pub fn branch(&self) -> &crate::git::BranchName {
        &self.record.payload.branch
    }
    pub fn revision(&self) -> &RevisionBinding {
        &self.record.payload.revision
    }
}
impl ScopedChange {
    pub fn review(&self) -> &crate::git::ReviewDelta {
        self.proposal.review()
    }
    pub fn review_digest(&self) -> Result<String> {
        self.proposal.review_digest()
    }
    pub fn patch(&self) -> &[u8] {
        self.proposal.patch()
    }
    fn matches(&self, admission: &DeliveryObservation) -> Result<()> {
        let p = self.review();
        if admission.gate.child_scope_digest() != Some(self.child_scope_digest.as_str())
            || p.repository != admission.scope.repository
            || p.parent_id != admission.scope.work.parent
            || Some(p.child_id) != admission.scope.work.child
            || p.work_generation != admission.scope.work.generation.definition
            || p.correction_generation != admission.scope.work.generation.correction
            || p.base.commit != admission.scope.revision.base
            || p.head.commit != admission.scope.revision.head
            || p.head.tree != admission.scope.revision.tested_tree
        {
            return Err(Error::new(
                "scope_violation",
                "proposed change lost its current work/path authorization",
            ));
        }
        Ok(())
    }
}
fn record_body<T: Serialize>(purpose: &str, payload: &Signed<T>) -> Result<String> {
    evidence::encode_signed_publication(purpose, payload)
}
fn remote_id(receipt: &Receipt, field: &str) -> Result<String> {
    receipt
        .remote_ids
        .get(field)
        .cloned()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "provider acknowledgement lacks observed resource identity",
            )
        })
}
impl DeliveryCoordinator {
    /// Publish exact host-created objects only after local checks on this head.
    /// The branch is created exclusively, or advanced from a previously sealed
    /// owned branch; arbitrary matching remote branch names never grant ownership.
    pub async fn publish_child_objects(
        &self,
        admission: &DeliveryAdmission,
        request: ChildObjectPublication<'_>,
    ) -> Result<PublishedChildBranch> {
        use base64::Engine as _;
        self.current(admission, &[DeliveryPhase::PublishChild])?;
        request.local.matches(admission)?;
        request.change.matches(admission)?;
        let committed = request.committed;
        if committed.revision().commit != admission.scope.revision.head
            || committed.revision().tree != admission.scope.revision.tested_tree
            || committed
                .entries()
                .iter()
                .any(|entry| !request.change.review().scope.permits(&entry.path))
        {
            return Err(Error::new(
                "scope_violation",
                "published source objects differ from the locally checked child revision/scope",
            ));
        }
        let linear =
            self.broker.linear.as_ref().ok_or_else(|| {
                Error::new("prerequisite", "selected Linear connection unavailable")
            })?;
        let child = linear
            .issue(admission.scope.work.child.ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "parent cannot publish implementation objects",
                )
            })?)
            .await?;
        let parent = linear.issue(admission.scope.work.parent).await?;
        let expected = self
            .git
            .branch_name(&child.identifier, &parent.title, 255)?;
        if child.parent_id != Some(parent.id)
            || child.workspace_id != admission.scope.work.linear_workspace
            || request.branch != &expected
        {
            return Err(Error::new(
                "branch_ownership",
                "child branch does not match the current parent identity and full title",
            ));
        }
        let contract = self.contract(admission)?;
        let previous_head = if let Some(previous) = request.previous {
            self.key.verify("child_branch", &previous.record)?;
            let old = &previous.record.payload;
            if old.schema_version != 1
                || old.runtime_id != admission.scope.runtime_id
                || old.repository != admission.scope.repository
                || old.work.linear_workspace != admission.scope.work.linear_workspace
                || old.work.parent != admission.scope.work.parent
                || old.work.child != admission.scope.work.child
                || old.branch != *request.branch
                || old.revision.head != committed.base().commit
            {
                return Err(Error::new(
                    "branch_ownership",
                    "branch update lacks its exact prior owned publication",
                ));
            }
            Some(old.revision.head.clone())
        } else {
            None
        };
        for (expected, bytes) in committed.blobs() {
            let receipt = self
                .effect(
                    admission,
                    &format!("publish-blob-{expected}"),
                    ProviderMutation::GithubBlob {
                        repository: contract.provider.clone(),
                        content_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
                    },
                )
                .await?;
            if remote_id(&receipt, "object_sha")? != *expected {
                return Err(Error::new(
                    "provenance_mismatch",
                    "provider blob differs from exact source bytes",
                ));
            }
        }
        let entries = committed
            .entries()
            .iter()
            .map(|entry| {
                Ok(crate::providers::TreeEntry {
                    path: entry.path.clone(),
                    mode: match entry.mode.as_str() {
                        "100644" => crate::providers::TreeMode::File,
                        "100755" => crate::providers::TreeMode::Executable,
                        "120000" => crate::providers::TreeMode::Symlink,
                        _ => {
                            return Err(Error::new(
                                "unsupported_repository",
                                "source object mode cannot be published",
                            ));
                        }
                    },
                    sha: entry.object.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let tree = self
            .effect(
                admission,
                "publish-source-tree",
                ProviderMutation::GithubTree {
                    repository: contract.provider.clone(),
                    base_tree: committed.base().tree.clone(),
                    entries,
                },
            )
            .await?;
        if remote_id(&tree, "object_sha")? != committed.revision().tree {
            return Err(Error::new(
                "provenance_mismatch",
                "provider tree differs from locally checked source tree",
            ));
        }
        let metadata = committed.metadata();
        let identity = crate::providers::CommitIdentity {
            name: metadata.identity.name.clone(),
            email: metadata.identity.email.clone(),
            date: metadata
                .timestamp
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        };
        let commit = self
            .effect(
                admission,
                "publish-source-commit",
                ProviderMutation::GithubCommit {
                    repository: contract.provider.clone(),
                    message: metadata.message.clone(),
                    tree: committed.revision().tree.clone(),
                    parent: committed.base().commit.clone(),
                    author: identity.clone(),
                    committer: identity,
                    expected_sha: committed.revision().commit.clone(),
                },
            )
            .await?;
        if remote_id(&commit, "object_sha")? != committed.revision().commit {
            return Err(Error::new(
                "provenance_mismatch",
                "published source commit identity differs",
            ));
        }
        let reference = format!("heads/{}", request.branch.name);
        let command = match previous_head {
            Some(expected_head) => ProviderMutation::GithubUpdateRef {
                repository: contract.provider.clone(),
                reference: reference.clone(),
                expected_head,
                head: committed.revision().commit.clone(),
            },
            None => ProviderMutation::GithubCreateRef {
                repository: contract.provider.clone(),
                reference: reference.clone(),
                head: committed.revision().commit.clone(),
            },
        };
        let receipt = self
            .effect(admission, "publish-child-branch", command)
            .await?;
        if self
            .github()?
            .reference(&contract.provider, &reference)
            .await?
            != committed.revision().commit
        {
            return Err(Error::new(
                "stale_revision",
                "published child branch moved before observation",
            ));
        }
        let observed = self
            .github()?
            .commit(&contract.provider, &committed.revision().commit)
            .await?;
        if observed.pointer("/tree/sha").and_then(Value::as_str)
            != Some(committed.revision().tree.as_str())
            || observed["parents"].as_array().is_none_or(|parents| {
                parents.len() != 1
                    || parents[0]["sha"].as_str() != Some(committed.base().commit.as_str())
            })
        {
            return Err(Error::new(
                "provenance_mismatch",
                "observed child commit tree or parent differs",
            ));
        }
        self.current(admission, &[DeliveryPhase::PublishChild])?;
        let record = self.key.sign(
            "child_branch",
            &ChildBranchRecord {
                schema_version: 1,
                runtime_id: admission.scope.runtime_id,
                repository: admission.scope.repository.clone(),
                work: admission.scope.work.clone(),
                revision: admission.scope.revision.clone(),
                source_parent: committed.base().commit.clone(),
                branch: request.branch.clone(),
                receipt_id: receipt.operation_id,
            },
        )?;
        self.records.write(
            &format!("child-branch-{}.json", receipt.operation_id),
            &record,
        )?;
        Ok(PublishedChildBranch { record })
    }

    pub async fn recover_child_branch(
        &self,
        admission: &DeliveryAdmission,
        receipt: Uuid,
    ) -> Result<PublishedChildBranch> {
        self.current(
            admission,
            &[DeliveryPhase::PublishChild, DeliveryPhase::Recovery],
        )?;
        let record: Signed<ChildBranchRecord> =
            self.records.read(&format!("child-branch-{receipt}.json"))?;
        self.key.verify("child_branch", &record)?;
        let owned = &record.payload;
        if owned.schema_version != 1
            || owned.receipt_id != receipt
            || owned.runtime_id != admission.scope.runtime_id
            || owned.repository != admission.scope.repository
            || owned.work.linear_endpoint != admission.scope.work.linear_endpoint
            || owned.work.linear_workspace != admission.scope.work.linear_workspace
            || owned.work.parent != admission.scope.work.parent
            || owned.work.child.is_none()
            || owned.work.child != admission.scope.work.child
        {
            return Err(Error::new(
                "branch_ownership",
                "retained child branch belongs to another runtime or work item",
            ));
        }
        let contract = self.contract(admission)?;
        let reference = format!("heads/{}", owned.branch.name);
        if self
            .github()?
            .reference(&contract.provider, &reference)
            .await?
            != owned.revision.head
        {
            return Err(Error::new(
                "stale_revision",
                "retained child branch moved; reconcile its actual owner before updating",
            ));
        }
        let commit = self
            .github()?
            .commit(&contract.provider, &owned.revision.head)
            .await?;
        if commit.pointer("/tree/sha").and_then(Value::as_str)
            != Some(owned.revision.tested_tree.as_str())
            || commit["parents"].as_array().is_none_or(|parents| {
                parents.len() != 1
                    || parents[0]["sha"].as_str() != Some(owned.source_parent.as_str())
            })
        {
            return Err(Error::new(
                "provenance_mismatch",
                "retained branch commit no longer matches its published tree/parent",
            ));
        }
        self.current(
            admission,
            &[DeliveryPhase::PublishChild, DeliveryPhase::Recovery],
        )?;
        Ok(PublishedChildBranch { record })
    }

    pub fn prepare_change(
        &self,
        admission: &DeliveryAdmission,
        source: &Path,
        scope: &crate::authority::ChildScope,
    ) -> Result<ScopedChange> {
        self.current(
            admission,
            &[
                DeliveryPhase::LocalValidation,
                DeliveryPhase::PublishChild,
                DeliveryPhase::ReviewPublication,
                DeliveryPhase::Staging,
            ],
        )?;
        let contract = self.contract(admission)?;
        if admission.gate.child_scope_digest() != Some(canonical_digest(1, scope)?.as_str())
            || scope.write_repository.as_deref() != Some(contract.configuration_name.as_str())
        {
            return Err(Error::new(
                "scope_violation",
                "proposed change lacks its current authorized child write boundary",
            ));
        }
        let proposal = self.git.propose_delta(
            source,
            crate::git::ProposeDelta {
                repository: admission.scope.repository.clone(),
                parent_id: admission.scope.work.parent,
                child_id: admission.scope.work.child.ok_or_else(|| {
                    Error::new("scope_violation", "parent cannot implement product changes")
                })?,
                work_generation: &admission.scope.work.generation.definition,
                correction_generation: &admission.scope.work.generation.correction,
                scope: crate::git::WriteScope {
                    paths: scope.write_paths.clone(),
                },
                base: &admission.scope.revision.base,
                head: &admission.scope.revision.head,
            },
        )?;
        Ok(ScopedChange {
            proposal,
            child_scope_digest: canonical_digest(1, scope)?,
        })
    }
    pub async fn open_child_pull_request(
        &self,
        admission: &DeliveryAdmission,
        local: &LocalPassed,
        branch: &crate::git::BranchName,
        title: &str,
        summary: &str,
    ) -> Result<ChildPullRequest> {
        self.current(admission, &[DeliveryPhase::PublishChild])?;
        local.matches(admission)?;
        let contract = self.contract(admission)?;
        let linear =
            self.broker.linear.as_ref().ok_or_else(|| {
                Error::new("prerequisite", "selected Linear connection unavailable")
            })?;
        let child =
            linear
                .issue(admission.scope.work.child.ok_or_else(|| {
                    Error::new("scope_violation", "PR requires an authorized child")
                })?)
                .await?;
        let parent = linear.issue(admission.scope.work.parent).await?;
        if child.parent_id != Some(parent.id)
            || child.workspace_id != admission.scope.work.linear_workspace
        {
            return Err(Error::new(
                "stale_result",
                "child/parent scope changed before PR publication",
            ));
        }
        let expected = self
            .git
            .branch_name(&child.identifier, &parent.title, 255)?;
        if &expected != branch {
            return Err(Error::new(
                "branch_ownership",
                "new PR branch is not the complete current canonical child name",
            ));
        }
        let correlation = self.key.sign("child_pull_request", &json!({"work":admission.scope.work,
            "operation_id":admission.operation_id(),"revision":admission.scope.revision,"branch":branch}))?;
        let body = format!(
            "{summary}\n\n{}",
            record_body("child_pull_request", &correlation)?
        );
        let receipt = self
            .effect(
                admission,
                "open-child-pr",
                ProviderMutation::GithubPullRequest {
                    repository: contract.provider.clone(),
                    head_ref: branch.name.clone(),
                    base_ref: "staging".into(),
                    expected_head: admission.scope.revision.head.clone(),
                    expected_base: admission.scope.revision.base.clone(),
                    title: title.into(),
                    body,
                },
            )
            .await?;
        let number = remote_id(&receipt, "pull_request")?
            .parse::<u64>()
            .map_err(|_| Error::new("provider_response", "created PR number is invalid"))?;
        let current = self
            .github()?
            .pull_request(&contract.provider, number)
            .await?;
        let identity = self.github()?.identity().await?;
        if current.author_id.to_string() != identity.account_id
            || current.head_ref != branch.name
            || current.head_sha != admission.scope.revision.head
            || current.base_sha != admission.scope.revision.base
            || current.base_ref != "staging"
            || current.draft
            || current.head_repository_id != Some(contract.provider.id)
        {
            return Err(Error::new(
                "provenance_mismatch",
                "observed created PR differs from owned child candidate",
            ));
        }
        self.current(admission, &[DeliveryPhase::PublishChild])?;
        Ok(ChildPullRequest {
            work: admission.scope.work.clone(),
            revision: admission.scope.revision.clone(),
            value: current,
        })
    }
    async fn github_publication<T: Serialize>(
        &self,
        admission: &DeliveryObservation,
        pr: u64,
        record_id: &str,
        purpose: &str,
        payload: &Signed<T>,
    ) -> Result<Publication> {
        let contract = self.contract(admission)?;
        let github = self.github()?;
        let writer = github.identity().await?.account_id;
        let record = github
            .comments(&contract.provider, pr)
            .await?
            .into_iter()
            .find(|c| c.id.to_string() == record_id)
            .ok_or_else(|| {
                Error::new(
                    "evidence_unpublished",
                    "published PR record has not been observed",
                )
            })?;
        if record.data.get("body").and_then(Value::as_str)
            != Some(record_body(purpose, payload)?.as_str())
            || record
                .data
                .pointer("/user/id")
                .and_then(Value::as_u64)
                .map(|v| v.to_string())
                .as_deref()
                != Some(writer.as_str())
        {
            return Err(Error::new(
                "evidence_unpublished",
                "PR publication writer or canonical payload changed",
            ));
        }
        Ok(Publication {
            provider: "github".into(),
            record_id: record_id.into(),
            writer_account_id: writer,
            payload_digest: canonical_digest(1, payload)?,
            observed_at: record.observed_at,
            url: record
                .data
                .get("html_url")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::new("provider_response", "PR publication URL missing"))?
                .into(),
        })
    }
    async fn linear_publication<T: Serialize>(
        &self,
        admission: &DeliveryObservation,
        issue: Uuid,
        record_id: &str,
        purpose: &str,
        payload: &Signed<T>,
    ) -> Result<Publication> {
        self.current(admission, &[admission.scope.phase])?;
        let linear =
            self.broker.linear.as_ref().ok_or_else(|| {
                Error::new("prerequisite", "selected Linear connection unavailable")
            })?;
        let writer = linear.identity().await?.account_id;
        let record = linear
            .comments(issue)
            .await?
            .into_iter()
            .find(|c| c.id.to_string() == record_id)
            .ok_or_else(|| {
                Error::new(
                    "evidence_unpublished",
                    "published Linear record has not been observed",
                )
            })?;
        if record.body != record_body(purpose, payload)?
            || record.issue_id != issue
            || record.bot
            || record.delegated
            || record.user_id.map(|id| id.to_string()).as_deref() != Some(writer.as_str())
        {
            return Err(Error::new(
                "evidence_unpublished",
                "Linear publication writer or canonical payload changed",
            ));
        }
        Ok(Publication {
            provider: "linear".into(),
            record_id: record_id.into(),
            writer_account_id: writer,
            payload_digest: canonical_digest(1, payload)?,
            observed_at: Utc::now(),
            url: record.url,
        })
    }
    pub async fn publish_review(
        &self,
        admission: &DeliveryAdmission,
        signed: &Signed<evidence::ReviewAttestation>,
    ) -> Result<PublishedReview> {
        self.publish_review_guarded(admission, signed, None).await
    }
    async fn publish_review_guarded(
        &self,
        admission: &DeliveryAdmission,
        signed: &Signed<evidence::ReviewAttestation>,
        extra: Option<GateCheck>,
    ) -> Result<PublishedReview> {
        if let Some(check) = &extra {
            check(admission.fresh_gate())?;
        }
        self.current(
            admission,
            &[
                DeliveryPhase::ReviewPublication,
                DeliveryPhase::NoChange,
                DeliveryPhase::Release,
            ],
        )?;
        self.key.verify("codex_review", signed)?;
        let a = &signed.payload;
        if a.runtime_id != admission.scope.runtime_id
            || a.owner_generation != admission.permit.lease.owner_generation
            || a.operation_id != admission.operation_id()
            || a.work != admission.scope.work
            || a.revision != admission.scope.revision
        {
            return Err(Error::new(
                "stale_result",
                "signed review does not belong to current admitted phase",
            ));
        }
        if matches!(a.schema_version, 2 | 3) {
            discovered_validation::publication_bounds("codex_review", signed)?;
        }
        // Resolve the original archive before any publication effect. A signed
        // reference alone is not sufficient when local retained history is gone.
        if let Some(reference) = a.review.archive_reference() {
            let archive = crate::validation_archive::read(
                &self.records,
                &self.key,
                &crate::validation_archive::ValidationArchiveNamespace {
                    workspace_id: admission.gate.workspace_id(),
                    runtime_id: admission.scope.runtime_id,
                },
                reference,
            )?;
            archive.evidence().validate(&a.work, &a.revision)?;
            if let evidence::ReviewKind::AgentValidationArchive { assessment, .. } = &a.review {
                archive
                    .evidence()
                    .verify_assessment(assessment, a.verdict == evidence::Verdict::Pass)?;
            }
        }
        let contract = self.contract(admission)?;
        let (publication, status) = if let Some(pr) = a.pull_request {
            let receipt = self
                .effect_guarded(
                    admission,
                    "publish-review",
                    ProviderMutation::GithubComment {
                        repository: contract.provider.clone(),
                        pr,
                        expected_head: a.revision.head.clone(),
                        body: record_body("codex_review", signed)?,
                    },
                    extra.clone(),
                )
                .await?;
            let publication = self
                .github_publication(
                    admission,
                    pr,
                    &remote_id(&receipt, "record_id")?,
                    "codex_review",
                    signed,
                )
                .await?;
            let passed = a.verdict == evidence::Verdict::Pass && a.unresolved_findings.is_empty();
            let status_receipt = self
                .effect_guarded(
                    admission,
                    "publish-review-status",
                    ProviderMutation::GithubStatus {
                        repository: contract.provider.clone(),
                        head: a.revision.head.clone(),
                        state: if passed {
                            crate::providers::StatusState::Success
                        } else {
                            crate::providers::StatusState::Failure
                        },
                        description: if passed
                            && matches!(a.review, evidence::ReviewKind::Derived { .. })
                        {
                            "Carries verified unchanged child reviews"
                        } else if passed {
                            "Authenticated current Codex review passed"
                        } else {
                            "Codex review has unresolved findings"
                        }
                        .into(),
                        target_url: publication.url.clone(),
                    },
                    extra.clone(),
                )
                .await?;
            let statuses = self
                .github()?
                .statuses(&contract.provider, &a.revision.head)
                .await?;
            let id = remote_id(&status_receipt, "record_id")?;
            let raw = statuses
                .iter()
                .find(|s| s.id.to_string() == id)
                .ok_or_else(|| {
                    Error::new("evidence_unpublished", "review status is not yet observed")
                })?;
            if raw.data.get("context").and_then(Value::as_str) != Some(evidence::REVIEW_CONTEXT)
                || raw.data.get("target_url").and_then(Value::as_str)
                    != Some(publication.url.as_str())
                || raw
                    .data
                    .pointer("/creator/id")
                    .and_then(Value::as_u64)
                    .map(|v| v.to_string())
                    .as_deref()
                    != Some(publication.writer_account_id.as_str())
            {
                return Err(Error::new(
                    "review_gate",
                    "review status writer or attestation link differs",
                ));
            }
            let status = evidence::ReviewStatus {
                repository: a.revision.repository.clone(),
                head: a.revision.head.clone(),
                context: evidence::REVIEW_CONTEXT.into(),
                state: raw
                    .data
                    .get("state")
                    .and_then(Value::as_str)
                    .ok_or_else(|| Error::new("provider_response", "review status state missing"))?
                    .into(),
                writer_account_id: publication.writer_account_id.clone(),
                attestation_record_id: publication.record_id.clone(),
            };
            (publication, Some(status))
        } else {
            let issue = a.work.child.ok_or_else(|| {
                Error::new("review_gate", "no-change review requires a real child")
            })?;
            let receipt = self
                .effect_guarded(
                    admission,
                    "publish-no-change-review",
                    ProviderMutation::LinearComment {
                        issue,
                        body: record_body("codex_review", signed)?,
                    },
                    extra.clone(),
                )
                .await?;
            (
                self.linear_publication(
                    admission,
                    issue,
                    &remote_id(&receipt, "comment_id")?,
                    "codex_review",
                    signed,
                )
                .await?,
                None,
            )
        };
        self.current(admission, &[admission.scope.phase])?;
        if let Some(check) = &extra {
            check(admission.fresh_gate())?;
        }
        let verified = self.verify_review_archive(
            signed,
            &publication,
            status.as_ref(),
            admission.gate.workspace_id(),
            evidence::ReviewRequirements {
                work: &admission.scope.work,
                revision: &admission.scope.revision,
                delta_digest: &a.delta_digest,
                writer: &publication.writer_account_id,
            },
        )?;
        Ok(PublishedReview {
            signed: signed.clone(),
            publication,
            status,
            verified,
        })
    }

    pub async fn observe_protection(
        &self,
        admission: &DeliveryAdmission,
        branch: &str,
    ) -> Result<evidence::RepositoryProtection> {
        self.current(admission, &[DeliveryPhase::Staging, DeliveryPhase::Release])?;
        let observed = self
            .current_repository_protection(admission, branch)
            .await?;
        let contract = self.contract(admission)?;
        let suites = if branch == "main" {
            &contract.configuration.production_ci
        } else {
            &contract.configuration.ci
        };
        let required = suites
            .iter()
            .flat_map(|s| s.required_checks.iter().cloned())
            .chain(std::iter::once(evidence::REVIEW_CONTEXT.into()))
            .collect();
        observed.verify(&admission.scope.revision, branch, &required)?;
        Ok(observed)
    }
    async fn current_repository_protection(
        &self,
        admission: &DeliveryObservation,
        branch: &str,
    ) -> Result<evidence::RepositoryProtection> {
        self.current(admission, &[admission.scope.phase])?;
        if !["main", "staging"].contains(&branch) {
            return Err(Error::invalid("delivery branch must be main or staging"));
        }
        let contract = self.contract(admission)?;
        let github = self.github()?;
        let repository = github.repository(&contract.provider).await?;
        let base = github
            .reference(&contract.provider, &format!("heads/{branch}"))
            .await?;
        let protection = github.protection(&contract.provider, branch).await?;
        let rules = github.branch_rules(&contract.provider, branch).await?;
        let suites = if branch == "main" {
            &contract.configuration.production_ci
        } else {
            &contract.configuration.ci
        };
        let mut required = suites
            .iter()
            .flat_map(|s| s.required_checks.iter().cloned())
            .collect::<BTreeSet<_>>();
        required.insert(evidence::REVIEW_CONTEXT.into());
        let observed = validate_repository_prerequisites(
            &admission.scope.repository,
            &repository,
            branch,
            &base,
            &protection,
            &rules,
            &required,
        )?;
        Ok(observed)
    }
    async fn reobserve_review(
        &self,
        admission: &DeliveryObservation,
        reviewed: &PublishedReview,
    ) -> Result<()> {
        let a = &reviewed.signed.payload;
        if a.work != admission.scope.work || a.revision != admission.scope.revision {
            return Err(Error::new(
                "stale_result",
                "review became stale before integration",
            ));
        }
        let publication = if let Some(pr) = a.pull_request {
            self.github_publication(
                admission,
                pr,
                &reviewed.publication.record_id,
                "codex_review",
                &reviewed.signed,
            )
            .await?
        } else {
            self.linear_publication(
                admission,
                a.work
                    .child
                    .ok_or_else(|| Error::new("review_gate", "no-change child absent"))?,
                &reviewed.publication.record_id,
                "codex_review",
                &reviewed.signed,
            )
            .await?
        };
        if let Some(status) = &reviewed.status {
            let raw = self
                .github()?
                .statuses(&self.contract(admission)?.provider, &a.revision.head)
                .await?;
            let mut matching: Vec<_> = raw
                .iter()
                .filter(|r| {
                    r.data.get("context").and_then(Value::as_str) == Some(evidence::REVIEW_CONTEXT)
                })
                .collect();
            matching.sort_by_key(|r| (r.data.get("created_at").and_then(Value::as_str), r.id));
            let latest = matching
                .last()
                .ok_or_else(|| Error::new("review_gate", "current review status missing"))?;
            if latest.data.get("state").and_then(Value::as_str) != Some(status.state.as_str())
                || latest.data.get("target_url").and_then(Value::as_str)
                    != Some(publication.url.as_str())
                || latest
                    .data
                    .pointer("/creator/id")
                    .and_then(Value::as_u64)
                    .map(|v| v.to_string())
                    .as_deref()
                    != Some(status.writer_account_id.as_str())
            {
                return Err(Error::new(
                    "review_gate",
                    "latest review status no longer matches authenticated attestation",
                ));
            }
        }
        self.verify_review_archive(
            &reviewed.signed,
            &publication,
            reviewed.status.as_ref(),
            admission.gate.workspace_id(),
            evidence::ReviewRequirements {
                work: &admission.scope.work,
                revision: &admission.scope.revision,
                delta_digest: &a.delta_digest,
                writer: &publication.writer_account_id,
            },
        )?;
        Ok(())
    }
    async fn reobserve_hosted(
        &self,
        admission: &DeliveryObservation,
        passed: &HostedPassed,
    ) -> Result<()> {
        passed.matches(
            admission,
            if admission.scope.phase == DeliveryPhase::Release {
                CheckKind::ProductionCi
            } else {
                CheckKind::HostedCi
            },
        )?;
        let contract = self.contract(admission)?;
        for expected in &passed.checks {
            let run = self
                .github()?
                .workflow_run(
                    &contract.provider,
                    expected.run_id.ok_or_else(|| {
                        Error::new("validation_gate", "hosted run identity missing")
                    })?,
                    None,
                )
                .await?;
            if run.attempt != u64::from(expected.attempt)
                || run.status != "completed"
                || run.conclusion.as_deref() != Some("success")
            {
                return Err(Error::new(
                    "validation_gate",
                    "hosted suite was rerun or is no longer successful",
                ));
            }
            let proof = passed
                .proofs
                .iter()
                .find(|p| {
                    Some(p.expectation.run_id) == expected.run_id
                        && p.expectation.run_attempt == u64::from(expected.attempt)
                        && p.expectation.tested_source_sha == admission.scope.revision.tested_commit
                        && canonical_digest(1, p).ok().as_ref()
                            == Some(&expected.environment_identity)
                })
                .ok_or_else(|| {
                    Error::new("hosted_proof", "retained current check proof missing")
                })?;
            if run.head_sha != proof.expectation.trigger_sha {
                return Err(Error::new("hosted_proof", "workflow trigger changed"));
            }
            let raw = self
                .github()?
                .checks(&contract.provider, &run.head_sha)
                .await?;
            let mut matching: Vec<_> = raw
                .iter()
                .filter(|r| {
                    r.data.get("name").and_then(Value::as_str) == Some(expected.id.as_str())
                })
                .collect();
            matching.sort_by_key(|r| (r.data.get("started_at").and_then(Value::as_str), r.id));
            let current = if proof.expectation.event_name == "workflow_dispatch" {
                matching
                    .iter()
                    .find(|r| r.id == proof.expectation.check_run_id)
            } else {
                matching.last()
            }
            .ok_or_else(|| {
                Error::new(
                    "validation_gate",
                    "required current hosted check disappeared",
                )
            })?;
            if current.id != proof.expectation.check_run_id
                || current.data.get("status").and_then(Value::as_str) != Some("completed")
                || current.data.get("conclusion").and_then(Value::as_str) != Some("success")
                || timestamp(&current.data, "started_at")?
                    != expected
                        .started_at
                        .ok_or_else(|| Error::new("validation_gate", "check start missing"))?
                || timestamp(&current.data, "completed_at")?
                    != expected
                        .completed_at
                        .ok_or_else(|| Error::new("validation_gate", "check end missing"))?
            {
                return Err(Error::new(
                    "validation_gate",
                    "latest hosted check differs from verified successful attempt",
                ));
            }
        }
        Ok(())
    }
    pub async fn request_staging_merge(
        &self,
        admission: &DeliveryAdmission,
        gates: StagingGates<'_>,
    ) -> Result<PendingStaging> {
        self.current(admission, &[DeliveryPhase::Staging])?;
        gates.pull_request.matches(admission)?;
        gates.local.matches(admission)?;
        gates.local.verify_review_adequacy(gates.review)?;
        gates.hosted.matches(admission, CheckKind::HostedCi)?;
        gates.proposal.matches(admission)?;
        let proposed = gates.proposal.review();
        if gates.review.verified.attestation().delta_digest != gates.proposal.review_digest()?
            || gates.review.verified.attestation().pull_request != Some(gates.pull_request.number())
            || proposed.repository != admission.scope.repository
            || proposed.parent_id != admission.scope.work.parent
            || Some(proposed.child_id) != admission.scope.work.child
            || proposed.base.commit != admission.scope.revision.base
            || proposed.head.commit != admission.scope.revision.head
            || proposed.head.tree != admission.scope.revision.tested_tree
        {
            return Err(Error::new(
                "review_gate",
                "current review is not for the exact proposed child patch",
            ));
        }
        self.reobserve_hosted(admission, gates.hosted).await?;
        self.reobserve_review(admission, gates.review).await?;
        let protection = self.observe_protection(admission, "staging").await?;
        let intent = self.key.sign("staging_intent", &json!({"work":admission.scope.work,"revision":admission.scope.revision,
            "proposal_digest":gates.proposal.review_digest()?,"local_checks":gates.local.checks,
            "hosted_checks":gates.hosted.checks,"review_digest":gates.review.verified.digest(),"protection":protection}))?;
        self.records.write(
            &format!("staging-intent-{}.json", admission.operation_id()),
            &intent,
        )?;
        let receipt = self
            .effect(
                admission,
                "merge-staging",
                ProviderMutation::GithubMerge {
                    repository: self.contract(admission)?.provider.clone(),
                    pr: gates.pull_request.number(),
                    expected_head: admission.scope.revision.head.clone(),
                    expected_base: admission.scope.revision.base.clone(),
                },
            )
            .await?;
        Ok(PendingStaging {
            receipt,
            work: admission.scope.work.clone(),
            revision: admission.scope.revision.clone(),
            protection,
        })
    }
    /// A merge acknowledgement is only a receipt. Completion requires actual
    /// provider ancestry/tree and retained Git object/patch equality.
    pub async fn observe_staging_merge(
        &self,
        admission: &DeliveryAdmission,
        source: &Path,
        proposal: &ScopedChange,
        pr: &ChildPullRequest,
        pending: &PendingStaging,
    ) -> Result<StagedDelta> {
        self.current(
            admission,
            &[DeliveryPhase::Staging, DeliveryPhase::Recovery],
        )?;
        pr.matches(admission)?;
        proposal.matches(admission)?;
        if pending.work != admission.scope.work || pending.revision != admission.scope.revision {
            return Err(Error::new(
                "stale_result",
                "staging completion belongs to another admitted work generation",
            ));
        }
        let receipt = &pending.receipt;
        let expected_receipt = self.host.operation(receipt.request_id)?;
        if expected_receipt.payload_digest != receipt.payload_digest
            || receipt.kind != "github_merge"
            || receipt.runtime_id != admission.scope.runtime_id
        {
            return Err(Error::new(
                "reconciliation_required",
                "merge receipt is not the durable owned operation",
            ));
        }
        let contract = self.contract(admission)?;
        let current = self
            .github()?
            .pull_request(&contract.provider, pr.number())
            .await?;
        let commit = current
            .merge_commit_sha
            .as_deref()
            .filter(|_| current.merged == Some(true))
            .ok_or_else(|| {
                Error::new(
                    "operation_pending",
                    "staging merge has not actually completed",
                )
            })?;
        if current.head_sha != admission.scope.revision.head || current.base_ref != "staging" {
            return Err(Error::new(
                "provenance_mismatch",
                "merged PR identity changed",
            ));
        }
        let object = self.github()?.commit(&contract.provider, commit).await?;
        let parents = object
            .get("parents")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new("provider_response", "observed merge parents missing"))?;
        if parents.len() != 1
            || parents[0].get("sha").and_then(Value::as_str)
                != Some(admission.scope.revision.base.as_str())
            || object.pointer("/tree/sha").and_then(Value::as_str)
                != Some(admission.scope.revision.tested_tree.as_str())
        {
            return Err(Error::new(
                "provenance_mismatch",
                "observed staging squash has unexpected base/tree; preserve produced effect",
            ));
        }
        let p = proposal.review();
        let delta = self.git.capture_delta(
            source,
            crate::git::CaptureDelta {
                repository: admission.scope.repository.clone(),
                parent_id: admission.scope.work.parent,
                child_id: admission
                    .scope
                    .work
                    .child
                    .ok_or_else(|| Error::new("scope_violation", "parent cannot stage code"))?,
                work_generation: &admission.scope.work.generation.definition,
                correction_generation: &admission.scope.work.generation.correction,
                scope: p.scope.clone(),
                base: &admission.scope.revision.base,
                head: &admission.scope.revision.head,
                staging_parent: &admission.scope.revision.base,
                staging_commit: commit,
            },
        )?;
        if delta.manifest.review_digest()? != proposal.review_digest()?
            || delta.patch != proposal.patch()
        {
            return Err(Error::new(
                "provenance_mismatch",
                "staging result differs from premerge reviewed artifact",
            ));
        }
        let digest = delta.manifest.digest()?;
        let chunks = delta
            .patch
            .chunks(4 * 1024 * 1024)
            .enumerate()
            .map(|(index, bytes)| {
                let leaf = format!("delta-{}-{index}.bin", &digest[7..]);
                self.records.write_bytes(&leaf, bytes)?;
                Ok(leaf)
            })
            .collect::<Result<Vec<_>>>()?;
        let retained = self.key.sign("staged_delta", &json!({"manifest":delta.manifest,"patch_chunks":chunks,"merge_request":receipt.request_id}))?;
        self.records
            .write(&format!("delta-{}.json", &digest[7..]), &retained)?;
        self.current(admission, &[admission.scope.phase])?;
        Ok(StagedDelta {
            delta,
            pull_request: pr.number(),
            merge_receipt: receipt.request_id,
            protection: pending.protection.clone(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildResultRecord {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub publication_operation: Uuid,
    pub contract_digest: String,
    pub workflow_digest: String,
    pub evidence: evidence::ChildEvidence,
    pub review: Signed<evidence::ReviewAttestation>,
    pub review_publication: Publication,
    pub review_status: Option<evidence::ReviewStatus>,
    pub hosted_proofs: Vec<crate::providers::VerifiedHostedJob>,
    pub protection: Option<evidence::RepositoryProtection>,
}
#[derive(Clone, Debug)]
pub struct PublishedChild {
    verified: evidence::VerifiedChild,
    signed: Signed<ChildResultRecord>,
    publication: Publication,
}
impl PublishedChild {
    pub fn verified(&self) -> &evidence::VerifiedChild {
        &self.verified
    }
    pub fn publication(&self) -> &Publication {
        &self.publication
    }
}
pub struct ChangedChildCompletion<'a> {
    pub workflow: &'a Workflow,
    pub criteria: &'a BTreeSet<String>,
    pub local: &'a LocalPassed,
    pub hosted: &'a HostedPassed,
    pub review: &'a PublishedReview,
    pub staged: &'a StagedDelta,
}
impl DeliveryCoordinator {
    pub async fn publish_changed_child(
        &self,
        admission: &DeliveryAdmission,
        completion: ChangedChildCompletion<'_>,
    ) -> Result<PublishedChild> {
        self.current(admission, &[DeliveryPhase::Staging])?;
        completion.local.matches(admission)?;
        completion.local.verify_review_adequacy(completion.review)?;
        completion.hosted.matches(admission, CheckKind::HostedCi)?;
        if completion.workflow.digest != admission.gate.workflow_digest()
            || completion.staged.delta.manifest.review_digest()?
                != completion.review.verified.attestation().delta_digest
            || completion.review.verified.attestation().work != admission.scope.work
            || completion.review.verified.attestation().revision != admission.scope.revision
        {
            return Err(Error::new(
                "stale_result",
                "child result gates differ from admitted work",
            ));
        }
        self.reobserve_review(admission, completion.review).await?;
        let checks: Vec<_> = completion
            .local
            .checks
            .iter()
            .chain(&completion.hosted.checks)
            .cloned()
            .collect();
        let mut results: Vec<_> = checks
            .iter()
            .flat_map(|c| c.artifacts.iter().cloned())
            .collect();
        results.push(ArtifactReference {
            reference: format!(
                "retained-delta:{}",
                completion.staged.delta.manifest.digest()?
            ),
            digest: completion.staged.delta.manifest.digest()?,
        });
        let child = evidence::ChildEvidence {
            schema_version: completion.local.child_schema(),
            validation_selected: completion
                .local
                .validation_archive
                .as_ref()
                .map(|archive| archive.evidence().selected_digests())
                .transpose()?,
            work: admission.scope.work.clone(),
            revision: admission.scope.revision.clone(),
            fulfilled_criteria: completion.criteria.clone(),
            outcome: evidence::ChildOutcome::Changed {
                pull_request: completion.staged.pull_request,
                delta: Box::new(completion.staged.delta.manifest.clone()),
            },
            checks: CheckSnapshot {
                pagination_complete: true,
                observed_at: self.operation_time(admission, "child-result")?,
                runs: if completion.local.validation_archive.is_some() {
                    checks
                        .into_iter()
                        .filter(|r| r.kind != CheckKind::Local)
                        .collect()
                } else {
                    checks
                },
            },
            review_digest: completion.review.verified.digest().into(),
            results_for_dependents: results,
        };
        let signed = self.key.sign(
            "child_result",
            &ChildResultRecord {
                schema_version: completion.local.child_schema(),
                runtime_id: admission.scope.runtime_id,
                publication_operation: admission.operation_id(),
                contract_digest: admission.scope.contract_digest.clone(),
                workflow_digest: completion.workflow.digest.clone(),
                evidence: child,
                review: completion.review.signed.clone(),
                review_publication: completion.review.publication.clone(),
                review_status: completion.review.status.clone(),
                hosted_proofs: completion.hosted.proofs.clone(),
                protection: Some(completion.staged.protection.clone()),
            },
        )?;
        self.publish_child_record(admission, completion.workflow, completion.criteria, signed)
            .await
    }
    async fn publish_child_record(
        &self,
        admission: &DeliveryAdmission,
        workflow: &Workflow,
        criteria: &BTreeSet<String>,
        signed: Signed<ChildResultRecord>,
    ) -> Result<PublishedChild> {
        let child = admission.scope.work.child.ok_or_else(|| {
            Error::new("scope_violation", "only a child may publish a child result")
        })?;
        self.validate_child_draft(admission, workflow, criteria, &signed)?;
        if matches!(signed.payload.schema_version, 2 | 3) {
            discovered_validation::publication_bounds("child_result", &signed)?;
        }
        let receipt = self
            .effect(
                admission,
                "publish-child-result",
                ProviderMutation::LinearComment {
                    issue: child,
                    body: record_body("child_result", &signed)?,
                },
            )
            .await?;
        let publication = self
            .linear_publication(
                admission,
                child,
                &remote_id(&receipt, "comment")?,
                "child_result",
                &signed,
            )
            .await?;
        self.current(admission, &[admission.scope.phase])?;
        let verified =
            self.validate_child_record(admission, workflow, criteria, &signed, &publication)?;
        self.records.write(
            &format!("child-result-{}.json", verified.digest().replace(':', "-")),
            &signed,
        )?;
        Ok(PublishedChild {
            verified,
            signed,
            publication,
        })
    }
    fn validate_child_draft(
        &self,
        admission: &DeliveryObservation,
        workflow: &Workflow,
        criteria: &BTreeSet<String>,
        signed: &Signed<ChildResultRecord>,
    ) -> Result<(
        evidence::VerifiedReview,
        Vec<CheckRequirement>,
        BTreeSet<String>,
    )> {
        self.key.verify("child_result", signed)?;
        let r = &signed.payload;
        let expected_schema = r.review.payload.review.schema();
        if r.schema_version != expected_schema
            || r.evidence.schema_version != expected_schema
            || r.runtime_id != admission.scope.runtime_id
            || r.publication_operation.is_nil()
            || r.contract_digest != admission.scope.contract_digest
            || r.workflow_digest != workflow.digest
            || workflow.digest != admission.gate.workflow_digest()
        {
            return Err(Error::new(
                "stale_result",
                "child publication has stale configuration or runtime identity",
            ));
        }
        let review = self.verify_review_archive(
            &r.review,
            &r.review_publication,
            r.review_status.as_ref(),
            admission.gate.workspace_id(),
            evidence::ReviewRequirements {
                work: &admission.scope.work,
                revision: &admission.scope.revision,
                delta_digest: &r.review.payload.delta_digest,
                writer: &r.review_publication.writer_account_id,
            },
        )?;
        let contract = self.contract(admission)?;
        for check in r
            .evidence
            .checks
            .runs
            .iter()
            .filter(|c| c.kind == CheckKind::HostedCi)
        {
            let proof = r
                .hosted_proofs
                .iter()
                .find(|p| {
                    Some(p.expectation.run_id) == check.run_id
                        && p.expectation.run_attempt == u64::from(check.attempt)
                        && p.expectation.repository_id == contract.provider.id
                        && p.expectation.tested_source_sha == admission.scope.revision.tested_commit
                        && canonical_digest(1, p).ok().as_ref() == Some(&check.environment_identity)
                })
                .ok_or_else(|| {
                    Error::new(
                        "hosted_proof",
                        "signed child lacks exact authenticated hosted proof inventory",
                    )
                })?;
            if proof.schema_version != 1
                || proof.expectation.runtime_id != admission.scope.runtime_id
                || proof.expectation.operation_id != check.operation_id
            {
                return Err(Error::new(
                    "hosted_proof",
                    "child hosted proof operation differs",
                ));
            }
        }
        let mut required = if let Some(validation) = review.validation() {
            if validation.current()?.workflow_digest != workflow.digest {
                return Err(Error::new(
                    "stale_result",
                    "discovered validation workflow changed",
                ));
            }
            validation.verify_configured(&contract.configuration.local_checks, workflow)?;
            validation.requirements()?
        } else {
            local_requirements(contract, workflow)?
        };
        if matches!(r.evidence.outcome, evidence::ChildOutcome::NoChange { .. }) {
            required.extend(exact_requirements(contract)?);
        } else {
            required.extend(hosted_requirements(
                &contract.configuration.ci,
                CheckKind::HostedCi,
            )?);
        }
        let contexts = contract
            .configuration
            .ci
            .iter()
            .flat_map(|s| s.required_checks.iter().cloned())
            .chain(std::iter::once(evidence::REVIEW_CONTEXT.into()))
            .collect();
        evidence::validate_child_candidate(
            &r.evidence,
            evidence::ChildRequirements {
                work: &admission.scope.work,
                criteria,
                revision: &admission.scope.revision,
                checks: &required,
                required_contexts: &contexts,
            },
            &review,
            r.protection.as_ref(),
        )?;
        Ok((review, required, contexts))
    }
    fn validate_child_record(
        &self,
        admission: &DeliveryObservation,
        workflow: &Workflow,
        criteria: &BTreeSet<String>,
        signed: &Signed<ChildResultRecord>,
        publication: &Publication,
    ) -> Result<evidence::VerifiedChild> {
        let (review, required, contexts) =
            self.validate_child_draft(admission, workflow, criteria, signed)?;
        publication.verify(signed, "linear", &publication.writer_account_id)?;
        let r = &signed.payload;
        let mut inner = publication.clone();
        inner.payload_digest = canonical_digest(1, &r.evidence)?;
        evidence::verify_child(
            &r.evidence,
            evidence::ChildRequirements {
                work: &admission.scope.work,
                criteria,
                revision: &admission.scope.revision,
                checks: &required,
                required_contexts: &contexts,
            },
            &review,
            r.protection.as_ref(),
            &inner,
            &publication.writer_account_id,
        )
    }
    /// Closing the tracker is separately acknowledged and observed. A published
    /// result remains durable even when closure is rejected or its response lost.
    pub async fn close_child(
        &self,
        admission: &DeliveryAdmission,
        result: &PublishedChild,
        config: &crate::config::Config,
    ) -> Result<Receipt> {
        self.current(
            admission,
            &[
                DeliveryPhase::Staging,
                DeliveryPhase::NoChange,
                DeliveryPhase::Recovery,
            ],
        )?;
        if config.revision()? != admission.gate.config_revision()
            || result.verified.evidence().work != admission.scope.work
            || result.verified.evidence().revision != admission.scope.revision
        {
            return Err(Error::new(
                "stale_result",
                "child closure configuration/result changed",
            ));
        }
        let child = admission
            .scope
            .work
            .child
            .ok_or_else(|| Error::new("scope_violation", "parent is not a child"))?;
        self.linear_publication(
            admission,
            child,
            &result.publication.record_id,
            "child_result",
            &result.signed,
        )
        .await?;
        let linear = self
            .broker
            .linear
            .as_ref()
            .ok_or_else(|| Error::new("prerequisite", "Linear connection unavailable"))?;
        let current = linear.issue(child).await?;
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == admission.gate.workspace_id())
            .ok_or_else(|| Error::new("scope_violation", "workspace no longer selected"))?;
        let state = workspace
            .linear
            .states
            .get(&current.team_id.to_string())
            .ok_or_else(|| Error::new("incompletely_prepared", "child team state mapping absent"))?
            .child_done;
        if current.parent_id != Some(admission.scope.work.parent) {
            return Err(Error::new(
                "stale_result",
                "child parent changed before closure",
            ));
        }
        let intent_leaf = format!("child-closure-intent-{}.json", admission.operation_id());
        let intent: Signed<ProviderMutation> = {
            let _lock = self.records.section_lock("delivery-effects.lock")?;
            match self.records.read(&intent_leaf) {
                Ok(existing) => existing,
                Err(e) if e.code == "record_missing" => {
                    let created = self.key.sign(
                        "child_closure",
                        &ProviderMutation::LinearState {
                            issue: child,
                            expected_state: current.state_id,
                            state,
                        },
                    )?;
                    self.records.write(&intent_leaf, &created)?;
                    created
                }
                Err(e) => return Err(e),
            }
        };
        self.key.verify("child_closure", &intent)?;
        if !matches!(&intent.payload,ProviderMutation::LinearState{issue,state:target,..} if *issue==child && *target==state)
        {
            return Err(Error::new(
                "operation_conflict",
                "retained child closure intent differs",
            ));
        }
        let receipt = self
            .effect(admission, "close-child", intent.payload)
            .await?;
        let observed = linear.issue(child).await?;
        if observed.state_id != state || observed.parent_id != Some(admission.scope.work.parent) {
            return Err(Error::new(
                "operation_pending",
                "child closure has not been observed",
            ));
        }
        self.current(admission, &[admission.scope.phase])?;
        Ok(receipt)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedChallenge {
    pub schema_version: u32,
    pub admission_digest: String,
    pub runtime_id: Uuid,
    pub operation_id: Uuid,
    pub source_sha: String,
    pub challenge: Uuid,
}
#[derive(Clone, Debug)]
pub struct PendingExactValidation {
    work: WorkIdentity,
    revision: RevisionBinding,
    receipt: Receipt,
    challenge: HostedChallenge,
}
impl PendingExactValidation {
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }
    pub fn challenge(&self) -> &HostedChallenge {
        &self.challenge
    }
}
#[derive(Clone, Debug)]
pub struct ExactValidationPassed {
    hosted: HostedPassed,
    dispatch: Uuid,
}
impl DeliveryCoordinator {
    /// Expose only the source-bound nonce contract to a prepared hosted route.
    /// The nonce is host-generated once and authenticated in private storage.
    pub fn prepare_hosted_challenge(
        &self,
        admission: &DeliveryAdmission,
    ) -> Result<HostedChallenge> {
        self.current(
            admission,
            &[
                DeliveryPhase::HostedValidation,
                DeliveryPhase::NoChange,
                DeliveryPhase::Release,
            ],
        )?;
        let leaf = format!("hosted-challenge-{}.json", admission.operation_id());
        let _lock = self.records.section_lock("delivery-effects.lock")?;
        let signed: Signed<HostedChallenge> = match self.records.read(&leaf) {
            Ok(s) => s,
            Err(e) if e.code == "record_missing" => {
                let c = HostedChallenge {
                    schema_version: 1,
                    admission_digest: admission.scope.digest()?,
                    runtime_id: admission.scope.runtime_id,
                    operation_id: admission.operation_id(),
                    source_sha: admission.scope.revision.tested_commit.clone(),
                    challenge: Uuid::new_v4(),
                };
                let signed = self.key.sign("hosted_challenge", &c)?;
                self.records.write(&leaf, &signed)?;
                signed
            }
            Err(e) => return Err(e),
        };
        self.validate_hosted_challenge(admission, signed)
    }
    fn validate_hosted_challenge(
        &self,
        admission: &DeliveryAdmission,
        signed: Signed<HostedChallenge>,
    ) -> Result<HostedChallenge> {
        self.key.verify("hosted_challenge", &signed)?;
        let c = signed.payload;
        if c.schema_version != 1
            || c.admission_digest != admission.scope.digest()?
            || c.runtime_id != admission.scope.runtime_id
            || c.operation_id != admission.operation_id()
            || c.source_sha != admission.scope.revision.tested_commit
            || c.challenge.is_nil()
        {
            return Err(Error::new(
                "hosted_proof",
                "hosted challenge differs from immutable admission",
            ));
        }
        Ok(c)
    }
    fn hosted_challenge(&self, admission: &DeliveryAdmission) -> Result<HostedChallenge> {
        self.validate_hosted_challenge(
            admission,
            self.records.read(&format!(
                "hosted-challenge-{}.json",
                admission.operation_id()
            ))?,
        )
    }
    pub async fn dispatch_exact_validation(
        &self,
        admission: &DeliveryAdmission,
    ) -> Result<PendingExactValidation> {
        self.current(admission, &[DeliveryPhase::NoChange])?;
        let contract = self.contract(admission)?;
        let route = &contract.configuration.no_change_validation;
        let challenge = self.prepare_hosted_challenge(admission)?;
        let (locator, reference) = route
            .proof
            .workflow_ref
            .rsplit_once('@')
            .ok_or_else(|| Error::invalid("workflow ref absent"))?;
        let workflow_path = locator
            .splitn(3, '/')
            .nth(2)
            .ok_or_else(|| Error::invalid("workflow path absent"))?;
        let reference = reference
            .strip_prefix("refs/")
            .filter(|r| r.starts_with("heads/") || r.starts_with("tags/"))
            .ok_or_else(|| {
                Error::new(
                    "incompletely_prepared",
                    "dispatch proof route requires exact branch/tag reference",
                )
            })?;
        let receipt = self
            .effect(
                admission,
                "dispatch-exact-validation",
                ProviderMutation::GithubDispatch {
                    expected_production: None,
                    repository: contract.provider.clone(),
                    workflow_id: route.workflow_id,
                    workflow_path: workflow_path.into(),
                    reference: reference.into(),
                    workflow_sha: route.workflow_revision.clone(),
                    source_sha: admission.scope.revision.tested_commit.clone(),
                    source_input: route.source_input.clone(),
                    operation_input: route.operation_input.clone(),
                    inputs: BTreeMap::from([
                        ("kegari_runtime_id".into(), challenge.runtime_id.to_string()),
                        (
                            "kegari_phase_operation".into(),
                            challenge.operation_id.to_string(),
                        ),
                        ("kegari_challenge".into(), challenge.challenge.to_string()),
                    ]),
                },
            )
            .await?;
        Ok(PendingExactValidation {
            work: admission.scope.work.clone(),
            revision: admission.scope.revision.clone(),
            receipt,
            challenge,
        })
    }
    async fn observe_exact_validation(
        &self,
        admission: &DeliveryAdmission,
        pending: &PendingExactValidation,
        tokens: &[HostedToken],
        verifier: &HostedProofVerifier,
    ) -> Result<ExactValidationPassed> {
        self.current(admission, &[DeliveryPhase::NoChange])?;
        if pending.work != admission.scope.work
            || pending.revision != admission.scope.revision
            || pending.challenge.admission_digest != admission.scope.digest()?
        {
            return Err(Error::new(
                "stale_result",
                "exact validation belongs to another work generation",
            ));
        }
        let run = remote_id(&pending.receipt, "workflow_run_id")?
            .parse()
            .map_err(|_| Error::new("provider_response", "dispatch run identity invalid"))?;
        let contract = self.contract(admission)?;
        let route = &contract.configuration.no_change_validation;
        let names: Vec<_> = contract
            .configuration
            .ci
            .iter()
            .flat_map(|c| c.required_checks.iter().cloned())
            .collect();
        if names.is_empty() || names.iter().collect::<BTreeSet<_>>().len() != names.len() {
            return Err(Error::new(
                "incompletely_prepared",
                "exact validation requires an unambiguous complete configured check inventory",
            ));
        }
        let suites = [CiSuite {
            workflow_id: route.workflow_id,
            required_checks: names,
            proof: route.proof.clone(),
            inapplicable_when: None,
        }];
        let hosted = self
            .observe_hosted_suite(
                admission,
                HostedObservation {
                    kind: CheckKind::HostedCi,
                    suites: &suites,
                    tokens,
                    verifier,
                    owned_run: Some(run),
                },
            )
            .await?;
        Ok(ExactValidationPassed {
            hosted,
            dispatch: pending.receipt.request_id,
        })
    }
}

fn exact_requirements(contract: &RepositoryContract) -> Result<Vec<CheckRequirement>> {
    let mut result = Vec::new();
    let mut names = BTreeSet::new();
    for suite in &contract.configuration.ci {
        for name in &suite.required_checks {
            if !names.insert(name.clone()) {
                return Err(Error::new(
                    "incompletely_prepared",
                    "exact validation check names must be unambiguous",
                ));
            }
            result.push(CheckRequirement {
                id: name.clone(),
                kind: CheckKind::HostedCi,
                workflow_id: Some(contract.configuration.no_change_validation.workflow_id),
                command: vec![],
            });
        }
    }
    if result.is_empty() {
        return Err(Error::new(
            "incompletely_prepared",
            "exact validation suite is empty",
        ));
    }
    Ok(result)
}
pub struct NoChangeCompletion<'a> {
    pub workflow: &'a Workflow,
    pub criteria: &'a BTreeSet<String>,
    pub reason: &'a str,
    pub local: &'a LocalPassed,
    pub exact: &'a ExactValidationPassed,
    pub review: &'a PublishedReview,
}
impl DeliveryCoordinator {
    fn operation_time(
        &self,
        admission: &DeliveryAdmission,
        purpose: &str,
    ) -> Result<DateTime<Utc>> {
        let leaf = format!(
            "delivery-time-{}.json",
            &bytes_digest(format!("{}:{purpose}", admission.operation_id()).as_bytes())[7..]
        );
        let _lock = self.records.section_lock("delivery-effects.lock")?;
        let record: Signed<(String, DateTime<Utc>)> = match self.records.read(&leaf) {
            Ok(r) => r,
            Err(e) if e.code == "record_missing" => {
                let r = self
                    .key
                    .sign("delivery_time", &(admission.scope.digest()?, Utc::now()))?;
                self.records.write(&leaf, &r)?;
                r
            }
            Err(e) => return Err(e),
        };
        self.key.verify("delivery_time", &record)?;
        if record.payload.0 != admission.scope.digest()? {
            return Err(Error::new(
                "operation_conflict",
                "operation creation time belongs to another admission",
            ));
        }
        Ok(record.payload.1)
    }
    pub async fn publish_no_change_child(
        &self,
        admission: &DeliveryAdmission,
        completion: NoChangeCompletion<'_>,
    ) -> Result<PublishedChild> {
        self.current(admission, &[DeliveryPhase::NoChange])?;
        completion.local.matches(admission)?;
        completion.local.verify_review_adequacy(completion.review)?;
        completion
            .exact
            .hosted
            .matches(admission, CheckKind::HostedCi)?;
        if completion.workflow.digest != admission.gate.workflow_digest()
            || completion.exact.dispatch.is_nil()
            || completion.review.verified.attestation().delta_digest
                != evidence::no_change_digest(
                    &admission.scope.work,
                    &admission.scope.revision,
                    completion.criteria,
                    completion.reason,
                )?
        {
            return Err(Error::new(
                "stale_result",
                "no-change criteria/review differ from current exact revision",
            ));
        }
        self.reobserve_hosted(admission, &completion.exact.hosted)
            .await?;
        self.reobserve_review(admission, completion.review).await?;
        let checks: Vec<_> = completion
            .local
            .checks
            .iter()
            .chain(&completion.exact.hosted.checks)
            .cloned()
            .collect();
        let results = checks
            .iter()
            .flat_map(|c| c.artifacts.iter().cloned())
            .collect();
        let child = evidence::ChildEvidence {
            schema_version: completion.local.child_schema(),
            validation_selected: completion
                .local
                .validation_archive
                .as_ref()
                .map(|archive| archive.evidence().selected_digests())
                .transpose()?,
            work: admission.scope.work.clone(),
            revision: admission.scope.revision.clone(),
            fulfilled_criteria: completion.criteria.clone(),
            outcome: evidence::ChildOutcome::NoChange {
                reason: completion.reason.into(),
                change_gates_inapplicable: BTreeSet::from([
                    "branch".into(),
                    "pull_request".into(),
                    "staging_merge".into(),
                ]),
                exact_sha_validation: format!("owned-dispatch:{}", completion.exact.dispatch),
            },
            checks: CheckSnapshot {
                pagination_complete: true,
                observed_at: self.operation_time(admission, "child-result")?,
                runs: if completion.local.validation_archive.is_some() {
                    checks
                        .into_iter()
                        .filter(|r| r.kind != CheckKind::Local)
                        .collect()
                } else {
                    checks
                },
            },
            review_digest: completion.review.verified.digest().into(),
            results_for_dependents: results,
        };
        let signed = self.key.sign(
            "child_result",
            &ChildResultRecord {
                schema_version: completion.local.child_schema(),
                runtime_id: admission.scope.runtime_id,
                publication_operation: admission.operation_id(),
                contract_digest: admission.scope.contract_digest.clone(),
                workflow_digest: completion.workflow.digest.clone(),
                evidence: child,
                review: completion.review.signed.clone(),
                review_publication: completion.review.publication.clone(),
                review_status: completion.review.status.clone(),
                hosted_proofs: completion.exact.hosted.proofs.clone(),
                protection: None,
            },
        )?;
        self.publish_child_record(admission, completion.workflow, completion.criteria, signed)
            .await
    }
    fn retained_delta(
        &self,
        manifest: &crate::git::DeltaManifest,
    ) -> Result<crate::git::DeltaArtifact> {
        let digest = manifest.digest()?;
        let retained: Signed<Value> = self.records.read(&format!("delta-{}.json", &digest[7..]))?;
        self.key.verify("staged_delta", &retained)?;
        if retained.payload.get("manifest")
            != Some(&serde_json::to_value(manifest).map_err(|_| Error::invalid("delta encoding"))?)
        {
            return Err(Error::new(
                "provenance_mismatch",
                "retained signed delta manifest differs",
            ));
        }
        let chunks = retained
            .payload
            .get("patch_chunks")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new("record_corrupt", "retained patch chunks absent"))?;
        if chunks.len() > 64 {
            return Err(Error::new(
                "bounded_input",
                "retained patch exceeds 256 MiB",
            ));
        }
        let mut patch = Vec::new();
        let mut seen = BTreeSet::new();
        for chunk in chunks {
            let leaf = chunk
                .as_str()
                .filter(|s| seen.insert(*s))
                .ok_or_else(|| Error::new("record_corrupt", "duplicate/invalid patch chunk"))?;
            patch.extend(self.records.read_bytes(leaf, 4 * 1024 * 1024)?);
        }
        if bytes_digest(&patch) != manifest.patch_digest {
            return Err(Error::new(
                "provenance_mismatch",
                "retained patch bytes changed",
            ));
        }
        Ok(crate::git::DeltaArtifact {
            manifest: manifest.clone(),
            patch,
        })
    }
    /// Read actual provider record and authenticate all retained gate provenance.
    /// Tracker state is intentionally irrelevant to this reconstruction.
    pub async fn recover_child_result(
        &self,
        admission: &DeliveryAdmission,
        workflow: &Workflow,
        criteria: &BTreeSet<String>,
        record_id: Uuid,
    ) -> Result<PublishedChild> {
        self.hydrate_child_result(admission, workflow, criteria, record_id)
            .await
    }
    /// Revalidate retained child completion from current provider observations
    /// without acquiring a worker permit or performing a remote mutation.
    pub async fn hydrate_child_result(
        &self,
        admission: &DeliveryObservation,
        workflow: &Workflow,
        criteria: &BTreeSet<String>,
        record_id: Uuid,
    ) -> Result<PublishedChild> {
        self.current(
            admission,
            &[
                DeliveryPhase::Recovery,
                DeliveryPhase::NoChange,
                DeliveryPhase::Staging,
            ],
        )?;
        let child = admission
            .scope
            .work
            .child
            .ok_or_else(|| Error::new("scope_violation", "child identity missing"))?;
        let linear = self
            .broker
            .linear
            .as_ref()
            .ok_or_else(|| Error::new("prerequisite", "Linear connection unavailable"))?;
        let comment = linear
            .comments(child)
            .await?
            .into_iter()
            .find(|c| c.id == record_id)
            .ok_or_else(|| Error::new("evidence_unpublished", "retained child record is absent"))?;
        let signed: Signed<ChildResultRecord> =
            evidence::decode_signed_publication(&comment.body, "child_result", &self.key)?;
        let publication = self
            .linear_publication(
                admission,
                child,
                &record_id.to_string(),
                "child_result",
                &signed,
            )
            .await?;
        let verified =
            self.validate_child_record(admission, workflow, criteria, &signed, &publication)?;
        let (review, _, _) = self.validate_child_draft(admission, workflow, criteria, &signed)?;
        self.reobserve_review(
            admission,
            &PublishedReview {
                signed: signed.payload.review.clone(),
                publication: signed.payload.review_publication.clone(),
                status: signed.payload.review_status.clone(),
                verified: review,
            },
        )
        .await?;
        self.reobserve_hosted(
            admission,
            &HostedPassed {
                work: admission.scope.work.clone(),
                revision: admission.scope.revision.clone(),
                kind: CheckKind::HostedCi,
                checks: signed
                    .payload
                    .evidence
                    .checks
                    .runs
                    .iter()
                    .filter(|c| c.kind == CheckKind::HostedCi)
                    .cloned()
                    .collect(),
                proofs: signed.payload.hosted_proofs.clone(),
            },
        )
        .await?;
        if let evidence::ChildOutcome::Changed {
            pull_request,
            delta,
        } = &signed.payload.evidence.outcome
        {
            // Staging may have advanced since this delta; reread its current
            // required protection without falsely requiring the old base tip.
            self.current_repository_protection(admission, "staging")
                .await?;
            let contract = self.contract(admission)?;
            let current = self
                .github()?
                .pull_request(&contract.provider, *pull_request)
                .await?;
            if current.merged != Some(true)
                || current.merge_commit_sha.as_deref() != Some(delta.staging_commit.commit.as_str())
                || current.head_sha != delta.validated_head.commit
                || current.base_ref != "staging"
            {
                return Err(Error::new(
                    "provenance_mismatch",
                    "staged PR no longer matches retained child delta",
                ));
            }
            let artifact = self.retained_delta(delta)?;
            let retained_source = self.retained_delta_object_source(delta)?;
            self.git.verify_delta(
                retained_source
                    .as_ref()
                    .map(|source| source.root())
                    .unwrap_or_else(|| {
                        self.retained_git_sources
                            .get(&contract.repository)
                            .map(PathBuf::as_path)
                            .unwrap_or_else(|| Path::new(&contract.configuration.source))
                    }),
                &artifact,
            )?;
        }
        self.current(admission, &[admission.scope.phase])?;
        Ok(PublishedChild {
            verified,
            signed,
            publication,
        })
    }
}

/// Untrusted artifact content. Parsing never establishes a gate; tokens are kept
/// out of Debug/Serialize and never written to public receipts or comments.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedSourceArtifact {
    schema_version: u32,
    runtime_id: Uuid,
    phase_operation: Uuid,
    dispatch_request: Uuid,
    challenge: Uuid,
    repository_id: u64,
    run_id: u64,
    run_attempt: u64,
    tested_tree: String,
    outputs: BTreeMap<String, String>,
    proofs: Vec<HostedArtifactJob>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostedArtifactJob {
    check_run_id: u64,
    token: String,
}
impl std::fmt::Debug for HostedSourceArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostedSourceArtifact([UNVERIFIED REDACTED])")
    }
}
/// Bounded in-memory ZIP reader; no archive member is ever extracted to disk.
pub fn read_hosted_source_archive(bytes: &[u8]) -> Result<HostedSourceArtifact> {
    use std::io::Read;
    const LIMIT: u64 = 1_048_576;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(Error::new("bounded_input", "hosted archive exceeds 4 MiB"));
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| Error::new("hosted_artifact", "invalid ZIP archive"))?;
    if archive.is_empty() || archive.len() > 16 || archive.metadata().has_duplicate_names() {
        return Err(Error::new(
            "hosted_artifact",
            "archive count/duplicate names violate contract",
        ));
    }
    let mut total = 0u64;
    let mut result = None;
    let mut names = BTreeSet::new();
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|_| Error::new("hosted_artifact", "unreadable or encrypted archive member"))?;
        let name = file
            .name()
            .map_err(|_| Error::new("hosted_artifact", "invalid archive filename"))?
            .into_owned();
        if file.encrypted()
            || file.is_symlink()
            || file.is_dir()
            || file.enclosed_name().is_none()
            || name.contains('\\')
            || name
                .split('/')
                .any(|c| c.is_empty() || c == ".." || c == ".")
            || !names.insert(name.clone())
            || !matches!(
                file.compression(),
                zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
            )
            || file
                .unix_mode()
                .is_some_and(|m| m & 0o170000 != 0 && m & 0o170000 != 0o100000)
        {
            return Err(Error::new(
                "hosted_artifact",
                "unsafe member type/path/compression",
            ));
        }
        total = total
            .checked_add(file.size())
            .ok_or_else(|| Error::new("bounded_input", "archive total overflow"))?;
        if file.size() > LIMIT || total > 4 * LIMIT {
            return Err(Error::new(
                "bounded_input",
                "hosted uncompressed data exceeds bound",
            ));
        }
        let expected = file.size();
        let mut data = Vec::new();
        // Read to EOF through the ZIP CRC verifier; a truncated/lying size fails.
        file.by_ref()
            .take(LIMIT + 1)
            .read_to_end(&mut data)
            .map_err(|_| {
                Error::new(
                    "hosted_artifact",
                    "archive content or CRC verification failed",
                )
            })?;
        if data.len() as u64 != expected || data.len() as u64 > LIMIT {
            return Err(Error::new(
                "hosted_artifact",
                "archive size differs from verified content",
            ));
        }
        if name == "kegari-validation.json" {
            result = Some(serde_json::from_slice(&data).map_err(|_| {
                Error::new("hosted_artifact", "validation result schema is invalid")
            })?);
        }
    }
    result.ok_or_else(|| Error::new("hosted_artifact", "exact validation manifest is absent"))
}
impl DeliveryCoordinator {
    pub async fn observe_owned_validation_current(
        &self,
        admission: &DeliveryAdmission,
        validation: &ExactValidationPassed,
    ) -> Result<()> {
        self.current(admission, &[DeliveryPhase::NoChange])?;
        self.reobserve_hosted(admission, &validation.hosted).await?;
        self.current(admission, &[DeliveryPhase::NoChange])
    }
    /// Acquire the actual private workflow artifact and derive expectations from
    /// the host nonce plus provider run/job identities, never token fields.
    pub async fn acquire_exact_validation(
        &self,
        admission: &DeliveryAdmission,
        pending: &PendingExactValidation,
        verifier: &HostedProofVerifier,
    ) -> Result<ExactValidationPassed> {
        self.current(admission, &[DeliveryPhase::NoChange])?;
        if pending.work != admission.scope.work || pending.revision != admission.scope.revision {
            return Err(Error::new(
                "stale_result",
                "dispatch does not belong to this current source",
            ));
        }
        let contract = self.contract(admission)?;
        let route = &contract.configuration.no_change_validation;
        let github = self.github()?;
        let run_id = remote_id(&pending.receipt, "workflow_run_id")?
            .parse()
            .map_err(|_| Error::new("provider_response", "dispatch run identity invalid"))?;
        let run = github
            .workflow_run(&contract.provider, run_id, None)
            .await?;
        let name = format!(
            "kegari-validation-{}-{}",
            admission.operation_id(),
            run.attempt
        );
        let artifacts = github.artifacts(&contract.provider, run.id).await?;
        let matching: Vec<_> = artifacts
            .iter()
            .filter(|a| a.data.get("name").and_then(Value::as_str) == Some(name.as_str()))
            .collect();
        if matching.len() != 1 {
            return Err(Error::new(
                "hosted_artifact",
                "exact owned attempt must have one unambiguous validation artifact",
            ));
        }
        let archive = github
            .artifact_archive(&contract.provider, matching[0].id, run.id, &run.head_sha)
            .await?;
        let source = read_hosted_source_archive(&archive.bytes)?;
        if source.schema_version != 1
            || source.runtime_id != admission.scope.runtime_id
            || source.phase_operation != admission.operation_id()
            || source.dispatch_request != pending.receipt.request_id
            || source.challenge != pending.challenge.challenge
            || source.repository_id != contract.provider.id
            || source.run_id != run.id
            || source.run_attempt != run.attempt
            || source.tested_tree != admission.scope.revision.tested_tree
            || source.outputs.get(&route.observed_source_output)
                != Some(&admission.scope.revision.tested_commit)
            || source.proofs.is_empty()
            || source.proofs.len() > 256
        {
            return Err(Error::new(
                "hosted_artifact",
                "actual source/result output differs from admitted exact workflow contract",
            ));
        }
        let mut tokens = Vec::new();
        let mut checks = BTreeSet::new();
        for job in source.proofs {
            if job.token.len() > 64 * 1024 || !checks.insert(job.check_run_id) {
                return Err(Error::new(
                    "hosted_artifact",
                    "duplicate or unbounded hosted proof",
                ));
            }
            tokens.push(HostedToken {
                expectation: HostedProofExpectation {
                    context_digest: None,
                    runtime_id: admission.scope.runtime_id,
                    operation_id: admission.operation_id(),
                    challenge: pending.challenge.challenge,
                    repository_id: contract.provider.id,
                    repository_owner_id: contract.provider.owner_id,
                    run_id: run.id,
                    run_attempt: run.attempt,
                    check_run_id: job.check_run_id,
                    actor_id: run.actor_id,
                    trigger_sha: run.head_sha.clone(),
                    tested_source_sha: admission.scope.revision.tested_commit.clone(),
                    workflow_ref: route.proof.workflow_ref.clone(),
                    workflow_sha: route.proof.workflow_sha.clone(),
                    job_workflow_ref: route.proof.job_workflow_ref.clone(),
                    job_workflow_sha: route.proof.job_workflow_sha.clone(),
                    event_name: route.proof.event_name.clone(),
                },
                token: job.token,
            });
        }
        let mut passed = self
            .observe_exact_validation(admission, pending, &tokens, verifier)
            .await?;
        let reference = ArtifactReference {
            reference: format!(
                "github-artifact:{}/{}/{}",
                contract.repository.hostname, contract.provider.id, archive.artifact_id
            ),
            digest: archive.archive_digest.clone(),
        };
        for check in &mut passed.hosted.checks {
            check.artifacts.push(reference.clone());
        }
        let retained=self.key.sign("exact_validation_source",&json!({"work":admission.scope.work,"revision":admission.scope.revision,"dispatch":pending.receipt.request_id,
            "artifact_id":archive.artifact_id,"archive_digest":archive.archive_digest,"run":run.id,"attempt":run.attempt,"checks":passed.hosted.checks}))?;
        self.records.write(
            &format!("exact-source-{}.json", pending.receipt.request_id),
            &retained,
        )?;
        self.current(admission, &[DeliveryPhase::NoChange])?;
        Ok(passed)
    }
}

/// Read-only target preparation and live integration share one policy parser.
pub fn validate_repository_prerequisites(
    identity: &RepositoryId,
    repository: &crate::providers::Repository,
    branch: &str,
    base: &str,
    protection: &Value,
    rules: &[Value],
    required: &BTreeSet<String>,
) -> Result<evidence::RepositoryProtection> {
    identity.validate()?;
    crate::git::validate_oid(base)?;
    if identity.id != repository.identity.id.to_string() || !["main", "staging"].contains(&branch) {
        return Err(Error::new(
            "repository_policy",
            "repository/branch prerequisite identity differs",
        ));
    }
    if let Some(checks) = protection
        .get("required_status_checks")
        .filter(|v| !v.is_null())
    {
        if !checks.is_object() || checks.get("strict").is_some_and(|v| !v.is_boolean()) {
            return Err(Error::new(
                "provider_response",
                "malformed classic check policy",
            ));
        }
        for (field, objects) in [("contexts", false), ("checks", true)] {
            if let Some(values) = checks.get(field) {
                let values = values.as_array().ok_or_else(|| {
                    Error::new(
                        "provider_response",
                        "check policy inventory is not an array",
                    )
                })?;
                for v in values {
                    if if objects {
                        v.get("context").and_then(Value::as_str).is_none()
                    } else {
                        !v.is_string()
                    } {
                        return Err(Error::new(
                            "provider_response",
                            "check policy identity is malformed",
                        ));
                    }
                }
            }
        }
    }
    let validate_reviews = |value: &Value, count: &str, owners: &str| -> Result<()> {
        if !value.is_object()
            || value.get(count).is_some_and(|v| v.as_u64().is_none())
            || value.get(owners).is_some_and(|v| !v.is_boolean())
        {
            return Err(Error::new(
                "provider_response",
                "review requirement types are malformed",
            ));
        }
        Ok(())
    };
    if let Some(reviews) = protection
        .get("required_pull_request_reviews")
        .filter(|v| !v.is_null())
    {
        validate_reviews(
            reviews,
            "required_approving_review_count",
            "require_code_owner_reviews",
        )?;
    }
    for rule in rules {
        match rule.get("type").and_then(Value::as_str) {
            Some("pull_request") => validate_reviews(
                rule.get("parameters").ok_or_else(|| {
                    Error::new("provider_response", "review rule parameters missing")
                })?,
                "required_approving_review_count",
                "require_code_owner_review",
            )?,
            Some("required_status_checks")
                if rule
                    .pointer("/parameters/strict_required_status_checks_policy")
                    .and_then(Value::as_bool)
                    .is_none()
                    || rule
                        .pointer("/parameters/required_status_checks")
                        .and_then(Value::as_array)
                        .is_none() =>
            {
                return Err(Error::new(
                    "provider_response",
                    "ruleset check policy is malformed",
                ));
            }
            None => return Err(Error::new("provider_response", "ruleset type missing")),
            _ => (),
        }
    }
    let mut contexts: BTreeSet<String> = protection
        .pointer("/required_status_checks/contexts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    for check in protection
        .pointer("/required_status_checks/checks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(context) = check.get("context").and_then(Value::as_str) {
            contexts.insert(context.into());
        }
    }
    let mut strict = protection
        .pointer("/required_status_checks/strict")
        .and_then(Value::as_bool)
        == Some(true);
    let reviews = protection.get("required_pull_request_reviews");
    let mut extra = reviews
        .and_then(|r| r.get("required_approving_review_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut codeowners = reviews
        .and_then(|r| r.get("require_code_owner_reviews"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    for rule in rules {
        match rule.get("type").and_then(Value::as_str) {
            Some("required_status_checks") => {
                strict |= rule
                    .pointer("/parameters/strict_required_status_checks_policy")
                    .and_then(Value::as_bool)
                    == Some(true);
                for check in rule
                    .pointer("/parameters/required_status_checks")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let context =
                        check
                            .get("context")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                Error::new("provider_response", "ruleset check identity missing")
                            })?;
                    contexts.insert(context.into());
                }
            }
            Some("pull_request") => {
                extra = extra.max(
                    rule.pointer("/parameters/required_approving_review_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                );
                codeowners |= rule
                    .pointer("/parameters/require_code_owner_review")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            }
            _ => {}
        }
    }
    // The one selected publishing user cannot formally approve its own PR.
    // Do not silently weaken existing human/code-owner review requirements.
    if extra != 0 || codeowners {
        return Err(Error::new(
            "repository_incompatible",
            "required approving/code-owner reviews cannot be satisfied by this same-account unattended route",
        ));
    }
    let observed = evidence::RepositoryProtection {
        repository: identity.clone(),
        branch: branch.into(),
        observed_base: base.into(),
        required_contexts: contexts,
        strict_current_base: strict,
        squash_allowed: repository.squash_allowed == Some(true),
        extra_approvals_satisfied: true,
        code_owner_reviews_satisfied: true,
        admin_bypass: false,
        observed_at: Utc::now(),
    };
    if !observed.strict_current_base
        || !observed.squash_allowed
        || !observed
            .required_contexts
            .contains(evidence::REVIEW_CONTEXT)
        || !required.is_subset(&observed.required_contexts)
    {
        return Err(Error::new(
            "repository_policy",
            "required strict status contexts or squash merging are not enforced",
        ));
    }
    Ok(observed)
}

impl DeliveryCoordinator {
    /// Reconcile a uniquely observed signed publication without replaying it.
    /// This read-only provider phase requires authenticated recovery authority,
    /// but does not require RUNNING intent or adopt a worker's proposed record.
    /// Find an actual retained publication effect by the user's receipt ID.
    /// This is metadata lookup only; subsequent reconciliation reobserves the
    /// current provider record and rechecks authenticated operator authority.
    pub fn publication_recovery_target(&self, request: Uuid) -> Result<Option<(Uuid, String)>> {
        let mut found = None;
        for entry in std::fs::read_dir(self.records.path())
            .map_err(|error| Error::io("list retained delivery effects", error))?
            .take(65_537)
            .enumerate()
        {
            let (index, entry) = entry;
            if index == 65_536 {
                return Err(Error::new(
                    "record_limit",
                    "delivery effect inventory exceeds its bound",
                ));
            }
            let entry = entry.map_err(|error| Error::io("read retained delivery effect", error))?;
            let leaf = entry.file_name().to_string_lossy().into_owned();
            if !leaf.starts_with("delivery-effect-") || !leaf.ends_with(".json") {
                continue;
            }
            let record: EffectRecord = self.records.read(&leaf)?;
            if record.request_id != request {
                continue;
            }
            self.key.verify("delivery_mutation", &record.command)?;
            if record.schema_version != 1
                || record.payload_digest != canonical_digest(1, &record.command.payload)?
                || found.is_some()
            {
                return Err(Error::new(
                    "record_corrupt",
                    "delivery receipt has ambiguous or invalid signed effect lineage",
                ));
            }
            let receipt = self.host.operation(record.request_id)?;
            let operation = receipt
                .expected
                .get("admission_operation")
                .and_then(|id| Uuid::parse_str(id).ok())
                .ok_or_else(|| {
                    Error::new(
                        "record_corrupt",
                        "delivery receipt lacks its actual phase admission",
                    )
                })?;
            if receipt.expected.get("delivery_scope") != Some(&record.admission_digest) {
                return Err(Error::new(
                    "record_corrupt",
                    "delivery receipt scope differs from retained signed effect",
                ));
            }
            found = Some((operation, record.action));
        }
        Ok(found)
    }
    pub async fn reconcile_receipt(
        &self,
        actor: &Actor,
        original_operation: Uuid,
        action: &str,
    ) -> Result<Receipt> {
        self.reconcile_receipt_checked(actor, original_operation, action, |_| Ok(()))
            .await
    }

    pub async fn reconcile_receipt_checked(
        &self,
        actor: &Actor,
        original_operation: Uuid,
        action: &str,
        final_gate: impl FnOnce(&crate::control::ControlSnapshot) -> Result<()> + Send,
    ) -> Result<Receipt> {
        actor.require("recover")?;
        if original_operation.is_nil()
            || ![
                "publish-review",
                "publish-no-change-review",
                "publish-child-result",
                "publish-parent-result",
            ]
            .contains(&action)
        {
            return Err(Error::invalid(
                "supported publication recovery requires its original operation/action",
            ));
        }
        let leaf = format!(
            "delivery-effect-{}.json",
            &bytes_digest(format!("{original_operation}:{action}").as_bytes())[7..]
        );
        let record: EffectRecord = self.records.read(&leaf)?;
        self.key.verify("delivery_mutation", &record.command)?;
        let original = self.host.operation(record.request_id)?;
        if record.schema_version != 1
            || record.action != action
            || record.payload_digest != canonical_digest(1, &record.command.payload)?
            || original.expected.get("delivery_scope") != Some(&record.admission_digest)
        {
            return Err(Error::new(
                "ownership_conflict",
                "recovery receipt does not match immutable signed mutation",
            ));
        }
        let (id, result, remote_key) = match &record.command.payload {
            ProviderMutation::LinearComment { issue, body } => {
                if original.kind != "linear_comment" {
                    return Err(Error::new(
                        "ownership_conflict",
                        "retained publication kind differs",
                    ));
                }
                let linear =
                    self.broker.linear.as_ref().ok_or_else(|| {
                        Error::new("prerequisite", "Linear connection unavailable")
                    })?;
                let writer = linear.identity().await?.account_id;
                let matches: Vec<_> = linear
                    .comments(*issue)
                    .await?
                    .into_iter()
                    .filter(|c| {
                        c.body == *body && c.user_id.is_some_and(|id| id.to_string() == writer)
                    })
                    .collect();
                if matches.len() > 1 {
                    return Err(Error::new(
                        "ownership_conflict",
                        "multiple owned canonical publications match; preserve uncertainty",
                    ));
                }
                let found = matches.first().ok_or_else(|| {
                    Error::new(
                        "operation_uncertain",
                        "owned publication is not visible; absence does not permit replay",
                    )
                })?;
                (
                    found.id.to_string(),
                    json!({"comment_id":found.id,"body_digest":bytes_digest(body.as_bytes()),"reconciled":true}),
                    "comment_id",
                )
            }
            ProviderMutation::GithubComment {
                repository,
                pr,
                body,
                ..
            } => {
                if original.kind != "github_comment" {
                    return Err(Error::new(
                        "ownership_conflict",
                        "retained publication kind differs",
                    ));
                }
                let github = self.github()?;
                let writer = github.identity().await?.account_id;
                let matches: Vec<_> = github
                    .comments(repository, *pr)
                    .await?
                    .into_iter()
                    .filter(|c| {
                        c.data.get("body").and_then(Value::as_str) == Some(body.as_str())
                            && c.data
                                .pointer("/user/id")
                                .and_then(Value::as_u64)
                                .is_some_and(|id| id.to_string() == writer)
                    })
                    .collect();
                if matches.len() > 1 {
                    return Err(Error::new(
                        "ownership_conflict",
                        "multiple owned canonical publications match; preserve uncertainty",
                    ));
                }
                let found = matches.first().ok_or_else(|| {
                    Error::new(
                        "operation_uncertain",
                        "owned publication is not visible; absence does not permit replay",
                    )
                })?;
                (
                    found.id.to_string(),
                    json!({"id":found.id,"body_digest":bytes_digest(body.as_bytes()),"reconciled":true}),
                    "record_id",
                )
            }
            _ => {
                return Err(Error::new(
                    "reconciliation_required",
                    "this effect requires its dedicated complete outcome reconciliation",
                ));
            }
        };
        let payload_digest = original.payload_digest.clone();
        let owner = original.actor.clone();
        let reconciled = self.host.reconcile_operation_observed(
            actor,
            record.request_id,
            |current, snapshot| {
                final_gate(snapshot)?;
                if current.payload_digest != payload_digest
                    || current.actor != owner
                    || current.expected.get("delivery_scope") != Some(&record.admission_digest)
                {
                    return Err(Error::new(
                        "ownership_conflict",
                        "immutable receipt changed during observation",
                    ));
                }
                Ok(crate::control::Reconciliation::UniqueVerified {
                    remote_ids: BTreeMap::from([(remote_key.into(), id)]),
                    result,
                    payload_digest,
                    actor: owner,
                })
            },
        )?;
        let mut updated = record;
        updated.receipt = Some(reconciled.clone());
        self.records.write(&leaf, &updated)?;
        Ok(reconciled)
    }
}

/// Preparation inputs are actual staged capabilities and already verified
/// production contributions. Candidate construction alone passes no parent,
/// acceptance or release gate.
pub struct SliceAssembly<'a> {
    pub execution_root: &'a Path,
    pub staged: &'a [&'a StagedDelta],
    pub required_children: &'a BTreeSet<Uuid>,
    pub child_dependencies: &'a BTreeMap<Uuid, BTreeSet<Uuid>>,
    pub satisfied_children: &'a BTreeSet<Uuid>,
    pub production_integrations: &'a [crate::git::VerifiedProductionIntegration],
    pub identity: crate::git::CommitIdentity,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SliceIntent {
    schema_version: u32,
    operation_id: Uuid,
    request_id: Uuid,
    scope: DeliveryScope,
    execution_root: PathBuf,
    production: crate::git::Revision,
    selected: Vec<String>,
    dependencies: BTreeMap<Uuid, BTreeSet<Uuid>>,
    satisfied_children: BTreeSet<Uuid>,
    integrations: Vec<crate::git::ProductionIntegration>,
    identity: crate::git::CommitIdentity,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SliceRecord {
    intent: SliceIntent,
    candidate: crate::git::CandidateManifest,
    receipt_id: Uuid,
}
#[derive(Debug)]
pub struct IsolatedSlice {
    record: Signed<SliceRecord>,
    candidate: crate::git::Candidate,
}
impl IsolatedSlice {
    pub fn provenance_digest(&self) -> Result<String> {
        canonical_digest(1, &self.record)
    }
    pub fn manifest(&self) -> &crate::git::CandidateManifest {
        &self.candidate.manifest
    }
    pub fn path(&self) -> &Path {
        self.candidate.path()
    }
    pub fn operation_id(&self) -> Uuid {
        self.candidate.manifest.operation_id
    }
}
impl DeliveryCoordinator {
    async fn main_revision(
        &self,
        observation: &DeliveryObservation,
    ) -> Result<crate::git::Revision> {
        self.current(observation, &[observation.scope.phase])?;
        let contract = self.contract(observation)?;
        let commit = self
            .github()?
            .reference(&contract.provider, "heads/main")
            .await?;
        let object = self.github()?.commit(&contract.provider, &commit).await?;
        if object.get("sha").and_then(Value::as_str) != Some(commit.as_str()) {
            return Err(Error::new(
                "provider_response",
                "production commit object differs from main",
            ));
        }
        let tree = object
            .pointer("/tree/sha")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new("provider_response", "production tree is unavailable"))?
            .to_owned();
        let revision = crate::git::Revision { commit, tree };
        revision.validate()?;
        Ok(revision)
    }
    /// One explicitly admitted local phase. Every Git subprocess is started
    /// under the current permit; joining and provider reads occur outside the
    /// control mutex. A partial candidate is retained and never reset/replayed.
    pub async fn assemble_isolated_slice(
        &self,
        admission: &DeliveryAdmission,
        input: SliceAssembly<'_>,
    ) -> Result<IsolatedSlice> {
        self.assemble_slice_from_source(admission, input, false)
            .await
    }
    /// Native runtime preparation acquires the complete selected remote roots
    /// into a private substrate before applying the unchanged selected patches.
    pub async fn assemble_remote_isolated_slice(
        &self,
        admission: &DeliveryAdmission,
        input: SliceAssembly<'_>,
    ) -> Result<IsolatedSlice> {
        self.assemble_slice_from_source(admission, input, true)
            .await
    }
    async fn assemble_slice_from_source(
        &self,
        admission: &DeliveryAdmission,
        input: SliceAssembly<'_>,
        acquire_objects: bool,
    ) -> Result<IsolatedSlice> {
        self.current(admission, &[DeliveryPhase::CandidateAssembly])?;
        if admission.scope.work.child.is_some()
            || input.required_children.is_empty()
            || input.required_children != admission.gate.required_children()
            || input
                .child_dependencies
                .keys()
                .any(|id| !input.required_children.contains(id))
            || input
                .satisfied_children
                .iter()
                .any(|id| !input.required_children.contains(id))
        {
            return Err(Error::new(
                "scope_violation",
                "candidate preparation needs this parent's complete declared child scope",
            ));
        }
        let production = self.main_revision(admission).await?;
        if admission.scope.revision.base != production.commit {
            return Err(Error::new(
                "stale_result",
                "production moved since candidate admission",
            ));
        }
        let mut deltas = Vec::new();
        let mut seen = BTreeSet::new();
        for staged in input.staged {
            let delta = &staged.delta.manifest;
            if delta.repository != admission.scope.repository
                || delta.parent_id != admission.scope.work.parent
                || delta.work_generation != admission.scope.work.generation.definition
                || delta.correction_generation != admission.scope.work.generation.correction
                || !input.required_children.contains(&delta.child_id)
                || !seen.insert(delta.digest()?)
            {
                return Err(Error::new(
                    "candidate_provenance",
                    "staged input is duplicate or outside current parent/repository/generation",
                ));
            }
            // Recheck the signed original patch bytes and live integration.
            let retained = self.retained_delta(delta)?;
            let pr = self
                .github()?
                .pull_request(&self.contract(admission)?.provider, staged.pull_request)
                .await?;
            if pr.merged != Some(true)
                || pr.merge_commit_sha.as_deref() != Some(delta.staging_commit.commit.as_str())
                || pr.head_sha != delta.validated_head.commit
                || pr.base_ref != "staging"
            {
                return Err(Error::new(
                    "candidate_provenance",
                    "staged source no longer matches its actual merged PR",
                ));
            }
            deltas.push(retained);
        }
        let root = input
            .execution_root
            .canonicalize()
            .map_err(|e| Error::io("resolve candidate root", e))?;
        let intent = SliceIntent {
            schema_version: 1,
            operation_id: admission.operation_id(),
            request_id: Uuid::new_v4(),
            scope: admission.scope.clone(),
            execution_root: root,
            production,
            selected: deltas
                .iter()
                .map(|d| d.manifest.digest())
                .collect::<Result<_>>()?,
            dependencies: input.child_dependencies.clone(),
            satisfied_children: input.satisfied_children.clone(),
            integrations: input
                .production_integrations
                .iter()
                .map(|p| p.record().clone())
                .collect(),
            identity: input.identity,
        };
        let intent_leaf = format!("slice-intent-{}.json", admission.operation_id());
        let intent: SliceIntent = {
            let _lock = self.records.section_lock("delivery-effects.lock")?;
            match self.records.read::<Signed<SliceIntent>>(&intent_leaf) {
                Ok(existing) => {
                    self.key.verify("slice_intent", &existing)?;
                    let mut comparable = intent.clone();
                    comparable.request_id = existing.payload.request_id;
                    if canonical_digest(1, &comparable)? != canonical_digest(1, &existing.payload)?
                    {
                        return Err(Error::new(
                            "operation_conflict",
                            "candidate attempt was reused with different source or dependencies",
                        ));
                    }
                    existing.payload
                }
                Err(error) if error.code == "record_missing" => {
                    self.records
                        .write(&intent_leaf, &self.key.sign("slice_intent", &intent)?)?;
                    intent
                }
                Err(error) => return Err(error),
            }
        };
        let record_leaf = format!("slice-result-{}.json", admission.operation_id());
        match self.records.read::<Signed<SliceRecord>>(&record_leaf) {
            Ok(_) => {
                return self
                    .recover_isolated_slice(admission, admission.operation_id())
                    .await;
            }
            Err(error) if error.code == "record_missing" => {}
            Err(error) => return Err(error),
        }
        let contract = self.contract(admission)?;
        let acquired = if acquire_objects {
            let request = crate::remote_objects::AuthorizedObjectRequest::for_candidate(
                contract,
                intent.production.clone(),
                &admission.permit,
                admission,
                input.staged,
                input.production_integrations,
            )?;
            Some(
                self.object_acquisition(
                    admission,
                    self.processes.clone(),
                    self.executions.clone(),
                )?
                .acquire(request)
                .await?,
            )
        } else {
            None
        };
        let source = acquired
            .as_ref()
            .map(|source| source.root().to_path_buf())
            .unwrap_or_else(|| {
                self.retained_git_sources
                    .get(&contract.repository)
                    .cloned()
                    .unwrap_or_else(|| PathBuf::from(&contract.configuration.source))
            });
        let owned_intent = intent.clone();
        let integrations = input.production_integrations.to_vec();
        let host = self.host.clone();
        let permit = admission.permit.clone();
        let current_gate = self.current_gate.clone();
        let gate = admission.gate.clone();
        let start_gate: crate::git::GitStartGate = Arc::new(move |spawn| {
            host.validate_permit(&permit, || {
                current_gate(&gate)?;
                spawn()
            })
        });
        let (effect, running) = self.host.begin_effect(
            MutationRequest {
                request_id: intent.request_id,
                runtime_id: admission.scope.runtime_id,
                actor: Actor {
                    principal: None,
                    identity: format!("owner:{}", admission.permit.lease.owner_id),
                    roles: BTreeSet::new(),
                },
                kind: "candidate_assembly".into(),
                canonical_targets: vec![admission.scope.repository.id.clone()],
                expected: BTreeMap::from([("delivery_scope".into(), admission.scope.digest()?)]),
                payload: serde_json::to_value(&intent)
                    .map_err(|_| Error::invalid("candidate intent encoding"))?,
            },
            &admission.permit,
            || (self.current_gate)(&admission.gate),
            |receipt| {
                let running = self.git.start_journaled_operation(
                    crate::git::GitLimits::default(),
                    start_gate,
                    self.processes.clone(),
                    admission.permit.clone(),
                    move |git| {
                        if let Some(source) = &acquired {
                            source.verify_for_clone(&git)?;
                        }
                        let order = git.order_deltas(
                            &source,
                            &deltas,
                            &owned_intent.dependencies,
                            &owned_intent.satisfied_children,
                        )?;
                        let ordered: Vec<_> = order
                            .into_iter()
                            .map(|index| deltas[index].clone())
                            .collect();
                        git.assemble_candidate(
                            &source,
                            &owned_intent.execution_root,
                            crate::git::CandidateRequest {
                                operation_id: owned_intent.scope_id(),
                                runtime_id: owned_intent.scope.runtime_id,
                                repository: owned_intent.scope.repository.clone(),
                                parent_id: owned_intent.scope.work.parent,
                                work_generation: &owned_intent.scope.work.generation.definition,
                                correction_generation: &owned_intent
                                    .scope
                                    .work
                                    .generation
                                    .correction,
                                production_commit: &owned_intent.production.commit,
                                deltas: &ordered,
                                child_dependencies: &owned_intent.dependencies,
                                satisfied_children: &owned_intent.satisfied_children,
                                production_integrations: &integrations,
                                identity: owned_intent.identity.clone(),
                            },
                        )
                    },
                )?;
                self.executions.register(
                    admission.operation_id(),
                    receipt.operation_id,
                    Arc::new(running.cancellation()),
                )?;
                Ok(running)
            },
        )?;
        let cancellation = running.cancellation();
        let result = running.wait().await;
        if !matches!(cancellation.exited(), Ok(true)) {
            self.host.fence(admission.operation_id())?;
            self.executions
                .cancel_fenced(&self.host, admission.operation_id())?;
            return Err(Error::new(
                "operation_uncertain",
                "candidate Git process exit is unconfirmed; retain partial work",
            ));
        }
        self.executions
            .remove(admission.operation_id(), effect.operation_id())?;
        let receipt = self.host.complete_effect(
            effect,
            &admission.permit,
            || (self.current_gate)(&admission.gate),
            match &result {
                Ok(candidate) => json!({"candidate":candidate.manifest}),
                Err(error) => json!({"error":error,"partial_work":"retained"}),
            },
        )?;
        let candidate = result?;
        if self.main_revision(admission).await? != intent.production {
            return Err(Error::new(
                "stale_result",
                "production moved during isolated candidate construction; retain candidate",
            ));
        }
        let record = self.key.sign(
            "isolated_slice",
            &SliceRecord {
                intent,
                candidate: candidate.manifest.clone(),
                receipt_id: receipt.request_id,
            },
        )?;
        self.records.write(&record_leaf, &record)?;
        Ok(IsolatedSlice { record, candidate })
    }
    pub async fn recover_isolated_slice(
        &self,
        observation: &DeliveryObservation,
        operation: Uuid,
    ) -> Result<IsolatedSlice> {
        self.current(
            observation,
            &[
                DeliveryPhase::CandidateAssembly,
                DeliveryPhase::ParentEvidence,
                DeliveryPhase::Release,
                DeliveryPhase::Recovery,
            ],
        )?;
        let record: Signed<SliceRecord> = self
            .records
            .read(&format!("slice-result-{operation}.json"))?;
        self.key.verify("isolated_slice", &record)?;
        let r = &record.payload;
        if r.intent.schema_version != 1
            || r.intent.scope.runtime_id != observation.scope.runtime_id
            || r.intent.scope.work != observation.scope.work
            || r.intent.scope.repository != observation.scope.repository
            || r.intent.scope.contract_digest != observation.scope.contract_digest
            || r.candidate.operation_id != r.intent.scope_id()
            || self.main_revision(observation).await? != r.intent.production
        {
            return Err(Error::new(
                "stale_result",
                "retained isolated slice differs from current parent or production",
            ));
        }
        let candidate = self
            .git
            .inspect_candidate(&r.intent.execution_root, r.intent.scope_id())?;
        if canonical_digest(1, &candidate.manifest)? != canonical_digest(1, &r.candidate)? {
            return Err(Error::new(
                "provenance_mismatch",
                "candidate objects or contribution map differ from authenticated preparation",
            ));
        }
        let receipt = self.host.operation(r.receipt_id)?;
        if !matches!(
            receipt.phase,
            crate::control::ReceiptPhase::Acknowledged | crate::control::ReceiptPhase::Reconciled
        ) || receipt.runtime_id != r.intent.scope.runtime_id
            || receipt.kind != "candidate_assembly"
            || receipt.payload_digest
                != canonical_digest(
                    1,
                    &json!({
                            "kind":"candidate_assembly", "targets":[r.intent.scope.repository.id],
                    "expected":{
                        "delivery_scope":r.intent.scope.digest()?,
                        "admitted_scope":r.intent.scope.digest()?,
                        "admission_operation":r.intent.operation_id.to_string(),
                        "admitted_work":crate::authority::IssueKey::new(&r.intent.scope.work.linear_endpoint,
                            r.intent.scope.work.linear_workspace, r.intent.scope.work.parent)?.identity()?,
                    }, "payload":r.intent,
                            "principal":null,
                        }),
                )?
        {
            return Err(Error::new(
                "operation_uncertain",
                "candidate preparation lacks its acknowledged exact local receipt",
            ));
        }
        self.current(observation, &[observation.scope.phase])?;
        Ok(IsolatedSlice { record, candidate })
    }
}
impl SliceIntent {
    fn scope_id(&self) -> Uuid {
        self.operation_id
    }
}

/// Complete host-derived parent contract used by candidate-vector and integrated
/// evidence phases. Repository and test-resource inventories come from the
/// current authorized child metadata, never a worker-selected subset.
#[derive(Clone, Debug)]
pub struct ParentContract {
    gate: FreshGate,
    required_repositories: BTreeSet<RepositoryId>,
    e2e_child: Uuid,
    visual: bool,
    production_targets: BTreeSet<String>,
    test_resources: BTreeSet<String>,
    integrated_commands: BTreeMap<String, Vec<String>>,
}
impl ParentContract {
    pub fn work(&self) -> &WorkIdentity {
        self.gate.work()
    }
    pub fn required_children(&self) -> &BTreeSet<Uuid> {
        self.gate.required_children()
    }
    pub fn repositories(&self) -> &BTreeSet<RepositoryId> {
        &self.required_repositories
    }
    pub fn e2e_child(&self) -> Uuid {
        self.e2e_child
    }
    pub fn test_resources(&self) -> &BTreeSet<String> {
        &self.test_resources
    }
    pub fn integrated_commands(&self) -> &BTreeMap<String, Vec<String>> {
        &self.integrated_commands
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &json!({"work":self.gate.work(),"config":self.gate.config_revision(),
            "workflow":self.gate.workflow_digest(),"required_children":self.gate.required_children(),
            "repositories":self.required_repositories,"e2e_child":self.e2e_child,"visual":self.visual,
            "production_targets":self.production_targets,"test_resources":self.test_resources,
            "integrated_commands":self.integrated_commands,"dependencies":self.gate.dependency_digest()}),
        )
    }
}
impl DeliveryCoordinator {
    pub fn parent_contract(
        &self,
        gate: FreshGate,
        projection: &crate::reconcile::ParentProjection,
        workflow: &Workflow,
    ) -> Result<ParentContract> {
        (self.current_gate)(&gate)?;
        if projection.config_revision != gate.config_revision() {
            return Err(Error::new(
                "stale_result",
                "parent contract differs from the current complete authorized generation",
            ));
        }
        self.parent_contract_body(gate, projection, workflow)
    }

    /// Only the runtime's serialized check of a real active phase can mint
    /// this borrowed input. Keep the observed revision intact; pinned execution
    /// settings and current adjusted observation are distinct proven inputs.
    pub(crate) fn active_parent_contract(
        &self,
        input: crate::runtime_delivery::ActiveParentContractInput<'_>,
    ) -> Result<ParentContract> {
        if input.contracts().len() != self.contracts.len() {
            return Err(Error::new(
                "scope_violation",
                "active parent coordinator differs from its pinned repository contracts",
            ));
        }
        for admitted in input.contracts() {
            let current = self.contracts.get(&admitted.repository).ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "active parent repository contract is unavailable",
                )
            })?;
            if current.digest()? != admitted.digest()? {
                return Err(Error::new(
                    "scope_violation",
                    "active parent repository contract changed",
                ));
            }
        }
        (self.current_gate)(input.gate())?;
        self.parent_contract_body(input.gate().clone(), input.parent(), input.workflow())
    }

    fn parent_contract_body(
        &self,
        gate: FreshGate,
        projection: &crate::reconcile::ParentProjection,
        workflow: &Workflow,
    ) -> Result<ParentContract> {
        if gate.work().child.is_some()
            || gate.purpose() == crate::reconcile::GatePurpose::Coordination
            || projection.work.as_ref() != Some(gate.work())
            || projection.workspace_id != gate.workspace_id()
            || projection.required_children != *gate.required_children()
            || projection.workflow_digest != workflow.digest
            || workflow.digest != gate.workflow_digest()
        {
            return Err(Error::new(
                "stale_result",
                "parent contract differs from the current complete authorized generation",
            ));
        }
        let authorized = projection.authorization_candidate.as_ref().ok_or_else(|| {
            Error::new(
                "authorization_unpublished",
                "parent scope snapshot is unavailable",
            )
        })?;
        if canonical_digest(1, &authorized.scope)? != gate.work().generation.scope
            || authorized.required_children != *gate.required_children()
            || authorized.parent != projection.key
            || authorized
                .scope
                .child_scopes
                .keys()
                .copied()
                .collect::<BTreeSet<_>>()
                != *gate.required_children()
        {
            return Err(Error::new(
                "scope_violation",
                "parent contract is not the exact authorized child/target scope",
            ));
        }
        let issue = projection
            .issue
            .as_ref()
            .ok_or_else(|| Error::new("incompletely_observed", "parent issue missing"))?;
        let definition = canonical_digest(
            1,
            &json!({"title":issue.title,"description":issue.description,"parent":issue.parent_id}),
        )?;
        let metadata: crate::authority::ParentMetadata =
            crate::authority::metadata(issue.description.as_deref().ok_or_else(|| {
                Error::new("incompletely_prepared", "parent definition is absent")
            })?)?;
        if definition != authorized.scope.parent_definition
            || projection.metadata.as_ref() != Some(&metadata)
            || !gate.required_children().contains(&metadata.e2e_child)
            || metadata
                .production_targets
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                != authorized.scope.production_targets
        {
            return Err(Error::new(
                "scope_violation",
                "parent result metadata changed outside its exact definition",
            ));
        }
        let mut repositories = BTreeSet::new();
        let mut e2e_resources = None;
        for child_id in gate.required_children() {
            let child = projection
                .children
                .get(child_id)
                .ok_or_else(|| Error::new("parent_incomplete", "required child disappeared"))?;
            let scope = child.scope.as_ref().ok_or_else(|| {
                Error::new("incompletely_prepared", "required child scope missing")
            })?;
            if authorized.scope.child_scopes.get(child_id) != Some(&canonical_digest(1, scope)?) {
                return Err(Error::new(
                    "scope_violation",
                    "child repository/resource scope differs from authorized parent",
                ));
            }
            for name in scope
                .write_repository
                .iter()
                .chain(&scope.read_repositories)
            {
                let found: Vec<_> = self
                    .contracts
                    .values()
                    .filter(|c| &c.configuration_name == name)
                    .collect();
                if found.len() != 1 {
                    return Err(Error::new(
                        "scope_violation",
                        "authorized child repository is not uniquely configured",
                    ));
                }
                repositories.insert(found[0].repository.clone());
            }
            if *child_id == metadata.e2e_child {
                e2e_resources = Some(scope.test_resources.iter().cloned().collect());
            }
        }
        for target in &authorized.scope.production_targets {
            let mut found = false;
            for contract in self.contracts.values() {
                if let crate::config::Deployment::Steps { steps } =
                    &contract.configuration.deployment
                    && steps.iter().any(|step| &step.target_id == target)
                {
                    found = true;
                    repositories.insert(contract.repository.clone());
                }
            }
            if !found {
                return Err(Error::new(
                    "incompletely_prepared",
                    "parent target has no configured production route",
                ));
            }
        }
        if repositories.is_empty() {
            return Err(Error::new(
                "incompletely_prepared",
                "parent needs an exact configured repository baseline",
            ));
        }
        let mut commands = BTreeMap::new();
        for repository in &repositories {
            let contract = &self.contracts[repository];
            for name in &contract.configuration.integrated_checks {
                let command = workflow
                    .front_matter
                    .checks
                    .integrated
                    .get(name)
                    .ok_or_else(|| {
                        Error::new(
                            "incompletely_prepared",
                            "configured integrated check missing from current workflow",
                        )
                    })?
                    .argv
                    .clone();
                if let Some(previous) = commands.insert(name.clone(), command.clone())
                    && previous != command
                {
                    return Err(Error::new(
                        "incompletely_prepared",
                        "integrated check name has conflicting commands",
                    ));
                }
            }
        }
        Ok(ParentContract {
            gate,
            required_repositories: repositories,
            e2e_child: metadata.e2e_child,
            visual: metadata.visual,
            production_targets: authorized.scope.production_targets.clone(),
            test_resources: e2e_resources.ok_or_else(|| {
                Error::new("incompletely_prepared", "E2E child resources unavailable")
            })?,
            integrated_commands: commands,
        })
    }
}

#[path = "delivery_parent.rs"]
mod parent;
pub use parent::*;
pub(crate) use parent::{ParentOriginRecord, PreparedParentRunOrigin, read_parent_run_origin};
#[path = "delivery_child.rs"]
mod child;
#[path = "delivery_discovered_validation.rs"]
mod discovered_validation;
pub use child::*;

#[path = "delivery_release.rs"]
mod production_release;
pub use production_release::*;

#[path = "delivery_assignment.rs"]
mod assignment;
pub(crate) use assignment::public_model_context;
pub use assignment::*;

#[cfg(test)]
pub(crate) use production_release::tests::observed_merge_for_runtime;
#[cfg(test)]
pub(crate) use production_release::tests::{
    RuntimeCandidateSeed, RuntimeLineageWire, RuntimeSeedWire, TargetReleaseFixture,
    runtime_candidate_seed,
};

#[path = "delivery_objects.rs"]
mod objects;
pub(crate) use objects::RetainedCarryIntegrations;

#[path = "delivery_lineage_drop.rs"]
mod lineage_drop;

#[path = "delivery_lineage.rs"]
mod lineage;

#[path = "delivery_continuation.rs"]
mod continuation;
pub use continuation::*;

#[path = "delivery_validation_retry.rs"]
mod validation_retry;
pub(crate) use validation_retry::{
    CompletedGrantedValidation, ConsumedValidationSuccessor, VerifiedPassedValidation,
};
pub use validation_retry::{ValidationRetryOrigin, ValidationRetryRequest, ValidationRetryWork};

#[path = "delivery_review_retry.rs"]
mod review_retry;
#[cfg(test)]
pub(crate) use review_retry::CompletedGrantedReview;
pub use review_retry::{ReviewRetryRequest, ReviewRetryWork};

// Opaque review-round provenance consumed by the runtime admission path.
#[path = "delivery_review_round.rs"]
mod review_round;
pub use review_round::VerifiedReviewRoundPlan;
pub(crate) use review_round::{
    CompletedReviewLineage, CompletedReviewRound, PairedRecoveryInput, PairedRecoveryReaders,
    PassedValidationSuccessor, ReviewRoundSource, VerifiedCompletedPairedResume,
    VerifiedPairedRecoveryCandidate, verify_validation_successor_admission,
};
