//! Same-process historical producer. Only a successful owned-task join can
//! install a candidate. The authenticated reader never constructs worker state.
use super::*;
use crate::control::{
    ParentEffectHistoryBinding, ParentEffectHistoryObservation, Receipt, ReceiptPhase,
};
use crate::delivery::PreparedParentRunOrigin;
use crate::evidence::Signed;
use serde::{Deserialize, Serialize};
const PURPOSE: &str = "parent_run_retirement";
const LIMIT: u64 = 8 * 1024 * 1024;

pub(crate) struct ParentRunRegistrationInput<'a> {
    pub(crate) assignment: &'a crate::delivery::DeliveryWorkerAssignment,
    pub(crate) configuration: &'a kegari_config::Config,
    pub(crate) preparation: &'a Receipt,
    pub(crate) records: &'a PrivateDir,
}
#[derive(Clone)]
pub(super) struct JoinedParentRetirementCandidate {
    origin: Arc<PreparedParentRunOrigin>,
    completion: Arc<RunCompletion>,
    observation: Value,
    permit: Permit,
    budget: RunReference,
}
/// Control membership provenance only. Even both indexed owners do not prove
/// process-record completeness, a fresh OS join, product success or cold authority.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum ParentEffectHistory {
    RetainedReceiptScanWithoutEmittedIndex,
    ControlMembershipV1(Box<ParentEffectHistoryBinding>),
}
impl ParentEffectHistory {
    fn retain_or_observe(
        prior: Option<(u32, &Self)>,
        binding: &ParentEffectHistoryBinding,
    ) -> Result<(u32, Self)> {
        if let Some((schema, history)) = prior {
            history.validate_binding(schema, binding)?;
            return Ok((schema, history.clone()));
        }
        Ok(if binding.has_indexed_owner() {
            (2, Self::ControlMembershipV1(Box::new(binding.clone())))
        } else {
            (1, Self::RetainedReceiptScanWithoutEmittedIndex)
        })
    }
    fn validate_binding(&self, schema: u32, binding: &ParentEffectHistoryBinding) -> Result<()> {
        match (schema, self) {
            // Original signed schema-1 data stays explicitly scan-only, even
            // if this fresh batch independently validates a declared index.
            (1, Self::RetainedReceiptScanWithoutEmittedIndex) => Ok(()),
            (2, Self::ControlMembershipV1(expected))
                if expected.has_indexed_owner() && expected.as_ref() == binding =>
            {
                Ok(())
            }
            _ => Err(invalid(
                "parent effect history schema or exact owner provenance changed",
            )),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentRunRetirementRecord {
    schema_version: u32,
    effect_history: ParentEffectHistory,
    runtime: Uuid,
    operation: Uuid,
    origin: String,
    preparation: Uuid,
    model: String,
    observation: Value,
    run: RunReference,
    effects: BTreeMap<Uuid, String>,
    grant_commands: BTreeMap<Uuid, String>,
    journal: crate::worker::ParentJournalHistory,
    processes: BTreeMap<Uuid, String>,
    files: BTreeMap<String, String>,
    retired_at: chrono::DateTime<Utc>,
    execution: Option<Value>,
}
#[derive(Clone)]
struct AuthenticatedParentRunRetirement {
    signed: Signed<ParentRunRetirementRecord>,
}
fn invalid(message: &str) -> Error {
    Error::new("parent_retirement", message)
}
fn leaf(operation: Uuid) -> String {
    format!("parent-run-retirement-{operation}.json")
}
fn closed(receipt: &Receipt) -> Result<()> {
    if receipt.kind != "admission"
        || receipt.phase != ReceiptPhase::Reconciled
        || receipt.observed_result.as_ref().is_none_or(|v| {
            v["local_execution"] != "exited" || v["unresolved_effects"] != json!([])
        })
    {
        return Err(invalid(
            "parent owner is not actually retired with all effects closed",
        ));
    }
    Ok(())
}
fn effects(receipts: &[Receipt], owners: &[&Receipt]) -> Result<BTreeMap<Uuid, String>> {
    if receipts.len() > 65_536 {
        return Err(invalid("parent effect history exceeds protected bound"));
    }
    let mut result = BTreeMap::new();
    for receipt in receipts {
        let Some(owner_id) = receipt.expected.get("admission_operation") else {
            continue;
        };
        let Some(owner) = owners
            .iter()
            .find(|o| o.operation_id.to_string() == *owner_id)
        else {
            continue;
        };
        if receipt.runtime_id != owner.runtime_id
            || receipt.owner_generation != owner.owner_generation
            || receipt.expected.get("admitted_work") != owner.canonical_targets.first()
            || receipt.expected.get("admitted_scope") != owner.expected.get("scope")
            || !matches!(
                receipt.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || receipt.observed_result.is_none()
        {
            return Err(invalid(
                "parent effect remains unresolved or crosses actual ownership",
            ));
        }
        if result
            .insert(receipt.operation_id, canonical_digest(1, receipt)?)
            .is_some()
        {
            return Err(invalid("parent effect identity duplicated"));
        }
    }
    Ok(result)
}
fn process_history(
    runtime: &RuntimeCoordinator,
    owners: [&Receipt; 2],
    effects: &[Receipt],
) -> Result<BTreeMap<Uuid, String>> {
    let root = PrivateDir::open(&runtime.registration.root().path().join("worker-processes"))?;
    // SpawnBinding.effect_id is a process-journal identity. Trusted Git,
    // version probes and the app server do not use a Control effect UUID.
    // Bind every retained record to its actual owner instead of conflating it
    // with provider/command mutation receipts. No live PID probe is performed.
    let journal = crate::process_records::ProcessJournal::new(root, runtime.runtime_id)?;
    let mut result = BTreeMap::new();
    let mut bindings = Vec::new();
    for owner in owners {
        let records = journal.retained_records(owner.operation_id)?;
        for record in &records {
            let group_exit_recorded = matches!(
                record.state,
                crate::process_records::ProcessState::GroupExited { .. }
            ) && (record.schema_version == 1
                || matches!(
                    record.state,
                    crate::process_records::ProcessState::GroupExited {
                        guard_release_verified: true,
                        ..
                    }
                ));
            if !group_exit_recorded
                || record.binding.lease.as_ref().is_none_or(|lease| {
                    lease.runtime_id != owner.runtime_id
                        || lease.owner_generation != owner.owner_generation
                        || owner.actor != format!("owner:{}", lease.owner_id)
                })
                || Some(&record.binding.work_key) != owner.canonical_targets.first()
                || Some(&record.binding.scope_digest) != owner.expected.get("scope")
                || record.binding.control_owner.is_some()
            {
                return Err(invalid(
                    "parent process history differs from its actual retired owner",
                ));
            }
        }
        bindings.extend(records.iter().map(|record| record.binding.clone()));
        result.insert(owner.operation_id, canonical_digest(1, &records)?);
    }
    runtime
        .control
        .verify_check_bundle_inventory(effects, &bindings)?;
    Ok(result)
}

fn worker_files(records: &PrivateDir) -> Result<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for name in records.records_bounded(65_536)? {
        result.insert(
            name.clone(),
            bytes_digest(&records.read_bytes(&name, LIMIT)?),
        );
    }
    let path = records.path().join("hooks");
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::io("inspect parent hook history", e)),
        Ok(_) => {
            let hooks = PrivateDir::open(&path)?;
            for name in hooks.records_bounded(65_536)? {
                result.insert(
                    format!("hooks/{name}"),
                    bytes_digest(&hooks.read_bytes(&name, LIMIT)?),
                );
            }
        }
    }
    for required in ["parent-origin.json", "preparation.json", "contract.json"] {
        if !result.contains_key(required) {
            return Err(invalid("parent immutable worker record is missing"));
        }
    }
    Ok(result)
}
impl AuthenticatedParentRunRetirement {
    fn read_in_gate(
        runtime: &RuntimeCoordinator,
        directory: &PrivateDir,
        operation: Uuid,
        history: &ParentEffectHistoryObservation,
    ) -> Result<Self> {
        let signed: Signed<ParentRunRetirementRecord> =
            serde_json::from_slice(&directory.read_bytes(&leaf(operation), LIMIT)?)
                .map_err(|_| invalid("retired parent seal is malformed"))?;
        runtime.services.key.verify(PURPOSE, &signed)?;
        if signed.payload.operation != operation {
            return Err(invalid(
                "parent seal filename operation differs from signed identity",
            ));
        }
        let value = Self { signed };
        value.validate_in_gate(runtime, history)?;
        Ok(value)
    }
    fn validate_in_gate(
        &self,
        runtime: &RuntimeCoordinator,
        history: &ParentEffectHistoryObservation,
    ) -> Result<()> {
        let record = &self.signed.payload;
        let records = PrivateDir::open(
            &runtime
                .registration
                .root()
                .path()
                .join("worker-records")
                .join(record.operation.to_string()),
        )?;
        let origin: Signed<crate::delivery::ParentOriginRecord> =
            crate::delivery::read_parent_run_origin(
                &records,
                &runtime.control,
                &runtime.services.key,
            )?;
        let model = history.model();
        let preparation = history.preparation();
        if model.operation_id != record.operation || preparation.operation_id != record.preparation
        {
            return Err(invalid(
                "parent seal differs from its actual Control observation pair",
            ));
        }
        record
            .effect_history
            .validate_binding(record.schema_version, history.binding())?;
        closed(model)?;
        closed(preparation)?;
        origin.payload.verify_run_cause(model)?;
        if origin.payload.grant_commands(&runtime.control)? != record.grant_commands {
            return Err(invalid("retired parent grant command closure changed"));
        }
        let actual_run = RunReference::from_receipt(model)?
            .ok_or_else(|| invalid("retired parent run reference missing"))?;
        let prep = crate::control::PreparationReference::from_receipt(preparation)?
            .ok_or_else(|| invalid("retired parent preparation reference missing"))?;
        if prep.worker_operation != record.operation
            || prep.phase != actual_run.phase
            || prep.workspace != actual_run.workspace
            || prep.account != actual_run.account
            || preparation.observed_result.as_ref().is_none_or(|v| {
                v["observation"]["phase"] != "worker_preparation"
                    || v["observation"]["result"] != "prepared"
            })
        {
            return Err(invalid(
                "retired parent did not consume its exact successful preparation",
            ));
        }
        if record.runtime != runtime.runtime_id
            || record.operation != origin.payload.operation
            || origin.digest()? != record.origin
            || origin.payload.runtime != runtime.runtime_id
            || origin.payload.preparation.operation != record.preparation
            || canonical_digest(1, model)? != record.model
            || canonical_digest(1, &actual_run)? != canonical_digest(1, &record.run)?
            || model
                .observed_result
                .as_ref()
                .and_then(|v| v.get("observation"))
                != Some(&record.observation)
            || record.retired_at != model.updated_at
            || actual_run.phase.phase != origin.payload.phase
            || actual_run.workspace != origin.payload.workspace
            || actual_run.account != origin.payload.account
            || preparation.owner_generation != model.owner_generation
            || preparation.runtime_id != model.runtime_id
        {
            return Err(invalid(
                "retired parent seal differs from immutable registered source/run",
            ));
        }
        if effects(history.effects(), &[model, preparation])? != record.effects
            || process_history(runtime, [preparation, model], history.effects())?
                != record.processes
            || worker_files(&records)? != record.files
        {
            return Err(invalid("retired parent exact closure changed"));
        }
        record.journal.validate(&records, model)?;
        Ok(())
    }
}
impl RuntimeCoordinator {
    pub(crate) fn register_parent_run(
        &self,
        permit: Permit,
        contract: Arc<WorkerContract>,
        input: ParentRunRegistrationInput<'_>,
    ) -> Result<()> {
        let expected = self
            .registration
            .root()
            .path()
            .join("worker-records")
            .join(permit.operation_id.to_string());
        if input.records.path() != expected {
            return Err(invalid(
                "parent registration records are not its actual operation namespace",
            ));
        }
        if self
            .state
            .lock()
            .map_err(|_| invalid("parent registration lock failed"))?
            .runs
            .contains_key(&permit.operation_id)
        {
            return Err(invalid(
                "parent run is already registered; do not recreate original records",
            ));
        }
        let mut origin = None;
        self.control.validate_permit(&permit, || {
            self.registration.verify_active_owner(&permit.lease)?;
            if self
                .state
                .lock()
                .map_err(|_| invalid("parent registration lock failed"))?
                .runs
                .contains_key(&permit.operation_id)
            {
                return Err(invalid(
                    "parent run is already registered; preserve its original records",
                ));
            }

            self.retained_authority(contract.authority())?;
            origin = input.assignment.prepare_parent_origin(
                input.configuration,
                &contract,
                input.preparation,
                input.records,
            )?;
            Ok(())
        })?;
        if matches!(
            input.assignment.phase(),
            "parent_integrated_validation" | "parent_integrated_review" | "parent_evidence_review"
        ) && origin.is_none()
        {
            return Err(invalid(
                "actual parent assignment origin was not retained by its producer",
            ));
        }
        self.register_run_inner(permit, contract, origin.map(Arc::new))
    }
    /// Invoked only by collect_finished after the actual owned task has joined.
    pub(super) fn parent_join_candidate(
        origin: &Arc<PreparedParentRunOrigin>,
        contract: &Arc<WorkerContract>,
        permit: &Permit,
        budget: &RunReference,
        completion: &Arc<RunCompletion>,
    ) -> Result<JoinedParentRetirementCandidate> {
        if completion.operation_id != origin.record().operation
            || completion
                .contract
                .as_ref()
                .is_none_or(|actual| !Arc::ptr_eq(actual, contract))
            || completion.state.binding.operation_id != completion.operation_id.to_string()
        {
            return Err(invalid(
                "joined parent result lost exact registered contract identity",
            ));
        }
        Ok(JoinedParentRetirementCandidate {
            origin: origin.clone(),
            completion: completion.clone(),
            observation: completion.observation(),
            permit: permit.clone(),
            budget: budget.clone(),
        })
    }
    pub(super) fn observe_parent_retirements(&self) -> Result<()> {
        let candidates: Vec<_> = self
            .state
            .lock()
            .map_err(|_| invalid("parent retirement state lock failed"))?
            .runs
            .iter()
            .filter(|(_, r)| r.parent_joined.is_some())
            .map(|(id, _)| *id)
            .collect();
        for operation in candidates {
            let result = self.try_seal_parent_retirement(operation);
            let mut state = self
                .state
                .lock()
                .map_err(|_| invalid("parent retirement diagnostics lock failed"))?;
            match result {
                Ok(()) => {
                    state.parent_retirement_errors.remove(&operation);
                }
                Err(error) => {
                    state.parent_retirement_errors.insert(operation, error);
                }
            }
        }
        Ok(())
    }
    fn try_seal_parent_retirement(&self, operation: Uuid) -> Result<()> {
        let candidate = self
            .state
            .lock()
            .map_err(|_| invalid("parent retirement state lock failed"))?
            .runs
            .get(&operation)
            .and_then(|r| r.parent_joined.clone())
            .ok_or_else(|| invalid("parent has no actual owned join candidate"))?;
        if !candidate.completion.local_exit_confirmed {
            return Err(invalid(
                "actual parent joined result did not establish complete local exit",
            ));
        }
        let contract = candidate
            .completion
            .contract
            .as_deref()
            .ok_or_else(|| invalid("joined parent contract missing"))?;
        let execution = match &candidate.completion.execution {
            Some(execution) => {
                execution
                    .ownership
                    .verify(&execution.binding, contract.isolation())?;
                if execution.binding.runtime_id != self.runtime_id
                    || execution.binding.operation_id != operation
                    || execution.binding.work != candidate.origin.record().worker
                    || execution.binding.isolation_digest != contract.isolation().digest()
                {
                    return Err(invalid(
                        "actual parent checkout differs from original joined worker",
                    ));
                }
                Some(
                    json!({"binding":execution.binding,"owned_directory":execution.ownership.digest(),"isolation":contract.isolation().digest()}),
                )
            }
            None if contract
                .instructions()
                .manifest
                .permissions
                .repository_code_execution =>
            {
                return Err(invalid("executable parent lost actual owned checkout"));
            }
            None => None,
        };
        let preparation_id = candidate.origin.record().preparation.operation;
        let active = self.executions.active_operations()?;
        if active.contains(&operation) || active.contains(&preparation_id) {
            return Err(invalid(
                "parent retains actual registered execution handles",
            ));
        }
        self.control
            .with_parent_effect_history(preparation_id, operation, |snapshot, history| {
                if snapshot
                    .active_operations
                    .iter()
                    .any(|p| [operation, preparation_id].contains(&p.operation_id))
                    || snapshot
                        .unresolved_operations
                        .keys()
                        .any(|id| [operation, preparation_id].contains(id))
                {
                    return Err(invalid(
                        "parent still has actual active/unresolved ownership",
                    ));
                }
                let known = {
                    let state = self
                        .state
                        .lock()
                        .map_err(|_| invalid("parent final identity lock failed"))?;
                    let run = state
                        .runs
                        .get(&operation)
                        .ok_or_else(|| invalid("parent registration disappeared"))?;
                    if !run.joined
                        || run
                            .parent_joined
                            .as_ref()
                            .is_none_or(|c| !Arc::ptr_eq(&c.completion, &candidate.completion))
                        || state
                            .completions
                            .get(&operation)
                            .is_none_or(|c| !Arc::ptr_eq(c, &candidate.completion))
                        || candidate
                            .completion
                            .contract
                            .as_ref()
                            .is_none_or(|c| !Arc::ptr_eq(c, &run.contract))
                        || run.last_observation.as_ref() != Some(&candidate.observation)
                    {
                        return Err(invalid("parent original joined registration changed"));
                    }
                    state.parent_retirement_seals.get(&operation).cloned()
                };
                candidate
                    .origin
                    .validate(&self.control, &self.services.key)?;
                let records = PrivateDir::open(
                    &self
                        .registration
                        .root()
                        .path()
                        .join("worker-records")
                        .join(operation.to_string()),
                )?;
                let model = history.model();
                let preparation = history.preparation();
                closed(model)?;
                closed(preparation)?;
                if model.runtime_id != candidate.permit.lease.runtime_id
                    || model.owner_generation != candidate.permit.lease.owner_generation
                    || model.expected.get("scope") != Some(&candidate.permit.scope_digest)
                    || model.canonical_targets != [candidate.permit.work_key.clone()]
                    || RunReference::from_receipt(model)?
                        .as_ref()
                        .map(|r| canonical_digest(1, r))
                        .transpose()?
                        != Some(canonical_digest(1, &candidate.budget)?)
                {
                    return Err(invalid(
                        "retired parent run changed original registered budget/owner/scope",
                    ));
                }

                if model
                    .observed_result
                    .as_ref()
                    .and_then(|v| v.get("observation"))
                    != Some(&candidate.observation)
                {
                    return Err(invalid(
                        "actual retired observation differs from joined result",
                    ));
                }
                let effect_map = effects(history.effects(), &[model, preparation])?;
                let journal = crate::worker::capture_parent_journal(
                    &records,
                    model,
                    &candidate.completion,
                    history.effects(),
                )?;
                let directory = self.registration.root().child("parent-run-retirements")?;
                let prior = match AuthenticatedParentRunRetirement::read_in_gate(
                    self, &directory, operation, history,
                ) {
                    Ok(prior) => Some(prior),
                    Err(error) if error.code == "record_missing" && known.is_none() => {
                        // Missing a referenced file is never own-seal absence.
                        match directory.read_bytes(&leaf(operation), LIMIT) {
                            Err(e) if e.code == "record_missing" => None,
                            Err(e) => return Err(e),
                            Ok(_) => return Err(error),
                        }
                    }
                    Err(error) => return Err(error),
                };
                // Do not rewrite an existing valid legacy seal just to add a new
                // provenance label. Fresh index validation above still applies.
                let (schema_version, effect_history) = ParentEffectHistory::retain_or_observe(
                    prior.as_ref().map(|prior| {
                        (
                            prior.signed.payload.schema_version,
                            &prior.signed.payload.effect_history,
                        )
                    }),
                    history.binding(),
                )?;
                let record = ParentRunRetirementRecord {
                    schema_version,
                    effect_history,
                    runtime: self.runtime_id,
                    operation,
                    origin: candidate.origin.digest()?,
                    preparation: preparation_id,
                    model: canonical_digest(1, model)?,
                    observation: candidate.observation.clone(),
                    run: RunReference::from_receipt(model)?
                        .ok_or_else(|| invalid("parent run reference absent"))?,
                    processes: process_history(self, [preparation, model], history.effects())?,
                    effects: effect_map,
                    grant_commands: candidate.origin.record().grant_commands(&self.control)?,
                    journal,
                    files: worker_files(&records)?,
                    retired_at: model.updated_at,
                    execution: execution.clone(),
                };
                let signed = self.services.key.sign(PURPOSE, &record)?;
                let digest = signed.digest()?;
                match prior {
                    Some(prior) if prior.signed.digest()? == digest => {}
                    Some(_) => return Err(invalid("retired parent seal was replaced")),
                    None => {
                        let bytes = serde_json::to_vec(&signed)
                            .map_err(|_| invalid("parent seal encoding failed"))?;
                        if bytes.len() as u64 > LIMIT {
                            return Err(invalid("parent seal exceeds protected record bound"));
                        }
                        directory.create_bytes_exclusive(&leaf(operation), &bytes)?;
                    }
                }
                if known.as_ref().is_some_and(|prior| prior != &digest) {
                    return Err(invalid("remembered parent seal identity changed"));
                }
                // Remember the authentic envelope before another fallible closure
                // read so a later missing file can never enable reminting.
                self.state
                    .lock()
                    .map_err(|_| invalid("parent seal anchor lock failed"))?
                    .parent_retirement_seals
                    .insert(operation, digest);
                AuthenticatedParentRunRetirement::read_in_gate(
                    self, &directory, operation, history,
                )?;
                Ok(())
            })
    }
}

#[cfg(test)]
#[path = "runtime_parent_retirement_tests.rs"]
mod tests;
