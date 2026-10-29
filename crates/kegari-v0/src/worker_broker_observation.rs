//! Same-owner diagnostic data. Neither these DTOs nor a protected journal read
//! construct a permit, joined worker, remote outcome or complete effect index.
use super::*;
use crate::control::{ControlSnapshot, ReceiptPhase};
use crate::evaluation::IndependentBoundaryIdentity;
use std::path::Component;

const CAPTURE_BYTES: usize = 8 * 1024 * 1024;
const RECORD_BYTES: usize = 128 * 1024;

#[derive(Default)]
pub(crate) struct CaptureBudget(Mutex<usize>);
impl CaptureBudget {
    pub(crate) fn reserve(&self, bytes: usize) -> Result<()> {
        let mut used = self
            .0
            .lock()
            .map_err(|_| Error::new("observation_unavailable", "capture budget unavailable"))?;
        if bytes > RECORD_BYTES
            || used
                .checked_add(bytes)
                .is_none_or(|sum| sum > CAPTURE_BYTES)
        {
            return Err(Error::new(
                "observation_limit",
                "shared declared capture byte budget exhausted",
            ));
        }
        // An uncertain write consumes capacity; observing again never renews it.
        *used += bytes;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerJournalReference {
    pub leaf: String,
    pub kind: String,
    pub digest: String,
    pub bytes: u64,
}
impl BrokerJournalReference {
    fn verify(&self, journal: &Journal) -> Result<()> {
        let value: Value = journal
            .dir
            .read_bounded(&self.leaf, self.bytes.saturating_add(1024).min(256 * 1024))?;
        if value["kind"].as_str() != Some(&self.kind)
            || canonical_digest(1, &value)? != self.digest
            || serde_json::to_vec(&value)
                .map_err(|_| Error::invalid("journal observation encoding"))?
                .len() as u64
                != self.bytes
        {
            return Err(Error::new(
                "observation_conflict",
                "original worker journal bytes changed",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum BrokerSemantic {
    Context {
        key: String,
        origin: String,
        digest: String,
        bytes: u64,
    },
    Source {
        requested_path: String,
        path: String,
        digest: String,
        bytes: u64,
        returned_value_digest: String,
    },
    FeedbackIntent {
        target: ReviewTarget,
        body_digest: String,
        body_bytes: u64,
        request_identity: String,
    },
    PendingProposal {
        proposal: Uuid,
        claim_digest: String,
    },
    PendingClaim {
        proposal: Uuid,
        reference_digest: String,
    },
}
impl BrokerSemantic {
    fn validate_for(&self, operation: Uuid) -> Result<()> {
        let invalid = || {
            Error::new(
                "observation_limit",
                "broker semantic data exceeds its diagnostic schema",
            )
        };
        match self {
            BrokerSemantic::Context {
                key,
                origin,
                digest,
                bytes,
            } => {
                if key.is_empty()
                    || key.len() > 512
                    || !matches!(
                        origin.as_str(),
                        "owner_correction_state" | "contract_context"
                    )
                    || *bytes > MAX_TOOL_BYTES as u64
                {
                    return Err(invalid());
                }
                crate::evidence::validate_digest(digest)?;
            }
            BrokerSemantic::Source {
                requested_path,
                path,
                digest,
                bytes,
                returned_value_digest,
            } => {
                if requested_path.is_empty()
                    || requested_path.len() > 4096
                    || path.is_empty()
                    || path.len() > 4096
                    || Path::new(path).is_absolute()
                    || Path::new(path)
                        .components()
                        .any(|c| !matches!(c, Component::Normal(_)))
                    || *bytes > 48000
                {
                    return Err(invalid());
                }
                crate::evidence::validate_digest(digest)?;
                crate::evidence::validate_digest(returned_value_digest)?;
            }
            BrokerSemantic::FeedbackIntent {
                target,
                body_digest,
                body_bytes,
                request_identity,
            } => {
                if target.repository.id == 0
                    || target.pull_request == 0
                    || *body_bytes > MAX_TOOL_BYTES as u64
                {
                    return Err(invalid());
                }
                crate::evidence::validate_digest(body_digest)?;
                crate::evidence::validate_digest(request_identity)?;
            }
            BrokerSemantic::PendingProposal {
                proposal,
                claim_digest,
            } => {
                if proposal != &operation {
                    return Err(invalid());
                }
                crate::evidence::validate_digest(claim_digest)?;
            }
            BrokerSemantic::PendingClaim {
                proposal,
                reference_digest,
            } => {
                if proposal != &operation {
                    return Err(invalid());
                }
                crate::evidence::validate_digest(reference_digest)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerSemanticReference {
    pub observation: BrokerSemantic,
    pub journal: BrokerJournalReference,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerMember {
    pub operation: Uuid,
    pub thread: String,
    pub turn: String,
    pub call: String,
    pub tool: String,
    pub argument_digest: String,
    pub started: BrokerJournalReference,
    pub finished: Option<BrokerJournalReference>,
    pub result_digest: Option<String>,
    pub result_error: Option<String>,
    pub semantic: Vec<BrokerSemanticReference>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerReceiptState {
    Unknown,
    Prepared,
    Sent,
    AcknowledgedOnly,
    NotSent,
    ReconciledUnknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerReceiptObservation {
    pub operation: Uuid,
    pub request_identity: String,
    pub state: BrokerReceiptState,
    pub held: bool,
    pub pending: bool,
    pub snapshot: Option<BrokerJournalReference>,
    pub error: Option<Error>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerObservationState {
    Partial,
    Unavailable,
    Overflow,
    Conflict,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndependentBrokerObservation {
    pub schema_version: u32,
    pub operation: Uuid,
    pub runtime: Uuid,
    pub workspace: Uuid,
    pub trial_digest: String,
    pub suite_digest: String,
    pub fixture_digest: String,
    pub instruction_digest: String,
    pub contract_scope_digest: String,
    pub context_digest: String,
    pub tool_contract_digest: String,
    pub isolation_digest: String,
    pub state: BrokerObservationState,
    pub members: Vec<BrokerMember>,
    pub receipts: Vec<BrokerReceiptObservation>,
    pub authority_at_collection: Option<IndependentBoundaryIdentity>,
    pub gaps: Vec<String>,
}
impl IndependentBrokerObservation {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.operation.is_nil()
            || self.runtime.is_nil()
            || self.workspace.is_nil()
            || self.members.len() > MAX_CALLS
            || self.receipts.len() > MAX_CALLS
            || self.gaps.is_empty()
            || self.gaps.len() > 32
        {
            return Err(Error::new(
                "observation_conflict",
                "invalid bounded broker diagnostic",
            ));
        }
        for digest in [
            &self.trial_digest,
            &self.suite_digest,
            &self.fixture_digest,
            &self.instruction_digest,
            &self.contract_scope_digest,
            &self.context_digest,
            &self.tool_contract_digest,
            &self.isolation_digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        let mut ids = BTreeSet::new();
        for member in &self.members {
            if !ids.insert(member.operation)
                || member.operation.is_nil()
                || member.semantic.len() > 4
                || [&member.thread, &member.turn, &member.call, &member.tool]
                    .iter()
                    .any(|s| s.is_empty() || s.len() > 512)
            {
                return Err(Error::new(
                    "observation_conflict",
                    "invalid broker member identity",
                ));
            }
            crate::evidence::validate_digest(&member.argument_digest)?;
        }
        let invalid = || {
            Error::new(
                "observation_conflict",
                "malformed broker diagnostic reference",
            )
        };
        let reference = |r: &BrokerJournalReference, kind: &str| -> Result<()> {
            if r.kind != kind
                || r.bytes == 0
                || r.bytes > 256 * 1024
                || r.leaf.len() > 128
                || !r.leaf.ends_with(".json")
                || r.leaf.contains('/')
                || r.leaf.contains('\\')
                || r.leaf.contains("..")
            {
                return Err(invalid());
            }
            crate::evidence::validate_digest(&r.digest)
        };
        for member in &self.members {
            reference(&member.started, "tool_started")?;
            if let Some(r) = &member.finished {
                reference(r, "tool_finished")?;
            }
            if let Some(digest) = &member.result_digest {
                crate::evidence::validate_digest(digest)?;
            }
            if member.result_error.as_ref().is_some_and(|s| s.len() > 128) {
                return Err(invalid());
            }
            for semantic in &member.semantic {
                reference(&semantic.journal, "broker_observation")?;
                semantic.observation.validate_for(member.operation)?;
            }
        }
        let mut receipts = BTreeSet::new();
        for r in &self.receipts {
            if !ids.contains(&r.operation) || !receipts.insert(r.operation) {
                return Err(invalid());
            }
            crate::evidence::validate_digest(&r.request_identity)?;
            if let Some(snapshot) = &r.snapshot {
                reference(snapshot, "broker_receipt_snapshot")?;
            }
            if (r.pending || r.held || r.error.is_some())
                && !matches!(r.state, BrokerReceiptState::Unknown)
            {
                return Err(invalid());
            }
        }
        if self
            .gaps
            .iter()
            .any(|gap| gap.is_empty() || gap.len() > 128)
        {
            return Err(invalid());
        }
        if let Some(identity) = &self.authority_at_collection {
            let boundary = crate::evaluation::IndependentBoundaryObservation {
                schema_version: 1,
                source_revision: identity.source_revision.clone(),
                candidate_digest: self.instruction_digest.clone(),
                check: crate::evaluation::IndependentBoundaryCheck::MatchedAtCollection {
                    identity: Box::new(identity.clone()),
                },
            };
            boundary.validate()?;
            if identity.operation != self.operation
                || identity.runtime != self.runtime
                || identity.workspace != self.workspace
                || identity.instruction_manifest_digest != self.instruction_digest
                || identity.contract_scope_digest != self.contract_scope_digest
                || identity.context_digest != self.context_digest
                || identity.tool_contract_digest != self.tool_contract_digest
                || identity.isolation_digest != self.isolation_digest
            {
                return Err(invalid());
            }
        }
        if serde_json::to_vec(self).map_err(|_| invalid())?.len() > CAPTURE_BYTES {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone)]
struct OriginalRequest {
    operation: Uuid,
    identity: String,
}
/// Private-field, same-process inventory. Descriptive JSON is never its input.
pub struct BrokerInventory {
    permit_digest: String,
    contract: Arc<WorkerContract>,
    requests: Vec<OriginalRequest>,
}
/// The host returns actual observations; a generic host defaults to unavailable.
pub struct BrokerControlBatch {
    pub(crate) authority: IndependentBoundaryIdentity,
    receipts: Vec<ReceiptRead>,
}
impl BrokerInventory {
    pub(crate) fn verify(&self, permit: &Permit, contract: &Arc<WorkerContract>) -> Result<()> {
        if !Arc::ptr_eq(&self.contract, contract)
            || self.permit_digest != canonical_digest(1, permit)?
            || self.requests.len() > MAX_CALLS
        {
            return Err(Error::new(
                "worker_binding",
                "broker inventory differs from actual original worker",
            ));
        }
        Ok(())
    }
    /// Called only inside the actual registered owner's Control snapshot gate.
    pub(crate) fn observe_in_gate(
        &self,
        control: &ControlHost,
        snapshot: &ControlSnapshot,
        authority: IndependentBoundaryIdentity,
    ) -> BrokerControlBatch {
        let receipts = observe_requests(control, snapshot, &self.requests);
        BrokerControlBatch {
            authority,
            receipts,
        }
    }
}

struct ReceiptRead {
    original: OriginalRequest,
    held: bool,
    pending: bool,
    receipt: Result<crate::control::Receipt>,
}
fn observe_requests(
    control: &ControlHost,
    snapshot: &ControlSnapshot,
    requests: &[OriginalRequest],
) -> Vec<ReceiptRead> {
    // Bounded working copy, not a second capture allowance. Snapshot writes
    // still consume the one shared protocol/broker budget outside Control.
    let mut retained_bytes = 0usize;
    requests
        .iter()
        .map(|request| {
            let read = if retained_bytes >= CAPTURE_BYTES {
                Err(Error::new(
                    "observation_limit",
                    "broker receipt batch is bounded",
                ))
            } else {
                control.operation(request.operation).and_then(|receipt| {
                    if crate::control::receipt_request_digest(&receipt)? != request.identity {
                        return Err(Error::new(
                            "observation_conflict",
                            "original normalized broker request changed",
                        ));
                    }
                    let bytes = serde_json::to_vec(&receipt)
                        .map_err(|_| Error::invalid("broker receipt encoding"))?
                        .len();
                    retained_bytes = retained_bytes.saturating_add(bytes);
                    if retained_bytes > CAPTURE_BYTES {
                        return Err(Error::new(
                            "observation_limit",
                            "broker receipt batch is bounded",
                        ));
                    }
                    Ok(receipt)
                })
            };
            ReceiptRead {
                original: request.clone(),
                held: snapshot
                    .unresolved_operations
                    .contains_key(&request.operation),
                pending: snapshot
                    .pending_external_effects
                    .contains(&request.operation)
                    || snapshot
                        .pending_local_processes
                        .contains(&request.operation),
                receipt: read,
            }
        })
        .collect()
}
fn receipt_state(
    receipt: &crate::control::Receipt,
    held: bool,
    pending: bool,
) -> BrokerReceiptState {
    if held || pending {
        return BrokerReceiptState::Unknown;
    }
    match receipt.phase {
        ReceiptPhase::Prepared => BrokerReceiptState::Prepared,
        ReceiptPhase::Sent => BrokerReceiptState::Sent,
        ReceiptPhase::Acknowledged => BrokerReceiptState::AcknowledgedOnly,
        ReceiptPhase::Reconciled
            if receipt
                .observed_result
                .as_ref()
                .is_some_and(|v| v["effect"] == "not_sent") =>
        {
            BrokerReceiptState::NotSent
        }
        ReceiptPhase::Reconciled => BrokerReceiptState::ReconciledUnknown,
    }
}
fn promote(state: &mut BrokerObservationState, code: &str) {
    if matches!(state, BrokerObservationState::Conflict) {
        return;
    }
    if matches!(code, "observation_conflict" | "request_conflict") {
        *state = BrokerObservationState::Conflict;
    } else if code == "observation_limit" {
        *state = BrokerObservationState::Overflow;
    } else if !matches!(state, BrokerObservationState::Overflow) {
        *state = BrokerObservationState::Unavailable;
    }
}

pub(super) struct BrokerCapture {
    report: IndependentBrokerObservation,
    requests: Vec<OriginalRequest>,
    budget: Arc<CaptureBudget>,
}
impl BrokerCapture {
    pub(super) fn new(
        permit: &Permit,
        contract: &WorkerContract,
        declaration: &crate::evaluation::DeclaredProtocolTrial,
        budget: Arc<CaptureBudget>,
    ) -> Option<Self> {
        Some(Self {
            report: IndependentBrokerObservation {
                schema_version: 1,
                operation: permit.operation_id,
                runtime: permit.lease.runtime_id,
                workspace: contract.authority.workspace(),
                trial_digest: declaration.trial_digest()?.into(),
                suite_digest: declaration.suite_digest().into(),
                fixture_digest: declaration.fixture_digest().into(),
                instruction_digest: contract.assembled.manifest_digest.clone(),
                contract_scope_digest: contract.scope_digest.clone(),
                context_digest: contract.assembled.manifest.context_digest.clone(),
                tool_contract_digest: contract.tools.digest().into(),
                isolation_digest: contract.isolation.digest().into(),
                state: BrokerObservationState::Partial,
                members: vec![],
                receipts: vec![],
                authority_at_collection: None,
                gaps: vec![
                    "same_owner_broker_subset_only".into(),
                    "rejected_protocol_attempts_remain_separate".into(),
                    "transport_and_remote_readback_incomplete".into(),
                ],
            },
            requests: vec![],
            budget,
        })
    }
    pub(super) fn gap(&mut self, error: &Error) {
        if self.report.gaps.len() < 32 && !self.report.gaps.contains(&error.code) {
            self.report.gaps.push(error.code.clone());
        }
        promote(&mut self.report.state, &error.code);
    }
    pub(super) fn started(
        &mut self,
        id: Uuid,
        call: &ToolCall,
        digest: &str,
        reference: BrokerJournalReference,
    ) {
        let check = self
            .budget
            .reserve(reference.bytes as usize)
            .and_then(|()| {
                if self.report.members.len() >= MAX_CALLS
                    || [&call.thread_id, &call.turn_id, &call.call_id, &call.name]
                        .iter()
                        .any(|s| s.is_empty() || s.len() > 512)
                {
                    Err(Error::new(
                        "observation_limit",
                        "broker member bound exceeded",
                    ))
                } else {
                    Ok(())
                }
            });
        if let Err(error) = check {
            self.gap(&error);
            return;
        }
        self.report.members.push(BrokerMember {
            operation: id,
            thread: call.thread_id.clone(),
            turn: call.turn_id.clone(),
            call: call.call_id.clone(),
            tool: call.name.clone(),
            argument_digest: digest.into(),
            started: reference,
            finished: None,
            result_digest: None,
            result_error: None,
            semantic: vec![],
        });
    }
    pub(super) fn returned(&mut self, id: Uuid, result: &Result<Value>) {
        match canonical_digest(1, result) {
            Ok(digest) => {
                if let Some(member) = self.report.members.iter_mut().find(|m| m.operation == id) {
                    member.result_digest = Some(digest);
                    member.result_error = result.as_ref().err().map(|e| e.code.clone());
                }
            }
            Err(error) => self.gap(&error),
        }
    }
    pub(super) fn finished(&mut self, id: Uuid, reference: BrokerJournalReference) {
        if let Err(error) = self.budget.reserve(reference.bytes as usize) {
            self.gap(&error);
            return;
        }
        if let Some(member) = self.report.members.iter_mut().find(|m| m.operation == id) {
            member.finished = Some(reference);
        }
    }
    pub(super) fn semantic(&mut self, journal: &Journal, id: Uuid, observation: BrokerSemantic) {
        let Some(index) = self.report.members.iter().position(|m| m.operation == id) else {
            return;
        };
        if let Err(error) = observation.validate_for(id) {
            self.gap(&error);
            return;
        }
        if self.report.members[index].semantic.len() >= 4 {
            self.gap(&Error::new(
                "observation_limit",
                "broker semantic bound exceeded",
            ));
            return;
        }
        let result = serde_json::to_value(&observation)
            .map_err(|_| Error::invalid("broker observation encoding"))
            .and_then(|value| {
                let record = json!({"operation":id,"observation":value});
                journal.append_observation("broker_observation", &record, &self.budget)
            });
        match result {
            Ok(reference) => self.report.members[index]
                .semantic
                .push(BrokerSemanticReference {
                    observation,
                    journal: reference,
                }),
            Err(error) => self.gap(&error),
        }
    }
    pub(super) fn feedback(
        &mut self,
        journal: &Journal,
        id: Uuid,
        target: ReviewTarget,
        body: &str,
        request: Result<String>,
    ) {
        if !self.report.members.iter().any(|m| m.operation == id) {
            return;
        }
        match request {
            Ok(identity) => {
                self.requests.push(OriginalRequest {
                    operation: id,
                    identity: identity.clone(),
                });
                self.semantic(
                    journal,
                    id,
                    BrokerSemantic::FeedbackIntent {
                        target,
                        body_digest: bytes_digest(body.as_bytes()),
                        body_bytes: body.len() as u64,
                        request_identity: identity,
                    },
                );
            }
            Err(error) => self.gap(&error),
        }
    }
    fn finish(&self, worker: &ScopedWorker) -> IndependentBrokerObservation {
        let mut report = self.report.clone();
        let mut gaps = Vec::new();
        for member in &report.members {
            for reference in std::iter::once(&member.started)
                .chain(member.finished.iter())
                .chain(member.semantic.iter().map(|s| &s.journal))
            {
                if let Err(error) = reference.verify(&worker.journal) {
                    gaps.push(error);
                }
            }
            if member.finished.is_none() {
                gaps.push(Error::new(
                    "broker_call_pending",
                    "call has no original finished record",
                ));
            }
        }
        let observed = canonical_digest(1, &worker.permit).and_then(|permit_digest| {
            let inventory = BrokerInventory {
                permit_digest,
                contract: worker.contract.clone(),
                requests: self.requests.clone(),
            };
            worker.host.observe_independent_broker(
                &worker.control,
                &worker.permit,
                &worker.contract,
                &inventory,
            )
        });
        match observed {
            Ok(batch) => {
                report.authority_at_collection = Some(batch.authority);
                for ReceiptRead {
                    original,
                    held,
                    pending,
                    receipt: result,
                } in batch.receipts
                {
                    let mut observed = BrokerReceiptObservation {
                        operation: original.operation,
                        request_identity: original.identity,
                        state: BrokerReceiptState::Unknown,
                        held,
                        pending,
                        snapshot: None,
                        error: None,
                    };
                    match result {
                        Ok(receipt) => {
                            observed.state = receipt_state(&receipt, held, pending);
                            let saved = worker
                                .journal
                                .append_observation(
                                    "broker_receipt_snapshot",
                                    &receipt,
                                    &self.budget,
                                )
                                .and_then(|reference| {
                                    reference.verify(&worker.journal)?;
                                    Ok(reference)
                                });
                            match saved {
                                Ok(reference) => observed.snapshot = Some(reference),
                                Err(error) => {
                                    gaps.push(error.clone());
                                    observed.error = Some(error);
                                    observed.state = BrokerReceiptState::Unknown;
                                }
                            }
                        }
                        Err(error) => {
                            gaps.push(error.clone());
                            observed.error = Some(error);
                        }
                    }
                    if held || pending {
                        gaps.push(Error::new(
                            "broker_effect_pending",
                            "owned provider effect remains pending or held",
                        ));
                    }
                    report.receipts.push(observed);
                }
            }
            Err(error) => gaps.push(error),
        }
        for error in gaps {
            if report.gaps.len() < 32 && !report.gaps.contains(&error.code) {
                report.gaps.push(error.code.clone());
            }
            promote(&mut report.state, &error.code);
        }
        report
    }
}
impl ScopedWorker {
    pub(super) fn broker_semantic(&mut self, id: Uuid, observation: BrokerSemantic) {
        if let Some(capture) = &mut self.broker_capture {
            capture.semantic(&self.journal, id, observation);
        }
    }
    pub(super) fn broker_gap(&mut self, error: &Error) {
        if let Some(capture) = &mut self.broker_capture {
            capture.gap(error);
        }
    }
    pub(super) fn broker_observation(&self) -> Option<IndependentBrokerObservation> {
        self.broker_capture.as_ref().map(|c| c.finish(self))
    }
}

#[cfg(test)]
#[path = "worker_broker_observation_tests.rs"]
mod tests;
