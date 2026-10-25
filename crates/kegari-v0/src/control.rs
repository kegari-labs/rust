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
            &serde_json::json!({"kind":request.kind,"targets":request.canonical_targets,"expected":request.expected,"payload":request.payload}),
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
            }),
        })
    }

    pub fn recover(root: PrivateDir, runtime_id: Uuid) -> Result<Self> {
        let lock = root.lock("control.lock")?;
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
        for receipt in receipts.all()? {
            if receipt.runtime_id != runtime_id {
                return Err(Error::new(
                    "identity_mismatch",
                    "receipt belongs to another runtime",
                ));
            }
            if matches!(receipt.phase, ReceiptPhase::Prepared | ReceiptPhase::Sent) {
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
        Ok(ControlSnapshot {
            intent: s.intent.clone(),
            admission_open: s.open,
            reconciled: s.reconciled,
            orchestration_alive: s.alive,
            draining: s.intent.desired != Intent::Running && (s.alive || !s.permits.is_empty()),
            active_operations: s.permits.values().cloned().collect(),
            fenced_operations: s.fenced.iter().copied().collect(),
            persistence_error: s.error.clone(),
            unresolved_operations: s.holds.clone(),
        })
    }

    /// Called only after the OS runtime lock is held and provider/process recovery
    /// has established the supplied lease. Saved STOPPED/PAUSED never admit work.
    pub fn attach_reconciled(&self, lease: Lease) -> Result<()> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if lease.runtime_id != s.intent.runtime_id || lease.owner_generation == 0 {
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
        request.actor.require("lifecycle")?;
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if request.runtime_id != s.intent.runtime_id {
            return Err(Error::new("identity_mismatch", "wrong runtime"));
        }
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
        let rejection = if expected_generation != s.intent.generation {
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
                (Lifecycle::Start, Intent::Stopped) if s.alive || !s.permits.is_empty() => Some(
                    Error::new("stop_in_progress", "owned operations are still draining"),
                ),
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
            let ids: Vec<_> = s.permits.keys().copied().collect();
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
        fresh_gate()?;
        let permit = Permit {
            operation_id: request_id,
            lease: lease.clone(),
            control_generation: s.intent.generation,
            work_key,
            scope_digest,
            registered_at: Utc::now(),
        };
        // A durable reference allows conservative recovery without persisting a shadow issue store.
        let request = MutationRequest {
            request_id,
            runtime_id: lease.runtime_id,
            actor: Actor {
                identity: format!("owner:{}", lease.owner_id),
                roles: BTreeSet::new(),
            },
            kind: "admission".into(),
            canonical_targets: vec![permit.work_key.clone()],
            expected: BTreeMap::from([
                ("scope".into(), permit.scope_digest.clone()),
                ("control".into(), permit.control_generation.to_string()),
            ]),
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
        s.fenced.insert(operation_id);
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
        s.alive = false;
        s.reconciled = false;
        s.lease = None;
        Ok(())
    }
    pub fn operation(&self, id: Uuid) -> Result<Receipt> {
        self.receipts.get(id)
    }

    /// The provider effect runs while the same serialized gate as pause/revocation
    /// is held. Callers use a blocking dispatcher with bounded provider deadlines;
    /// never validate a permit, release the gate, and then perform a mutation.
    pub fn mutate(
        &self,
        mut request: MutationRequest,
        permit: Option<&Permit>,
        required_operator_role: Option<&str>,
        fresh_gate: impl FnOnce() -> Result<()>,
        effect: impl FnOnce(&Receipt) -> Result<(BTreeMap<String, String>, Value)>,
    ) -> Result<Receipt> {
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if request.runtime_id != s.intent.runtime_id {
            return Err(Error::new("identity_mismatch", "wrong mutation runtime"));
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
        let gate = if held {
            Err(Error::new(
                "reconciliation_required",
                "an unresolved effect holds an overlapping canonical target",
            ))
        } else {
            fresh_gate()
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

    pub fn reconcile_operation(
        &self,
        actor: &Actor,
        id: Uuid,
        read_provider: impl FnOnce(&Receipt) -> Result<Reconciliation>,
    ) -> Result<Receipt> {
        actor.require("recover")?;
        let mut s = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        let receipt = self.receipts.get(id)?;
        let observation = read_provider(&receipt)?;
        let reconciled = self.receipts.reconcile(id, observation)?;
        s.holds.remove(&id);
        Ok(reconciled)
    }
}
