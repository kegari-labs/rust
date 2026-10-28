//! Scoped production tools shared by orchestration and the actual-agent evaluator.
//! Worker output is a proposal. Only host observations can establish delivery gates.
#[path = "worker_retirement.rs"]
mod retirement;
pub(crate) use retirement::{ParentJournalHistory, capture_parent_journal};
#[path = "worker_boundary.rs"]
mod boundary;
#[path = "worker_broker_observation.rs"]
mod broker_observation;
#[path = "worker_command.rs"]
mod command;
#[path = "worker_command_history.rs"]
mod command_history;
#[path = "worker_read_sources.rs"]
mod read_sources;
#[path = "worker_resources.rs"]
mod resources;
#[path = "worker_validation.rs"]
mod validation;
pub(crate) use broker_observation::CaptureBudget;
use broker_observation::{BrokerCapture, BrokerJournalReference, BrokerSemantic};
pub use broker_observation::{BrokerControlBatch, BrokerInventory, IndependentBrokerObservation};
pub use validation::ValidationHistorySeal;

use crate::codex::{SessionState, ToolBroker, ToolCall};
use crate::control::{Actor, ControlHost, Lease, MutationRequest, Permit};
use crate::evaluation::{
    AgentSummary, BoundToolContracts, CheckStatus, ObservedArtifact, ObservedCheck, ObservedFacts,
    TrialHost,
};
use crate::evidence::{CheckKind, WorkIdentity};
use crate::execution::{
    CheckInvocation, CommandBoundary, ExecutionBinding, ExecutionLimits, GitInspection,
    LocalExecutor, OwnedDirectory, RetainedArtifact, SourceWrite, VerifiedCheckExecution,
};
use crate::git::Git;
use crate::instructions::{
    AgentRole, AssembledInstructions, ExposedTool, InstructionManifest, ToolCapability,
};
use crate::isolation::{ExecutionRole, ValidatedIsolation};
use crate::providers::{ProviderBroker, ProviderMutation};
use crate::reconcile::{FreshGate, IndependentReviewGate};
use crate::workflow::Workflow;
use chrono::{Duration, Utc};
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_providers::RepositoryIdentity;
use kegari_storage::{ExclusiveLock, PrivateDir};
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;

const MAX_TOOL_BYTES: usize = 65_536;
const MAX_CALLS: usize = 4096;
// Match the per-owner tool/host-process ceiling and the whole Control inventory
// ceiling. Reservations check both limits before their command is started.
const MAX_REGISTERED_COMMANDS: usize = 65_536;

type ActiveCommands =
    BTreeMap<Uuid, BTreeMap<Uuid, Arc<dyn crate::execution::CommandCancellation>>>;
/// Same-process handles only. Never restore numeric PIDs from journal data.
#[derive(Default)]
pub struct ExecutionRegistry {
    active: Mutex<ActiveCommands>,
    // Generic invocation closure includes its journal and Control acknowledgment.
    command_settlements: Mutex<BTreeSet<(Uuid, Uuid)>>,
}

enum ReservedCommand {
    Owned(Arc<dyn crate::execution::CommandCancellation>),
    NeverStarted,
}
#[derive(Default)]
struct CommandSlot {
    command: OnceLock<ReservedCommand>,
    cancel_requested: AtomicBool,
}
impl crate::execution::CommandCancellation for CommandSlot {
    fn check_bundle_owner(&self) -> Option<&crate::execution::CheckBundleOwner> {
        match self.command.get() {
            Some(ReservedCommand::Owned(command)) => command.check_bundle_owner(),
            _ => None,
        }
    }
    fn cancel(&self) -> Result<()> {
        self.cancel_requested.store(true, Ordering::SeqCst);
        match self.command.get() {
            Some(ReservedCommand::Owned(command)) => command.cancel(),
            Some(ReservedCommand::NeverStarted) => Ok(()),
            None => Err(Error::new(
                "operation_uncertain",
                "reserved command start has not returned its owned handle",
            )),
        }
    }
    fn exited(&self) -> Result<bool> {
        match self.command.get() {
            Some(ReservedCommand::Owned(command)) => command.exited(),
            Some(ReservedCommand::NeverStarted) => Ok(true),
            None => Ok(false),
        }
    }
}
/// The sole attachment authority for a registry-held slot. Dropping it leaves
/// unknown ownership; a boundary error cannot be inferred to mean no spawn.
pub(crate) struct ExecutionReservation {
    slot: Arc<CommandSlot>,
}
impl ExecutionReservation {
    pub(crate) fn attach(
        self,
        command: Arc<dyn crate::execution::CommandCancellation>,
    ) -> Result<()> {
        // This non-Clone token is the only writer. Store ownership before the
        // fallible cancellation, including cancellation requested before attach.
        let bound = self
            .slot
            .command
            .get_or_init(|| ReservedCommand::Owned(command));
        if self.slot.cancel_requested.load(Ordering::SeqCst)
            && let ReservedCommand::Owned(command) = bound
        {
            command.cancel()?;
        }
        Ok(())
    }
    /// Only a known no-spawn path (including an actual skipped hook) may call
    /// this. In particular, an opaque start error must retain the unknown slot.
    pub(crate) fn never_started(self) {
        self.slot
            .command
            .get_or_init(|| ReservedCommand::NeverStarted);
    }
}
impl ExecutionRegistry {
    /// Retain the exact original aggregate under its operation/effect key.
    /// Release the registry mutex before inspecting the owner's state or taking
    /// any Control gate. Pending reservations and ordinary commands return None.
    pub(crate) fn retained_check_bundle(
        &self,
        operation: Uuid,
        effect: Uuid,
    ) -> Result<Option<Arc<dyn crate::execution::CommandCancellation>>> {
        let handle = self
            .active
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "execution registry lock failed"))?
            .get(&operation)
            .and_then(|effects| effects.get(&effect))
            .cloned();
        let Some(handle) = handle else {
            return Ok(None);
        };
        let Some(owner) = handle.check_bundle_owner() else {
            return Ok(None);
        };
        if owner.plan().owner_operation() != operation || owner.plan().effect_id() != effect {
            return Err(Error::new(
                "ownership_conflict",
                "registered bundle differs from its exact owner/effect key",
            ));
        }
        Ok(Some(handle))
    }
    pub(crate) fn reserve(&self, operation: Uuid, effect: Uuid) -> Result<ExecutionReservation> {
        self.reserve_inner(operation, effect, false)
    }
    fn reserve_command(&self, operation: Uuid, effect: Uuid) -> Result<ExecutionReservation> {
        self.reserve_inner(operation, effect, true)
    }
    fn reserve_inner(
        &self,
        operation: Uuid,
        effect: Uuid,
        retain_until_settled: bool,
    ) -> Result<ExecutionReservation> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "execution registry lock failed"))?;
        if active
            .get(&operation)
            .is_some_and(|effects| effects.contains_key(&effect))
        {
            return Err(Error::new(
                "ownership_conflict",
                "local effect already has an owned process",
            ));
        }
        if active
            .get(&operation)
            .is_some_and(|effects| effects.len() >= MAX_CALLS)
            || active.values().map(BTreeMap::len).sum::<usize>() >= MAX_REGISTERED_COMMANDS
        {
            return Err(Error::new(
                "record_limit",
                "owned command registry exceeds its finite inventory",
            ));
        }
        if retain_until_settled {
            self.command_settlements
                .lock()
                .map_err(|_| {
                    Error::new("ownership_conflict", "command settlement registry failed")
                })?
                .insert((operation, effect));
        }
        let slot = Arc::new(CommandSlot::default());
        active
            .entry(operation)
            .or_default()
            .insert(effect, slot.clone());
        Ok(ExecutionReservation { slot })
    }
    pub(crate) fn register(
        &self,
        operation: Uuid,
        effect: Uuid,
        cancel: Arc<dyn crate::execution::CommandCancellation>,
    ) -> Result<()> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "execution registry lock failed"))?;
        let effects = active.entry(operation).or_default();
        if effects.contains_key(&effect) {
            return Err(Error::new(
                "ownership_conflict",
                "local effect already has an owned process",
            ));
        }
        effects.insert(effect, cancel);
        Ok(())
    }
    pub(crate) fn remove(&self, operation: Uuid, effect: Uuid) -> Result<()> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "execution registry lock failed"))?;
        self.command_settlements
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "command settlement registry failed"))?
            .remove(&(operation, effect));
        if let Some(effects) = active.get_mut(&operation) {
            effects.remove(&effect);
            if effects.is_empty() {
                active.remove(&operation);
            }
        }
        Ok(())
    }
    pub fn active_operations(&self) -> Result<BTreeSet<Uuid>> {
        Ok(self
            .active
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "execution registry lock failed"))?
            .keys()
            .copied()
            .collect())
    }
    /// Drop only known no-spawn slots or handles whose actual owned process/task
    /// has confirmed exit.
    /// This does not acknowledge any receipt or promote retained evidence.
    pub fn observe_exited(&self, operation: Uuid) -> Result<usize> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "execution registry lock failed"))?;
        let settlements = self
            .command_settlements
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "command settlement registry failed"))?;
        let mut removed = 0;
        if let Some(effects) = active.get_mut(&operation) {
            let exited = effects
                .iter()
                .map(|(id, handle)| {
                    // Process exit precedes bundle artifact capture and Control
                    // ACK. Keep that original owner until terminal-aware pruning
                    // or the worker's explicit post-ACK removal.
                    if handle.check_bundle_owner().is_some()
                        || settlements.contains(&(operation, *id))
                    {
                        Ok((*id, false))
                    } else {
                        handle.exited().map(|exited| (*id, exited))
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            for (id, exited) in exited {
                if exited {
                    effects.remove(&id);
                    removed += 1;
                }
            }
            if effects.is_empty() {
                active.remove(&operation);
            }
        }
        Ok(removed)
    }
    /// Caller owns the Control observation gate and supplies its actual receipt
    /// batch. No registry mutex is held while inspecting bundle/process state.
    pub(crate) fn observe_settled_check_bundles(
        &self,
        control: &ControlHost,
        receipts: &[crate::control::Receipt],
    ) -> Result<usize> {
        let handles = self
            .active
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "execution registry lock failed"))?
            .iter()
            .flat_map(|(operation, effects)| {
                effects
                    .iter()
                    .map(|(effect, handle)| (*operation, *effect, handle.clone()))
            })
            .collect::<Vec<_>>();
        let by_effect = receipts
            .iter()
            .map(|receipt| (receipt.operation_id, receipt))
            .collect::<BTreeMap<_, _>>();
        let mut removed = 0;
        for (operation, effect, handle) in handles {
            let Some(owner) = handle.check_bundle_owner() else {
                continue;
            };
            let Some(receipt) = by_effect.get(&effect) else {
                continue;
            };
            if !control.check_bundle_owner_closed(receipt, owner)? {
                continue;
            }
            let mut active = self
                .active
                .lock()
                .map_err(|_| Error::new("ownership_conflict", "execution registry lock failed"))?;
            // Discovered validation additionally owns its exact protected
            // completion/ACK record. A terminal Control receipt alone cannot
            // prune that owner during the subsequent journal write window.
            if self
                .command_settlements
                .lock()
                .map_err(|_| {
                    Error::new("ownership_conflict", "command settlement registry failed")
                })?
                .contains(&(operation, effect))
            {
                continue;
            }
            if let Some(effects) = active.get_mut(&operation) {
                if effects
                    .get(&effect)
                    .is_some_and(|current| Arc::ptr_eq(current, &handle))
                {
                    effects.remove(&effect);
                    removed += 1;
                }
                if effects.is_empty() {
                    active.remove(&operation);
                }
            }
        }
        Ok(removed)
    }
    /// Invoke after ControlHost has durably persisted force-stop/fenced work.
    /// Failed intent persistence cannot authorize process termination here.
    pub fn cancel_fenced(&self, control: &ControlHost, operation: Uuid) -> Result<usize> {
        let state = control.snapshot()?;
        if !state.fenced_operations.contains(&operation) || state.persistence_error.is_some() {
            return Err(Error::new(
                "access_denied",
                "persist and fence this operation before cancellation",
            ));
        }
        let handles: Vec<_> = self
            .active
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "execution registry lock failed"))?
            .get(&operation)
            .map(|effects| {
                effects
                    .iter()
                    .map(|(id, handle)| (*id, handle.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let count = handles.len();
        let mut failures = Vec::new();
        for (effect, handle) in handles {
            if let Err(error) = handle.cancel() {
                failures.push((effect, error));
            }
        }
        if let Some((_, first)) = failures.first() {
            return Err(first.clone().detail(json!({
                "original_details": first.details,
                "owned_cancellation_failures": failures
            })));
        }
        Ok(count)
    }
}

#[derive(Clone, Debug)]
pub enum WorkerAuthority {
    Authorized(FreshGate),
    Independent(IndependentReviewGate),
}
impl WorkerAuthority {
    pub fn digest(&self) -> Result<String> {
        match self {
            Self::Authorized(g) => g.digest(),
            Self::Independent(g) => g.digest(),
        }
    }
    pub fn work_key(&self) -> Result<String> {
        match self {
            Self::Authorized(g) => {
                let work = g.work();
                crate::authority::IssueKey::new(
                    &work.linear_endpoint,
                    work.linear_workspace,
                    work.child.unwrap_or(work.parent),
                )?
                .identity()
            }
            Self::Independent(g) => {
                let url = url::Url::parse(&g.pull_request().url)
                    .map_err(|_| Error::invalid("review URL is invalid"))?;
                if url.scheme() != "https"
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(Error::invalid(
                        "review URL lacks a canonical HTTPS provider identity",
                    ));
                }
                canonical_digest(
                    1,
                    &json!({"provider":"github","hostname":url.host_str(),"port":url.port_or_known_default(),"repository_id":g.repository_id(),"pull_request":g.pull_request().number}),
                )
            }
        }
    }
    pub fn work(&self) -> Option<&WorkIdentity> {
        match self {
            Self::Authorized(g) => Some(g.work()),
            Self::Independent(_) => None,
        }
    }
    fn workspace(&self) -> Uuid {
        match self {
            Self::Authorized(g) => g.workspace_id(),
            Self::Independent(g) => g.workspace_id(),
        }
    }
    fn workflow(&self) -> &str {
        match self {
            Self::Authorized(g) => g.workflow_digest(),
            Self::Independent(g) => g.workflow_digest(),
        }
    }
    pub(crate) fn compare(&self, current: &Self) -> Result<()> {
        match (self, current) {
            (Self::Authorized(a), Self::Authorized(b)) => {
                a.compare_current(b, Utc::now(), Duration::seconds(30))
            }
            (Self::Independent(a), Self::Independent(b)) => {
                a.compare_current(b, Utc::now(), Duration::seconds(30))
            }
            _ => Err(Error::new(
                "scope_violation",
                "worker authority kind changed",
            )),
        }
    }
}

pub(crate) fn effective_authority(
    host: &dyn WorkerHost,
    operation: Uuid,
    original: &WorkerAuthority,
) -> Result<WorkerAuthority> {
    if let Some(transition) = host.correction_transition(operation)? {
        let WorkerAuthority::Authorized(admitted) = original else {
            return Err(Error::new(
                "scope_violation",
                "independent review cannot acquire Linear correction authority",
            ));
        };
        if transition.original_authority().digest()? != admitted.digest()? {
            return Err(Error::new(
                "worker_binding",
                "correction transition belongs to another original worker contract",
            ));
        }
        let expected = WorkerAuthority::Authorized(transition.current_authority().clone());
        let current = host.current_authority(&expected)?;
        expected.compare(&current)?;
        Ok(current)
    } else {
        let current = host.current_authority(original)?;
        original.compare(&current)?;
        Ok(current)
    }
}

/// Trusted host integration, never implemented by worker input. These callbacks
/// used by fresh-gate validation run inside ControlHost's serialized gate and
/// MUST NOT reenter it. Session observations explicitly run outside that gate.
/// Production implementations reread provider projections and verify the actual
/// orchestration PID/start identity plus held OS ownership lock.
pub trait WorkerHost: Send + Sync {
    /// Called immediately after a durable exact-bound checkpoint, outside the
    /// control gate. The host serializes account/work holds against new admission.
    fn observed_session(&self, operation: Uuid, state: &SessionState) -> Result<()>;
    fn current_authority(&self, expected: &WorkerAuthority) -> Result<WorkerAuthority>;
    /// Absence grants no transition; ordinary strict original authority checks
    /// still apply. Production returns only installed, freshly revalidated proof.
    fn correction_transition(
        &self,
        _operation: Uuid,
    ) -> Result<Option<Arc<crate::corrections::VerifiedCorrectionTransition>>> {
        Ok(None)
    }
    /// A finite wait keeps this worker owned while native input is classified;
    /// it grants no effects and never bypasses the ordinary current gate.
    fn correction_waiting(&self, _operation: Uuid) -> Result<bool> {
        Ok(false)
    }
    fn correction_wake(&self, _operation: Uuid) -> Result<Option<Arc<tokio::sync::Notify>>> {
        Ok(None)
    }
    fn verify_owner(&self, lease: &Lease) -> Result<()>;
    fn rate_limits(&self, operation: Uuid, observed: &Value) -> Result<()>;
    fn reconcile_prior(&self, operation: Uuid, prior: Option<&Value>) -> Result<()>;
    /// Host-authorized fixture/control event ingestion. Must use the ordinary
    /// correction classification/routing path and return its durable reference.
    fn correction_event(&self, operation: Uuid, watermark: u64, input: &str) -> Result<String>;
    /// Authoritative workflow state, not a model's claimed completion.
    fn observed_state(&self, operation: Uuid) -> Result<Value>;
    /// Observation-only callback, called outside Control. Production checks the
    /// exact original registered operation under its own single Control gate.
    /// A generic host cannot infer registration from equal serialized fields.
    fn observe_independent_broker(
        &self,
        _control: &Arc<ControlHost>,
        _permit: &Permit,
        _contract: &Arc<WorkerContract>,
        _inventory: &BrokerInventory,
    ) -> Result<BrokerControlBatch> {
        Err(Error::new(
            "boundary_observation_unavailable",
            "host has no actual registered broker observation",
        ))
    }
    fn observe_independent_boundary(
        &self,
        _control: &Arc<ControlHost>,
        _permit: &Permit,
        _contract: &Arc<WorkerContract>,
    ) -> Result<crate::evaluation::IndependentBoundaryIdentity> {
        Err(Error::new(
            "boundary_observation_unavailable",
            "host does not supply actual independent worker registration evidence",
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTarget {
    pub repository: RepositoryIdentity,
    pub pull_request: u64,
    pub head: String,
    pub base: String,
}

/// Diagnostic proposal with exact host provenance. The delivery coordinator
/// stores this as pending; deserializing it does not establish any passed gate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessedCorrection {
    pub watermark: u64,
    pub input_digest: String,
    pub transition_digest: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerOutcome {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub operation_id: Uuid,
    pub proposal_id: Uuid,
    pub completed_turn: String,
    pub work: Option<WorkIdentity>,
    pub independent_review: Option<ReviewTarget>,
    pub session: SessionState,
    pub instructions: InstructionManifest,
    pub instruction_digest: String,
    pub tool_contract_digest: String,
    pub fresh_gate_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processed_correction: Option<ProcessedCorrection>,
    pub claim: Value,
    pub artifact_refs: Vec<String>,
    /// Original host inventory marker; absence is retained only for historical
    /// readers. New production outcomes always bind their actual history seal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_history_digest: Option<String>,
    pub checks: Vec<Value>,
}
pub trait OutcomeSink: Send + Sync {
    fn record(&self, outcome: &WorkerOutcome) -> Result<String>;
}

/// All fields are fixed before admission. The digest is the Permit scope_digest.
#[derive(Clone, Debug)]
pub struct WorkerContract {
    assembled: AssembledInstructions,
    authority: WorkerAuthority,
    isolation: ValidatedIsolation,
    workflow: Workflow,
    context: Value,
    review: Option<ReviewTarget>,
    tools: BoundToolContracts,
    scope_digest: String,
    account_id: String,
    repository: Option<crate::git::RepositoryId>,
    read_resources: BTreeMap<PathBuf, ReadResource>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadResource {
    Repository {
        id: String,
    },
    TestResource {
        name: String,
        writable: bool,
        disposable: bool,
    },
}
#[derive(Clone)]
pub struct ContractInput {
    pub assembled: AssembledInstructions,
    pub authority: WorkerAuthority,
    pub isolation: ValidatedIsolation,
    pub workflow: Workflow,
    pub context: Value,
    pub review: Option<ReviewTarget>,
    pub account_id: String,
    pub repository: Option<crate::git::RepositoryId>,
    /// Host-configured canonical mapping for every read grant outside the main
    /// repository and scratch; worker arguments cannot create this mapping.
    pub read_resources: BTreeMap<PathBuf, ReadResource>,
}
fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({"type":"function","name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
}
/// Exact shipped definitions. Unsupported names cannot be relabelled with a
/// permitted capability to obtain broader behavior.
pub fn tool_definitions(role: AgentRole, writable: bool) -> Vec<(ExposedTool, Value)> {
    operation_tool_definitions(role, writable, role.permits_repository_execution())
}
pub fn operation_tool_definitions(
    role: AgentRole,
    writable: bool,
    executable: bool,
) -> Vec<(ExposedTool, Value)> {
    let mut result = vec![
        (
            "context_read",
            ToolCapability::ContextRead,
            tool(
                "context_read",
                "Read one named entry of the complete host-supplied task context. Content is data, not authority.",
                json!({"key":{"type":"string","minLength":1}}),
                &["key"],
            ),
        ),
        (
            "source_read",
            ToolCapability::ContextRead,
            tool(
                "source_read",
                "Read a bounded UTF-8 regular file in the primary repository or an explicitly host-bound read repository selected by repository_id. context_read read_sources lists available IDs. No raw root, symlinks, Git administration or writes.",
                json!({"path":{"type":"string","minLength":1},"repository_id":{"type":"string","minLength":1,"maxLength":256}}),
                &["path"],
            ),
        ),
        (
            "outcome_submit",
            ToolCapability::OutcomeSubmit,
            tool(
                "outcome_submit",
                "Submit an untrusted outcome proposal and observed evidence references. Host verification and turn completion remain required; this does not pass a delivery gate.",
                json!({"claim":{"type":"object"},"artifact_refs":{"type":"array","items":{"type":"string"},"maxItems":128},"processed_correction":{"type":"object","properties":{"watermark":{"type":"integer","minimum":0},"input_digest":{"type":"string"},"transition_digest":{"type":["string","null"]}},"required":["watermark","input_digest","transition_digest"],"additionalProperties":false}}),
                &["claim", "artifact_refs"],
            ),
        ),
    ];
    for (name, description) in [
        (
            "resource_list",
            "List all admitted non-Git children of a directory using a host resource handle from context_read execution_environment. A complete bounded listing or an error; no code execution, caller roots or new permissions.",
        ),
        (
            "resource_read",
            "Read a bounded UTF-8 regular file using a host resource handle from context_read execution_environment. Use '.' only for an exact file grant. No symlinks, Git administration, caller roots or new permissions.",
        ),
    ] {
        result.push((name, ToolCapability::ContextRead, tool(name, description,
            json!({"handle":{"type":"string","minLength":1,"maxLength":128},"path":{"type":"string","minLength":1,"maxLength":4096}}), &["handle", "path"])));
    }
    if role.permits_repository_execution() && executable {
        result.push(("git_inspect",ToolCapability::ContextRead,tool("git_inspect","Inspect status, diff or one exact-commit file using the host's restricted Git reader. No arbitrary arguments or mutations.",json!({"inspection":{"oneOf":[{"type":"object","properties":{"kind":{"const":"status"}},"required":["kind"],"additionalProperties":false},{"type":"object","properties":{"kind":{"const":"diff"},"staged":{"type":"boolean"}},"required":["kind","staged"],"additionalProperties":false},{"type":"object","properties":{"kind":{"const":"show_file"},"commit":{"type":"string"},"path":{"type":"string"}},"required":["kind","commit","path"],"additionalProperties":false}]}}), &["inspection"])));
        result.push(("command_run",ToolCapability::CommandExecution,tool("command_run","Run a literal argv and repository-relative or absolute cwd through the current admitted command boundary. No shell is added, environment overrides or new permissions. Returns bounded invocation facts and protected output references; exit status does not satisfy a validation obligation.",json!({"argv":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":128},"cwd":{"type":"string","minLength":1}}), &["argv","cwd"])));
        result.push((
            "validation_propose",
            ToolCapability::ValidationProposal,
            validation::proposal_tool(),
        ));
        result.push((
            "validation_run",
            ToolCapability::ValidationResult,
            validation::run_tool(),
        ));
    }
    if writable && role.permits_source_writes() {
        result.push(("source_write",ToolCapability::SourceWrite,tool("source_write","Replace one regular UTF-8 source file within the child's exact write grants after matching its previous digest; null means the file must not exist. Does not authorize commits or scope expansion.",json!({"path":{"type":"string"},"expected_digest":{"type":["string","null"]},"content":{"type":"string"},"executable":{"type":"boolean"}}), &["path","expected_digest","content","executable"])));
    }
    if role == AgentRole::Coordination {
        result.push(("correction_propose",ToolCapability::CorrectionProposal,tool("correction_propose","Propose correction routing to existing authorized children with the exact source comment revision. Host classification, publication, deduplication and steering are separate; never marks a correction consumed.",json!({"source_comment":{"type":"string","format":"uuid"},"source_revision":{"type":"string"},"children":{"type":"array","items":{"type":"string","format":"uuid"},"minItems":1},"message":{"type":"string"}}), &["source_comment","source_revision","children","message"])));
    }
    if matches!(role, AgentRole::Review | AgentRole::IndependentReview) {
        result.push(("review_feedback",ToolCapability::ReviewFeedback,tool("review_feedback","Post bounded review feedback only to the host-selected exact-head PR; without a PR, retain a pending no-change review proposal. Feedback is not an authenticated review attestation or human approval.",json!({"body":{"type":"string","minLength":1}}), &["body"])));
    }
    result
        .into_iter()
        .map(|(name, capability, definition)| {
            (
                ExposedTool {
                    name: name.into(),
                    capability,
                },
                definition,
            )
        })
        .collect()
}
impl WorkerContract {
    pub fn new(input: ContractInput) -> Result<Self> {
        let executes = input
            .assembled
            .manifest
            .permissions
            .repository_code_execution
            && input.repository.is_some();
        verify_command_domain(&input.isolation, executes)?;
        Self::new_inner(input)
    }
    pub(crate) fn new_configured(
        input: ContractInput,
        workspace: &kegari_config::Workspace,
    ) -> Result<Self> {
        if workspace.id != input.authority.workspace() {
            return Err(Error::new(
                "worker_binding",
                "managed workspace identity differs",
            ));
        }
        Self::new(input)
    }
    fn new_inner(input: ContractInput) -> Result<Self> {
        let ContractInput {
            assembled,
            authority,
            isolation,
            workflow,
            context,
            review,
            account_id,
            repository,
            read_resources,
        } = input;
        isolation.revalidate()?;
        let manifest = &assembled.manifest;
        let role = manifest.role;
        let expected_role = match role {
            AgentRole::Implementation | AgentRole::Testing | AgentRole::Documentation => {
                ExecutionRole::Child
            }
            AgentRole::Coordination => ExecutionRole::Coordination,
            AgentRole::Review => ExecutionRole::Review,
            AgentRole::IndependentReview => ExecutionRole::IndependentReview,
            AgentRole::Validation => ExecutionRole::Validation,
        };
        let concrete = operation_tool_definitions(
            role,
            !isolation.plan().source_writes.is_empty(),
            manifest.permissions.repository_code_execution,
        );
        let exposed: Vec<_> = concrete.iter().map(|(e, _)| e).collect();
        if account_id.trim().is_empty()
            || manifest.runtime_build != crate::BUILD_ID
            || manifest.runtime_version != env!("CARGO_PKG_VERSION")
            || canonical_digest(1, manifest)? != assembled.manifest_digest
            || bytes_digest(assembled.prompt.as_bytes()) != manifest.prompt_digest
            || canonical_digest(1, &context)? != manifest.context_digest
            || manifest.workflow_digest != workflow.digest
            || manifest.workflow_path != workflow.path
            || manifest.workflow_embedded != workflow.embedded
            || authority.workflow() != workflow.digest
            || manifest.workspace_id != authority.workspace()
            || manifest.operation_id.is_nil()
            || manifest.cwd != isolation.plan().cwd
            || expected_role != isolation.plan().role
            || canonical_digest(1, &exposed)? != canonical_digest(1, &manifest.tools)?
            || manifest.permissions.source_write_paths
                != isolation
                    .plan()
                    .source_writes
                    .iter()
                    .map(|g| g.path.clone())
                    .collect::<Vec<_>>()
            || manifest.permissions.scratch_roots != vec![isolation.plan().scratch.clone()]
            || manifest.permissions.resource_write_paths
                != isolation
                    .plan()
                    .resource_writes
                    .iter()
                    .map(|grant| grant.path.clone())
                    .collect::<Vec<_>>()
            || manifest.permissions.read_roots
                != isolation
                    .plan()
                    .read_roots
                    .iter()
                    .map(|g| g.path.clone())
                    .collect::<Vec<_>>()
            || (manifest.permissions.repository_code_execution
                && !role.permits_repository_execution())
            || (!manifest.permissions.repository_code_execution
                && isolation.plan().managed_execution.is_some())
            || manifest.permissions.enforcement_profile != isolation.digest()
            || serde_json::to_vec(&context)
                .map_err(|_| Error::invalid("context encoding"))?
                .len()
                > 1_048_576
        {
            return Err(Error::new(
                "worker_binding",
                "production instruction, workflow, role, tool or isolation contract differs",
            ));
        }
        match &authority {
            WorkerAuthority::Authorized(g) => {
                g.work().validate()?;
                if manifest.work_generation != g.work().digest()?
                    || manifest.correction_revision != g.work().generation.correction
                    || &manifest.profile != g.profile()
                {
                    return Err(Error::new(
                        "worker_binding",
                        "work/correction generation differs from current authority",
                    ));
                }
                if role == AgentRole::IndependentReview {
                    return Err(Error::new(
                        "worker_binding",
                        "independent review cannot manufacture Linear authorization",
                    ));
                }
                if let Some(scope) = g.child_scope() {
                    let responsibility = match scope.responsibility {
                        crate::authority::Responsibility::Implementation => {
                            AgentRole::Implementation
                        }
                        crate::authority::Responsibility::Testing => AgentRole::Testing,
                        crate::authority::Responsibility::Documentation => AgentRole::Documentation,
                    };
                    if role.permits_source_writes() && role != responsibility {
                        return Err(Error::new(
                            "scope_violation",
                            "worker role differs from authorized child responsibility",
                        ));
                    }
                    let expected: Vec<_> = scope
                        .write_paths
                        .iter()
                        .map(|path| crate::isolation::PathGrant {
                            path: isolation.plan().repository.join(path.trim_end_matches('/')),
                            directory: path.ends_with('/'),
                        })
                        .collect();
                    if role.permits_source_writes() && isolation.plan().source_writes != expected {
                        return Err(Error::new(
                            "scope_violation",
                            "worker source grants differ from authorized child paths",
                        ));
                    }
                    if !scope.write_paths.is_empty()
                        && repository.as_ref().map(|r| r.id.as_str())
                            != scope.write_repository.as_deref()
                    {
                        return Err(Error::new(
                            "scope_violation",
                            "worker repository differs from authorized child write repository",
                        ));
                    }
                    for grant in &isolation.plan().read_roots {
                        if grant.path.starts_with(&isolation.plan().repository)
                            || grant.path.starts_with(&isolation.plan().scratch)
                        {
                            continue;
                        }
                        let allowed = match read_resources.get(&grant.path) {
                            Some(ReadResource::Repository { id }) => {
                                scope.read_repositories.contains(id)
                            }
                            Some(ReadResource::TestResource { name, .. }) => {
                                scope.test_resources.contains(name)
                            }
                            None => false,
                        };
                        if !allowed {
                            return Err(Error::new(
                                "scope_violation",
                                "external worker read grant lacks a configured authorized resource",
                            ));
                        }
                    }
                    for grant in &isolation.plan().resource_writes {
                        if !matches!(read_resources.get(&grant.path), Some(ReadResource::TestResource {name,writable:true,disposable:true}) if scope.test_resources.contains(name))
                        {
                            return Err(Error::new(
                                "scope_violation",
                                "test-resource writes require explicit authorized disposable writable resources",
                            ));
                        }
                    }
                } else if role.permits_source_writes() {
                    return Err(Error::new(
                        "scope_violation",
                        "parent authority never permits child source implementation",
                    ));
                }
            }
            WorkerAuthority::Independent(g) => {
                if role != AgentRole::IndependentReview
                    || &manifest.profile != g.profile()
                    || manifest.work_generation != g.review_identity()
                    || !isolation.plan().source_writes.is_empty()
                    || !isolation.plan().resource_writes.is_empty()
                    || isolation.plan().managed_execution.is_some()
                {
                    return Err(Error::new(
                        "worker_binding",
                        "independent PR review must remain read/feedback-only",
                    ));
                }
                let r = review.as_ref().ok_or_else(|| {
                    Error::new(
                        "worker_binding",
                        "independent review requires exact PR target",
                    )
                })?;
                if r.repository.id != g.repository_id()
                    || r.pull_request != g.pull_request().number
                    || r.head != g.pull_request().head_sha
                    || r.base != g.pull_request().base_sha
                {
                    return Err(Error::new(
                        "worker_binding",
                        "review target differs from selected independent job",
                    ));
                }
            }
        }
        if let Some(r) = &review {
            crate::git::validate_oid(&r.head)?;
            crate::git::validate_oid(&r.base)?;
            if !matches!(role, AgentRole::Review | AgentRole::IndependentReview)
                || r.pull_request == 0
                || manifest.repository_revision.as_deref() != Some(&r.head)
            {
                return Err(Error::new(
                    "worker_binding",
                    "PR feedback requires exact admitted review revision",
                ));
            }
        }
        let tools =
            BoundToolContracts::bind(&assembled, concrete.into_iter().map(|(_, d)| d).collect())?;
        let scope_digest = canonical_digest(
            1,
            &json!({"instructions":assembled.manifest_digest,"authority":authority.digest()?,"isolation":isolation.digest(),"tool_contract":tools.digest(),"review":review,"account":account_id,"repository":repository,"read_resources":read_resources}),
        )?;
        Ok(Self {
            assembled,
            authority,
            isolation,
            workflow,
            context,
            review,
            tools,
            scope_digest,
            account_id,
            repository,
            read_resources,
        })
    }
    pub(crate) fn workflow(&self) -> &Workflow {
        &self.workflow
    }
    pub(crate) fn isolation(&self) -> &ValidatedIsolation {
        &self.isolation
    }
    pub fn account_id(&self) -> &str {
        &self.account_id
    }
    pub fn scope_digest(&self) -> &str {
        &self.scope_digest
    }
    pub fn work_key(&self) -> Result<String> {
        self.authority.work_key()
    }
    pub fn instructions(&self) -> &AssembledInstructions {
        &self.assembled
    }
    pub fn tools(&self) -> &BoundToolContracts {
        &self.tools
    }
    pub fn authority(&self) -> &WorkerAuthority {
        &self.authority
    }
    pub(crate) fn context(&self) -> &Value {
        &self.context
    }
}

fn verify_command_domain(isolation: &ValidatedIsolation, required: bool) -> Result<()> {
    match (&isolation.plan().managed_execution, required) {
        (None, false) => Ok(()),
        (Some(actual), true) => {
            // Revalidate the exact automatic domain against this retained plan,
            // not merely the self-consistency of a supplied serialized record.
            isolation.revalidate()?;
            actual.revalidate()
        }
        _ => Err(Error::new(
            "worker_binding",
            "command domain is missing for execution or present for a nonexecuting contract",
        )),
    }
}

pub struct WorkerExecution {
    pub git: Arc<Git>,
    pub git_executable: PathBuf,
    pub boundary: Arc<dyn CommandBoundary>,
    pub ownership: Arc<OwnedDirectory>,
    pub binding: ExecutionBinding,
    pub limits: ExecutionLimits,
    /// Fixed by host setup, not tool arguments.
    pub check_outputs: BTreeMap<String, Vec<PathBuf>>,
}
impl WorkerExecution {
    fn executor<'a>(&'a self, isolation: &'a ValidatedIsolation) -> LocalExecutor<'a> {
        LocalExecutor {
            isolation,
            git: &self.git,
            boundary: self.boundary.as_ref(),
            limits: self.limits.clone(),
            ownership: &self.ownership,
        }
    }
}
struct Journal {
    dir: PrivateDir,
    _lock: ExclusiveLock,
    next: Mutex<u64>,
    // Accessed only while `next` is locked; reservations never roll back after I/O.
    bytes: std::sync::atomic::AtomicUsize,
    failed: AtomicBool,
}
impl Journal {
    fn verify_scope(dir: &PrivateDir, contract: &WorkerContract) -> Result<()> {
        let root = dir.path();
        for grant in std::iter::once(&contract.isolation.plan().repository)
            .chain(std::iter::once(&contract.isolation.plan().scratch))
            .chain(contract.isolation.plan().read_roots.iter().map(|g| &g.path))
        {
            if root.starts_with(grant) || grant.starts_with(root) {
                return Err(Error::new(
                    "scope_violation",
                    "worker journal and retained evidence must be outside all worker grants",
                ));
            }
        }
        if !contract
            .isolation
            .plan()
            .forbidden_roots
            .iter()
            .any(|f| root.starts_with(f))
        {
            return Err(Error::new(
                "scope_violation",
                "worker journal requires an explicit forbidden root",
            ));
        }
        Ok(())
    }
    fn create(dir: PrivateDir, contract: &WorkerContract) -> Result<Self> {
        Self::verify_scope(&dir, contract)?;
        let lock = dir.lock("worker.lock")?;
        if dir.records()?.iter().any(|name| name != "worker.lock") {
            return Err(Error::new(
                "reconciliation_required",
                "existing worker journal requires explicit recovery; a new broker cannot replay it",
            ));
        }
        dir.write("binding.json",&json!({"schema_version":1,"scope_digest":contract.scope_digest,"instruction_digest":contract.assembled.manifest_digest,"tool_contract_digest":contract.tools.digest()}))?;
        Ok(Self {
            dir,
            _lock: lock,
            next: Mutex::new(0),
            bytes: std::sync::atomic::AtomicUsize::new(0),
            failed: AtomicBool::new(false),
        })
    }
    fn append(&self, kind: &str, value: &impl Serialize) -> Result<String> {
        self.append_reference(kind, value)
            .map(|reference| reference.leaf)
    }
    fn append_reference(
        &self,
        kind: &str,
        value: &impl Serialize,
    ) -> Result<BrokerJournalReference> {
        self.append_reference_with_budget(kind, value, None)
    }
    fn append_observation(
        &self,
        kind: &str,
        value: &impl Serialize,
        budget: &CaptureBudget,
    ) -> Result<BrokerJournalReference> {
        self.append_reference_with_budget(kind, value, Some(budget))
    }
    fn append_reference_with_budget(
        &self,
        kind: &str,
        value: &impl Serialize,
        budget: Option<&CaptureBudget>,
    ) -> Result<BrokerJournalReference> {
        let mut seq = self
            .next
            .lock()
            .map_err(|_| Error::new("journal_failed", "worker journal serialization failed"))?;
        if self.failed.load(Ordering::SeqCst) {
            return Err(Error::new(
                "journal_failed",
                "prior worker journal write is unresolved",
            ));
        }
        if *seq >= 65_536 {
            return Err(Error::new(
                "journal_limit",
                "worker operation exhausted its bounded journal",
            ));
        }
        let leaf = format!("event-{:08}.json", *seq);
        let written = json!({"kind":kind,"record":value,"observed_at":Utc::now()});
        let reference = BrokerJournalReference {
            leaf: leaf.clone(),
            kind: kind.into(),
            digest: canonical_digest(1, &written)?,
            bytes: serde_json::to_vec(&written)
                .map_err(|_| Error::invalid("journal value encoding"))?
                .len() as u64,
        };
        if let Some(budget) = budget {
            budget.reserve(reference.bytes as usize)?;
        }
        self.reserve_bytes(reference.bytes as usize)?;
        if let Err(error) = self.dir.write(&leaf, &written) {
            self.failed.store(true, Ordering::SeqCst);
            return Err(error);
        }
        *seq += 1;
        Ok(reference)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    id: Uuid,
    thread: String,
    turn: String,
    claim: Value,
    artifact_refs: Vec<String>,
    #[serde(default)]
    processed_correction: Option<ProcessedCorrection>,
}
#[derive(Clone)]
struct CallReceipt {
    operation: Uuid,
    digest: String,
    result: Option<std::result::Result<Value, Error>>,
}

pub struct WorkerServices {
    pub control: Arc<ControlHost>,
    pub providers: ProviderBroker,
    pub host: Arc<dyn WorkerHost>,
    pub sink: Arc<dyn OutcomeSink>,
    pub execution: Option<Arc<WorkerExecution>>,
    pub journal: PrivateDir,
    pub executions: Arc<ExecutionRegistry>,
}

pub struct ScopedWorker {
    contract: Arc<WorkerContract>,
    permit: Permit,
    control: Arc<ControlHost>,
    providers: ProviderBroker,
    host: Arc<dyn WorkerHost>,
    sink: Arc<dyn OutcomeSink>,
    execution: Option<Arc<WorkerExecution>>,
    journal: Arc<Journal>,
    executions: Arc<ExecutionRegistry>,
    session: Option<SessionState>,
    // Enabled only by the declared evaluator, never ordinary worker creation.
    protocol_capture: Option<crate::codex::ProtocolCapture>,
    capture_budget: Option<Arc<CaptureBudget>>,
    canary_output: Option<Arc<crate::evaluation::CanaryOutputEnrollment>>,
    broker_capture: Option<BrokerCapture>,
    applied_transition: Option<Arc<crate::corrections::VerifiedCorrectionTransition>>,
    correction_turn: Option<(String, String)>,
    pending: Option<String>,
    calls: BTreeMap<String, CallReceipt>,
    proposals: Vec<Proposal>,
    published: BTreeMap<Uuid, String>,
    checks: Vec<VerifiedCheckExecution>,
    failed_checks: Vec<crate::execution::FailedCheckExecution>,
    check_records: Vec<Value>,
    command_records: Vec<Value>,
    validation: validation::DiscoveryState,
    resources: resources::ResourceState,
    // Per-owner deterministic race seam; never present in production builds.
    #[cfg(test)]
    inspection_observed: Option<Arc<dyn Fn() + Send + Sync>>,
    retained: Arc<Mutex<Vec<RetainedArtifact>>>,
    effects: Vec<String>,
    evidence: Vec<String>,
}
impl ScopedWorker {
    pub fn new(
        contract: Arc<WorkerContract>,
        permit: Permit,
        services: WorkerServices,
    ) -> Result<Self> {
        let WorkerServices {
            control,
            providers,
            host,
            sink,
            execution,
            journal,
            executions,
        } = services;
        if permit.operation_id != contract.assembled.manifest.operation_id
            || permit.scope_digest != contract.scope_digest
            || permit.work_key != contract.work_key()?
        {
            return Err(Error::new(
                "scope_violation",
                "permit does not bind the complete worker contract",
            ));
        }
        if let WorkerAuthority::Authorized(g) = &contract.authority {
            g.bind_permit(&permit, &contract.scope_digest)?;
        }
        if let Some(e) = &execution {
            if contract.authority.work() != Some(&e.binding.work)
                || contract.repository.as_ref() != Some(&e.binding.repository)
                || e.binding.operation_id != permit.operation_id
                || e.binding.runtime_id != permit.lease.runtime_id
                || e.binding.isolation_digest != contract.isolation.digest()
                || Some(e.binding.revision.commit.as_str())
                    != contract.assembled.manifest.repository_revision.as_deref()
            {
                return Err(Error::new(
                    "worker_binding",
                    "execution resources do not identify this exact admitted operation",
                ));
            }
            e.binding.validate(&contract.isolation)?;
        } else if contract
            .assembled
            .manifest
            .permissions
            .repository_code_execution
        {
            return Err(Error::new(
                "worker_binding",
                "executable role requires the actual owned LocalExecutor resources",
            ));
        }
        control.validate_permit(&permit, || {
            host.verify_owner(&permit.lease)?;
            contract
                .authority
                .compare(&host.current_authority(&contract.authority)?)
        })?;
        let resources = resources::ResourceState::new(&contract)?;
        let journal = Arc::new(Journal::create(journal, &contract)?);
        Ok(Self {
            contract,
            permit,
            control,
            providers,
            host,
            sink,
            execution,
            journal,
            executions,
            session: None,
            protocol_capture: None,
            capture_budget: None,
            canary_output: None,
            broker_capture: None,
            applied_transition: None,
            correction_turn: None,
            pending: None,
            calls: BTreeMap::new(),
            proposals: vec![],
            published: BTreeMap::new(),
            checks: vec![],
            failed_checks: vec![],
            check_records: vec![],
            command_records: vec![],
            validation: validation::DiscoveryState::default(),
            resources,
            #[cfg(test)]
            inspection_observed: None,
            retained: Arc::new(Mutex::new(Vec::new())),
            effects: vec![],
            evidence: vec![],
        })
    }
    fn verify_session_contract(&self, state: &SessionState) -> Result<()> {
        let m = &self.contract.assembled.manifest;
        if state.binding.operation_id != self.permit.operation_id.to_string()
            || state.binding.account_id != self.contract.account_id
            || state.binding.instruction_digest != self.contract.assembled.manifest_digest
            || state.binding.isolation_digest != self.contract.isolation.digest()
            || state.binding.work_identity != self.permit.work_key
            || state.binding.generation != m.work_generation
            || state.binding.cwd != m.cwd
            || state.binding.profile.model != m.profile.model
            || state.binding.profile.reasoning != m.profile.reasoning
            || state.binding.profile.speed
                != match m.profile.speed {
                    kegari_config::Speed::Standard => "standard",
                    kegari_config::Speed::Fast => "fast",
                }
        {
            return Err(Error::new(
                "worker_binding",
                "provider session differs from immutable worker admission",
            ));
        }
        if let Some(prior) = &self.session
            && prior.thread_id.is_some()
            && prior.thread_id != state.thread_id
        {
            return Err(Error::new(
                "worker_binding",
                "provider thread identity changed within an operation",
            ));
        }
        Ok(())
    }
    /// Obtain a launch capability from this exact admitted production broker.
    /// Live evaluation uses the same native launch, process journal and RPC gate.
    pub fn provider_launch(
        &self,
        processes: Arc<crate::process_records::ProcessJournal>,
        executable: &std::path::Path,
        auth_home: &std::path::Path,
        permissions: &crate::codex::PermissionConfiguration,
        timeout: std::time::Duration,
    ) -> Result<crate::worker_run::WorkerProviderLaunch> {
        crate::worker_run::WorkerProviderLaunch::new(
            crate::worker_run::ProviderLaunchInput {
                control: self.control.clone(),
                permit: self.permit.clone(),
                processes,
                executable: kegari_platform::BinaryIdentity::inspect(executable, false)?,
                auth_home: auth_home.to_owned(),
                permissions: permissions.clone(),
                isolation: self.contract.isolation().clone(),
                timeout,
            },
            &self.contract,
            self.host.clone(),
            self.executions.clone(),
        )
    }
    pub(crate) fn correction_wake(&self) -> Result<Option<Arc<tokio::sync::Notify>>> {
        self.host.correction_wake(self.permit.operation_id)
    }
    pub(crate) fn independent_pull_request(&self) -> Option<&kegari_providers::PullRequest> {
        match self.contract.authority() {
            WorkerAuthority::Independent(gate) => Some(gate.pull_request()),
            WorkerAuthority::Authorized(_) => None,
        }
    }
    pub(crate) fn verify_canary_owner(
        &self,
        owner: &crate::evaluation::CanaryMaterialization,
    ) -> Result<Arc<CaptureBudget>> {
        let identity =
            self.host
                .observe_independent_boundary(&self.control, &self.permit, &self.contract)?;
        owner.verify_registered(&self.permit, &self.contract, &identity)
    }
    pub(crate) fn bound_context(&self) -> &Value {
        &self.contract.context
    }
    pub fn correction_requirement(&self) -> ProcessedCorrection {
        self.applied_transition.as_ref().map_or_else(
            || ProcessedCorrection {
                watermark: match &self.contract.authority {
                    WorkerAuthority::Authorized(gate) => gate.correction_watermark(),
                    WorkerAuthority::Independent(_) => 0,
                },
                input_digest: self.contract.assembled.manifest.context_digest.clone(),
                transition_digest: None,
            },
            |transition| ProcessedCorrection {
                watermark: transition.watermark(),
                input_digest: transition.input_digest(),
                transition_digest: Some(transition.digest().into()),
            },
        )
    }
    pub(crate) fn correction_waiting(&self) -> Result<bool> {
        self.control
            .validate_permit(&self.permit, || self.host.verify_owner(&self.permit.lease))?;
        self.host.correction_waiting(self.permit.operation_id)
    }
    pub(crate) fn pending_correction(
        &self,
    ) -> Result<Option<Arc<crate::corrections::VerifiedCorrectionTransition>>> {
        if self.correction_waiting()? {
            return Ok(None);
        }
        let current = self.host.correction_transition(self.permit.operation_id)?;
        if current.is_none() {
            return Ok(None);
        }
        self.control.validate_permit(&self.permit, || {
            self.host.verify_owner(&self.permit.lease)?;
            effective_authority(
                self.host.as_ref(),
                self.permit.operation_id,
                &self.contract.authority,
            )
            .map(|_| ())
        })?;
        Ok(current.filter(|transition| {
            self.applied_transition
                .as_ref()
                .is_none_or(|applied| applied.digest() != transition.digest())
        }))
    }
    /// Called only after the adapter acknowledges this exact turn. An observed
    /// completion racing the acknowledgement leaves the input queued.
    pub(crate) fn deliver_correction(
        &mut self,
        state: &SessionState,
        transition: Arc<crate::corrections::VerifiedCorrectionTransition>,
    ) -> Result<bool> {
        let (Some(thread), Some(turn)) = (&state.thread_id, &state.turn_id) else {
            return Ok(false);
        };
        if state.completed_turns.contains_key(turn) {
            return Ok(false);
        }
        self.control.validate_permit(&self.permit, || {
            self.host.verify_owner(&self.permit.lease)?;
            if self
                .host
                .correction_transition(self.permit.operation_id)?
                .as_ref()
                .map(|p| p.digest())
                != Some(transition.digest())
            {
                return Err(Error::new(
                    "stale_result",
                    "correction delivery was superseded before acknowledgement",
                ));
            }
            effective_authority(
                self.host.as_ref(),
                self.permit.operation_id,
                &self.contract.authority,
            )
            .map(|_| ())
        })?;
        self.journal.append("correction_delivered", &json!({"thread":thread,"turn":turn,"watermark":transition.watermark(),"transition":transition.digest(),"input_digest":transition.input_digest()}))?;
        self.correction_turn = Some((thread.clone(), turn.clone()));
        self.applied_transition = Some(transition);
        Ok(true)
    }
    /// A worker report is consumed only with the actual matching completed turn
    /// and still-current native correction proof, never on steer acknowledgement.
    pub(crate) fn finalize_correction(&mut self, state: &mut SessionState) -> Result<()> {
        if self.correction_waiting()? || self.pending_correction()?.is_some() {
            return Ok(());
        }
        if state.terminal.as_deref() != Some("completed")
            || state.turn_id.is_some()
            || state.intervention.is_some()
            || state.input_required
        {
            return Ok(());
        }
        let Some(turn) = state.last_completed_turn.as_deref() else {
            return Ok(());
        };
        let Some(thread) = state.thread_id.as_deref() else {
            return Ok(());
        };
        let required = self.correction_requirement();
        if state.correction_required != required.watermark
            || state.correction_consumed == required.watermark
        {
            return Ok(());
        }
        if self.applied_transition.is_some()
            && self
                .correction_turn
                .as_ref()
                .is_none_or(|(t, r)| t != thread || r != turn)
        {
            return Ok(());
        }
        if !self.proposals.iter().any(|proposal| {
            proposal.thread == thread
                && proposal.turn == turn
                && proposal.processed_correction.as_ref() == Some(&required)
        }) {
            return Ok(());
        }
        self.control
            .validate_permit(&self.permit, || self.fresh())?;
        self.journal.append(
            "correction_processed",
            &json!({"thread":thread,"turn":turn,"processed":required}),
        )?;
        state.acknowledge_correction(required.watermark)
    }
    /// The evaluator may inspect the pending method but cannot clear it merely
    /// because a failed or cancelled task returned. Its terminal capability is
    /// separately bound to the exact persisted checkpoint.
    pub(crate) fn trial_pending_method(&self) -> Option<String> {
        self.pending.clone()
    }
    pub(crate) fn trial_checkpoint(
        &self,
        expected: &SessionState,
        pending: Option<&str>,
    ) -> Result<SessionState> {
        let actual = self.session.as_ref().ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "worker has no durable final checkpoint",
            )
        })?;
        verify_trial_checkpoint(actual, self.pending.as_deref(), expected, pending)?;
        Ok(actual.clone())
    }
    pub(crate) fn completed_checkpoint(&self) -> Result<SessionState> {
        let state = self.session.as_ref().ok_or_else(|| {
            Error::new(
                "reconciliation_required",
                "worker has no durable final checkpoint",
            )
        })?;
        if self.pending.is_some()
            || state.turn_id.is_some()
            || !state.completion_is_current()
            || state.turns_started == 0
            || state.thread_id.is_none()
        {
            return Err(Error::new(
                "reconciliation_required",
                "worker checkpoint is not a current completed provider turn",
            ));
        }
        Ok(state.clone())
    }
    pub(crate) fn record_after_hook(
        &self,
        observation: std::result::Result<&crate::worker_run::AfterHookObservation, &Error>,
    ) -> Result<()> {
        let record = match observation {
            Ok(observed) => json!({"operation":self.permit.operation_id,"observed":observed}),
            Err(error) => json!({"operation":self.permit.operation_id,"error":error}),
        };
        self.journal.append("after_hook", &record)?;
        Ok(())
    }
    pub fn take_verified_checks(&mut self) -> Vec<VerifiedCheckExecution> {
        std::mem::take(&mut self.checks)
    }
    pub fn take_failed_checks(&mut self) -> Vec<crate::execution::FailedCheckExecution> {
        std::mem::take(&mut self.failed_checks)
    }
    pub fn validate_live_operation(
        &self,
        assembled: &AssembledInstructions,
        isolation: &ValidatedIsolation,
        tools: &BoundToolContracts,
        work_key: &str,
    ) -> Result<()> {
        if self.contract.assembled.manifest_digest != assembled.manifest_digest
            || self.contract.isolation.digest() != isolation.digest()
            || self.permit.work_key != work_key
            || self.contract.tools.digest() != tools.digest()
        {
            return Err(Error::new(
                "evaluation_fidelity",
                "evaluation must use the exact admitted production worker contract",
            ));
        }
        tools.verify(assembled, self.contract.tools.definitions())?;
        if self.execution.as_ref().is_some_and(|e| {
            e.boundary.evidence_class()
                != crate::execution::ExecutionEvidenceClass::ConfinedExecution
        }) {
            return Err(Error::new(
                "unqualified_fixture",
                "deterministic command boundary cannot participate in actual-agent qualification",
            ));
        }
        self.control.validate_permit(&self.permit, || self.fresh())
    }
    fn fresh(&self) -> Result<()> {
        self.host.verify_owner(&self.permit.lease)?;
        effective_authority(
            self.host.as_ref(),
            self.permit.operation_id,
            &self.contract.authority,
        )?;
        if self
            .host
            .correction_transition(self.permit.operation_id)?
            .as_ref()
            .map(|p| p.digest())
            != self.applied_transition.as_ref().map(|p| p.digest())
        {
            return Err(Error::new(
                "correction_pending",
                "current correction input has not reached this provider turn",
            ));
        }
        Ok(())
    }
    fn validate_call(&self, call: &ToolCall) -> Result<()> {
        if serde_json::to_vec(&call.arguments)
            .map_err(|_| Error::invalid("tool arguments encoding"))?
            .len()
            > MAX_TOOL_BYTES
            || call.call_id.is_empty()
            || call.call_id.len() > 256
        {
            return Err(Error::new(
                "tool_limit",
                "tool input exceeds its finite bound",
            ));
        }
        let session = self.session.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "provider session has not been checkpointed",
            )
        })?;
        if self
            .pending
            .as_deref()
            .is_some_and(|method| !matches!(method, "turn/start" | "turn/steer" | "review/start"))
            || session.thread_id.as_deref() != Some(&call.thread_id)
            || session.turn_id.as_deref() != Some(&call.turn_id)
            || session.terminal.is_some()
            || session.intervention.is_some()
            || session.input_required
        {
            return Err(Error::new(
                "stale_result",
                "tool call does not belong to the current usable provider turn",
            ));
        }
        if !self.contract.assembled.manifest.tools.iter().any(|t| {
            t.name == call.name && self.contract.assembled.manifest.role.permits(t.capability)
        }) {
            return Err(Error::new(
                "unsupported_tool",
                "tool is unavailable for this immutable role",
            ));
        }
        let definition = self
            .contract
            .tools
            .definitions()
            .iter()
            .find(|tool| tool["name"] == call.name)
            .ok_or_else(|| Error::new("unsupported_tool", "tool definition unavailable"))?;
        let args = call
            .arguments
            .as_object()
            .ok_or_else(|| Error::new("invalid_input", "tool arguments must be an object"))?;
        let schema = &definition["inputSchema"];
        if schema["required"].as_array().is_none_or(|required| {
            required
                .iter()
                .any(|key| key.as_str().is_none_or(|key| !args.contains_key(key)))
        }) || args
            .keys()
            .any(|key| schema["properties"].get(key).is_none())
        {
            return Err(Error::new(
                "invalid_input",
                "tool arguments omit required fields or contain unadvertised fields",
            ));
        }
        self.control.validate_permit(&self.permit, || self.fresh())
    }
    fn mutation(&self, id: Uuid, kind: &str, payload: Value) -> MutationRequest {
        MutationRequest {
            request_id: id,
            runtime_id: self.permit.lease.runtime_id,
            actor: Actor {
                principal: None,
                identity: format!("worker:{}", self.permit.operation_id),
                roles: BTreeSet::new(),
            },
            kind: kind.into(),
            canonical_targets: vec![self.permit.work_key.clone()],
            expected: BTreeMap::from([
                (
                    "instruction_digest".into(),
                    self.contract.assembled.manifest_digest.clone(),
                ),
                (
                    "tool_contract_digest".into(),
                    self.contract.tools.digest().into(),
                ),
            ]),
            payload,
        }
    }
    fn proposal(
        &mut self,
        call: &ToolCall,
        id: Uuid,
        claim: Value,
        artifact_refs: Vec<String>,
        processed_correction: Option<ProcessedCorrection>,
    ) -> Result<Value> {
        if artifact_refs.len() > 128
            || artifact_refs
                .iter()
                .any(|a| a.len() > 2048 || a.chars().any(char::is_control))
        {
            return Err(Error::new(
                "invalid_input",
                "artifact reference bounds exceeded",
            ));
        }
        let p = Proposal {
            id,
            thread: call.thread_id.clone(),
            turn: call.turn_id.clone(),
            claim,
            artifact_refs,
            processed_correction,
        };
        let reference = self.journal.append("proposal_pending", &p)?;
        match canonical_digest(1, &p.claim) {
            Ok(claim_digest) => self.broker_semantic(
                id,
                BrokerSemantic::PendingProposal {
                    proposal: id,
                    claim_digest,
                },
            ),
            Err(error) => self.broker_gap(&error),
        }
        self.proposals.push(p);
        Ok(json!({"proposal_id":id,"state":"pending_host_verification","receipt":reference}))
    }
    async fn perform(&mut self, call: &ToolCall, id: Uuid) -> Result<Value> {
        match call.name.as_str() {
            "context_read" => {
                let a: ContextRead = parse(&call.arguments)?;
                let result = if a.key == "correction_state" {
                    serde_json::to_value(self.correction_requirement())
                        .map_err(|_| Error::invalid("correction reference encoding"))
                } else if a.key == "command_history" {
                    self.command_history()
                } else if a.key == "validation_evidence" {
                    self.validation_evidence()
                } else if a.key == "read_sources" {
                    self.read_sources()
                } else if a.key == "execution_environment" {
                    self.execution_environment()
                } else {
                    let v = self.contract.context.get(&a.key).ok_or_else(|| {
                        Error::new("missing_context", "named context entry unavailable")
                    })?;
                    bounded_value(v.clone())
                };
                if let Ok(value) = &result {
                    self.retain_context_read(call, id, &a.key, value)?;
                    match canonical_digest(1, value).and_then(|digest| {
                        Ok((
                            digest,
                            serde_json::to_vec(value)
                                .map_err(|_| Error::invalid("context observation encoding"))?
                                .len(),
                        ))
                    }) {
                        Ok((digest, bytes)) => self.broker_semantic(
                            id,
                            BrokerSemantic::Context {
                                key: a.key.clone(),
                                origin: if a.key == "correction_state" {
                                    "owner_correction_state"
                                } else if a.key == "command_history" {
                                    "owner_command_history"
                                } else if a.key == "validation_evidence" {
                                    "owner_validation_evidence"
                                } else if a.key == "read_sources" {
                                    "owner_read_sources"
                                } else if a.key == "execution_environment" {
                                    "owner_execution_environment"
                                } else {
                                    "contract_context"
                                }
                                .into(),
                                digest,
                                bytes: bytes as u64,
                            },
                        ),
                        Err(error) => self.broker_gap(&error),
                    }
                }
                result
            }
            "resource_list" | "resource_read" => self.inspect_resource(call, id),
            "source_read" => {
                let a: SourceRead = parse(&call.arguments)?;
                let root = self.contract.read_source_root(a.repository_id.as_deref())?;
                let mut fact = None;
                let result = read_source_at(
                    &self.contract.isolation,
                    &root,
                    &a.path,
                    a.repository_id.as_deref(),
                    |path, bytes, value| {
                        fact = Some(canonical_digest(1, value).map(|returned_value_digest| {
                            BrokerSemantic::Source {
                                requested_path: a.path.clone(),
                                path: path.into(),
                                digest: bytes_digest(bytes),
                                bytes: bytes.len() as u64,
                                returned_value_digest,
                            }
                        }));
                    },
                );
                if let Some(fact) = fact {
                    match fact {
                        Ok(fact) => {
                            if let Ok(value) = &result {
                                self.retain_source_read(call, id, &root, &fact, value)?;
                            }
                            if root == self.contract.isolation.plan().repository {
                                self.broker_semantic(id, fact);
                            } else {
                                self.broker_gap(&Error::new("additional_source_observation", "evaluation broker projection does not describe cross-repository source reads"));
                            }
                        }
                        Err(error) => self.broker_gap(&error),
                    }
                }
                result
            }
            "outcome_submit" => {
                let a: OutcomeArgs = parse(&call.arguments)?;
                if !a.claim.is_object() {
                    return Err(Error::new(
                        "invalid_input",
                        "outcome claim must be an object",
                    ));
                }
                self.proposal(call, id, a.claim, a.artifact_refs, a.processed_correction)
            }
            "correction_propose" => {
                let a: CorrectionArgs = parse(&call.arguments)?;
                let WorkerAuthority::Authorized(gate) = &self.contract.authority else {
                    return Err(Error::new(
                        "scope_violation",
                        "correction proposals require parent authority",
                    ));
                };
                if a.source_comment.is_nil()
                    || a.children.is_empty()
                    || a.children.len() > 128
                    || a.children
                        .iter()
                        .any(|c| !gate.required_children().contains(c))
                    || a.source_revision.is_empty()
                    || a.message.trim().is_empty()
                {
                    return Err(Error::new(
                        "scope_violation",
                        "correction proposal must name existing authorized children and exact source revision",
                    ));
                }
                self.proposal(
                    call,
                    id,
                    json!({"kind":"correction_proposal","proposal":a}),
                    vec![],
                    None,
                )
            }
            "review_feedback" => {
                let a: ReviewArgs = parse(&call.arguments)?;
                if a.body.trim().is_empty() {
                    return Err(Error::new("invalid_input", "review feedback is empty"));
                }
                let Some(target) = self.contract.review.clone() else {
                    return self.proposal(
                        call,
                        id,
                        json!({"kind":"no_change_review","body":a.body}),
                        vec![],
                        None,
                    );
                };
                let host = self.host.clone();
                let authority = effective_authority(
                    self.host.as_ref(),
                    self.permit.operation_id,
                    &self.contract.authority,
                )?;
                let lease = self.permit.lease.clone();
                let request = self.mutation(id, "review_feedback", json!({}));
                let command = ProviderMutation::GithubComment {
                    repository: target.repository.clone(),
                    pr: target.pull_request,
                    expected_head: target.head.clone(),
                    body: a.body.clone(),
                };
                if self.broker_capture.is_some() || self.canary_output.is_some() {
                    let identity = self
                        .providers
                        .independent_feedback_request(request.clone(), &command)
                        .and_then(|request| {
                            crate::control::broker_request_digest(&request, &self.permit)
                        });
                    if let Some(output) = &self.canary_output {
                        output.feedback(id, &target, &a.body, identity.clone());
                    }
                    if let Some(capture) = &mut self.broker_capture {
                        capture.feedback(&self.journal, id, target, &a.body, identity);
                    }
                }
                let response = self
                    .providers
                    .mutate(
                        self.control.clone(),
                        request,
                        Some(self.permit.clone()),
                        None,
                        command,
                        move || {
                            host.verify_owner(&lease)?;
                            authority.compare(&host.current_authority(&authority)?)
                        },
                    )
                    .await?;
                self.effects.push("review_feedback".into());
                Ok(
                    json!({"operation_id":response.operation_id,"state":"feedback_recorded","remote_ids":response.remote_ids,"attestation":false}),
                )
            }
            "source_write" => {
                let a: SourceArgs = parse(&call.arguments)?;
                let relative = Path::new(&a.path);
                if a.path.is_empty()
                    || relative.is_absolute()
                    || relative
                        .components()
                        .any(|c| !matches!(c, Component::Normal(_)))
                    || a.path.split('/').any(|p| p.eq_ignore_ascii_case(".git"))
                {
                    return Err(Error::new(
                        "scope_violation",
                        "source write path cannot escape the admitted repository",
                    ));
                }
                let target = crate::isolation::canonical_target(
                    &self.contract.isolation.plan().repository.join(relative),
                )?;
                if !self
                    .contract
                    .isolation
                    .plan()
                    .source_writes
                    .iter()
                    .any(|g| target == g.path || g.directory && target.starts_with(&g.path))
                {
                    return Err(Error::new(
                        "scope_violation",
                        "source write lies outside authorized child grants",
                    ));
                }
                let execution = self
                    .execution
                    .clone()
                    .ok_or_else(|| Error::new("unsupported_tool", "owned execution unavailable"))?;
                let contract = self.contract.clone();
                let expected_authority = effective_authority(
                    self.host.as_ref(),
                    self.permit.operation_id,
                    &self.contract.authority,
                )?;
                let journal = self.journal.clone();
                let control = self.control.clone();
                let host = self.host.clone();
                let permit = self.permit.clone();
                let request = self.mutation(id, "worker_source_write", call.arguments.clone());
                let receipt = tokio::task::spawn_blocking(move || {
                    control.mutate(
                        request,
                        Some(&permit),
                        None,
                        || {
                            host.verify_owner(&permit.lease)?;
                            expected_authority.compare(&effective_authority(
                                host.as_ref(),
                                permit.operation_id,
                                &contract.authority,
                            )?)
                        },
                        |_| {
                            let record = execution.executor(&contract.isolation).write_source(
                                &execution.binding,
                                SourceWrite {
                                    relative_path: &a.path,
                                    expected_digest: a.expected_digest.as_deref(),
                                    bytes: a.content.as_bytes(),
                                    executable: a.executable,
                                },
                                |record| journal.append("source_write", record).map(|_| ()),
                            )?;
                            Ok((
                                BTreeMap::new(),
                                serde_json::to_value(record)
                                    .map_err(|_| Error::invalid("source receipt encoding"))?,
                            ))
                        },
                    )
                })
                .await
                .map_err(|_| {
                    Error::new("operation_uncertain", "source write dispatcher interrupted")
                })??;
                self.effects.push("source_write".into());
                Ok(receipt.observed_result.unwrap_or(Value::Null))
            }
            "git_inspect" => {
                let a: GitArgs = parse(&call.arguments)?;
                let execution = self.execution.clone().ok_or_else(|| {
                    Error::new("unsupported_tool", "owned Git reader unavailable")
                })?;
                let contract = self.contract.clone();
                let handle = tokio::runtime::Handle::current();
                let result = tokio::task::spawn_blocking(move || {
                    handle.block_on(execution.executor(&contract.isolation).inspect_git(
                        &execution.binding,
                        &execution.git_executable,
                        a.inspection,
                    ))
                })
                .await
                .map_err(|_| {
                    Error::new("operation_uncertain", "owned Git inspection interrupted")
                })??;
                bounded_value(
                    serde_json::to_value(result)
                        .map_err(|_| Error::invalid("Git receipt encoding"))?,
                )
            }
            "command_run" => self.run_command(call, id).await,
            "validation_propose" => self.propose_validation(call, id).await,
            "validation_run" => self.run_validation(call, id).await,
            _ => Err(Error::new(
                "unsupported_tool",
                "no implementation for this tool",
            )),
        }
    }
    async fn run_validation(&mut self, call: &ToolCall, id: Uuid) -> Result<Value> {
        let selected = self.select_validation(parse(&call.arguments)?)?;
        let attempt = selected.attempt(id);
        let execution = self
            .execution
            .clone()
            .ok_or_else(|| Error::new("unsupported_tool", "owned execution unavailable"))?;
        let contract = self.contract.clone();
        let expected_authority = effective_authority(
            self.host.as_ref(),
            self.permit.operation_id,
            &self.contract.authority,
        )?;
        let effective = self
            .applied_transition
            .clone()
            .map(|transition| {
                crate::execution::EffectiveExecutionBinding::new(
                    &execution.binding,
                    &execution.ownership,
                    &contract.isolation,
                    transition,
                )
            })
            .transpose()?;
        let checked_work = effective
            .as_ref()
            .map_or(&execution.binding.work, |binding| &binding.binding().work);
        if expected_authority.work() != Some(checked_work) {
            return Err(Error::new(
                "correction_execution_binding",
                "validation requires the exact acknowledged correction input and current execution binding",
            ));
        }
        let journal = self.journal.clone();
        let control = self.control.clone();
        let host = self.host.clone();
        let permit = self.permit.clone();
        let prior = self
            .retained
            .lock()
            .map_err(|_| {
                Error::new(
                    "artifact_conflict",
                    "retained artifact inventory lock failed",
                )
            })?
            .clone();
        let retained_owner = self.retained.clone();
        let request = self.mutation(id, "worker_validation", call.arguments.clone());
        let handle = tokio::runtime::Handle::current();
        let executions = self.executions.clone();
        let discovered = attempt.is_some();
        let attempt_records = discovered.then(|| self.start_validation_attempt(id));
        let dispatched = Arc::new(AtomicBool::new(false));
        let dispatched_in_task = dispatched.clone();
        let task_result = tokio::task::spawn_blocking(move || {
            let executor = execution.executor(&contract.isolation);
            let completed_entry = Mutex::new(None);
            let persist = |entry: &crate::execution::CheckJournalEntry| {
                if discovered {
                    if let crate::execution::CheckJournalEntry::Finished(record) = entry {
                        attempt_records
                            .as_ref()
                            .expect("discovered owner")
                            .lock()
                            .map_err(|_| {
                                Error::new("journal_failed", "validation observation lock failed")
                            })?
                            .insert(id, (**record).clone());
                    }
                    let reference = journal.append_scoped_entry(
                        id,
                        entry,
                        "validation_check_chunk",
                        "validation_check_entry",
                    )?;
                    if matches!(entry, crate::execution::CheckJournalEntry::Finished(_)) {
                        *completed_entry.lock().map_err(|_| {
                            Error::new("journal_failed", "validation reference lock failed")
                        })? = Some(reference);
                    }
                    Ok(())
                } else {
                    journal.append("check", entry).map(|_| ())
                }
            };
            if let Some(attempt) = &attempt {
                journal.append("validation_attempt", attempt)?;
            }
            let prepared = match &selected {
                validation::SelectedRun::Configured { name, kind } => {
                    let outputs = execution
                        .check_outputs
                        .get(name)
                        .cloned()
                        .unwrap_or_default();
                    let invocation = CheckInvocation {
                        binding: effective
                            .as_ref()
                            .map_or(&execution.binding, |binding| binding.binding()),
                        workflow: &contract.workflow,
                        name,
                        kind: kind.clone(),
                        additional_outputs: &outputs,
                        previous_outputs: &prior,
                    };
                    match &effective {
                        Some(effective) => {
                            executor.prepare_effective_check(invocation, effective, persist)?
                        }
                        None => executor.prepare_check(invocation, persist)?,
                    }
                }
                validation::SelectedRun::Discovered { plan, obligation } => executor
                    .prepare_discovered_check(
                        effective
                            .as_ref()
                            .map_or(&execution.binding, |binding| binding.binding()),
                        plan,
                        obligation,
                        id,
                        &prior,
                        effective.as_ref(),
                        persist,
                    )?,
            };
            let current = || {
                host.verify_owner(&permit.lease)?;
                expected_authority.compare(&effective_authority(
                    host.as_ref(),
                    permit.operation_id,
                    &contract.authority,
                )?)
            };
            enum ValidationEffect {
                Single(Box<crate::control::InFlightEffect>),
                Bundle(
                    Box<crate::control::InFlightCheckBundle>,
                    Arc<crate::execution::CheckBundleOwner>,
                ),
            }
            let (effect, completion) = if prepared.requires_bundle() {
                let prepared = executor.prepare_bundle(prepared, id)?;
                let owner = prepared.owner();
                let (mut bundle, ()) = control.begin_check_bundle(
                    request,
                    &permit,
                    prepared.plan().clone(),
                    current,
                    |receipt| {
                        // Known no-spawn refusals precede reservation. The
                        // aggregate owner is attached before any member starts.
                        executor.preflight_bundle(&prepared)?;
                        let reservation = if attempt.is_some() {
                            executions.reserve_command(permit.operation_id, receipt.operation_id)?
                        } else {
                            executions.reserve(permit.operation_id, receipt.operation_id)?
                        };
                        reservation.attach(owner.clone())?;
                        Ok(())
                    },
                )?;
                dispatched_in_task.store(true, Ordering::SeqCst);
                let completed = handle.block_on(executor.complete_bundle(
                    prepared,
                    |prepared, ordinal| {
                        control.advance_check_bundle(
                            &mut bundle,
                            &permit,
                            ordinal,
                            current,
                            |launch| executor.launch_bundle_member(prepared, launch),
                        )
                    },
                    persist,
                ));
                let completion = match completed {
                    Ok(completion) => completion,
                    Err(error) => {
                        // A real closed prefix may settle a known service
                        // failure, but never creates a failed-check repair cap.
                        // Unknown starts/collectors/journals retain the hold.
                        match control
                            .complete_check_bundle_failure(bundle, &permit, &owner, current, &error)
                        {
                            Ok(receipt) => {
                                if let Some(attempt) = &attempt {
                                    journal.append(
                                        "validation_attempt_ack",
                                        &json!({"attempt":attempt,"receipt":receipt,"error":error}),
                                    )?;
                                }
                                executions.remove(permit.operation_id, id)?;
                                return Err(error);
                            }
                            Err(settlement) => {
                                return Err(error.clone().detail(json!({
                                    "original_details": error.details,
                                    "bundle_settlement_error": settlement,
                                })));
                            }
                        }
                    }
                };
                (
                    ValidationEffect::Bundle(Box::new(bundle), owner),
                    completion,
                )
            } else {
                let (effect, running) =
                    control.begin_effect(request, &permit, current, |receipt| {
                        executor.preflight_check(&prepared)?;
                        let reservation = if attempt.is_some() {
                            executions.reserve_command(permit.operation_id, receipt.operation_id)?
                        } else {
                            executions.reserve(permit.operation_id, receipt.operation_id)?
                        };
                        dispatched_in_task.store(true, Ordering::SeqCst);
                        let running = executor.start_check(prepared)?;
                        reservation.attach(running.cancellation())?;
                        Ok(running)
                    })?;
                // No Control mutex is held across output or process waits.
                let completion = handle.block_on(executor.complete_check(running, persist))?;
                (ValidationEffect::Single(Box::new(effect)), completion)
            };
            let retain = |artifact| {
                let mut retained = retained_owner.lock().map_err(|_| {
                    Error::new(
                        "artifact_conflict",
                        "retained artifact inventory lock failed",
                    )
                })?;
                if !retained.contains(&artifact) {
                    retained.push(artifact);
                }
                Ok(())
            };
            let (value, verified, failed) = match completion {
                crate::execution::CheckCompletion::Passed(checked) => {
                    checked.retain_outputs_into(&contract.isolation, &journal.dir, retain)?;
                    let value = serde_json::to_value(checked.record())
                        .map_err(|_| Error::invalid("check receipt encoding"))?;
                    (value, Some(checked), None)
                }
                crate::execution::CheckCompletion::Failed(failed) => {
                    failed.retain_outputs_into(&contract.isolation, &journal.dir, retain)?;
                    (
                        serde_json::to_value(failed.record())
                            .map_err(|_| Error::invalid("check receipt encoding"))?,
                        None,
                        Some(failed),
                    )
                }
            };
            let value = if discovered {
                let reference = completed_entry
                    .into_inner()
                    .map_err(|_| Error::new("journal_failed", "validation reference lock failed"))?
                    .ok_or_else(|| {
                        Error::new(
                            "journal_failed",
                            "validation completion has no protected record reference",
                        )
                    })?;
                validation::result_summary(&value, &reference)?
            } else {
                value
            };
            let receipt = match effect {
                ValidationEffect::Single(effect) => {
                    if attempt.is_none() {
                        executions.remove(permit.operation_id, id)?;
                    }
                    control.complete_effect(*effect, &permit, current, value)?
                }
                ValidationEffect::Bundle(bundle, owner) => {
                    control.complete_check_bundle(*bundle, &permit, &owner, current, value)?
                }
            };
            if let Some(attempt) = &attempt {
                journal.append(
                    "validation_attempt_ack",
                    &json!({"attempt":attempt,"receipt":receipt}),
                )?;
            }
            executions.remove(permit.operation_id, id)?;
            Ok::<_, Error>((receipt, verified, failed))
        })
        .await
        .map_err(|_| Error::new("operation_uncertain", "validation dispatcher interrupted"))
        .and_then(|result| result);
        if discovered {
            self.finish_validation_attempt(
                id,
                dispatched.load(Ordering::SeqCst),
                &task_result.as_ref().map(|_| ()).map_err(Clone::clone),
            );
        }
        let (receipt, verified, failed) = task_result?;
        let record = receipt.observed_result.unwrap_or(Value::Null);
        if discovered {
            let actual = self.validation_attempt_record(id)?;
            self.evidence.push(canonical_digest(1, &actual)?);
            self.check_records.push(
                serde_json::to_value(actual)
                    .map_err(|_| Error::invalid("validation record encoding"))?,
            );
        } else if record.get("successful").is_some_and(Value::is_boolean) {
            self.evidence.push(canonical_digest(1, &record)?);
            self.check_records.push(record.clone());
        }
        if let Some(verified) = verified {
            self.checks.push(verified);
        }
        if let Some(failed) = failed {
            self.failed_checks.push(failed);
        }
        self.effects.push("validation_run".into());
        bounded_value(record)
    }
    fn finish_proposals(&mut self, state: &SessionState, completed_turn: &str) -> Result<()> {
        if !state.completion_is_current()
            || state.terminal.as_deref() != Some("completed")
            || state.intervention.is_some()
            || state.input_required
        {
            return Ok(());
        }
        if self.correction_waiting()? || self.pending_correction()?.is_some() {
            return Ok(());
        }
        self.control
            .validate_permit(&self.permit, || self.fresh())?;
        for proposal in &self.proposals {
            if self.published.contains_key(&proposal.id)
                || state.thread_id.as_deref() != Some(&proposal.thread)
                || proposal.turn != completed_turn
                || self.correction_requirement().watermark > 0
                    && proposal.processed_correction.as_ref()
                        != Some(&self.correction_requirement())
                || state
                    .last_completed_turn
                    .as_deref()
                    .is_some_and(|turn| turn != completed_turn)
            {
                continue;
            }
            let outcome = WorkerOutcome {
                schema_version: 2,
                runtime_id: self.permit.lease.runtime_id,
                operation_id: self.permit.operation_id,
                proposal_id: proposal.id,
                completed_turn: completed_turn.into(),
                work: self
                    .applied_transition
                    .as_ref()
                    .map(|p| p.effective_work().clone())
                    .or_else(|| self.contract.authority.work().cloned()),
                independent_review: matches!(
                    self.contract.authority,
                    WorkerAuthority::Independent(_)
                )
                .then(|| self.contract.review.clone())
                .flatten(),
                session: state.clone(),
                instructions: self.contract.assembled.manifest.clone(),
                instruction_digest: self.contract.assembled.manifest_digest.clone(),
                tool_contract_digest: self.contract.tools.digest().into(),
                fresh_gate_digest: self
                    .applied_transition
                    .as_ref()
                    .map(|p| p.current_authority().digest())
                    .unwrap_or_else(|| self.contract.authority.digest())?,
                processed_correction: proposal.processed_correction.clone(),
                validation_history_digest: Some(self.seal_validation_history().digest()?),
                claim: proposal.claim.clone(),
                artifact_refs: proposal.artifact_refs.clone(),
                checks: self.check_records.clone(),
            };
            self.journal.append("outcome_pending", &outcome)?;
            let reference = self.sink.record(&outcome)?;
            if let Some(capture) = &mut self.broker_capture {
                capture.semantic(
                    &self.journal,
                    proposal.id,
                    BrokerSemantic::PendingClaim {
                        proposal: proposal.id,
                        reference_digest: bytes_digest(reference.as_bytes()),
                    },
                );
                capture.gap(&Error::new(
                    "pending_claim_content_not_independently_read",
                    "sink return is a pending claim reference, not acceptance",
                ));
            }
            self.journal.append(
                "outcome_recorded",
                &json!({"proposal_id":proposal.id,"reference":reference}),
            )?;
            self.evidence.push(reference.clone());
            self.published.insert(proposal.id, reference);
        }
        Ok(())
    }
}
impl ToolBroker for ScopedWorker {
    fn begin_evaluation_protocol_observation(
        &mut self,
        declaration: &crate::evaluation::DeclaredProtocolTrial,
    ) {
        if let Some(capture) = &mut self.protocol_capture {
            capture.disable(crate::codex::ProtocolCaptureIssue::ConflictingDuplicate);
            self.broker_gap(&Error::new(
                "observation_conflict",
                "duplicate declaration cannot reset observation",
            ));
            return;
        }
        let mut capture = crate::codex::ProtocolCapture::new(
            crate::codex::ProtocolOperationBinding {
                operation: self.permit.operation_id,
                runtime: self.permit.lease.runtime_id,
                workspace: self.contract.authority.workspace(),
            },
            self.contract.scope_digest.clone(),
            self.contract.assembled.manifest_digest.clone(),
            declaration.trial_digest().map(str::to_owned),
            declaration.suite_digest().into(),
            declaration.fixture_digest().into(),
        );
        if declaration.operation() != self.permit.operation_id
            || declaration.instruction_digest() != self.contract.assembled.manifest_digest
        {
            capture.disable(crate::codex::ProtocolCaptureIssue::BindingChanged);
        } else if !matches!(self.contract.authority, WorkerAuthority::Independent(_)) {
            capture.disable(crate::codex::ProtocolCaptureIssue::ScopeNotDeclared);
        } else if declaration.trial_digest().is_none() {
            capture.disable(crate::codex::ProtocolCaptureIssue::DeclarationUnavailable);
        }
        if declaration.operation() == self.permit.operation_id
            && declaration.instruction_digest() == self.contract.assembled.manifest_digest
            && matches!(self.contract.authority, WorkerAuthority::Independent(_))
            && declaration.trial_digest().is_some()
        {
            let budget = declaration
                .capture_budget()
                .unwrap_or_else(|| Arc::new(CaptureBudget::default()));
            self.broker_capture =
                BrokerCapture::new(&self.permit, &self.contract, declaration, budget.clone());
            self.capture_budget = Some(budget);
            self.canary_output = declaration
                .canary_output()
                .filter(|output| output.matches_owner(&self.permit, &self.contract));
        }
        self.protocol_capture = Some(capture);
    }
    fn observe_protocol(&mut self, event: crate::codex::ProtocolObservation<'_>) {
        if self.protocol_capture.is_none() {
            return;
        }
        let state = match &event {
            crate::codex::ProtocolObservation::UnboundNotification { .. } => None,
            crate::codex::ProtocolObservation::Issue { state, .. }
            | crate::codex::ProtocolObservation::MatchedResponse { state, .. }
            | crate::codex::ProtocolObservation::ServerRequest { state, .. }
            | crate::codex::ProtocolObservation::ToolResult { state, .. } => *state,
            crate::codex::ProtocolObservation::ThreadEstablished { state }
            | crate::codex::ProtocolObservation::TransportEnd { state, .. }
            | crate::codex::ProtocolObservation::AdapterInventory { state, .. }
            | crate::codex::ProtocolObservation::TerminalTurn { state, .. }
            | crate::codex::ProtocolObservation::CompletedItem { state, .. }
            | crate::codex::ProtocolObservation::AgentMessageDelta { state, .. } => Some(*state),
        };
        let bound = state.is_none_or(|s| self.verify_session_contract(s).is_ok());
        // Raw attempt/text capture precedes the legacy semantic bound filter;
        // foreign/unbound observations retain explicit attribution only.
        if let Some(output) = &self.canary_output {
            output.observe_protocol(&event, bound);
        }
        let capture = self
            .protocol_capture
            .as_mut()
            .expect("checked opted-in capture");
        if !bound {
            capture.issue(crate::codex::ProtocolCaptureIssue::BindingChanged);
            return;
        }
        capture.observe(event, &mut |value| match &self.capture_budget {
            Some(budget) => self
                .journal
                .append_observation("protocol_observation", value, budget)
                .map(|reference| reference.leaf),
            None => self.journal.append("protocol_observation", value),
        });
    }
    fn observe_session(&mut self, state: &SessionState) -> Result<()> {
        let pending = self.pending.clone();
        self.checkpoint(state, pending.as_deref())
    }
    fn call<'a>(
        &'a mut self,
        call: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            if let Err(error) = self.validate_call(call) {
                self.broker_gap(&error);
                return Err(error);
            }
            let key = format!("{}:{}:{}", call.thread_id, call.turn_id, call.call_id);
            let digest =
                canonical_digest(1, &json!({"tool":call.name,"arguments":call.arguments}))?;
            if let Some(prior) = self.calls.get(&key) {
                if prior.digest != digest {
                    let error = Error::new(
                        "request_conflict",
                        "provider reused a tool call identity with changed arguments",
                    );
                    self.broker_gap(&error);
                    return Err(error);
                }
                return prior.result.clone().unwrap_or_else(|| {
                    Err(Error::new(
                        "operation_uncertain",
                        "previous tool transmission must be reconciled",
                    ))
                });
            }
            if self.calls.len() >= MAX_CALLS {
                return Err(Error::new("tool_limit", "worker exceeded bounded calls"));
            }
            let id = Uuid::new_v4();
            let started = match self.journal.append_reference(
                "tool_started",
                &json!({"call":key,"request_id":id,"tool":call.name,"arguments_digest":digest}),
            ) {
                Ok(reference) => reference,
                Err(error) => {
                    self.broker_gap(&error);
                    return Err(error);
                }
            };
            self.calls.insert(
                key.clone(),
                CallReceipt {
                    operation: id,
                    digest: digest.clone(),
                    result: None,
                },
            );
            if let Some(capture) = &mut self.broker_capture {
                capture.started(id, call, &digest, started);
            }
            let result = self.perform(call, id).await;
            if let Some(capture) = &mut self.broker_capture {
                capture.returned(id, &result);
            }
            let finished = match self.journal.append_reference(
                "tool_finished",
                &json!({"call":key,"request_id":id,"result":result}),
            ) {
                Ok(reference) => reference,
                Err(error) => {
                    self.broker_gap(&error);
                    return Err(error);
                }
            };
            self.settle_validation_call(id, &finished, &result)?;
            let actual = self.calls.get_mut(&key).expect("inserted call");
            if let Some(capture) = &mut self.broker_capture {
                capture.finished(actual.operation, finished);
            }
            actual.result = Some(result.clone());
            result
        })
    }
}
impl TrialHost for ScopedWorker {
    fn observe(&self) -> Result<ObservedFacts> {
        let mut checks = BTreeMap::new();
        let mut artifacts = BTreeMap::new();
        self.contract.isolation.revalidate()?;
        let current = crate::evaluation::snapshot(&self.contract.isolation.plan().repository)?;
        let candidate = canonical_digest(1, &current)?;
        let revision = match &self.execution {
            Some(e) => {
                e.git
                    .revision(&self.contract.isolation.plan().repository, "HEAD")?
                    .commit
            }
            None => self
                .contract
                .assembled
                .manifest
                .repository_revision
                .clone()
                .unwrap_or_default(),
        };
        for name in self
            .contract
            .workflow
            .front_matter
            .checks
            .local
            .keys()
            .chain(self.contract.workflow.front_matter.checks.integrated.keys())
        {
            checks.insert(
                name.clone(),
                ObservedCheck {
                    status: CheckStatus::NotRun,
                    candidate_digest: candidate.clone(),
                    receipt_digest: None,
                },
            );
        }
        for value in &self.check_records {
            let record: crate::execution::CheckExecutionRecord =
                serde_json::from_value(value.clone())
                    .map_err(|_| Error::new("record_corrupt", "host check record malformed"))?;
            let digest = canonical_digest(1, &record)?;
            let mut observed = crate::evaluation::observed_check(&record, &current, &revision)?;
            let effective_work = self
                .applied_transition
                .as_ref()
                .map(|transition| transition.effective_work())
                .or_else(|| self.contract.authority.work());
            if effective_work != Some(&record.binding.work) {
                // Equal bytes do not make pre-correction evidence current.
                observed.status = CheckStatus::Unknown;
            }
            let checked_candidate = observed.candidate_digest.clone();
            checks.insert(record.check_name.clone(), observed);
            for artifact in record.artifacts {
                if !self
                    .retained
                    .lock()
                    .map_err(|_| {
                        Error::new(
                            "artifact_conflict",
                            "retained artifact inventory lock failed",
                        )
                    })?
                    .iter()
                    .any(|retained| retained.verify_reference(&artifact).is_ok())
                {
                    return Err(Error::new(
                        "artifact_conflict",
                        "observed check artifact lacks its exact intact retained bytes",
                    ));
                }
                artifacts.insert(
                    artifact.path.to_string_lossy().into_owned(),
                    ObservedArtifact {
                        digest: artifact.digest,
                        candidate_digest: checked_candidate.clone(),
                        visual: false,
                        receipt_digest: digest.clone(),
                    },
                );
            }
        }
        let review_role = matches!(
            self.contract.assembled.manifest.role,
            AgentRole::Review | AgentRole::IndependentReview
        );
        // Review observations come only from a proposal actually recorded for
        // the current completed turn. Submission alone is not completion, and
        // a prior/unfinished turn cannot substitute an old clean review.
        let review_claim = if review_role {
            self.session
                .as_ref()
                .filter(|state| {
                    self.pending.is_none()
                        && state.turn_id.is_none()
                        && state.completion_is_current()
                        && state.terminal.as_deref() == Some("completed")
                        && state.intervention.is_none()
                        && !state.input_required
                })
                .and_then(|state| {
                    self.proposals.iter().rev().find(|proposal| {
                        self.published.contains_key(&proposal.id)
                            && state.thread_id.as_deref() == Some(&proposal.thread)
                            && state.last_completed_turn.as_deref() == Some(&proposal.turn)
                            && (self.correction_requirement().watermark == 0
                                || proposal.processed_correction.as_ref()
                                    == Some(&self.correction_requirement()))
                    })
                })
                .map(|proposal| {
                    let observed = crate::evaluation::ObservedReviewClaim {
                        schema_version: 1,
                        operation: self.permit.operation_id,
                        proposal: proposal.id,
                        thread: proposal.thread.clone(),
                        completed_turn: proposal.turn.clone(),
                        source_revision: self
                            .contract
                            .assembled
                            .manifest
                            .repository_revision
                            .clone()
                            .unwrap_or_default(),
                        repository: self.contract.repository.as_ref().map(|repository| {
                            crate::evaluation::ReviewSourceRepository {
                                hostname: repository.hostname.clone(),
                                id: repository.id.clone(),
                                slug: self
                                    .contract
                                    .review
                                    .as_ref()
                                    .filter(|review| {
                                        review.repository.id.to_string() == repository.id
                                    })
                                    .map(|review| {
                                        format!(
                                            "{}/{}",
                                            review.repository.owner, review.repository.name
                                        )
                                    }),
                            }
                        }),
                        instruction_digest: self.contract.assembled.manifest_digest.clone(),
                        tool_contract_digest: self.contract.tools.digest().into(),
                        outcome_reference: self
                            .published
                            .get(&proposal.id)
                            .expect("recorded proposal selected")
                            .clone(),
                        claim: serde_json::from_value(proposal.claim.clone()).map_err(|_| {
                            Error::new(
                                "invalid_outcome",
                                "recorded review does not match its declared schema",
                            )
                        })?,
                    };
                    observed.validate()?;
                    Ok::<_, Error>(observed)
                })
                .transpose()?
        } else {
            None
        };
        // Legacy non-review summaries remain explicit grader inputs. Production
        // review claims are not wrapped in an invented AgentSummary contract.
        let summary = if review_role {
            None
        } else {
            self.proposals
                .iter()
                .rev()
                .find_map(|p| p.claim.get("summary").cloned())
                .map(serde_json::from_value::<AgentSummary>)
                .transpose()
                .map_err(|_| {
                    Error::new(
                        "invalid_outcome",
                        "worker summary does not match its declared schema",
                    )
                })?
        };
        let independent_boundary = self.independent_boundary_observation(&revision, &candidate);
        Ok(ObservedFacts {
            revision,
            candidate_digest: candidate,
            checks,
            artifacts,
            effects: self.effects.clone(),
            state: self.host.observed_state(self.permit.operation_id)?,
            summary,
            review_claim,
            independent_boundary,
            independent_broker: self.broker_observation(),
            canary_diagnostic: None,
            canary_output: self.canary_output.as_ref().map(|output| output.snapshot()),
            declared_protocol: self.protocol_capture.as_ref().map(|capture| {
                capture.finish(self.session.as_ref(), self.pending.as_deref(), &|leaf| {
                    self.journal.dir.read_bounded(leaf, 256 * 1024)
                })
            }),
            evidence_references: self.evidence.clone(),
        })
    }
    fn reconcile_prior(&mut self, prior: Option<&Value>) -> Result<()> {
        self.control
            .validate_permit(&self.permit, || self.fresh())?;
        self.host.reconcile_prior(self.permit.operation_id, prior)?;
        self.journal.append(
            "prior_reconciled",
            &json!({"prior_digest":prior.map(|v|canonical_digest(1,v)).transpose()?}),
        )?;
        Ok(())
    }
    fn rate_limits(&mut self, observed: &Value) -> Result<()> {
        if serde_json::to_vec(observed)
            .map_err(|_| Error::invalid("rate limits encoding"))?
            .len()
            > MAX_TOOL_BYTES
        {
            return Err(Error::new("protocol_error", "rate limits exceed bound"));
        }
        self.host.rate_limits(self.permit.operation_id, observed)?;
        self.journal.append("rate_limits", observed)?;
        Ok(())
    }
    fn correction_event(&mut self, watermark: u64, input: &str) -> Result<()> {
        if watermark == 0 || input.trim().is_empty() || input.len() > MAX_TOOL_BYTES {
            return Err(Error::new(
                "invalid_input",
                "correction event is not bounded and actionable",
            ));
        }
        self.control.validate_permit(&self.permit, || {
            self.host.verify_owner(&self.permit.lease)?;
            effective_authority(
                self.host.as_ref(),
                self.permit.operation_id,
                &self.contract.authority,
            )
            .map(|_| ())
        })?;
        self.journal.append(
            "correction_pending",
            &json!({"watermark":watermark,"input_digest":bytes_digest(input.as_bytes())}),
        )?;
        let reference = self
            .host
            .correction_event(self.permit.operation_id, watermark, input)?;
        self.journal.append(
            "correction_ingested",
            &json!({"watermark":watermark,"reference":reference}),
        )?;
        self.evidence.push(reference);
        Ok(())
    }
    fn checkpoint(&mut self, state: &SessionState, pending_method: Option<&str>) -> Result<()> {
        let admission = self.control.operation(self.permit.operation_id)?;
        match (
            crate::control::RunResumeReference::from_receipt(&admission)?,
            &state.resume_origin,
        ) {
            (Some(reference), Some(origin))
                if reference.predecessor() == origin.predecessor
                    && reference.recovery_digest() == origin.recovery_digest
                    && reference.instruction_compatibility_digest
                        == origin.instruction_compatibility_digest => {}
            (None, None) => {}
            _ => {
                return Err(Error::new(
                    "worker_binding",
                    "resumed session differs from its exact admitted recovery lineage",
                ));
            }
        }
        if let Some(admitted) = crate::control::RunReference::from_receipt(
            &self.control.operation(self.permit.operation_id)?,
        )? && (state.max_turns > admitted.max_turns || state.turns_started > state.max_turns)
        {
            return Err(Error::new(
                "admission_budget",
                "session exceeds its original finite admitted turn bound",
            ));
        }
        self.verify_session_contract(state)?;
        self.journal.append(
            "session",
            &json!({"state":state,"pending_method":pending_method}),
        )?;
        self.host
            .observed_session(self.permit.operation_id, state)?;
        let completed_turn = self
            .session
            .as_ref()
            .and_then(|prior| prior.turn_id.clone())
            .filter(|turn| {
                state.turn_id.is_none()
                    && state
                        .completed_turns
                        .get(turn)
                        .is_some_and(|status| status == "completed")
            });
        self.session = Some(state.clone());
        self.pending = pending_method.map(str::to_owned);
        if pending_method.is_none()
            && let Some(turn) = state.last_completed_turn.clone().or(completed_turn)
        {
            self.finish_proposals(state, &turn)?;
        }
        Ok(())
    }
}
fn verify_trial_checkpoint(
    actual: &SessionState,
    actual_pending: Option<&str>,
    expected: &SessionState,
    expected_pending: Option<&str>,
) -> Result<()> {
    if canonical_digest(1, actual)? != canonical_digest(1, expected)?
        || actual_pending != expected_pending
    {
        return Err(Error::new(
            "worker_binding",
            "trial terminal observation differs from its durable session checkpoint",
        ));
    }
    Ok(())
}

fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone()).map_err(|_| {
        Error::new(
            "invalid_input",
            "tool arguments do not match the exact schema",
        )
    })
}
fn bounded_value(value: Value) -> Result<Value> {
    if serde_json::to_vec(&value)
        .map_err(|_| Error::invalid("tool result encoding"))?
        .len()
        > MAX_TOOL_BYTES
    {
        return Err(Error::new(
            "tool_limit",
            "tool result exceeds bound; no truncated result is returned",
        ));
    }
    Ok(value)
}
fn read_source(
    boundary: &ValidatedIsolation,
    path: &str,
    observe: impl FnMut(&str, &[u8], &Value),
) -> Result<Value> {
    read_source_at(boundary, &boundary.plan().repository, path, None, observe)
}
fn read_source_at(
    boundary: &ValidatedIsolation,
    root: &Path,
    path: &str,
    repository_id: Option<&str>,
    mut observe: impl FnMut(&str, &[u8], &Value),
) -> Result<Value> {
    boundary.revalidate()?;
    let relative = Path::new(path);
    if path.is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || path.split('/').any(|p| p.eq_ignore_ascii_case(".git"))
    {
        return Err(Error::new(
            "scope_violation",
            "source read path must stay inside repository and exclude Git administration",
        ));
    }
    let candidate = root.join(relative);
    let actual = crate::isolation::canonical_target(&candidate)?;
    if actual != candidate
        || !actual.starts_with(root)
        || boundary
            .plan()
            .forbidden_roots
            .iter()
            .any(|f| actual.starts_with(f))
        || !boundary
            .plan()
            .read_roots
            .iter()
            .any(|g| actual == g.path || g.directory && actual.starts_with(&g.path))
    {
        return Err(Error::new(
            "scope_violation",
            "source read is outside exact read grants",
        ));
    }
    let file = open_source(&actual)?;
    let meta = file
        .metadata()
        .map_err(|e| Error::io("inspect admitted source", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(Error::new(
                "scope_violation",
                "source hard links are unavailable",
            ));
        }
    }
    if !meta.is_file() || meta.len() > 48_000 {
        return Err(Error::new(
            "tool_limit",
            "source read requires a bounded regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(48_001)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read admitted source", e))?;
    if bytes.len() > 48_000 || crate::isolation::canonical_target(&candidate)? != candidate {
        return Err(Error::new(
            "scope_violation",
            "source changed path or exceeded bound",
        ));
    }
    let content = std::str::from_utf8(&bytes)
        .map_err(|_| Error::new("invalid_input", "source is not UTF-8"))?;
    let mut value = json!({"path":path,"digest":bytes_digest(&bytes),"content":content});
    if let Some(repository_id) = repository_id {
        value["repository_id"] = json!(repository_id);
    }
    let admitted = actual
        .strip_prefix(root)
        .map_err(|_| Error::new("scope_violation", "admitted source left repository"))?;
    let admitted = admitted
        .to_str()
        .ok_or_else(|| Error::new("scope_violation", "admitted source path is not UTF-8"))?;
    // Record a successful read result only after the exact public Value passes
    // its unchanged encoded-size bound (escaped UTF-8 may expand substantially).
    let value = bounded_value(value)?;
    observe(admitted, &bytes, &value);
    Ok(value)
}
#[cfg(unix)]
fn open_source(path: &Path) -> Result<std::fs::File> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let mut held = std::fs::File::open("/").map_err(|e| Error::io("open source root", e))?;
    let parts: Vec<_> = path
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name),
            _ => None,
        })
        .collect();
    for (index, name) in parts.iter().enumerate() {
        let leaf = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| Error::invalid("source component contains NUL"))?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            // A replaced FIFO/device must not block before the regular-file
            // check. Ordinary regular-file and directory reads are unchanged.
            | libc::O_NONBLOCK
            | if index + 1 == parts.len() {
                0
            } else {
                libc::O_DIRECTORY
            };
        // SAFETY: the retained directory descriptor and single NUL-terminated
        // component are live. Each ancestry component is opened without links.
        let fd = unsafe { libc::openat(held.as_raw_fd(), leaf.as_ptr(), flags) };
        if fd < 0 {
            return Err(Error::io(
                "open source without following links",
                std::io::Error::last_os_error(),
            ));
        }
        // SAFETY: successful openat returned a new owned descriptor.
        held = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    Ok(held)
}
#[cfg(not(unix))]
fn open_source(_: &Path) -> Result<std::fs::File> {
    Err(Error::new(
        "isolation_unavailable",
        "source reads require the supported descriptor boundary",
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextRead {
    key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRead {
    path: String,
    #[serde(default)]
    repository_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeArgs {
    claim: Value,
    artifact_refs: Vec<String>,
    #[serde(default)]
    processed_correction: Option<ProcessedCorrection>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CorrectionArgs {
    source_comment: Uuid,
    source_revision: String,
    children: Vec<Uuid>,
    message: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewArgs {
    body: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceArgs {
    path: String,
    expected_digest: Option<String>,
    content: String,
    executable: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GitArgs {
    inspection: GitInspection,
}

#[derive(Clone, Debug, Serialize)]
pub struct RecoveredWorkerSession {
    pub state: SessionState,
    pub pending_method: Option<String>,
    pub session_records: usize,
    #[serde(skip)]
    resume_origin: Option<ResumeOrigin>,
}
#[derive(Clone, Debug)]
struct ResumeOrigin {
    admission_digest: String,
    reference: crate::control::RunReference,
    journal_digest: String,
    state_digest: String,
    after_hook: Option<crate::worker_run::AfterHookObservation>,
    after_hook_error: bool,
    after_hook_is_latest: bool,
    executable_role: bool,
}
impl RecoveredWorkerSession {
    pub fn resume_candidate(&self) -> Result<crate::codex::RetainedResumeSession> {
        let origin = self.resume_origin.as_ref().ok_or_else(|| {
            Error::new(
                "resume_incompatible",
                "session has no protected recovery origin",
            )
        })?;
        let current = canonical_digest(
            1,
            &json!({"state":self.state,"pending":self.pending_method,"records":self.session_records}),
        )?;
        let applicable = matches!(
            (&origin.after_hook, origin.executable_role),
            (
                Some(crate::worker_run::AfterHookObservation::NotApplicable),
                false
            ) | (
                Some(crate::worker_run::AfterHookObservation::Skipped {
                    reason: crate::worker_run::AfterHookSkip::HookNotConfigured
                }),
                true
            )
        );
        if self.pending_method.is_some()
            || current != origin.state_digest
            || !applicable
            || origin.after_hook_error
            || !origin.after_hook_is_latest
        {
            return Err(Error::new(
                "resume_incompatible",
                "recovery lacks an unchanged final session and actual inapplicable/absent after-hook observation",
            ));
        }
        crate::codex::RetainedResumeSession::from_journal(
            self.state.clone(),
            origin.admission_digest.clone(),
            origin.reference.clone(),
            origin.journal_digest.clone(),
        )
    }
}
/// Read complete protected session history before recovery/retry. The supplied
/// admission is an actual control receipt; model JSON is never an input here.
/// Earlier interventions/input requirements survive a later ordinary checkpoint.
pub fn recover_session(
    records: &PrivateDir,
    admission: &crate::control::Receipt,
) -> Result<Option<RecoveredWorkerSession>> {
    let run = crate::control::RunReference::from_receipt(admission)?.ok_or_else(|| {
        Error::new(
            "record_corrupt",
            "session recovery requires an actual run admission",
        )
    })?;
    let contract: Value = records.read("contract.json")?;
    let manifest: InstructionManifest = serde_json::from_value(contract["instructions"].clone())
        .map_err(|_| {
            Error::new(
                "record_corrupt",
                "retained worker instruction manifest is invalid",
            )
        })?;
    let manifest_digest = canonical_digest(1, &manifest)?;
    let scope = admission
        .expected
        .get("scope")
        .ok_or_else(|| Error::new("record_corrupt", "run admission lacks scope binding"))?;
    let tool_contract = contract["tool_contract"]
        .as_str()
        .ok_or_else(|| Error::new("record_corrupt", "retained tool contract digest is missing"))?;
    crate::evidence::validate_digest(tool_contract)?;
    if admission.kind != "admission"
        || admission.canonical_targets != vec![run.phase.work.clone()]
        || contract["scope"].as_str() != Some(scope)
        || manifest.operation_id != admission.operation_id
        || manifest.workspace_id != run.workspace
        || manifest_digest != run.instruction_digest
        || canonical_digest(1, &manifest.profile)? != run.profile_digest
        || canonical_digest(1, &contract["isolation"])? != manifest.permissions.enforcement_profile
    {
        return Err(Error::new(
            "record_corrupt",
            "retained worker contract differs from its durable admission",
        ));
    }
    let path = records.path().join("journal");
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io("inspect worker session journal", error)),
        Ok(_) => {}
    }
    let journal = PrivateDir::open(&path)?;
    let binding: Value = journal.read("binding.json")?;
    if binding["schema_version"] != 1
        || binding["scope_digest"].as_str() != Some(scope)
        || binding["instruction_digest"].as_str() != Some(&manifest_digest)
        || binding["tool_contract_digest"].as_str() != Some(tool_contract)
    {
        return Err(Error::new(
            "record_corrupt",
            "worker journal belongs to another admitted contract",
        ));
    }
    let files = journal.records_bounded(65_537).map_err(|error| {
        if error.code == "record_limit" {
            Error::new(
                "journal_limit",
                "worker recovery exceeds the declared record bound",
            )
        } else {
            error
        }
    })?;
    let mut total = 0usize;
    let mut latest: Option<RecoveredWorkerSession> = None;
    let mut intervention = None;
    let mut input_required = false;
    let mut sessions = 0usize;
    let admission_digest = canonical_digest(1, admission)?;
    let mut journal_digest = canonical_digest(
        1,
        &json!({"admission":admission_digest,"contract":contract,"binding":binding}),
    )?;
    let mut after_hook = None;
    let mut after_hook_error = false;
    let mut after_hook_is_latest = false;
    let mut command_history =
        command_history::CommandHistory::new(admission, &manifest, tool_contract);
    for (index, file) in files
        .into_iter()
        .filter(|file| file != "binding.json")
        .enumerate()
    {
        if file != format!("event-{index:08}.json") {
            return Err(Error::new(
                "record_corrupt",
                "worker journal has a missing, reordered or unexpected event",
            ));
        }
        let event: Value = journal.read(&file)?;
        journal_digest = canonical_digest(1, &json!({"previous":journal_digest,"event":event}))?;
        total = total
            .checked_add(
                serde_json::to_vec(&event)
                    .map_err(|_| Error::invalid("journal encoding failed"))?
                    .len(),
            )
            .ok_or_else(|| Error::new("journal_limit", "worker history size overflow"))?;
        if total > 64 * 1024 * 1024 {
            return Err(Error::new(
                "journal_limit",
                "complete worker history exceeds the finite recovery byte bound",
            ));
        }
        validation::refuse_cold_history(&event)?;
        if command_history.observe(&file, &event)? {
            after_hook_is_latest = false;
            continue;
        }
        if event["kind"] == "after_hook" {
            if after_hook.is_some()
                || after_hook_error
                || event["record"]["operation"].as_str()
                    != Some(&admission.operation_id.to_string())
            {
                return Err(Error::new(
                    "record_corrupt",
                    "after-hook observation has duplicate or unrelated operation identity",
                ));
            }
            after_hook_error = !event["record"]["error"].is_null();
            if !after_hook_error {
                after_hook = Some(
                    serde_json::from_value(event["record"]["observed"].clone()).map_err(|_| {
                        Error::new("record_corrupt", "invalid typed after-hook observation")
                    })?,
                );
            }
            after_hook_is_latest = latest.is_some();
            continue;
        }
        if event["kind"] != "session" {
            continue;
        }
        after_hook_is_latest = false;
        let state = SessionState::restore(
            &serde_json::to_vec(&event["record"]["state"])
                .map_err(|_| Error::invalid("session encoding failed"))?,
        )?;
        match (
            crate::control::RunResumeReference::from_receipt(admission)?,
            &state.resume_origin,
        ) {
            (Some(reference), Some(origin))
                if reference.predecessor() == origin.predecessor
                    && reference.recovery_digest() == origin.recovery_digest
                    && reference.instruction_compatibility_digest
                        == origin.instruction_compatibility_digest => {}
            (None, None) => {}
            _ => {
                return Err(Error::new(
                    "record_corrupt",
                    "session journal changed its admitted resume lineage",
                ));
            }
        }
        let expected_profile = crate::codex::CodexProfile {
            model: manifest.profile.model.clone(),
            reasoning: manifest.profile.reasoning.clone(),
            speed: match manifest.profile.speed {
                kegari_config::Speed::Standard => "standard",
                kegari_config::Speed::Fast => "fast",
            }
            .into(),
        };
        if state.binding.operation_id != admission.operation_id.to_string()
            || state.binding.account_id != run.account
            || state.binding.work_identity != run.phase.work
            || state.binding.generation != manifest.work_generation
            || state.binding.instruction_digest != manifest_digest
            || state.binding.isolation_digest != manifest.permissions.enforcement_profile
            || state.binding.cwd != manifest.cwd
            || state.binding.profile != expected_profile
            || state.max_turns > run.max_turns
        {
            return Err(Error::new(
                "record_corrupt",
                "session checkpoint crosses exact account, work, profile or operation binding",
            ));
        }
        if let Some(prior) = &latest
            && prior.state.thread_id.is_some()
            && prior.state.thread_id != state.thread_id
        {
            return Err(Error::new(
                "record_corrupt",
                "worker journal switches provider thread identity",
            ));
        }
        if let Some(prior) = latest
            .as_ref()
            .and_then(|prior| prior.state.submissions.as_ref())
        {
            state
                .submissions
                .as_ref()
                .ok_or_else(|| Error::new("record_corrupt", "submission history disappeared"))?
                .preserves(prior)?;
        }
        if let Some(prior) = latest
            .as_ref()
            .and_then(|prior| prior.state.resume_origin.as_ref())
        {
            state
                .resume_origin
                .as_ref()
                .ok_or_else(|| Error::new("record_corrupt", "session history prefix disappeared"))?
                .preserves(prior)?;
        }
        if let Some(observed) = &state.intervention {
            if observed.account_id != run.account {
                return Err(Error::new(
                    "record_corrupt",
                    "provider intervention account differs from admitted session",
                ));
            }
            if intervention.is_none() {
                intervention = Some(observed.clone());
            }
        }
        input_required |= state.input_required;
        let pending_method = match &event["record"]["pending_method"] {
            Value::Null => None,
            Value::String(method) if !method.is_empty() && method.len() <= 128 => {
                Some(method.clone())
            }
            _ => {
                return Err(Error::new(
                    "record_corrupt",
                    "session pending method is invalid",
                ));
            }
        };
        sessions += 1;
        latest = Some(RecoveredWorkerSession {
            state,
            pending_method,
            session_records: sessions,
            resume_origin: None,
        });
    }
    command_history.ensure_settled()?;
    if let Some(latest) = &mut latest {
        if intervention.is_some() {
            latest.state.intervention = intervention;
        }
        latest.state.input_required |= input_required;
        latest.resume_origin = Some(ResumeOrigin {
            admission_digest,
            reference: run,
            journal_digest,
            state_digest: canonical_digest(
                1,
                &json!({"state":latest.state,"pending":latest.pending_method,"records":latest.session_records}),
            )?,
            after_hook,
            after_hook_error,
            after_hook_is_latest,
            executable_role: manifest.permissions.repository_code_execution,
        });
    }
    Ok(latest)
}

/// Exclusively owns the original protected journal during metadata readback.
/// It cannot invoke tools or revive an old worker permit.
pub(crate) struct RecoveryJournal {
    journal: Journal,
    candidate: crate::codex::RetainedResumeSession,
    host: Arc<dyn WorkerHost>,
    observed: bool,
}
impl RecoveryJournal {
    pub(crate) fn open(
        records: &PrivateDir,
        admission: &crate::control::Receipt,
        candidate: &crate::codex::RetainedResumeSession,
        contract: &WorkerContract,
        host: Arc<dyn WorkerHost>,
    ) -> Result<Self> {
        let dir = PrivateDir::open(&records.path().join("journal"))?;
        Journal::verify_scope(&dir, contract)?;
        let lock = dir.lock("worker.lock")?;
        let actual = recover_session(records, admission)?
            .ok_or_else(|| Error::new("resume_incompatible", "original session journal is absent"))?
            .resume_candidate()?;
        if actual.digest() != candidate.digest()
            || contract.instructions().manifest_digest
                != candidate.state().binding.instruction_digest
            || contract.account_id() != candidate.state().binding.account_id
            || contract.scope_digest()
                != admission
                    .expected
                    .get("scope")
                    .map(String::as_str)
                    .unwrap_or_default()
        {
            return Err(Error::new(
                "resume_incompatible",
                "recovery journal differs from its retained contract and candidate",
            ));
        }
        let files = dir.records_bounded(65_537)?;
        let mut bytes = 0usize;
        let mut count = 0u64;
        for name in files.into_iter().filter(|name| name != "binding.json") {
            let value: Value = dir.read(&name)?;
            bytes = bytes
                .checked_add(
                    serde_json::to_vec(&value)
                        .map_err(|_| Error::invalid("journal encoding"))?
                        .len(),
                )
                .ok_or_else(|| Error::new("journal_limit", "journal size overflow"))?;
            count += 1;
        }
        if bytes > command::MAX_HISTORY_BYTES {
            return Err(Error::new(
                "journal_limit",
                "worker history exceeds its recovery bound",
            ));
        }
        Ok(Self {
            journal: Journal {
                dir,
                _lock: lock,
                next: Mutex::new(count),
                bytes: std::sync::atomic::AtomicUsize::new(bytes),
                failed: AtomicBool::new(false),
            },
            candidate: actual,
            host,
            observed: false,
        })
    }
    pub(crate) fn observed(&self) -> bool {
        self.observed
    }
}
impl ToolBroker for RecoveryJournal {
    fn observe_session(&mut self, state: &SessionState) -> Result<()> {
        if state.binding != self.candidate.state().binding
            || state.max_turns != self.candidate.state().max_turns
            || state.turns_started != self.candidate.state().turns_started
        {
            return Err(Error::new(
                "resume_incompatible",
                "metadata observation changed original session identity or budget",
            ));
        }
        self.journal
            .append("session", &json!({"state":state,"pending_method":null}))?;
        self.observed = true;
        let operation = Uuid::parse_str(&state.binding.operation_id)
            .map_err(|_| Error::new("record_corrupt", "original session operation is invalid"))?;
        self.host.observed_session(operation, state)
    }
    fn call<'a>(
        &'a mut self,
        _: &'a ToolCall,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async {
            Err(Error::new(
                "unsupported_tool",
                "metadata recovery cannot invoke worker tools",
            ))
        })
    }
}

#[cfg(test)]
#[path = "worker_execution_registry_tests.rs"]
mod execution_registry_tests;

#[cfg(test)]
#[path = "worker_managed_binding_tests.rs"]
mod managed_binding_tests;

#[cfg(test)]
mod trial_checkpoint_tests {
    use super::*;
    #[test]
    fn terminal_trial_requires_exact_checkpoint_and_preserves_pending_request() {
        // Session DTO only: this helper cannot mint a scoped worker or a joined
        // completion. It tests the exact comparison used by the real broker.
        let state = SessionState::new(
            crate::codex::SessionBinding {
                schema_version: 1,
                account_id: "account".into(),
                work_identity: "work".into(),
                operation_id: Uuid::new_v4().to_string(),
                generation: kegari_digest::bytes_digest(b"generation"),
                instruction_digest: kegari_digest::bytes_digest(b"instructions"),
                isolation_digest: kegari_digest::bytes_digest(b"isolation"),
                profile: crate::codex::CodexProfile {
                    model: "model".into(),
                    reasoning: "high".into(),
                    speed: "standard".into(),
                },
                cwd: std::path::PathBuf::from("/tmp/trial-checkpoint"),
            },
            2,
        )
        .unwrap();
        verify_trial_checkpoint(&state, Some("turn/start"), &state, Some("turn/start")).unwrap();
        assert!(verify_trial_checkpoint(&state, Some("turn/start"), &state, None).is_err());
        assert!(verify_trial_checkpoint(&state, None, &state, Some("turn/start")).is_err());
        let mut changed = state.clone();
        changed.turns_started = 1;
        assert!(verify_trial_checkpoint(&state, None, &changed, None).is_err());
        changed = state.clone();
        changed.binding.operation_id = Uuid::new_v4().to_string();
        assert!(verify_trial_checkpoint(&state, None, &changed, None).is_err());
        changed = state.clone();
        changed.binding.instruction_digest = kegari_digest::bytes_digest(b"changed contract");
        assert!(verify_trial_checkpoint(&state, None, &changed, None).is_err());
    }
}
