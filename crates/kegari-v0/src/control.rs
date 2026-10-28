//! Serialized admission and durable operator intent. The host owns this object;
//! the scheduler and workers only receive revocable operation permits.
use chrono::{DateTime, Utc};
use kegari_control_api::identity::Actor;
use kegari_digest::canonical_digest;
use kegari_storage::{ExclusiveLock, PrivateDir};
use kegari_types::control::Lease;
use kegari_types::error::{Error, Result};
use kegari_types::run::RunReference;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};
use uuid::Uuid;

#[path = "control_check_bundle.rs"]
mod check_bundle;
pub use check_bundle::CheckBundleLaunch;
pub(crate) use check_bundle::{CheckBundlePlan, CheckBundleStepSpec, InFlightCheckBundle};

#[path = "control_effect_index.rs"]
mod effect_index;
pub(crate) use effect_index::{ParentEffectHistoryBinding, ParentEffectHistoryObservation};

#[path = "control_admission_identity.rs"]
mod admission_identity;
pub(crate) use admission_identity::{
    broker_request_digest, ordinary_preparation_request_digest, ordinary_run_request_digest,
    receipt_request_digest,
};

#[path = "control_parent_reassembly.rs"]
mod parent_reassembly;
pub(crate) use parent_reassembly::{
    ParentGrantReference, ParentReassemblyReference, parent_reassembly_inventory,
    parent_transient_failure, reject_unmarked_parent_phase, require_successful_parent_preparation,
    verify_parent_reassembly_reference,
};

#[path = "control_continuation.rs"]
mod continuation;
pub use continuation::{RunContinuationReference, SourceAdvancedReference};
#[path = "control_decisions.rs"]
mod decisions;
#[path = "control_preparation.rs"]
mod preparation;
pub(crate) use preparation::preparation_operation;
pub use preparation::{PreparationAdmission, PreparationReference};
#[path = "control_resume.rs"]
mod session_resume;
pub use session_resume::RunResumeReference;
#[path = "control_recovery_preparation.rs"]
mod recovery_preparation;
pub use recovery_preparation::SessionRecoveryPreparationReference;
#[path = "control_release.rs"]
mod release_recovery;
pub use release_recovery::ReleaseResumeReference;
pub(crate) use release_recovery::VerifiedReleaseResume;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Intent {
    Running,
    Paused,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentRecord {
    pub runtime_id: Uuid,
    pub desired: Intent,
    pub generation: u64,
    pub last_command_id: Uuid,
    pub changed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Pause,
    Resume,
    Start,
    Stop,
    ForceStop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permit {
    pub operation_id: Uuid,
    pub lease: Lease,
    pub control_generation: u64,
    pub work_key: String,
    pub scope_digest: String,
    pub registered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptPhase {
    Prepared,
    Sent,
    Acknowledged,
    Reconciled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub operation_id: Uuid,
    pub request_id: Uuid,
    pub runtime_id: Uuid,
    pub owner_generation: u64,
    pub actor: String,
    pub kind: String,
    pub canonical_targets: Vec<String>,
    pub payload_digest: String,
    pub expected: BTreeMap<String, String>,
    pub phase: ReceiptPhase,
    pub remote_ids: BTreeMap<String, String>,
    pub observed_result: Option<Value>,
    pub cancellation: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct MutationRequest {
    pub request_id: Uuid,
    pub runtime_id: Uuid,
    pub actor: Actor,
    pub kind: String,
    pub canonical_targets: Vec<String>,
    pub expected: BTreeMap<String, String>,
    pub payload: Value,
}

pub fn run_reference_from_receipt(receipt: &Receipt) -> Result<Option<RunReference>> {
    receipt
        .expected
        .get("run")
        .map(|encoded| {
            if receipt.kind != "admission"
                || receipt.expected.contains_key("review_round_recovery")
                || receipt.expected.contains_key("worker_preparation")
                || receipt
                    .expected
                    .contains_key("session_recovery_preparation")
            {
                return Err(Error::new(
                    "record_corrupt",
                    "run budget is attached to a non-admission receipt",
                ));
            }
            let value: RunReference = serde_json::from_str(encoded)
                .map_err(|_| Error::new("record_corrupt", "run budget reference is invalid"))?;
            validate_run_reference(&value)?;
            if receipt.expected.contains_key("review_round_resume") {
                ReviewRoundResumeReference::from_model_receipt(receipt)?;
            }
            Ok(value)
        })
        .transpose()
}
fn validate_run_reference(reference: &RunReference) -> Result<()> {
    if reference.workspace.is_nil()
        || reference.account.is_empty()
        || reference.account.len() > 512
        || reference.normalized_state.as_ref().is_some_and(|state| {
            state.is_empty() || state.len() > 256 || state.trim().to_lowercase() != *state
        })
        || reference.phase.work.is_empty()
        || reference.phase.work.len() > 512
        || reference.phase.phase.is_empty()
        || reference.phase.phase.len() > 64
        || !reference
            .phase
            .phase
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        || reference.phase.retry_grant.is_some_and(|id| id.is_nil())
        || [
            reference.max_attempts,
            reference.max_turns,
            reference.max_review_rounds,
            reference.attempt,
        ]
        .contains(&0)
        || reference.attempt > reference.max_attempts
    {
        return Err(Error::new(
            "record_corrupt",
            "run identity or finite phase budget is invalid",
        ));
    }
    for digest in [
        &reference.phase.generation,
        &reference.profile_digest,
        &reference.instruction_digest,
    ] {
        crate::evidence::validate_digest(digest)?;
    }
    Ok(())
}
pub struct RunAdmission {
    pub request_id: Uuid,
    pub work_key: String,
    pub scope_digest: String,
    pub reference: RunReference,
}

/// Finite host preparation budget. It never represents a model session or an
/// orchestration lease; the actual lease is separately required at admission.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataReference {
    pub purpose: String,
    pub generation: String,
    pub attempt: u32,
    pub max_attempts: u32,
}
impl MetadataReference {
    pub fn from_receipt(receipt: &Receipt) -> Result<Option<Self>> {
        receipt
            .expected
            .get("metadata")
            .map(|encoded| {
                if receipt.kind != "admission"
                    || receipt.expected.contains_key("run")
                    || receipt.expected.contains_key("review_round_recovery")
                    || receipt.expected.contains_key("review_round_resume")
                    || receipt.expected.contains_key("worker_preparation")
                    || receipt
                        .expected
                        .contains_key("session_recovery_preparation")
                    || receipt.expected.contains_key("session_resume")
                {
                    return Err(Error::new(
                        "record_corrupt",
                        "metadata budget is attached to another operation kind",
                    ));
                }
                let reference: Self = serde_json::from_str(encoded).map_err(|_| {
                    Error::new("record_corrupt", "metadata budget reference is invalid")
                })?;
                reference.validate()?;
                Ok(reference)
            })
            .transpose()
    }
    fn validate(&self) -> Result<()> {
        if !matches!(
            self.purpose.as_str(),
            "parent_authorization" | "no_release_required"
        ) || self.attempt == 0
            || self.attempt > self.max_attempts
        {
            return Err(Error::new(
                "record_corrupt",
                "metadata purpose or finite attempt budget is invalid",
            ));
        }
        crate::evidence::validate_digest(&self.generation)
    }
}
pub struct MetadataAdmission {
    pub request_id: Uuid,
    pub work_key: String,
    pub scope_digest: String,
    pub reference: MetadataReference,
}
enum AdmissionReference {
    Preparation(PreparationReference),
    ParentPreparation(PreparationReference, Box<ParentReassemblyReference>),
    ParentRun(
        RunReference,
        Box<ParentReassemblyReference>,
        ReviewReceiptReference,
    ),
    RoundPreparation(PreparationReference, Box<ReviewRoundReference>),
    SessionRecovery(SessionRecoveryPreparationReference),
    RoundSessionRecovery(
        SessionRecoveryPreparationReference,
        Box<RoundRecoveryProvenance>,
    ),
    Run(RunReference),
    RoundRun(RunReference, Box<ReviewRoundRunProvenance>),
    ContinuedRun(RunReference, RunContinuationReference),
    SourceAdvancedRun(RunReference, SourceAdvancedReference),
    ResumedRun(RunReference, RunResumeReference),
    RoundResumedRun(RunReference, RunResumeReference, Box<RoundResumeProvenance>),
    Metadata(MetadataReference),
    ReleaseResume(ReleaseResumeReference),
}

/// Only created after a durable Sent receipt and a synchronous owned launch.
/// A worker cannot deserialize or synthesize this acknowledgement capability.
pub(crate) enum RemoteReservation {
    Existing(Receipt),
    Prepared(PreparedRemote),
}
/// A non-serializable reservation binds the original actor, authority and payload.
/// Read-only network preflight runs while this receipt remains Prepared.
pub(crate) struct PreparedRemote {
    request: MutationRequest,
    permit: Option<Permit>,
    role: Option<String>,
    digest: String,
    identity: String,
}
impl PreparedRemote {
    pub(crate) fn operation_id(&self) -> Uuid {
        self.request.request_id
    }
}
pub(crate) struct InFlightRemote {
    operation: Uuid,
    digest: String,
    identity: String,
    owner: Option<Uuid>,
}

pub struct InFlightEffect {
    request_id: Uuid,
    permit: Permit,
    payload_digest: String,
    identity: String,
}
impl InFlightEffect {
    pub fn operation_id(&self) -> Uuid {
        self.request_id
    }
}

/// Persisted receipts contain references/digests, never copies of issue descriptions.
pub struct ReceiptStore {
    directory: PrivateDir,
}
impl ReceiptStore {
    pub fn new(directory: PrivateDir) -> Self {
        Self { directory }
    }
    pub fn get(&self, id: Uuid) -> Result<Receipt> {
        self.directory.read(&format!("{id}.json"))
    }
    pub fn all(&self) -> Result<Vec<Receipt>> {
        self.directory
            .records()?
            .into_iter()
            .map(|name| self.directory.read(&name))
            .collect()
    }
    pub fn save(&self, receipt: &Receipt) -> Result<()> {
        self.directory
            .write(&format!("{}.json", receipt.request_id), receipt)
    }
    pub fn prepare(
        &self,
        request: &MutationRequest,
        owner_generation: u64,
    ) -> Result<(Receipt, bool)> {
        if request.actor.identity.trim().is_empty()
            || request.kind.is_empty()
            || request.canonical_targets.is_empty()
        {
            return Err(Error::new(
                "invalid_input",
                "mutation needs verified actor, kind and canonical targets",
            ));
        }
        let digest = admission_identity::payload_digest(request)?;
        match self.get(request.request_id) {
            Ok(receipt) => {
                if receipt.runtime_id != request.runtime_id
                    || receipt.actor != request.actor.identity
                    || receipt.payload_digest != digest
                {
                    return Err(Error::new(
                        "request_conflict",
                        "request ID is already bound to another actor, runtime or payload",
                    ));
                }
                return Ok((receipt, false));
            }
            Err(e) if e.code == "record_missing" => (),
            Err(e) => return Err(e),
        }
        let receipt = Receipt {
            operation_id: request.request_id,
            request_id: request.request_id,
            runtime_id: request.runtime_id,
            owner_generation,
            actor: request.actor.identity.clone(),
            kind: request.kind.clone(),
            canonical_targets: request.canonical_targets.clone(),
            payload_digest: digest,
            expected: request.expected.clone(),
            phase: ReceiptPhase::Prepared,
            remote_ids: BTreeMap::new(),
            observed_result: None,
            cancellation: "not_requested".into(),
            updated_at: Utc::now(),
        };
        self.save(&receipt)?;
        Ok((receipt, true))
    }
    pub fn mark_sent(&self, receipt: &mut Receipt) -> Result<()> {
        if receipt.phase != ReceiptPhase::Prepared {
            return Err(Error::new(
                "operation_uncertain",
                "only a fresh prepared mutation may be transmitted",
            ));
        }
        receipt.phase = ReceiptPhase::Sent;
        receipt.updated_at = Utc::now();
        self.save(receipt)
    }
    pub fn acknowledge(
        &self,
        receipt: &mut Receipt,
        remote_ids: BTreeMap<String, String>,
        observed: Value,
    ) -> Result<()> {
        if receipt.phase != ReceiptPhase::Sent {
            return Err(Error::new(
                "operation_uncertain",
                "mutation was not durably marked sent",
            ));
        }
        receipt.remote_ids = remote_ids;
        receipt.observed_result = Some(observed);
        receipt.phase = ReceiptPhase::Acknowledged;
        receipt.updated_at = Utc::now();
        self.save(receipt)
    }
    pub fn reconcile(&self, id: Uuid, observation: Reconciliation) -> Result<Receipt> {
        let mut receipt = self.get(id)?;
        match observation {
            Reconciliation::UniqueVerified {
                remote_ids,
                result,
                payload_digest,
                actor,
            } => {
                if payload_digest != receipt.payload_digest || actor != receipt.actor {
                    return Err(Error::new(
                        "ownership_conflict",
                        "remote action does not match immutable receipt ownership",
                    ));
                }
                receipt.remote_ids = remote_ids;
                receipt.observed_result = Some(result);
                receipt.phase = ReceiptPhase::Reconciled;
                receipt.updated_at = Utc::now();
                self.save(&receipt)?;
            }
            Reconciliation::Ambiguous => {
                return Err(Error::new(
                    "ownership_conflict",
                    "multiple matching remote actions; preserve and reconcile",
                ));
            }
            Reconciliation::NotVisible | Reconciliation::Incomplete => {
                return Err(Error::new(
                    "operation_uncertain",
                    "remote outcome is not established; mutation will not be replayed",
                ));
            }
        }
        Ok(receipt)
    }
}

pub enum Reconciliation {
    UniqueVerified {
        remote_ids: BTreeMap<String, String>,
        result: Value,
        payload_digest: String,
        actor: String,
    },
    Ambiguous,
    NotVisible,
    Incomplete,
}

#[derive(Debug, Clone, Serialize)]
pub struct ControlSnapshot {
    pub intent: IntentRecord,
    pub admission_open: bool,
    pub reconciled: bool,
    pub orchestration_alive: bool,
    pub draining: bool,
    pub active_operations: Vec<Permit>,
    pub fenced_operations: Vec<Uuid>,
    pub persistence_error: Option<String>,
    pub unresolved_operations: BTreeMap<Uuid, Vec<String>>,
    pub service_stopping: bool,
    pub background_commands: Vec<Uuid>,
    pub pending_external_effects: Vec<Uuid>,
    pub pending_local_processes: Vec<Uuid>,
}

struct State {
    intent: IntentRecord,
    open: bool,
    reconciled: bool,
    alive: bool,
    lease: Option<Lease>,
    permits: BTreeMap<Uuid, Permit>,
    effect_indexes: BTreeMap<Uuid, effect_index::LiveIndex>,
    effect_history: effect_index::History,
    #[cfg(test)]
    effect_index_fault: Option<kegari_storage::WriteBoundary>,
    fenced: BTreeSet<Uuid>,
    error: Option<String>,
    holds: BTreeMap<Uuid, Vec<String>>,
    service_stopping: bool,
    background_commands: BTreeSet<Uuid>,
    pending_external_effects: BTreeSet<Uuid>,
    local_processes: BTreeSet<Uuid>,
    suspension: Option<Uuid>,
}

impl State {
    fn snapshot(&self) -> ControlSnapshot {
        ControlSnapshot {
            intent: self.intent.clone(),
            admission_open: self.open,
            reconciled: self.reconciled,
            orchestration_alive: self.alive,
            draining: (self.intent.desired != Intent::Running
                && (!self.permits.is_empty() || !self.local_processes.is_empty()))
                || (self.intent.desired == Intent::Stopped && self.alive),
            active_operations: self.permits.values().cloned().collect(),
            fenced_operations: self.fenced.iter().copied().collect(),
            persistence_error: self.error.clone(),
            unresolved_operations: self.holds.clone(),
            service_stopping: self.service_stopping,
            background_commands: self.background_commands.iter().copied().collect(),
            pending_external_effects: self.pending_external_effects.iter().copied().collect(),
            pending_local_processes: self.local_processes.iter().copied().collect(),
        }
    }
}

fn validate_live_permit(state: &State, permit: &Permit) -> Result<()> {
    if state.lease.as_ref() != Some(&permit.lease)
        || state.fenced.contains(&permit.operation_id)
        || state
            .permits
            .get(&permit.operation_id)
            .is_none_or(|registered| {
                registered.scope_digest != permit.scope_digest
                    || registered.work_key != permit.work_key
                    || registered.lease != permit.lease
            })
    {
        Err(Error::new(
            "access_denied",
            "operation no longer owns an unfenced execution permit",
        ))
    } else {
        Ok(())
    }
}

fn remote_authority(
    state: &State,
    request: &MutationRequest,
    permit: Option<&Permit>,
    role: Option<&str>,
) -> Result<()> {
    if request.runtime_id != state.intent.runtime_id {
        return Err(Error::new("identity_mismatch", "wrong mutation runtime"));
    }
    if state.service_stopping {
        return Err(Error::new(
            "service_stopping",
            "control host shutdown is already fenced",
        ));
    }
    match (permit, role) {
        (Some(permit), None) => validate_live_permit(state, permit),
        (None, Some(role)) => request.actor.require(role),
        _ => Err(Error::new(
            "access_denied",
            "mutation requires exactly one operator or autonomous authority",
        )),
    }
}
fn remote_held(state: &State, request: &MutationRequest, permit: Option<&Permit>) -> bool {
    state.holds.iter().any(|(id, targets)| {
        *id != request.request_id
            && targets.iter().any(|target| {
                request.canonical_targets.contains(target)
                    || permit.is_some_and(|permit| &permit.work_key == target)
                    || target == &state.intent.runtime_id.to_string()
            })
    })
}

pub struct ControlHost {
    root: PrivateDir,
    receipts: ReceiptStore,
    _lock: ExclusiveLock,
    state: Mutex<State>,
}

impl ControlHost {
    /// Explicit local setup only. Reusing this method is not a recovery shortcut.
    pub fn setup(root: PrivateDir, runtime_id: Uuid, request_id: Uuid) -> Result<Self> {
        let lock = root.lock("control.lock")?;
        Self::setup_locked(root, runtime_id, request_id, lock)
    }

    pub(crate) fn setup_locked(
        root: PrivateDir,
        runtime_id: Uuid,
        request_id: Uuid,
        lock: ExclusiveLock,
    ) -> Result<Self> {
        match root.read::<IntentRecord>("intent.json") {
            Ok(_) => {
                return Err(Error::new(
                    "ownership_conflict",
                    "runtime already initialized; open the registered host",
                ));
            }
            Err(e) if e.code == "record_missing" => (),
            Err(e) => return Err(e),
        }
        let intent = IntentRecord {
            runtime_id,
            desired: Intent::Stopped,
            generation: 0,
            last_command_id: request_id,
            changed_at: Utc::now(),
        };
        root.write("intent.json", &intent)?;
        let receipts = ReceiptStore::new(root.child("receipts")?);
        Ok(Self {
            root,
            receipts,
            _lock: lock,
            state: Mutex::new(State {
                intent,
                open: false,
                reconciled: false,
                alive: false,
                lease: None,
                permits: BTreeMap::new(),
                effect_indexes: BTreeMap::new(),
                effect_history: effect_index::History::default(),
                #[cfg(test)]
                effect_index_fault: None,
                fenced: BTreeSet::new(),
                error: None,
                holds: BTreeMap::new(),
                service_stopping: false,
                background_commands: BTreeSet::new(),
                pending_external_effects: BTreeSet::new(),
                local_processes: BTreeSet::new(),
                suspension: None,
            }),
        })
    }

    pub fn recover(root: PrivateDir, runtime_id: Uuid) -> Result<Self> {
        let lock = root.lock("control.lock")?;
        Self::recover_locked(root, runtime_id, lock)
    }

    pub(crate) fn recover_locked(
        root: PrivateDir,
        runtime_id: Uuid,
        lock: ExclusiveLock,
    ) -> Result<Self> {
        let intent: IntentRecord = root.read("intent.json")?;
        if intent.runtime_id != runtime_id {
            return Err(Error::new(
                "identity_mismatch",
                "intent belongs to another runtime",
            ));
        }
        let receipts = ReceiptStore::new(root.child("receipts")?);
        let effect_history = effect_index::recover(&root, &receipts)?;
        // Parse every receipt before permitting any work. A corrupt record cannot vanish.
        let mut holds = BTreeMap::new();
        let mut local_processes = BTreeSet::new();
        for receipt in receipts.all()? {
            if receipt.runtime_id != runtime_id {
                return Err(Error::new(
                    "identity_mismatch",
                    "receipt belongs to another runtime",
                ));
            }
            if matches!(receipt.phase, ReceiptPhase::Prepared | ReceiptPhase::Sent) {
                if matches!(
                    receipt.kind.as_str(),
                    "host_metadata" | "host_cleanup" | "cleanup"
                ) {
                    local_processes.insert(receipt.operation_id);
                }
                let mut targets = receipt.canonical_targets;
                if let Some(work) = receipt.expected.get("admitted_work") {
                    targets.push(work.clone());
                }
                holds.insert(receipt.operation_id, targets);
            }
        }
        Ok(Self {
            root,
            receipts,
            _lock: lock,
            state: Mutex::new(State {
                intent,
                open: false,
                reconciled: false,
                alive: false,
                lease: None,
                permits: BTreeMap::new(),
                effect_indexes: BTreeMap::new(),
                effect_history,
                #[cfg(test)]
                effect_index_fault: None,
                fenced: BTreeSet::new(),
                error: None,
                holds,
                service_stopping: false,
                background_commands: BTreeSet::new(),
                pending_external_effects: BTreeSet::new(),
                local_processes,
                suspension: None,
            }),
        })
    }

    pub fn snapshot(&self) -> Result<ControlSnapshot> {
        let s = self.state.lock().map_err(|_| {
            Error::new(
                "control_failed",
                "control serialization failed; admission closed",
            )
        })?;
        Ok(s.snapshot())
    }

    /// Observe the actually attached running owner; this never restores a
    /// serialized lease or grants a permit. Admission rechecks it atomically.
    pub(crate) fn active_lease(&self) -> Result<Lease> {
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if !state.open
            || !state.alive
            || !state.reconciled
            || state.intent.desired != Intent::Running
            || state.service_stopping
        {
            return Err(Error::new(
                "admission_closed",
                "no current running reconciled owner",
            ));
        }
        state
            .lease
            .clone()
            .ok_or_else(|| Error::new("ownership_conflict", "running host has no attached lease"))
    }

    /// Atomically publish a completed read-only observation relative to mutation
    /// admission/dispatch. The callback must not reenter ControlHost or await I/O.
    pub fn serialize_observation<T>(
        &self,
        commit: impl FnOnce(&ControlSnapshot) -> Result<T>,
    ) -> Result<T> {
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        commit(&state.snapshot())
    }

    /// Called only after the OS runtime lock is held and provider/process recovery
    /// has established the supplied lease. Saved STOPPED/PAUSED never admit work.
    pub fn attach_reconciled(&self, lease: Lease) -> Result<()> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if s.service_stopping
            || lease.runtime_id != s.intent.runtime_id
            || lease.owner_generation == 0
        {
            return Err(Error::new(
                "ownership_conflict",
                "invalid orchestration lease",
            ));
        }
        if s.lease.as_ref().is_some_and(|prior| prior != &lease) {
            return Err(Error::new(
                "ownership_conflict",
                "another orchestration owner remains attached",
            ));
        }
        if s.intent.desired == Intent::Stopped {
            return Err(Error::new(
                "requires_start",
                "saved STOPPED forbids orchestration start",
            ));
        }
        s.lease = Some(lease);
        s.suspension = None;
        s.alive = true;
        s.reconciled = true;
        s.open = s.intent.desired == Intent::Running && s.error.is_none();
        Ok(())
    }

    pub fn lifecycle(
        &self,
        request: MutationRequest,
        command: Lifecycle,
        expected_generation: u64,
    ) -> Result<Receipt> {
        self.lifecycle_checked(request, command, expected_generation, || Ok(()))
    }
    pub fn lifecycle_checked(
        &self,
        request: MutationRequest,
        command: Lifecycle,
        expected_generation: u64,
        fresh_actor: impl FnOnce() -> Result<()>,
    ) -> Result<Receipt> {
        request.actor.require("lifecycle")?;
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if request.runtime_id != s.intent.runtime_id {
            return Err(Error::new("identity_mismatch", "wrong runtime"));
        }
        fresh_actor()?;
        // Bind the actual parsed command and expected revision, never caller-supplied duplicate text.
        let mut request = request;
        request.kind = "lifecycle".into();
        request.payload = serde_json::json!({"command":command,"expected_control_generation":expected_generation});
        let owner_generation = s.lease.as_ref().map_or(0, |l| l.owner_generation);
        self.guard_indexed_request_id(&mut s, request.request_id)?;
        let (mut receipt, fresh) = self.receipts.prepare(&request, owner_generation)?;
        if !fresh {
            if let Some(error) = receipt
                .observed_result
                .as_ref()
                .and_then(|r| r.get("error"))
            {
                return Err(serde_json::from_value(error.clone()).map_err(|_| {
                    Error::new("record_corrupt", "invalid stored lifecycle rejection")
                })?);
            }
            if receipt.phase == ReceiptPhase::Prepared
                && s.intent.last_command_id == request.request_id
            {
                // The intent replacement survived a crash before its command receipt.
                receipt.phase = ReceiptPhase::Reconciled;
                receipt.observed_result = Some(
                    serde_json::json!({"desired":s.intent.desired,"control_generation":s.intent.generation,"recovered":true}),
                );
                self.receipts.save(&receipt)?;
                s.holds.remove(&receipt.operation_id);
            }
            return Ok(receipt);
        }
        let rejection = if s.service_stopping {
            Some(Error::new(
                "service_stopping",
                "control host shutdown is already fenced",
            ))
        } else if expected_generation != s.intent.generation {
            Some(Error::new(
                "stale_control",
                "a newer lifecycle decision exists",
            ))
        } else {
            match (&command, s.intent.desired) {
                (Lifecycle::Start, Intent::Paused) => Some(Error::new(
                    "requires_resume",
                    "start preserves PAUSED; explicitly resume",
                )),
                (Lifecycle::Resume, Intent::Stopped) => Some(Error::new(
                    "requires_start",
                    "resume preserves STOPPED; explicitly start",
                )),
                (Lifecycle::Start, Intent::Stopped)
                    if s.alive || !s.permits.is_empty() || !s.local_processes.is_empty() =>
                {
                    Some(Error::new(
                        "stop_in_progress",
                        "owned operations are still draining",
                    ))
                }
                _ => None,
            }
        };
        if let Some(error) = rejection {
            receipt.phase = ReceiptPhase::Reconciled;
            receipt.observed_result = Some(serde_json::json!({"error":error}));
            self.receipts.save(&receipt)?;
            return Err(error);
        }
        let desired = match (&command, s.intent.desired) {
            (Lifecycle::Pause, Intent::Stopped) => Intent::Stopped,
            (Lifecycle::Pause, _) => Intent::Paused,
            (Lifecycle::Start | Lifecycle::Resume, _) => Intent::Running,
            (Lifecycle::Stop | Lifecycle::ForceStop, _) => Intent::Stopped,
        };
        // Fence admission before any durability boundary, including a failed fsync.
        s.open = false;
        if command == Lifecycle::ForceStop {
            let ids: Vec<_> = s
                .permits
                .keys()
                .chain(s.local_processes.iter())
                .copied()
                .collect();
            s.fenced.extend(ids);
        }
        let mut next = s.intent.clone();
        if desired != next.desired || command == Lifecycle::ForceStop {
            next.generation += 1;
        }
        next.desired = desired;
        next.last_command_id = request.request_id;
        next.changed_at = Utc::now();
        if let Err(error) = self.root.write("intent.json", &next) {
            s.error = Some(error.code.clone());
            return Err(error);
        }
        s.intent = next;
        s.open = desired == Intent::Running && s.reconciled && s.alive && s.error.is_none();
        receipt.phase = ReceiptPhase::Reconciled;
        receipt.observed_result = Some(
            serde_json::json!({"desired":s.intent.desired,"control_generation":s.intent.generation,"draining":desired != Intent::Running && (s.alive || !s.permits.is_empty())}),
        );
        if let Err(error) = self.receipts.save(&receipt) {
            s.open = false;
            s.error = Some(error.code.clone());
            return Err(error);
        }
        Ok(receipt)
    }

    pub fn admit(
        &self,
        lease: &Lease,
        request_id: Uuid,
        work_key: String,
        scope_digest: String,
        fresh_gate: impl FnOnce() -> Result<()>,
    ) -> Result<Permit> {
        self.admit_observed(lease, request_id, work_key, scope_digest, |_| fresh_gate())
    }

    /// Inspect the exact current control generation inside the admission gate.
    /// The callback must not reenter control, perform network I/O or infer that
    /// a Permit exists before this method returns successfully.
    pub fn admit_observed(
        &self,
        lease: &Lease,
        request_id: Uuid,
        work_key: String,
        scope_digest: String,
        fresh_gate: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        self.admit_inner(lease, request_id, work_key, scope_digest, None, fresh_gate)
    }

    pub fn admit_run(
        &self,
        lease: &Lease,
        input: RunAdmission,
        fresh_gate: impl FnOnce() -> Result<()>,
    ) -> Result<Permit> {
        self.admit_run_observed(lease, input, |_| fresh_gate())
    }
    /// Capacity and current authority are checked against this exact serialized
    /// snapshot; a competing session cannot enter between count and reservation.
    pub fn admit_run_observed(
        &self,
        lease: &Lease,
        input: RunAdmission,
        fresh_gate: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        validate_run_reference(&input.reference)?;
        self.admit_inner(
            lease,
            input.request_id,
            input.work_key,
            input.scope_digest,
            Some(AdmissionReference::Run(input.reference)),
            fresh_gate,
        )
    }

    pub fn admit_metadata(
        &self,
        lease: &Lease,
        input: MetadataAdmission,
        fresh_gate: impl FnOnce() -> Result<()>,
    ) -> Result<Permit> {
        input.reference.validate()?;
        self.admit_inner(
            lease,
            input.request_id,
            input.work_key,
            input.scope_digest,
            Some(AdmissionReference::Metadata(input.reference)),
            |_| fresh_gate(),
        )
    }

    fn admit_inner(
        &self,
        lease: &Lease,
        request_id: Uuid,
        work_key: String,
        scope_digest: String,
        reference: Option<AdmissionReference>,
        fresh_gate: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if !s.open || s.intent.desired != Intent::Running || s.lease.as_ref() != Some(lease) {
            return Err(Error::new(
                "admission_closed",
                "intent, recovery or orchestration lease prohibits new work",
            ));
        }
        self.guard_indexed_request_id(&mut s, request_id)?;
        if s.holds.values().any(|targets| {
            targets.contains(&work_key) || targets.contains(&s.intent.runtime_id.to_string())
        }) {
            return Err(Error::new(
                "reconciliation_required",
                "prior unresolved operation holds this canonical work",
            ));
        }
        if s.permits.values().any(|p| p.work_key == work_key) {
            return Err(Error::new(
                "ownership_conflict",
                "work identity already has an active operation",
            ));
        }
        let round_receipts = if matches!(
            &reference,
            Some(
                AdmissionReference::RoundPreparation(..)
                    | AdmissionReference::RoundRun(..)
                    | AdmissionReference::RoundSessionRecovery(..)
                    | AdmissionReference::RoundResumedRun(..)
            )
        ) {
            Some(self.review_round_receipts()?)
        } else {
            None
        };
        if let Some(
            AdmissionReference::Run(run)
            | AdmissionReference::ParentRun(run, _, _)
            | AdmissionReference::RoundRun(run, _)
            | AdmissionReference::ContinuedRun(run, _)
            | AdmissionReference::SourceAdvancedRun(run, _)
            | AdmissionReference::ResumedRun(run, _)
            | AdmissionReference::RoundResumedRun(run, _, _),
        ) = &reference
        {
            let mut prior = BTreeMap::new();
            let mut cap = run.max_attempts;
            let observed = match &round_receipts {
                Some(receipts) => receipts.clone(),
                None => self.receipts.all()?,
            };
            if !matches!(&reference, Some(AdmissionReference::ParentRun(..))) {
                reject_unmarked_parent_phase(&run.phase, &observed)?;
            }
            for receipt in observed {
                if let Some(old) = crate::control::run_reference_from_receipt(&receipt)?
                    && old.phase == run.phase
                {
                    if old.workspace != run.workspace
                        || prior.insert(old.attempt, receipt).is_some()
                    {
                        return Err(Error::new(
                            "record_corrupt",
                            "phase history has conflicting ownership or attempt ordinals",
                        ));
                    }
                    cap = cap.min(old.max_attempts);
                }
            }
            if prior.keys().copied().ne(1..=prior.len() as u32) {
                return Err(Error::new(
                    "record_corrupt",
                    "phase attempt history has a gap",
                ));
            }
            if run.attempt != prior.len() as u32 + 1 || run.attempt > cap {
                return Err(Error::new(
                    "attempts_exhausted",
                    "admission would reset, duplicate or exceed the durable phase budget",
                ));
            }
            if !matches!(
                &reference,
                Some(AdmissionReference::RoundRun(..) | AdmissionReference::RoundResumedRun(..))
            ) && prior
                .values()
                .any(|receipt| receipt.expected.contains_key("review_round"))
            {
                return Err(Error::new(
                    "review_round_required",
                    "admission cannot drop retained round provenance",
                ));
            }
            if let Some(AdmissionReference::ParentRun(_, proof, preparation)) = &reference {
                self.verify_parent_reassembly_run(&s.snapshot(), lease, proof, preparation)?;
            }
            if let Some(AdmissionReference::RoundRun(_, provenance)) = &reference {
                self.verify_round_run(RoundRunCheck {
                    snapshot: &s.snapshot(),
                    lease,
                    operation: request_id,
                    run,
                    provenance,
                    history: &prior,
                    receipts: round_receipts.as_deref().ok_or_else(|| {
                        Error::new("control_failed", "round inventory was not loaded")
                    })?,
                })?;
            }
            if let Some(AdmissionReference::ContinuedRun(_, continuation)) = &reference {
                self.verify_run_continuation(&s.snapshot(), run, continuation, &prior)?;
            }
            if let Some(AdmissionReference::SourceAdvancedRun(_, source)) = &reference {
                self.verify_source_advanced(&s.snapshot(), run, source, &prior)?;
            }
            if let Some(AdmissionReference::ResumedRun(_, resume)) = &reference {
                self.verify_run_resume(&s.snapshot(), run, resume, &prior)?;
            }
            if let Some(AdmissionReference::RoundResumedRun(_, resume, provenance)) = &reference {
                self.verify_round_run_resume(RoundResumeCheck {
                    snapshot: &s.snapshot(),
                    lease,
                    run,
                    resume,
                    provenance,
                    history: &prior,
                    receipts: round_receipts.as_deref().ok_or_else(|| {
                        Error::new("control_failed", "paired inventory was not loaded")
                    })?,
                })?;
            }
        }
        if let Some(AdmissionReference::ParentPreparation(_, proof)) = &reference {
            self.verify_parent_reassembly_preparation(&s.snapshot(), proof)?;
        }
        if let Some(AdmissionReference::Preparation(prep)) = &reference {
            let receipts = self.review_round_receipts()?;
            reject_unmarked_parent_phase(&prep.phase, &receipts)?;
            for receipt in receipts {
                if let Some(run) = crate::control::run_reference_from_receipt(&receipt)?
                    && run.phase == prep.phase
                    && receipt.expected.contains_key("review_round")
                {
                    return Err(Error::new(
                        "review_round_required",
                        "preparation cannot drop retained round provenance",
                    ));
                }
            }
        }
        if let Some(AdmissionReference::RoundPreparation(prep, round)) = &reference {
            self.verify_round_preparation(RoundPreparationCheck {
                snapshot: &s.snapshot(),
                operation: request_id,
                prep,
                round,
                receipts: round_receipts.as_deref().ok_or_else(|| {
                    Error::new("control_failed", "round inventory was not loaded")
                })?,
            })?;
        }
        if let Some(AdmissionReference::SessionRecovery(reference)) = &reference {
            self.verify_session_recovery_preparation(&s.snapshot(), reference)?;
        }
        if let Some(AdmissionReference::RoundSessionRecovery(reference, provenance)) = &reference {
            self.verify_round_session_recovery(
                &s.snapshot(),
                lease,
                reference,
                provenance,
                round_receipts.as_deref().ok_or_else(|| {
                    Error::new("control_failed", "paired inventory was not loaded")
                })?,
            )?;
        }
        if let Some(AdmissionReference::Metadata(reference)) = &reference {
            let mut prior = BTreeMap::new();
            let mut cap = reference.max_attempts;
            for receipt in self.receipts.all()? {
                if let Some(old) = MetadataReference::from_receipt(&receipt)?
                    && old.purpose == reference.purpose
                    && old.generation == reference.generation
                    && receipt.canonical_targets == vec![work_key.clone()]
                {
                    if prior.insert(old.attempt, receipt.operation_id).is_some() {
                        return Err(Error::new(
                            "record_corrupt",
                            "metadata budget contains duplicate attempts",
                        ));
                    }
                    cap = cap.min(old.max_attempts);
                }
            }
            if prior.keys().copied().ne(1..=prior.len() as u32) {
                return Err(Error::new(
                    "record_corrupt",
                    "metadata attempt history has a gap",
                ));
            }
            if reference.attempt != prior.len() as u32 + 1
                || reference.attempt > cap
                || reference.max_attempts > cap
            {
                return Err(Error::new(
                    "attempts_exhausted",
                    "metadata admission would reset, duplicate or exceed its durable budget",
                ));
            }
        }
        fresh_gate(&s.snapshot())?;
        let permit = Permit {
            operation_id: request_id,
            lease: lease.clone(),
            control_generation: s.intent.generation,
            work_key,
            scope_digest,
            registered_at: Utc::now(),
        };
        let request = admission_identity::request(&permit, reference)?;
        let (admission, fresh) = match self.receipts.prepare(&request, lease.owner_generation) {
            Ok(value) => value,
            Err(error) => {
                // Even a failed write may have renamed the Prepared admission.
                // No index or live permit can be inferred from that uncertainty.
                s.open = false;
                s.error = Some(error.code.clone());
                s.holds.insert(request_id, vec![permit.work_key.clone()]);
                return Err(error);
            }
        };
        if !fresh {
            return Err(Error::new(
                "operation_uncertain",
                "existing operation must be reconciled before admitting another",
            ));
        }
        self.create_effect_index(&mut s, &admission)?;
        s.permits.insert(request_id, permit.clone());
        Ok(permit)
    }

    pub fn validate_permit(
        &self,
        permit: &Permit,
        fresh_gate: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        self.with_valid_permit(permit, |_| fresh_gate())
    }

    pub(crate) fn validate_permit_observed<T>(
        &self,
        permit: &Permit,
        observe: impl FnOnce(&ControlSnapshot) -> Result<T>,
    ) -> Result<T> {
        self.with_valid_permit(permit, |state| observe(&state.snapshot()))
    }
    fn with_valid_permit<T>(
        &self,
        permit: &Permit,
        observe: impl FnOnce(&State) -> Result<T>,
    ) -> Result<T> {
        let s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if s.lease.as_ref() != Some(&permit.lease)
            || s.fenced.contains(&permit.operation_id)
            || s.permits.get(&permit.operation_id).is_none_or(|p| {
                p.scope_digest != permit.scope_digest || p.work_key != permit.work_key
            })
        {
            return Err(Error::new(
                "access_denied",
                "operation permit is absent, stale or fenced",
            ));
        }
        observe(&s)
    }

    pub fn fence(&self, operation_id: Uuid) -> Result<()> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        self.expand_operation_holds(&mut s, operation_id)?;
        s.fenced.insert(operation_id);
        Ok(())
    }

    fn expand_operation_holds(&self, s: &mut State, operation_id: Uuid) -> Result<()> {
        let Some(permit) = s.permits.get(&operation_id) else {
            return Ok(());
        };
        let work = permit.work_key.clone();
        for (id, targets) in &mut s.holds {
            let receipt = self.receipts.get(*id)?;
            if receipt.expected.get("admission_operation") == Some(&operation_id.to_string())
                && !targets.contains(&work)
            {
                targets.push(work.clone());
            }
        }
        Ok(())
    }

    pub fn finish(&self, operation_id: Uuid, observed_result: Value) -> Result<()> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if !s.permits.contains_key(&operation_id) {
            return Err(Error::new(
                "ownership_conflict",
                "completion has no active operation ownership",
            ));
        }
        let mut receipt = self.receipts.get(operation_id)?;
        receipt.phase = ReceiptPhase::Reconciled;
        receipt.observed_result = Some(observed_result.clone());
        receipt.updated_at = Utc::now();
        let work = &s.permits[&operation_id].work_key;
        if s.holds.contains_key(&operation_id)
            || s.holds.values().any(|targets| targets.contains(work))
        {
            return Err(Error::new(
                "reconciliation_required",
                "recovered operation needs verified recovery, not a worker completion",
            ));
        }
        self.freeze_effect_index(&mut s, &mut receipt, "finish", &observed_result)?;
        if let Err(error) = self.save_indexed_owner(&mut s, &receipt) {
            s.open = false;
            s.fenced.insert(operation_id);
            s.error = Some(error.code.clone());
            return Err(error);
        }
        s.permits.remove(&operation_id);
        Ok(())
    }

    /// Release only the local execution claim after its actual owned task has
    /// joined. Dependent unknown effects retain their own immutable receipts and
    /// overlapping holds; this observation never establishes delivered success.
    pub fn retire_local(&self, operation_id: Uuid, observation: Value) -> Result<Receipt> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        self.expand_operation_holds(&mut s, operation_id)?;
        let permit = s.permits.get(&operation_id).ok_or_else(|| {
            Error::new(
                "ownership_conflict",
                "local exit has no active operation ownership",
            )
        })?;
        let unresolved: Vec<_> = s
            .holds
            .iter()
            .filter(|(_, targets)| targets.contains(&permit.work_key))
            .map(|(id, _)| *id)
            .collect();
        let mut receipt = self.receipts.get(operation_id)?;
        if receipt.kind != "admission" || s.holds.contains_key(&operation_id) {
            return Err(Error::new(
                "reconciliation_required",
                "local exit cannot resolve an uncertain admission or external action",
            ));
        }
        receipt.phase = ReceiptPhase::Reconciled;
        receipt.observed_result = Some(
            serde_json::json!({"local_execution":"exited","unresolved_effects":unresolved,"observation":&observation}),
        );
        receipt.updated_at = Utc::now();
        self.freeze_effect_index(&mut s, &mut receipt, "retire_local", &observation)?;
        if let Err(error) = self.save_indexed_owner(&mut s, &receipt) {
            s.open = false;
            s.fenced.insert(operation_id);
            s.error = Some(error.code.clone());
            return Err(error);
        }
        s.permits.remove(&operation_id);
        Ok(receipt)
    }

    pub fn orchestration_exited(&self, lease: &Lease) -> Result<()> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if s.lease.as_ref() != Some(lease) {
            return Err(Error::new(
                "ownership_conflict",
                "exit from unrelated orchestration owner",
            ));
        }
        s.open = false;
        let active: Vec<_> = s.permits.keys().copied().collect();
        s.fenced.extend(active);
        s.alive = false;
        s.reconciled = false;
        s.lease = None;
        Ok(())
    }

    /// Fence a failed owner's effects without claiming its process has exited or
    /// rewriting operator intent. Repeated reports share one durable hold.
    pub fn suspend_owner(&self, lease: &Lease, reason: &str) -> Result<Receipt> {
        if reason.is_empty()
            || reason.len() > 128
            || !reason
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'))
        {
            return Err(Error::invalid(
                "owner failure needs a bounded stable reason code",
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if state.lease.as_ref() != Some(lease) {
            return Err(Error::new(
                "ownership_conflict",
                "cannot suspend another owner",
            ));
        }
        state.open = false;
        state.reconciled = false;
        let operations: Vec<_> = state.permits.keys().copied().collect();
        state.fenced.extend(operations);
        if let Some(id) = state.suspension {
            return self.receipts.get(id);
        }
        let id = Uuid::new_v4();
        let request = MutationRequest {
            request_id: id,
            runtime_id: lease.runtime_id,
            actor: Actor {
                identity: format!("owner:{}", lease.owner_id),
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: "owner_suspension".into(),
            canonical_targets: vec![lease.runtime_id.to_string()],
            expected: BTreeMap::from([(
                "owner_generation".into(),
                lease.owner_generation.to_string(),
            )]),
            payload: serde_json::json!({"lease":lease,"reason":reason}),
        };
        let record = (|| -> Result<Receipt> {
            self.guard_indexed_request_id(&mut state, request.request_id)?;
            let (mut receipt, _) = self.receipts.prepare(&request, lease.owner_generation)?;
            self.receipts.mark_sent(&mut receipt)?;
            receipt.observed_result = Some(
                serde_json::json!({"owner_state":"suspended","reason":reason,"process_exit":"unconfirmed"}),
            );
            self.receipts.save(&receipt)?;
            Ok(receipt)
        })();
        let receipt = match record {
            Ok(receipt) => receipt,
            Err(error) => {
                state.error = Some(error.code.clone());
                return Err(error);
            }
        };
        state.suspension = Some(id);
        state.holds.insert(id, receipt.canonical_targets.clone());
        Ok(receipt)
    }
    pub fn operation(&self, id: Uuid) -> Result<Receipt> {
        self.receipts.get(id)
    }

    pub fn operations(&self) -> Result<Vec<Receipt>> {
        self.receipts.all()
    }

    /// Internal read-only recovery of an immutable owned receipt. Caller must
    /// independently verify the provider/local facts before passing this result.
    /// It grants no permission to retransmit or create replacement work.
    pub(crate) fn reconcile_owned(
        &self,
        expected: &Receipt,
        observation: Reconciliation,
    ) -> Result<Receipt> {
        self.reconcile_owned_checked(expected, observation, || Ok(()))
    }
    pub(crate) fn reconcile_owned_checked(
        &self,
        expected: &Receipt,
        observation: Reconciliation,
        current_owner: impl FnOnce() -> Result<()>,
    ) -> Result<Receipt> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if s.pending_external_effects.contains(&expected.operation_id)
            || s.background_commands.contains(&expected.operation_id)
            || s.permits.contains_key(&expected.operation_id)
        {
            return Err(Error::new(
                "operation_active",
                "owned task must exit before receipt recovery",
            ));
        }
        let current = self.receipts.get(expected.operation_id)?;
        if current.runtime_id != s.intent.runtime_id
            || current.payload_digest != expected.payload_digest
            || current.actor != expected.actor
            || current.kind != expected.kind
        {
            return Err(Error::new(
                "ownership_conflict",
                "recovery observation differs from immutable receipt",
            ));
        }
        let observation = self.indexed_reconciliation(&mut s, &current, observation)?;
        current_owner()?;
        let receipt = self
            .receipts
            .reconcile(expected.operation_id, observation)?;
        s.holds.remove(&expected.operation_id);
        Ok(receipt)
    }

    /// Durable admission of a bounded host command, distinct from an autonomous
    /// worker permit. Recovery preserves its uncertainty and never replays it.
    pub fn queue_command(
        &self,
        request: MutationRequest,
        role: &str,
        fresh_actor: impl FnOnce() -> Result<()>,
    ) -> Result<(Receipt, bool)> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        request.actor.require(role)?;
        fresh_actor()?;
        if request.runtime_id != state.intent.runtime_id
            || !request.kind.starts_with("command/")
            || state.service_stopping
        {
            return Err(Error::new(
                "access_denied",
                "command cannot enter this runtime",
            ));
        }
        if state.background_commands.len() >= 32
            && matches!(self.receipts.get(request.request_id), Err(error) if error.code=="record_missing")
        {
            return Err(Error::new("capacity", "too many active control commands"));
        }
        self.guard_indexed_request_id(&mut state, request.request_id)?;
        let (receipt, fresh) = self.receipts.prepare(
            &request,
            state.lease.as_ref().map_or(0, |l| l.owner_generation),
        )?;
        if fresh {
            state.background_commands.insert(receipt.operation_id);
        }
        Ok((receipt, fresh))
    }
    pub fn complete_command(&self, id: Uuid, result: Result<Value>) -> Result<Receipt> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if !state.background_commands.contains(&id) {
            return Err(Error::new(
                "ownership_conflict",
                "command has no owned background task",
            ));
        }
        let mut receipt = self.receipts.get(id)?;
        if !receipt.kind.starts_with("command/") || receipt.phase != ReceiptPhase::Prepared {
            return Err(Error::new(
                "ownership_conflict",
                "background command receipt changed",
            ));
        }
        receipt.updated_at = Utc::now();
        match result {
            Ok(value) => {
                receipt.phase = ReceiptPhase::Reconciled;
                receipt.observed_result = Some(value);
            }
            Err(error) => {
                receipt.phase = if error.code == "operation_uncertain" {
                    ReceiptPhase::Sent
                } else {
                    ReceiptPhase::Reconciled
                };
                receipt.observed_result = Some(serde_json::json!({"error":error}));
            }
        }
        if let Err(error) = self.receipts.save(&receipt) {
            state.open = false;
            state.error = Some(error.code.clone());
            return Err(error);
        }
        if receipt.phase == ReceiptPhase::Sent {
            state.holds.insert(id, receipt.canonical_targets.clone());
        }
        state.background_commands.remove(&id);
        Ok(receipt)
    }

    /// Register and launch an owned local process while admission is serialized,
    /// then release the mutex before waiting. The launch callback must be bounded
    /// and synchronous: it spawns/registers, and never waits for process output.
    pub fn begin_effect<T>(
        &self,
        request: MutationRequest,
        permit: &Permit,
        fresh_gate: impl FnOnce() -> Result<()>,
        launch: impl FnOnce(&Receipt) -> Result<T>,
    ) -> Result<(InFlightEffect, T)> {
        if request.expected.contains_key(check_bundle::PLAN_KEY) {
            return Err(Error::new(
                "scope_violation",
                "a check bundle requires its typed admission path",
            ));
        }
        self.begin_owned_effect(request, permit, fresh_gate, launch, false)
    }

    /// A bounded frame write starts under the mutation gate; provider replies
    /// and interleaved dynamic tools are processed outside it. A pending request
    /// holds this exact session, and expands to the work on fencing, local exit
    /// or recovery. This exception cannot be used for a repository/provider tool.
    pub fn begin_protocol_effect<T>(
        &self,
        request: MutationRequest,
        permit: &Permit,
        fresh_gate: impl FnOnce() -> Result<()>,
        write_frame: impl FnOnce(&Receipt) -> Result<T>,
    ) -> Result<(InFlightEffect, T)> {
        if !request.kind.starts_with("codex/")
            || request.kind.len() > 128
            || request.canonical_targets != vec![format!("codex-session:{}", permit.operation_id)]
        {
            return Err(Error::new(
                "scope_violation",
                "protocol dispatch requires the exact admitted Codex session target",
            ));
        }
        self.begin_owned_effect(request, permit, fresh_gate, write_frame, true)
    }

    fn begin_owned_effect<T>(
        &self,
        mut request: MutationRequest,
        permit: &Permit,
        fresh_gate: impl FnOnce() -> Result<()>,
        launch: impl FnOnce(&Receipt) -> Result<T>,
        protocol: bool,
    ) -> Result<(InFlightEffect, T)> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        validate_live_permit(&s, permit)?;
        if request.expected.contains_key(check_bundle::PLAN_KEY) {
            let registered = s.permits.get(&permit.operation_id).ok_or_else(|| {
                Error::new(
                    "check_bundle_identity",
                    "bundle has no actual registered permit",
                )
            })?;
            if canonical_digest(1, registered)? != canonical_digest(1, permit)? {
                return Err(Error::new(
                    "check_bundle_identity",
                    "bundle permit differs from its complete original registration",
                ));
            }
        }
        if request.runtime_id != s.intent.runtime_id || s.service_stopping {
            return Err(Error::new(
                "access_denied",
                "local effect does not belong to this active host",
            ));
        }
        request
            .expected
            .insert("admitted_work".into(), permit.work_key.clone());
        request.expected.insert(
            "admission_operation".into(),
            permit.operation_id.to_string(),
        );
        request
            .expected
            .insert("admitted_scope".into(), permit.scope_digest.clone());
        let (mut receipt, fresh) = self.prepare_indexed_effect(
            &mut s,
            &request,
            Some(permit),
            permit.lease.owner_generation,
        )?;
        self.register_indexed_effect(&mut s, Some(permit), &receipt, fresh)?;
        if !fresh {
            return Err(Error::new(
                "operation_uncertain",
                "local effect already exists; inspect and reconcile without relaunching",
            )
            .detail(serde_json::json!({"operation_id":receipt.operation_id})));
        }
        let held = s.holds.iter().any(|(id, targets)| {
            *id != request.request_id
                && targets.iter().any(|target| {
                    request.canonical_targets.contains(target)
                        || target == &permit.work_key
                        || target == &s.intent.runtime_id.to_string()
                })
        });
        let check = if held {
            Err(Error::new(
                "reconciliation_required",
                "prior unresolved effect holds this work",
            ))
        } else {
            fresh_gate()
        };
        if let Err(error) = check {
            if receipt.expected.contains_key(check_bundle::PLAN_KEY) {
                check_bundle::record_never_started(&mut receipt)?;
            }
            receipt.phase = ReceiptPhase::Reconciled;
            receipt.observed_result =
                Some(serde_json::json!({"effect":"not_started","error":error}));
            self.save_indexed_receipt(&mut s, &receipt)?;
            return Err(error);
        }
        self.send_indexed_receipt(&mut s, &mut receipt)?;
        let mut targets = receipt.canonical_targets.clone();
        if !protocol {
            targets.push(permit.work_key.clone());
        }
        s.holds.insert(receipt.operation_id, targets);
        let owned = launch(&receipt).map_err(|error| {
            Error::new(
                "operation_uncertain",
                "local launch requires process and receipt reconciliation",
            )
            .detail(serde_json::json!({"operation_id":receipt.operation_id,"cause":error.code}))
        })?;
        Ok((
            InFlightEffect {
                request_id: request.request_id,
                permit: permit.clone(),
                identity: receipt_request_digest(&receipt)?,
                payload_digest: receipt.payload_digest,
            },
            owned,
        ))
    }

    /// Called only after the owned process has exited. Invalidated success is
    /// recorded as rejected evidence; cancellation cannot become a passing gate.
    pub fn complete_effect(
        &self,
        effect: InFlightEffect,
        permit: &Permit,
        fresh_gate: impl FnOnce() -> Result<()>,
        observed: Value,
    ) -> Result<Receipt> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        let mut receipt = self.receipts.get(effect.request_id)?;
        if receipt.expected.contains_key(check_bundle::PLAN_KEY) {
            return Err(Error::new(
                "scope_violation",
                "a check bundle requires its actual settled owner",
            ));
        }
        if effect.permit.operation_id != permit.operation_id
            || effect.permit.lease != permit.lease
            || effect.permit.scope_digest != permit.scope_digest
            || receipt.payload_digest != effect.payload_digest
            || receipt.phase != ReceiptPhase::Sent
        {
            return Err(Error::new(
                "ownership_conflict",
                "local completion differs from its launched effect",
            ));
        }
        self.verify_effect_identity(
            &mut s,
            &receipt,
            effect.request_id,
            &effect.identity,
            Some(effect.permit.operation_id),
        )?;
        let current = validate_live_permit(&s, permit).and_then(|()| fresh_gate());
        let result = match &current {
            Ok(()) => observed,
            Err(error) => {
                serde_json::json!({"effect":"exited_evidence_rejected","error":error,"observed":observed})
            }
        };
        if let Err(error) =
            self.acknowledge_indexed_receipt(&mut s, &mut receipt, BTreeMap::new(), result)
        {
            s.open = false;
            s.error = Some(error.code.clone());
            return Err(error);
        }
        s.holds.remove(&effect.request_id);
        current?;
        Ok(receipt)
    }

    /// Reserve a unique remote action before asynchronous provider preflight.
    /// Replays return the original receipt and never repeat a dispatch. The
    /// owned broker task retains this reservation when its caller disconnects.
    pub(crate) fn reserve_remote(
        &self,
        mut request: MutationRequest,
        permit: Option<Permit>,
        role: Option<String>,
        fresh_gate: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<RemoteReservation> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        remote_authority(&state, &request, permit.as_ref(), role.as_deref())?;
        if let Some(permit) = &permit {
            request
                .expected
                .insert("admitted_work".into(), permit.work_key.clone());
            request.expected.insert(
                "admission_operation".into(),
                permit.operation_id.to_string(),
            );
            request
                .expected
                .insert("admitted_scope".into(), permit.scope_digest.clone());
        }
        let generation = state
            .lease
            .as_ref()
            .map_or(0, |lease| lease.owner_generation);
        let (mut receipt, fresh) =
            self.prepare_indexed_effect(&mut state, &request, permit.as_ref(), generation)?;
        self.register_indexed_effect(&mut state, permit.as_ref(), &receipt, fresh)?;
        if !fresh {
            if let Some(error) = receipt
                .observed_result
                .as_ref()
                .and_then(|value| value.get("error"))
            {
                return Err(serde_json::from_value(error.clone()).map_err(|_| {
                    Error::new("record_corrupt", "invalid stored mutation rejection")
                })?);
            }
            return Ok(RemoteReservation::Existing(receipt));
        }
        if remote_held(&state, &request, permit.as_ref())
            || state.pending_external_effects.len() >= 64
        {
            let error = Error::new(
                "reconciliation_required",
                "an overlapping or saturated remote operation is pending",
            );
            receipt.phase = ReceiptPhase::Reconciled;
            receipt.observed_result = Some(serde_json::json!({"effect":"not_sent","error":error}));
            self.save_indexed_receipt(&mut state, &receipt)?;
            return Err(error);
        }
        if let Err(error) = fresh_gate(&state.snapshot()) {
            receipt.phase = ReceiptPhase::Reconciled;
            receipt.observed_result = Some(serde_json::json!({"effect":"not_sent","error":error}));
            self.save_indexed_receipt(&mut state, &receipt)?;
            return Err(error);
        }
        let mut targets = request.canonical_targets.clone();
        if let Some(permit) = &permit {
            targets.push(permit.work_key.clone());
        }
        state.holds.insert(receipt.operation_id, targets);
        state.pending_external_effects.insert(receipt.operation_id);
        Ok(RemoteReservation::Prepared(PreparedRemote {
            request,
            permit,
            role,
            identity: receipt_request_digest(&receipt)?,
            digest: receipt.payload_digest,
        }))
    }

    /// Revalidate after network preflight and start exactly one owned transport
    /// task under the lifecycle lock. Launch must only spawn, never await I/O.
    pub(crate) fn launch_remote<T>(
        &self,
        reservation: PreparedRemote,
        preflight: Result<()>,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
        launch: impl FnOnce(&Receipt) -> T,
    ) -> Result<(InFlightRemote, T)> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        let mut receipt = self.receipts.get(reservation.request.request_id)?;
        if !state
            .pending_external_effects
            .contains(&receipt.operation_id)
            || receipt.phase != ReceiptPhase::Prepared
            || receipt.payload_digest != reservation.digest
        {
            return Err(Error::new(
                "ownership_conflict",
                "remote preflight reservation changed",
            ));
        }
        self.verify_effect_identity(
            &mut state,
            &receipt,
            reservation.request.request_id,
            &reservation.identity,
            reservation.permit.as_ref().map(|p| p.operation_id),
        )?;
        let gate = preflight
            .and_then(|()| {
                remote_authority(
                    &state,
                    &reservation.request,
                    reservation.permit.as_ref(),
                    reservation.role.as_deref(),
                )
            })
            .and_then(|()| {
                if remote_held(&state, &reservation.request, reservation.permit.as_ref()) {
                    Err(Error::new(
                        "reconciliation_required",
                        "overlapping effect changed during provider preflight",
                    ))
                } else {
                    fresh(&state.snapshot())
                }
            });
        if let Err(error) = gate {
            receipt.phase = ReceiptPhase::Reconciled;
            receipt.observed_result = Some(serde_json::json!({"effect":"not_sent","error":error}));
            self.save_indexed_receipt(&mut state, &receipt)?;
            state.holds.remove(&receipt.operation_id);
            state.pending_external_effects.remove(&receipt.operation_id);
            return Err(error);
        }
        if let Err(error) = self.send_indexed_receipt(&mut state, &mut receipt) {
            state.open = false;
            state.error = Some(error.code.clone());
            return Err(error);
        }
        let task = launch(&receipt);
        Ok((
            InFlightRemote {
                operation: receipt.operation_id,
                digest: receipt.payload_digest,
                identity: reservation.identity,
                owner: reservation.permit.as_ref().map(|p| p.operation_id),
            },
            task,
        ))
    }

    /// Transport response records what actually happened even if later authority
    /// changed. Such acknowledgement grants no new worker or evidence authority.
    pub(crate) fn complete_remote(
        &self,
        effect: InFlightRemote,
        observed: Result<(BTreeMap<String, String>, Value)>,
    ) -> Result<Receipt> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        let mut receipt = self.receipts.get(effect.operation)?;
        if !state.pending_external_effects.contains(&effect.operation)
            || receipt.payload_digest != effect.digest
            || receipt.phase != ReceiptPhase::Sent
        {
            return Err(Error::new(
                "ownership_conflict",
                "remote completion differs from its owned dispatch",
            ));
        }
        self.verify_effect_identity(
            &mut state,
            &receipt,
            effect.operation,
            &effect.identity,
            effect.owner,
        )?;
        state.pending_external_effects.remove(&effect.operation);
        let (ids,result)=observed.map_err(|error| Error::new("operation_uncertain", "provider mutation outcome requires reconciliation").detail(serde_json::json!({"operation_id":effect.operation,"request_id":effect.operation,"cause":error.code})))?;
        if let Err(error) = self.acknowledge_indexed_receipt(&mut state, &mut receipt, ids, result)
        {
            state.open = false;
            state.error = Some(error.code.clone());
            return Err(error);
        }
        state.holds.remove(&effect.operation);
        Ok(receipt)
    }

    /// Called after the owned broker task has joined, including panic. It never
    /// clears the durable receipt or unresolved target hold.
    pub(crate) fn remote_task_exited(&self, operation: Uuid) -> Result<()> {
        self.state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?
            .pending_external_effects
            .remove(&operation);
        Ok(())
    }

    /// A bounded synchronous host effect shares the lifecycle gate. Network
    /// providers use reserve_remote/launch_remote/complete_remote instead so
    /// preflight and response waits never retain the control mutex.
    pub fn mutate(
        &self,
        request: MutationRequest,
        permit: Option<&Permit>,
        required_operator_role: Option<&str>,
        fresh_gate: impl FnOnce() -> Result<()>,
        effect: impl FnOnce(&Receipt) -> Result<(BTreeMap<String, String>, Value)>,
    ) -> Result<Receipt> {
        self.mutate_observed(
            request,
            permit,
            required_operator_role,
            |_| fresh_gate(),
            effect,
        )
    }

    /// Supplies the same locked lifecycle observation used by admission. The
    /// callback must not reenter this host; it may inspect external facts and
    /// configuration using the host-before-config lock order.
    pub fn mutate_observed(
        &self,
        request: MutationRequest,
        permit: Option<&Permit>,
        required_operator_role: Option<&str>,
        fresh_gate: impl FnOnce(&ControlSnapshot) -> Result<()>,
        effect: impl FnOnce(&Receipt) -> Result<(BTreeMap<String, String>, Value)>,
    ) -> Result<Receipt> {
        self.mutate_synchronous(
            request,
            permit,
            required_operator_role,
            fresh_gate,
            effect,
            false,
        )
    }

    /// Configuration-only monotone local restrictions must remain recordable
    /// even while unknown external effects hold a runtime target. This bypass
    /// grants no provider effect, worker permit, or permission restoration.
    pub(crate) fn record_permission_reduction(
        &self,
        request: MutationRequest,
        fresh_gate: impl FnOnce(&ControlSnapshot) -> Result<()>,
        persist: impl FnOnce(&Receipt) -> Result<(BTreeMap<String, String>, Value)>,
    ) -> Result<Receipt> {
        if request.kind != "permission_fence"
            || request.canonical_targets != vec![request.runtime_id.to_string()]
        {
            return Err(Error::new(
                "scope_violation",
                "local permission reduction requires exact runtime fence target",
            ));
        }
        self.mutate_synchronous(request, None, Some("configure"), fresh_gate, persist, true)
    }

    fn mutate_synchronous(
        &self,
        mut request: MutationRequest,
        permit: Option<&Permit>,
        required_operator_role: Option<&str>,
        fresh_gate: impl FnOnce(&ControlSnapshot) -> Result<()>,
        effect: impl FnOnce(&Receipt) -> Result<(BTreeMap<String, String>, Value)>,
        permission_reduction: bool,
    ) -> Result<Receipt> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if request.runtime_id != s.intent.runtime_id {
            return Err(Error::new("identity_mismatch", "wrong mutation runtime"));
        }
        if s.service_stopping {
            return Err(Error::new(
                "service_stopping",
                "control host shutdown is already fenced",
            ));
        }
        match (permit, required_operator_role) {
            (Some(permit), None) => {
                if s.lease.as_ref() != Some(&permit.lease)
                    || s.fenced.contains(&permit.operation_id)
                    || s.permits.get(&permit.operation_id).is_none_or(|p| {
                        p.scope_digest != permit.scope_digest || p.work_key != permit.work_key
                    })
                {
                    return Err(Error::new(
                        "access_denied",
                        "autonomous mutation has no current registered permit",
                    ));
                }
            }
            (None, Some(role)) => request.actor.require(role)?,
            _ => {
                return Err(Error::new(
                    "access_denied",
                    "mutation must have exactly one authenticated operator or autonomous authority",
                ));
            }
        }
        if let Some(permit) = permit {
            request
                .expected
                .insert("admitted_work".into(), permit.work_key.clone());
            request.expected.insert(
                "admission_operation".into(),
                permit.operation_id.to_string(),
            );
            request
                .expected
                .insert("admitted_scope".into(), permit.scope_digest.clone());
        }
        let generation = s.lease.as_ref().map_or(0, |lease| lease.owner_generation);
        let (mut receipt, fresh) =
            self.prepare_indexed_effect(&mut s, &request, permit, generation)?;
        self.register_indexed_effect(&mut s, permit, &receipt, fresh)?;
        if !fresh {
            if let Some(error) = receipt
                .observed_result
                .as_ref()
                .and_then(|value| value.get("error"))
            {
                return Err(serde_json::from_value(error.clone()).map_err(|_| {
                    Error::new("record_corrupt", "invalid stored mutation rejection")
                })?);
            }
            return Ok(receipt);
        }
        let held = s.holds.iter().any(|(id, targets)| {
            *id != request.request_id
                && targets.iter().any(|target| {
                    request.canonical_targets.contains(target)
                        || permit.is_some_and(|p| &p.work_key == target)
                        || target == &s.intent.runtime_id.to_string()
                })
        });
        let gate = if held && !permission_reduction {
            Err(Error::new(
                "reconciliation_required",
                "an unresolved effect holds an overlapping canonical target",
            ))
        } else {
            fresh_gate(&s.snapshot())
        };
        if let Err(error) = gate {
            receipt.phase = ReceiptPhase::Reconciled;
            receipt.observed_result = Some(serde_json::json!({"effect":"not_sent","error":error}));
            self.save_indexed_receipt(&mut s, &receipt)?;
            return Err(error);
        }
        self.send_indexed_receipt(&mut s, &mut receipt)?;
        let mut held_targets = receipt.canonical_targets.clone();
        if let Some(permit) = permit {
            held_targets.push(permit.work_key.clone());
        }
        s.holds.insert(receipt.operation_id, held_targets);
        let (ids, result) = match effect(&receipt) {
            Ok(observed) => observed,
            Err(error) => {
                // Transmission was durably recorded. Even a transport failure does
                // not establish absence of an external effect.
                return Err(Error::new("operation_uncertain","provider mutation outcome requires reconciliation").detail(serde_json::json!({"operation_id":receipt.operation_id,"request_id":receipt.request_id,"cause":error.code})));
            }
        };
        self.verify_indexed_receipt(&mut s, &receipt)?;
        if let Err(error) = self.acknowledge_indexed_receipt(&mut s, &mut receipt, ids, result) {
            s.open = false;
            s.error = Some(error.code.clone());
            return Err(error);
        }
        s.holds.remove(&receipt.operation_id);
        Ok(receipt)
    }

    /// Reserve local service shutdown under the lifecycle/admission mutex. The
    /// durable platform reservation prevents launchd recovery from reversing it.
    pub fn stop_service(
        &self,
        request: MutationRequest,
        verify_and_persist: impl FnOnce() -> Result<()>,
    ) -> Result<Receipt> {
        request.actor.require("lifecycle")?;
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if request.runtime_id != s.intent.runtime_id {
            return Err(Error::new("identity_mismatch", "wrong service runtime"));
        }
        let mut request = request;
        request.kind = "service_stop".into();
        request.canonical_targets = vec![s.intent.runtime_id.to_string()];
        request.payload = serde_json::json!({"service":"stop"});
        self.guard_indexed_request_id(&mut s, request.request_id)?;
        let (mut receipt, fresh) = self.receipts.prepare(&request, 0)?;
        if !fresh {
            if let Some(error) = receipt
                .observed_result
                .as_ref()
                .and_then(|v| v.get("error"))
            {
                return Err(serde_json::from_value(error.clone()).map_err(|_| {
                    Error::new("record_corrupt", "invalid stored service rejection")
                })?);
            }
            if matches!(receipt.phase, ReceiptPhase::Prepared | ReceiptPhase::Sent) {
                return Err(Error::new(
                    "operation_uncertain",
                    "prior service shutdown has no confirmed result; reconcile before retrying",
                )
                .detail(serde_json::json!({"operation_id":receipt.operation_id})));
            }
            return Ok(receipt);
        }
        // Cold recovery restores unfinished admissions and suspended owners as
        // holds, not live handles. Do not mistake empty process maps for their
        // exit. Remote-only holds may remain after actual local retirement.
        let local_ownership_pending = || -> Result<bool> {
            for id in s.holds.keys() {
                if matches!(
                    self.receipts.get(*id)?.kind.as_str(),
                    "admission" | "owner_suspension"
                ) {
                    return Ok(true);
                }
            }
            Ok(false)
        };
        let check = if s.error.is_some() {
            Err(Error::new(
                "reconciliation_required",
                "service shutdown requires reconciled durable control state",
            ))
        } else if s.intent.desired != Intent::Stopped
            || s.alive
            || !s.permits.is_empty()
            || !s.background_commands.is_empty()
            || !s.pending_external_effects.is_empty()
            || !s.local_processes.is_empty()
        {
            Err(Error::new(
                "stop_in_progress",
                "service shutdown requires STOPPED and no active owned local work",
            ))
        } else {
            match local_ownership_pending() {
                Ok(true) => Err(Error::new(
                    "stop_in_progress",
                    "service shutdown requires reconciliation of prior local ownership",
                )),
                Ok(false) => verify_and_persist(),
                Err(error) => Err(error),
            }
        };
        if let Err(error) = check {
            receipt.phase = ReceiptPhase::Reconciled;
            receipt.observed_result = Some(serde_json::json!({"error":error}));
            self.receipts.save(&receipt)?;
            return Err(error);
        }
        s.open = false;
        s.service_stopping = true;
        receipt.phase = ReceiptPhase::Reconciled;
        receipt.observed_result =
            Some(serde_json::json!({"service":"stopping","desired":s.intent.desired}));
        self.receipts.save(&receipt)?;
        Ok(receipt)
    }

    pub fn reconcile_operation(
        &self,
        actor: &Actor,
        id: Uuid,
        read_provider: impl FnOnce(&Receipt) -> Result<Reconciliation>,
    ) -> Result<Receipt> {
        self.reconcile_operation_observed(actor, id, |receipt, _| read_provider(receipt))
    }

    /// Copy the already reconciled requested receipt into its operator command
    /// result. Other unresolved effects cannot block this metadata-only result,
    /// and this closed method cannot reconcile them or transmit an effect.
    pub(crate) fn record_operation_reconciliation_completion(
        &self,
        request: MutationRequest,
        parent: &Receipt,
        result: Value,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Receipt> {
        let backend: Receipt = serde_json::from_value(result["receipt"].clone()).map_err(|_| {
            Error::new(
                "missing_evidence",
                "reconciliation needs its actual target receipt",
            )
        })?;
        let operation = request.payload["operation"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(|| Error::invalid("reconciliation target is invalid"))?;
        let targets = vec![operation.to_string()];
        if request.kind != "decision/OperationReconcile"
            || operation == parent.operation_id
            || operation == request.request_id
            || backend.operation_id != operation
            || result["reconciled_operation"] != serde_json::json!(operation)
            || request.canonical_targets != targets
            || request.expected.get("parent_command_id") != Some(&parent.operation_id.to_string())
            || request.expected.get("parent_payload_digest") != Some(&parent.payload_digest)
        {
            return Err(Error::new(
                "scope_violation",
                "completion differs from the requested reconciliation",
            ));
        }
        self.mutate_synchronous(
            request.clone(),
            None,
            Some("recover"),
            |snapshot| {
                let command = self.receipts.get(parent.operation_id)?;
                let actual = self.receipts.get(operation)?;
                if command.runtime_id != request.runtime_id
                    || command.kind != "command/OperationReconcile"
                    || command.actor != request.actor.identity
                    || command.payload_digest != parent.payload_digest
                    || command.canonical_targets != targets
                    || command.phase != ReceiptPhase::Prepared
                    || !snapshot.background_commands.contains(&command.operation_id)
                    || actual.runtime_id != request.runtime_id
                    || canonical_digest(1, &actual)? != canonical_digest(1, &backend)?
                {
                    return Err(Error::new(
                        "ownership_conflict",
                        "reconciliation result differs from its owned command or actual target",
                    ));
                }
                if actual.phase != ReceiptPhase::Reconciled || actual.observed_result.is_none() {
                    return Err(Error::new(
                        "missing_evidence",
                        "reconciliation target has no durable terminal observation",
                    ));
                }
                fresh(snapshot)
            },
            |_| {
                Ok((
                    BTreeMap::new(),
                    serde_json::json!({"reconciled_operation":operation,"receipt":backend}),
                ))
            },
            true,
        )
    }
    /// Apply a previously collected provider observation only after rechecking
    /// the current actor/configuration/lifecycle against this serialized snapshot.
    /// The callback must not wait for provider I/O or reacquire ControlHost.
    pub fn reconcile_operation_observed(
        &self,
        actor: &Actor,
        id: Uuid,
        observe: impl FnOnce(&Receipt, &ControlSnapshot) -> Result<Reconciliation>,
    ) -> Result<Receipt> {
        actor.require("recover")?;
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if s.background_commands.contains(&id)
            || s.pending_external_effects.contains(&id)
            || s.local_processes.contains(&id)
        {
            return Err(Error::new(
                "operation_active",
                "wait for the owned background command to reach a terminal observation before reconciliation",
            ));
        }
        let receipt = self.receipts.get(id)?;
        let observation = observe(&receipt, &s.snapshot())?;
        let observation = self.indexed_reconciliation(&mut s, &receipt, observation)?;
        let reconciled = self.receipts.reconcile(id, observation)?;
        s.holds.remove(&id);
        Ok(reconciled)
    }
}

/// Host-local process capability. It is neither a scheduler lease nor an agent
/// permit, and cannot be reconstructed from its retained identity-only receipt.
pub(crate) struct HostProcessToken {
    pub(crate) operation: Uuid,
    digest: String,
    owner_generation: u64,
}
impl ControlHost {
    /// Own the explicit operator cleanup across asynchronous inspection/hooks.
    /// The runtime target hold prevents new model/preparation admissions while
    /// the control mutex remains available to status and force-stop requests.
    pub(crate) fn begin_cleanup_effect(
        &self,
        request: MutationRequest,
        current: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<(HostProcessToken, Receipt)> {
        request.actor.require("recover")?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "cleanup serialization failed"))?;
        if request.kind != "cleanup"
            || request.runtime_id != state.intent.runtime_id
            || !request
                .canonical_targets
                .contains(&request.runtime_id.to_string())
            || state.service_stopping
            || state.error.is_some()
        {
            return Err(Error::new(
                "admission_closed",
                "cleanup cannot enter this runtime",
            ));
        }
        current(&state.snapshot())?;
        self.guard_indexed_request_id(&mut state, request.request_id)?;
        let (mut receipt, fresh) = self.receipts.prepare(&request, state.intent.generation)?;
        if !fresh {
            return Err(Error::new(
                "operation_uncertain",
                "cleanup already has a retained receipt; reconcile before any new action",
            ));
        }
        self.receipts.mark_sent(&mut receipt)?;
        state
            .holds
            .insert(receipt.operation_id, receipt.canonical_targets.clone());
        state.local_processes.insert(receipt.operation_id);
        let token = HostProcessToken {
            operation: receipt.operation_id,
            digest: receipt.payload_digest.clone(),
            owner_generation: receipt.owner_generation,
        };
        Ok((token, receipt))
    }
    pub(crate) fn begin_host_process(
        &self,
        request: MutationRequest,
        owner_generation: u64,
        current: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<HostProcessToken> {
        if !matches!(request.kind.as_str(), "host_metadata" | "host_cleanup")
            || request.canonical_targets.is_empty()
            || request.canonical_targets.len() > 128
            || request.actor.principal.is_some()
            || !request.actor.roles.is_empty()
        {
            return Err(Error::new(
                "access_denied",
                "invalid host process admission",
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "host process serialization failed"))?;
        if state.service_stopping
            || state.error.is_some()
            || request.runtime_id != state.intent.runtime_id
        {
            return Err(Error::new(
                "admission_closed",
                "host process admission is unavailable",
            ));
        }
        current(&state.snapshot())?;
        self.guard_indexed_request_id(&mut state, request.request_id)?;
        let (mut receipt, fresh) = self.receipts.prepare(&request, owner_generation)?;
        if !fresh {
            return Err(Error::new(
                "operation_uncertain",
                "host process already has a receipt; never relaunch the same ID",
            ));
        }
        self.receipts.mark_sent(&mut receipt)?;
        state
            .holds
            .insert(receipt.operation_id, receipt.canonical_targets.clone());
        state.local_processes.insert(receipt.operation_id);
        Ok(HostProcessToken {
            operation: receipt.operation_id,
            digest: receipt.payload_digest,
            owner_generation,
        })
    }
    /// Fence only the still-owned host effect. A rejected call after completion
    /// must not recreate cancellation state for an already acknowledged receipt.
    pub(crate) fn fence_host_process(&self, token: &HostProcessToken) -> Result<bool> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "host process serialization failed"))?;
        if !state.local_processes.contains(&token.operation) {
            return Ok(false);
        }
        let receipt = self.receipts.get(token.operation)?;
        if receipt.phase != ReceiptPhase::Sent
            || receipt.payload_digest != token.digest
            || receipt.owner_generation != token.owner_generation
        {
            return Err(Error::new(
                "ownership_conflict",
                "host cancellation differs from original process authority",
            ));
        }
        state.fenced.insert(token.operation);
        Ok(true)
    }
    pub(crate) fn with_host_process<T>(
        &self,
        token: &HostProcessToken,
        current: impl FnOnce(&ControlSnapshot) -> Result<()>,
        launch: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "host process serialization failed"))?;
        let receipt = self.receipts.get(token.operation)?;
        if !state.local_processes.contains(&token.operation)
            || state.fenced.contains(&token.operation)
            || state.service_stopping
            || state.error.is_some()
            || receipt.phase != ReceiptPhase::Sent
            || receipt.payload_digest != token.digest
            || receipt.owner_generation != token.owner_generation
        {
            return Err(Error::new(
                "access_denied",
                "host process receipt is stale or fenced",
            ));
        }
        current(&state.snapshot())?;
        launch()
    }
    /// Caller holds the original private token and has joined every actual
    /// process launched through it. Recovered records never mint this token.
    pub(crate) fn complete_host_process(
        &self,
        token: &HostProcessToken,
        current_owner: impl FnOnce() -> Result<()>,
        observed: Value,
    ) -> Result<Receipt> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "host process serialization failed"))?;
        current_owner()?;
        let mut receipt = self.receipts.get(token.operation)?;
        if !state.local_processes.contains(&token.operation)
            || receipt.phase != ReceiptPhase::Sent
            || receipt.payload_digest != token.digest
            || receipt.owner_generation != token.owner_generation
        {
            return Err(Error::new(
                "ownership_conflict",
                "host process completion differs from its owned admission",
            ));
        }
        self.receipts
            .acknowledge(&mut receipt, BTreeMap::new(), observed)?;
        state.local_processes.remove(&token.operation);
        state.holds.remove(&token.operation);
        state.fenced.remove(&token.operation);
        Ok(receipt)
    }
}

// Typed round provenance shared by Control and native scheduling.
#[path = "control_review_round.rs"]
mod review_round;
pub(crate) use review_round::{
    FindingsReference, PassedValidationReference, RepairValidationLineage,
    RepairValidationReference, ReviewReceiptReference, ReviewRoundCause, ReviewRoundReference,
    ReviewedSourceReference, TransientFailureCode,
};

// Atomic round admission uses the same Control lock and durable receipt writer.
#[path = "control_review_round_admission.rs"]
mod round_admission;
pub(crate) use round_admission::{ReviewRoundRunCause, ReviewRoundRunProvenance};
use round_admission::{
    RoundPreparationCheck, RoundRecoveryContext, RoundResumeCheck, RoundRunCheck,
};
pub(crate) use round_admission::{RoundRecoveryProvenance, RoundResumeProvenance};
#[path = "control_review_round_recovery.rs"]
mod round_recovery;
pub(crate) use round_recovery::{
    PairedResumeResultReference, ReviewRoundRecoveryReference, ReviewRoundResumeReference,
};
