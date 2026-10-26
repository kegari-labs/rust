//! Typed parent baseline-change provenance; never current delivery authority.
//! The native callback must validate the opaque reassembly/assignment and actual
//! joined predecessor. These records independently preserve finite phase bounds.
use super::*;
use crate::evidence::{WorkIdentity, validate_digest};
use crate::scheduler::PhaseKey;

#[path = "control_parent_reassembly_grants.rs"]
mod grants;
pub(crate) use grants::ParentGrantReference;

const KEY: &str = "parent_reassembly";
const PREPARATION: &str = "parent_reassembly_preparation";
const MAX_ENCODED: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParentReassemblyReference {
    pub(crate) schema_version: u32,
    pub(crate) runtime: Uuid,
    pub(crate) workspace: Uuid,
    pub(crate) account: String,
    pub(crate) operation: Uuid,
    pub(crate) parent_work: WorkIdentity,
    pub(crate) worker_work: WorkIdentity,
    #[serde(with = "strict_phase")]
    pub(crate) phase: PhaseKey,
    pub(crate) predecessor: ReviewReceiptReference,
    pub(crate) source: ReviewReceiptReference,
    pub(crate) original_parent: ReviewReceiptReference,
    pub(crate) original_acceptance: ReviewReceiptReference,
    pub(crate) original_token: String,
    pub(crate) vector_digest: String,
    pub(crate) baseline_digest: String,
    pub(crate) assignment_digest: String,
    pub(crate) progress_digest: String,
    pub(crate) inventory_digest: String,
    pub(crate) attempt: u32,
    pub(crate) max_attempts: u32,
    pub(crate) max_turns: u32,
    pub(crate) max_review_rounds: u32,
    /// Old schema-one references serialize identically when no grant exists.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) grants: Vec<ParentGrantReference>,
    /// Exact failed marked predecessor. Only this cause starts a real new budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) retry_of: Option<String>,
    /// A classified known transient failure continues this same grant budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) transient_of: Option<String>,
    /// Flat immutable closure of consumed model/preparation/source receipts.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) history: BTreeMap<Uuid, String>,
}

fn invalid(message: &str) -> Error {
    Error::new("parent_reassembly", message)
}
fn work_key(work: &WorkIdentity) -> Result<String> {
    crate::authority::IssueKey::new(
        &work.linear_endpoint,
        work.linear_workspace,
        work.child.unwrap_or(work.parent),
    )?
    .identity()
}
impl ParentReassemblyReference {
    pub(crate) fn digest(&self) -> Result<String> {
        self.validate()?;
        canonical_digest(1, self)
    }
    pub(crate) fn validate(&self) -> Result<()> {
        self.parent_work.validate()?;
        self.worker_work.validate()?;
        if self.schema_version != 1
            || self.runtime.is_nil()
            || self.workspace.is_nil()
            || self.operation.is_nil()
            || self.account.trim().is_empty()
            || self.account.len() > 512
            || self.parent_work.child.is_some()
            || self.worker_work.parent != self.parent_work.parent
            || self.worker_work.linear_endpoint != self.parent_work.linear_endpoint
            || self.worker_work.linear_workspace != self.parent_work.linear_workspace
            || self.phase.work != work_key(&self.worker_work)?
            || self.phase.generation != canonical_digest(1, &self.worker_work.generation)?
            || !matches!(
                self.phase.phase.as_str(),
                "parent_integrated_validation" | "parent_evidence_review"
            )
            || (self.phase.phase == "parent_integrated_validation")
                != self.worker_work.child.is_some()
            || self.attempt < if self.retry_of.is_some() { 1 } else { 2 }
            || self.attempt > self.max_attempts
            || self.max_turns == 0
            || self.max_review_rounds == 0
        {
            return Err(invalid(
                "invalid exact parent reassembly subject or finite budget",
            ));
        }
        grants::validate_shape(self)?;
        if let Some(prior) = &self.retry_of {
            validate_digest(prior)?;
            if self.attempt != 1 || self.phase.retry_grant.is_none() {
                return Err(invalid(
                    "explicit parent retry must start its actual granted budget",
                ));
            }
        }
        if let Some(prior) = &self.transient_of {
            validate_digest(prior)?;
            if self.retry_of.is_some() || self.phase.retry_grant.is_none() || self.attempt < 2 {
                return Err(invalid(
                    "transient parent retry must remain in one existing granted budget",
                ));
            }
        }
        for (operation, digest) in &self.history {
            if operation.is_nil() || *operation == self.operation {
                return Err(invalid(
                    "parent ancestry contains the prospective operation",
                ));
            }
            validate_digest(digest)?;
        }
        let mut ids = BTreeSet::from([self.operation]);
        for reference in [
            &self.predecessor,
            &self.source,
            &self.original_parent,
            &self.original_acceptance,
        ] {
            if reference.operation.is_nil() || !ids.insert(reference.operation) {
                return Err(invalid("parent reassembly operation identities overlap"));
            }
            validate_digest(&reference.receipt_digest)?;
        }
        for digest in [
            &self.original_token,
            &self.vector_digest,
            &self.baseline_digest,
            &self.assignment_digest,
            &self.progress_digest,
            &self.inventory_digest,
        ] {
            validate_digest(digest)?;
        }
        if serde_json::to_vec(self)
            .map_err(|_| invalid("cannot encode parent reassembly"))?
            .len()
            > MAX_ENCODED
        {
            return Err(invalid(
                "parent reassembly reference exceeds bounded encoding",
            ));
        }
        Ok(())
    }
    pub(crate) fn from_receipt(receipt: &Receipt) -> Result<Option<Self>> {
        let Some(encoded) = receipt.expected.get(KEY) else {
            return Ok(None);
        };
        if encoded.len() > MAX_ENCODED {
            return Err(invalid(
                "parent reassembly reference exceeds bounded encoding",
            ));
        }
        let reference: Self = serde_json::from_str(encoded)
            .map_err(|_| invalid("parent reassembly reference is invalid"))?;
        reference.validate()?;
        let prep = PreparationReference::from_receipt(receipt)?;
        let run = RunReference::from_receipt(receipt)?;
        if receipt.kind != "admission"
            || receipt.runtime_id != reference.runtime
            || receipt.canonical_targets != [reference.phase.work.clone()]
            || [
                "continuation",
                "source_advanced",
                "review_round",
                "review_round_preparation",
                "review_round_resume",
                "review_round_recovery",
                "session_resume",
                "session_recovery_preparation",
                "release_resume",
                "metadata",
            ]
            .iter()
            .any(|key| receipt.expected.contains_key(*key))
            || match (prep, run) {
                (Some(prep), None) => {
                    receipt.expected.contains_key(PREPARATION)
                        || prep.worker_operation != reference.operation
                        || prep.workspace != reference.workspace
                        || prep.account != reference.account
                        || prep.phase != reference.phase
                }
                (None, Some(run)) => {
                    receipt.operation_id != reference.operation
                        || !reference.matches_run(&run)
                        || !receipt.expected.contains_key(PREPARATION)
                }
                _ => true,
            }
        {
            return Err(invalid(
                "parent reassembly marker differs from exact preparation/run",
            ));
        }
        if let Some(encoded) = receipt.expected.get(PREPARATION) {
            if encoded.len() > MAX_ENCODED {
                return Err(invalid("parent preparation link exceeds bounded encoding"));
            }
            let link: ReviewReceiptReference = serde_json::from_str(encoded)
                .map_err(|_| invalid("parent preparation link is invalid"))?;
            validate_digest(&link.receipt_digest)?;
            if link.operation != preparation_operation(reference.operation)? {
                return Err(invalid("parent preparation link identifies another worker"));
            }
        }
        Ok(Some(reference))
    }
    pub(crate) fn matches_run(&self, run: &RunReference) -> bool {
        run.workspace == self.workspace
            && run.account == self.account
            && run.phase == self.phase
            && run.attempt == self.attempt
            && run.max_attempts <= self.max_attempts
            && run.max_turns <= self.max_turns
            && run.max_review_rounds <= self.max_review_rounds
    }
}

pub(crate) fn parent_reassembly_inventory(
    receipts: &[Receipt],
    reference: &ParentReassemblyReference,
) -> Result<String> {
    grants::inventory(receipts, reference)
}

fn exact<'a>(receipts: &'a [Receipt], reference: &ReviewReceiptReference) -> Result<&'a Receipt> {
    let found = receipts
        .iter()
        .filter(|r| r.operation_id == reference.operation)
        .collect::<Vec<_>>();
    let [receipt] = found.as_slice() else {
        return Err(invalid(
            "parent reassembly receipt is missing or duplicated",
        ));
    };
    if canonical_digest(1, *receipt)? != reference.receipt_digest {
        return Err(invalid("parent reassembly receipt changed"));
    }
    Ok(receipt)
}
fn closed(snapshot: &ControlSnapshot, receipt: &Receipt) -> Result<()> {
    if receipt.runtime_id != snapshot.intent.runtime_id
        || receipt.phase != ReceiptPhase::Reconciled
        || receipt.observed_result.as_ref().is_none_or(|v| {
            v["local_execution"] != "exited"
                || v["unresolved_effects"]
                    .as_array()
                    .is_none_or(|a| !a.is_empty())
        })
        || snapshot
            .active_operations
            .iter()
            .any(|p| p.operation_id == receipt.operation_id)
        || snapshot.unresolved_operations.values().any(|targets| {
            targets
                .iter()
                .any(|target| receipt.canonical_targets.contains(target))
        })
    {
        return Err(Error::new(
            "reconciliation_required",
            "parent reassembly requires actual closed local ownership and effects",
        ));
    }
    Ok(())
}
pub(crate) fn verify_parent_reassembly_reference(
    snapshot: &ControlSnapshot,
    reference: &ParentReassemblyReference,
    receipts: &[Receipt],
) -> Result<()> {
    reference.validate()?;
    if snapshot.intent.runtime_id != reference.runtime
        || parent_reassembly_inventory(receipts, reference)? != reference.inventory_digest
    {
        return Err(invalid("parent reassembly phase inventory changed"));
    }
    for (operation, digest) in &reference.history {
        exact(
            receipts,
            &ReviewReceiptReference {
                operation: *operation,
                receipt_digest: digest.clone(),
            },
        )?;
    }
    grants::verify_receipts(snapshot, reference, receipts)?;
    let previous = exact(receipts, &reference.predecessor)?;
    closed(snapshot, previous)?;
    let previous_run = RunReference::from_receipt(previous)?
        .ok_or_else(|| invalid("parent predecessor is not an actual worker"))?;
    let observation = previous
        .observed_result
        .as_ref()
        .and_then(|v| v.get("observation"))
        .ok_or_else(|| invalid("parent predecessor has no terminal observation"))?;
    let expected_terminal = if reference.retry_of.is_some() || reference.transient_of.is_some() {
        "failed"
    } else {
        "completed"
    };
    let predecessor_matches = if let Some(prior_digest) = &reference.retry_of {
        let prior = ParentReassemblyReference::from_receipt(previous)?
            .ok_or_else(|| invalid("granted retry predecessor lost marked parent source"))?;
        let grant = reference
            .grants
            .last()
            .ok_or_else(|| invalid("granted retry has no actual grant transition"))?;
        prior.digest()? == *prior_digest
            && grant.previous_phase == previous_run.phase
            && reference.attempt == 1
            && prior.parent_work == reference.parent_work
            && prior.worker_work == reference.worker_work
            && prior.source == reference.source
            && prior.original_parent == reference.original_parent
            && prior.original_acceptance == reference.original_acceptance
            && prior.original_token == reference.original_token
            && prior.vector_digest == reference.vector_digest
            && prior.baseline_digest == reference.baseline_digest
            && prior.assignment_digest == reference.assignment_digest
            && prior.progress_digest == reference.progress_digest
            && prior
                .history
                .iter()
                .all(|(id, hash)| reference.history.get(id) == Some(hash))
    } else if let Some(prior_digest) = &reference.transient_of {
        let prior = ParentReassemblyReference::from_receipt(previous)?
            .ok_or_else(|| invalid("transient parent predecessor lost its marked source"))?;
        parent_transient_failure(previous)?;
        prior.digest()? == *prior_digest
            && previous_run.phase == reference.phase
            && previous_run.attempt.checked_add(1) == Some(reference.attempt)
            && prior.parent_work == reference.parent_work
            && prior.worker_work == reference.worker_work
            && prior.source == reference.source
            && prior.original_parent == reference.original_parent
            && prior.original_acceptance == reference.original_acceptance
            && prior.original_token == reference.original_token
            && prior.vector_digest == reference.vector_digest
            && prior.baseline_digest == reference.baseline_digest
            && prior.assignment_digest == reference.assignment_digest
            && prior.progress_digest == reference.progress_digest
            && prior.grants == reference.grants
            && prior
                .history
                .iter()
                .all(|(id, hash)| reference.history.get(id) == Some(hash))
    } else {
        previous_run.phase == reference.phase
            && previous_run.attempt.checked_add(1) == Some(reference.attempt)
    };
    if !predecessor_matches
        || observation["outcome"] != expected_terminal
        || observation["local_exit_confirmed"] != true
        || (observation["session"]["terminal"] != expected_terminal
            && !(reference.transient_of.is_some() && observation["session"]["terminal"].is_null()))
        || !observation["session"]["turn_id"].is_null()
        || !observation["session"]["unresolved_turn"].is_null()
        || observation["session"]["input_required"] != false
        || !observation["session"]["intervention"].is_null()
        || !observation["hook_error"].is_null()
    {
        return Err(invalid(
            "parent predecessor is not the exact completed terminal session",
        ));
    }
    for receipt in receipts {
        if let Some(run) = RunReference::from_receipt(receipt)?
            && run.phase == reference.phase
        {
            closed(snapshot, receipt)?;
            if reference.max_attempts > run.max_attempts
                || reference.max_turns > run.max_turns
                || reference.max_review_rounds > run.max_review_rounds
            {
                return Err(Error::new(
                    "attempts_exhausted",
                    "parent reassembly cannot reset original phase bounds",
                ));
            }
            if reference.retry_of.is_none()
                && reference.transient_of.is_none()
                && let Some(prior) = ParentReassemblyReference::from_receipt(receipt)?
                && (prior.progress_digest == reference.progress_digest
                    || prior.vector_digest == reference.vector_digest)
            {
                return Err(Error::new(
                    "no_progress",
                    "parent candidate already consumed this phase admission",
                ));
            }
        }
    }
    let source = exact(receipts, &reference.source)?;
    closed(snapshot, source)?;
    if source.kind != "admission"
        || source.canonical_targets != [work_key(&reference.parent_work)?]
        || [
            "run",
            "worker_preparation",
            "metadata",
            "continuation",
            "source_advanced",
            "parent_reassembly",
            "review_round",
            "release_resume",
            "session_resume",
            "session_recovery_preparation",
        ]
        .iter()
        .any(|key| source.expected.contains_key(*key))
        || source.observed_result.as_ref().is_none_or(|v| {
            v["observation"]["parent_reassembly_vector"] != reference.vector_digest
                || v["observation"]["parent_reassembly_baseline"] != reference.baseline_digest
                || v["observation"]["original_reviewed_token"] != reference.original_token
        })
    {
        return Err(invalid(
            "parent reassembly source lacks exact actual closed vector assembly",
        ));
    }
    let native_target = format!(
        "linear:{}:{}:{}",
        reference.parent_work.linear_endpoint,
        reference.parent_work.linear_workspace,
        reference.parent_work.parent
    );
    for retained in [&reference.original_parent, &reference.original_acceptance] {
        let receipt = exact(receipts, retained)?;
        if receipt.kind != "linear_comment"
            || receipt.runtime_id != reference.runtime
            || !matches!(
                receipt.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || receipt.canonical_targets != [native_target.clone()]
            || !receipt.remote_ids.contains_key("comment_id")
            || !receipt.expected.contains_key("provider_body_digest")
        {
            return Err(invalid(
                "parent reassembly original native publication provenance differs",
            ));
        }
    }
    if Utc::now().signed_duration_since(previous.updated_at) < chrono::Duration::seconds(1)
        || Utc::now().signed_duration_since(source.updated_at) < chrono::Duration::seconds(1)
    {
        return Err(Error::new(
            "continuation_wait",
            "parent reassembly requires bounded spacing after actual retirement",
        ));
    }
    Ok(())
}

/// Shared record-level check for actual successful preparation. Closed failed
/// or interrupted metadata is not preparation success; model admission and
/// completed-result validation both require this exact shape.
pub(crate) fn require_successful_parent_preparation(receipt: &Receipt) -> Result<()> {
    if receipt.kind != "admission"
        || receipt.phase != ReceiptPhase::Reconciled
        || PreparationReference::from_receipt(receipt)?.is_none()
        || receipt.observed_result.as_ref().is_none_or(|result| {
            result["local_execution"] != "exited"
                || result["unresolved_effects"]
                    .as_array()
                    .is_none_or(|effects| !effects.is_empty())
                || result["observation"]["phase"] != "worker_preparation"
                || result["observation"]["result"] != "prepared"
        })
    {
        return Err(invalid(
            "parent reassembly requires an actually successful closed preparation",
        ));
    }
    Ok(())
}
/// A new grant may renew attempts only through the exact typed parent lane;
/// ordinary admission must never erase retained baseline/source provenance.
pub(crate) fn reject_unmarked_parent_phase(phase: &PhaseKey, receipts: &[Receipt]) -> Result<()> {
    for receipt in receipts {
        let Some(prior) = ParentReassemblyReference::from_receipt(receipt)? else {
            continue;
        };
        if prior.phase.work == phase.work
            && prior.phase.generation == phase.generation
            && prior.phase.phase == phase.phase
        {
            return Err(Error::new(
                "parent_reassembly_required",
                "admission cannot drop retained parent reassembly provenance across retry grants",
            ));
        }
    }
    Ok(())
}

impl ControlHost {
    pub(crate) fn admit_parent_reassembly_preparation_observed(
        &self,
        lease: &Lease,
        input: PreparationAdmission,
        reference: ParentReassemblyReference,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        input.reference.validate()?;
        reference.validate()?;
        if input.request_id != input.reference.operation_id()?
            || input.reference.worker_operation != reference.operation
            || input.reference.workspace != reference.workspace
            || input.reference.account != reference.account
            || input.reference.phase != reference.phase
            || input.work_key != reference.phase.work
        {
            return Err(invalid(
                "parent reassembly preparation identifies another exact worker",
            ));
        }
        self.admit_inner(
            lease,
            input.request_id,
            input.work_key,
            input.scope_digest,
            Some(AdmissionReference::ParentPreparation(
                input.reference,
                Box::new(reference),
            )),
            fresh,
        )
    }
    pub(crate) fn admit_parent_reassembly_run_observed(
        &self,
        lease: &Lease,
        input: RunAdmission,
        reference: ParentReassemblyReference,
        preparation: ReviewReceiptReference,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        input.reference.validate()?;
        reference.validate()?;
        if input.request_id != reference.operation
            || input.work_key != reference.phase.work
            || !reference.matches_run(&input.reference)
        {
            return Err(invalid(
                "parent reassembly model differs from its captured phase bounds",
            ));
        }
        self.admit_inner(
            lease,
            input.request_id,
            input.work_key,
            input.scope_digest,
            Some(AdmissionReference::ParentRun(
                input.reference,
                Box::new(reference),
                preparation,
            )),
            fresh,
        )
    }
    pub(super) fn verify_parent_reassembly_preparation(
        &self,
        snapshot: &ControlSnapshot,
        reference: &ParentReassemblyReference,
    ) -> Result<()> {
        verify_parent_reassembly_reference(snapshot, reference, &self.receipts.all()?)
    }
    pub(super) fn verify_parent_reassembly_run(
        &self,
        snapshot: &ControlSnapshot,
        lease: &Lease,
        reference: &ParentReassemblyReference,
        preparation: &ReviewReceiptReference,
    ) -> Result<()> {
        let receipts = self.receipts.all()?;
        verify_parent_reassembly_reference(snapshot, reference, &receipts)?;
        let receipt = exact(&receipts, preparation)?;
        closed(snapshot, receipt)?;
        require_successful_parent_preparation(receipt)?;
        if preparation.operation != preparation_operation(reference.operation)?
            || receipt.actor != format!("owner:{}", lease.owner_id)
            || receipt.owner_generation != lease.owner_generation
            || ParentReassemblyReference::from_receipt(receipt)?.as_ref() != Some(reference)
        {
            return Err(invalid(
                "parent reassembly model lacks its exact closed preparation",
            ));
        }
        Ok(())
    }
}

mod strict_phase {
    use super::*;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Fields {
        work: String,
        generation: String,
        phase: String,
        retry_grant: Option<Uuid>,
    }
    pub fn serialize<S: serde::Serializer>(
        v: &PhaseKey,
        s: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        v.serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<PhaseKey, D::Error> {
        let v = Fields::deserialize(d)?;
        Ok(PhaseKey {
            work: v.work,
            generation: v.generation,
            phase: v.phase,
            retry_grant: v.retry_grant,
        })
    }
}

/// Data-level classification repeats the actual runtime observation-only rule.
/// A retryable flag or provider message cannot manufacture a transient cause.
pub(crate) fn parent_transient_failure(receipt: &Receipt) -> Result<Option<u64>> {
    let observation = receipt
        .observed_result
        .as_ref()
        .and_then(|r| r.get("observation"))
        .ok_or_else(|| invalid("transient parent failure lacks actual observation"))?;
    let error: Error = serde_json::from_value(
        observation
            .get("error")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(|_| invalid("transient parent failure lacks typed actual error"))?;
    match error.code.as_str() {
        "process_exit" | "provider_transport" | "timeout" | "stream_timeout" | "stall_timeout" => {}
        "turn_incomplete" => {
            let session: crate::codex::SessionState =
                serde_json::from_value(observation["session"].clone())
                    .map_err(|_| invalid("transient parent terminal session is invalid"))?;
            if session.terminal_transient_failure()?.is_none() {
                return Err(Error::new(
                    "persistent_blocker",
                    "parent failure has no supported transient terminal observation",
                ));
            }
        }
        _ => {
            return Err(Error::new(
                "persistent_blocker",
                "parent failure requires explicit resolution before automatic retry",
            ));
        }
    }
    Ok(error
        .details
        .get("retry_after_ms")
        .and_then(serde_json::Value::as_u64))
}

#[cfg(test)]
#[path = "control_parent_reassembly_tests.rs"]
mod tests;
