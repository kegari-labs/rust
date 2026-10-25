//! Serialized admission and durable operator intent. The host owns this object;
//! the scheduler and workers only receive revocable operation permits.
use crate::{
    digest::canonical_digest,
    error::{Error, Result},
    storage::{ExclusiveLock, PrivateDir},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};
use uuid::Uuid;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub identity: String,
    pub roles: BTreeSet<String>,
    pub principal: Option<Principal>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
pub enum Principal {
    Unix { uid: u32 },
    Tailscale { user_id: u64, node_id: String },
}
impl Actor {
    pub fn require(&self, role: &str) -> Result<()> {
        if self.identity.trim().is_empty() || !self.roles.contains(role) {
            Err(Error::new(
                "access_denied",
                format!("authenticated actor lacks {role} authority"),
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lease {
    pub runtime_id: Uuid,
    pub owner_generation: u64,
    pub owner_id: Uuid,
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

/// Identity-only run-budget record embedded in the atomic admission receipt.
/// It contains no issue text, prompts, credentials or replicated provider state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunReference {
    pub workspace: Uuid,
    pub account: String,
    #[serde(default)]
    pub normalized_state: Option<String>,
    pub phase: crate::scheduler::PhaseKey,
    pub profile_digest: String,
    pub instruction_digest: String,
    pub max_attempts: u32,
    pub max_turns: u32,
    pub max_review_rounds: u32,
    pub attempt: u32,
}
impl RunReference {
    pub fn from_receipt(receipt: &Receipt) -> Result<Option<Self>> {
        receipt
            .expected
            .get("run")
            .map(|encoded| {
                if receipt.kind != "admission" {
                    return Err(Error::new(
                        "record_corrupt",
                        "run budget is attached to a non-admission receipt",
                    ));
                }
                let value: Self = serde_json::from_str(encoded)
                    .map_err(|_| Error::new("record_corrupt", "run budget reference is invalid"))?;
                value.validate()?;
                Ok(value)
            })
            .transpose()
    }
    fn validate(&self) -> Result<()> {
        if self.workspace.is_nil()
            || self.account.is_empty()
            || self.account.len() > 512
            || self.normalized_state.as_ref().is_some_and(|state| {
                state.is_empty() || state.len() > 256 || state.trim().to_lowercase() != *state
            })
            || self.phase.work.is_empty()
            || self.phase.work.len() > 512
            || self.phase.phase.is_empty()
            || self.phase.phase.len() > 64
            || !self
                .phase
                .phase
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
            || self.phase.retry_grant.is_some_and(|id| id.is_nil())
            || [
                self.max_attempts,
                self.max_turns,
                self.max_review_rounds,
                self.attempt,
            ]
            .contains(&0)
            || self.attempt > self.max_attempts
        {
            return Err(Error::new(
                "record_corrupt",
                "run identity or finite phase budget is invalid",
            ));
        }
        for digest in [
            &self.phase.generation,
            &self.profile_digest,
            &self.instruction_digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        Ok(())
    }
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
                if receipt.kind != "admission" || receipt.expected.contains_key("run") {
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
        if self.purpose != "parent_authorization"
            || self.attempt == 0
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
    Run(RunReference),
    Metadata(MetadataReference),
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
}
impl PreparedRemote {
    pub(crate) fn operation_id(&self) -> Uuid {
        self.request.request_id
    }
}
pub(crate) struct InFlightRemote {
    operation: Uuid,
    digest: String,
}

pub struct InFlightEffect {
    request_id: Uuid,
    permit: Permit,
    payload_digest: String,
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
        let digest = canonical_digest(
            1,
            &serde_json::json!({"kind":request.kind,"targets":request.canonical_targets,"expected":request.expected,"payload":request.payload,"principal":request.actor.principal}),
        )?;
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
        input.reference.validate()?;
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
        if let Some(AdmissionReference::Run(reference)) = &reference {
            let mut prior = BTreeMap::new();
            let mut cap = reference.max_attempts;
            for receipt in self.receipts.all()? {
                if let Some(old) = RunReference::from_receipt(&receipt)?
                    && old.phase == reference.phase
                {
                    if old.workspace != reference.workspace
                        || prior.insert(old.attempt, receipt.operation_id).is_some()
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
            if reference.attempt != prior.len() as u32 + 1 || reference.attempt > cap {
                return Err(Error::new(
                    "attempts_exhausted",
                    "admission would reset, duplicate or exceed the durable phase budget",
                ));
            }
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
        // A durable reference allows conservative recovery without persisting a shadow issue store.
        let mut expected = BTreeMap::from([
            ("scope".into(), permit.scope_digest.clone()),
            ("control".into(), permit.control_generation.to_string()),
        ]);
        if let Some(AdmissionReference::Run(reference)) = &reference {
            expected.insert(
                "run".into(),
                serde_json::to_string(&reference)
                    .map_err(|_| Error::invalid("run reference encoding"))?,
            );
        }
        if let Some(AdmissionReference::Metadata(reference)) = reference {
            expected.insert(
                "metadata".into(),
                serde_json::to_string(&reference)
                    .map_err(|_| Error::invalid("metadata reference encoding"))?,
            );
        }
        let request = MutationRequest {
            request_id,
            runtime_id: lease.runtime_id,
            actor: Actor {
                identity: format!("owner:{}", lease.owner_id),
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: "admission".into(),
            canonical_targets: vec![permit.work_key.clone()],
            expected,
            payload: serde_json::to_value(&permit)
                .map_err(|_| Error::invalid("permit encoding"))?,
        };
        let (_, fresh) = self.receipts.prepare(&request, lease.owner_generation)?;
        if !fresh {
            return Err(Error::new(
                "operation_uncertain",
                "existing operation must be reconciled before admitting another",
            ));
        }
        s.permits.insert(request_id, permit.clone());
        Ok(permit)
    }

    pub fn validate_permit(
        &self,
        permit: &Permit,
        fresh_gate: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
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
        fresh_gate()
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
        receipt.observed_result = Some(observed_result);
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
        self.receipts.save(&receipt)?;
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
            serde_json::json!({"local_execution":"exited","unresolved_effects":unresolved,"observation":observation}),
        );
        receipt.updated_at = Utc::now();
        if let Err(error) = self.receipts.save(&receipt) {
            s.open = false;
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
        let (mut receipt, fresh) = self
            .receipts
            .prepare(&request, permit.lease.owner_generation)?;
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
            receipt.phase = ReceiptPhase::Reconciled;
            receipt.observed_result =
                Some(serde_json::json!({"effect":"not_started","error":error}));
            self.receipts.save(&receipt)?;
            return Err(error);
        }
        self.receipts.mark_sent(&mut receipt)?;
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
        let current = validate_live_permit(&s, permit).and_then(|()| fresh_gate());
        let result = match &current {
            Ok(()) => observed,
            Err(error) => {
                serde_json::json!({"effect":"exited_evidence_rejected","error":error,"observed":observed})
            }
        };
        if let Err(error) = self
            .receipts
            .acknowledge(&mut receipt, BTreeMap::new(), result)
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
        let (mut receipt, fresh) = self.receipts.prepare(&request, generation)?;
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
            self.receipts.save(&receipt)?;
            return Err(error);
        }
        if let Err(error) = fresh_gate(&state.snapshot()) {
            receipt.phase = ReceiptPhase::Reconciled;
            receipt.observed_result = Some(serde_json::json!({"effect":"not_sent","error":error}));
            self.receipts.save(&receipt)?;
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
            self.receipts.save(&receipt)?;
            state.holds.remove(&receipt.operation_id);
            state.pending_external_effects.remove(&receipt.operation_id);
            return Err(error);
        }
        if let Err(error) = self.receipts.mark_sent(&mut receipt) {
            state.open = false;
            state.error = Some(error.code.clone());
            return Err(error);
        }
        let task = launch(&receipt);
        Ok((
            InFlightRemote {
                operation: receipt.operation_id,
                digest: receipt.payload_digest,
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
        state.pending_external_effects.remove(&effect.operation);
        let (ids,result)=observed.map_err(|error| Error::new("operation_uncertain", "provider mutation outcome requires reconciliation").detail(serde_json::json!({"operation_id":effect.operation,"request_id":effect.operation,"cause":error.code})))?;
        if let Err(error) = self.receipts.acknowledge(&mut receipt, ids, result) {
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
        let (mut receipt, fresh) = self.receipts.prepare(&request, generation)?;
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
            self.receipts.save(&receipt)?;
            return Err(error);
        }
        self.receipts.mark_sent(&mut receipt)?;
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
        if let Err(error) = self.receipts.acknowledge(&mut receipt, ids, result) {
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
            return Ok(receipt);
        }
        let check = if s.intent.desired != Intent::Stopped
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
            verify_and_persist()
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
