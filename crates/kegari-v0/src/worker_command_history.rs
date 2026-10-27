//! Bounded descriptive replay of command evidence. No decoded value restores a
//! launch capability, an old permit, or a validation result.
use super::command::{CommandChunk, CommandEffect, CommandEntryReference};
use super::*;
use crate::control::{Receipt, ReceiptPhase};
use crate::execution::{CommandExecutionRecord, CommandJournalEntry, CommandPreparationRecord};

const MAX_ENTRY_BYTES: usize = 32 * 1024 * 1024;
const CHUNK_BYTES: usize = 16 * 1024;
fn corrupt(message: &str) -> Error {
    Error::new("record_corrupt", message)
}
fn decode<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
    serde_json::from_value(value.clone())
        .map_err(|_| corrupt("invalid strict command history record"))
}
#[derive(PartialEq)]
enum Stage {
    Prepared,
    Dispatched,
    Started,
    Completed,
    Settled,
    NotStarted,
    Uncertain,
}
struct Invocation {
    preparation: Box<CommandPreparationRecord>,
    digest: String,
    stage: Stage,
    dispatched_at: Option<chrono::DateTime<Utc>>,
    started_at: Option<chrono::DateTime<Utc>>,
    completed: Option<(CommandEntryReference, Value)>,
}
struct PendingEntry {
    invocation: Uuid,
    entry: Uuid,
    bytes: Vec<u8>,
    leaves: Vec<String>,
}
pub(super) struct CommandHistory<'a> {
    admission: &'a Receipt,
    manifest: &'a InstructionManifest,
    tool_contract: &'a str,
    pending: Option<PendingEntry>,
    entries: BTreeSet<Uuid>,
    invocations: BTreeMap<Uuid, Invocation>,
}
impl<'a> CommandHistory<'a> {
    pub(super) fn new(
        admission: &'a Receipt,
        manifest: &'a InstructionManifest,
        tool_contract: &'a str,
    ) -> Self {
        Self {
            admission,
            manifest,
            tool_contract,
            pending: None,
            entries: BTreeSet::new(),
            invocations: BTreeMap::new(),
        }
    }
    pub(super) fn observe(&mut self, leaf: &str, event: &Value) -> Result<bool> {
        match event["kind"].as_str() {
            Some("command_chunk") => self.chunk(leaf, decode(&event["record"])?)?,
            Some("command_entry") => self.entry(decode(&event["record"])?)?,
            Some("command_effect") => self.effect(decode(&event["record"])?)?,
            _ => {
                if self.pending.is_some() {
                    return Err(corrupt("command chunk sequence was interrupted"));
                }
                return Ok(false);
            }
        }
        Ok(true)
    }
    fn chunk(&mut self, leaf: &str, chunk: CommandChunk) -> Result<()> {
        if chunk.schema_version != 1
            || chunk.invocation_id.is_nil()
            || chunk.entry_id.is_nil()
            || chunk.payload_hex.is_empty()
            || chunk.payload_hex.len() > CHUNK_BYTES * 2
            || !chunk.payload_hex.len().is_multiple_of(2)
            || self.entries.contains(&chunk.entry_id)
        {
            return Err(corrupt("command chunk schema, identity or size is invalid"));
        }
        let payload = hex::decode(&chunk.payload_hex)
            .map_err(|_| corrupt("command chunk has invalid bytes"))?;
        if hex::encode(&payload) != chunk.payload_hex {
            return Err(corrupt("command chunk encoding is not canonical"));
        }
        if self.pending.is_none() {
            if chunk.index != 0 {
                return Err(corrupt("command chunk prefix is missing"));
            }
            self.pending = Some(PendingEntry {
                invocation: chunk.invocation_id,
                entry: chunk.entry_id,
                bytes: Vec::new(),
                leaves: Vec::new(),
            });
        }
        let pending = self.pending.as_mut().expect("initialized above");
        if pending.invocation != chunk.invocation_id
            || pending.entry != chunk.entry_id
            || chunk.index as usize != pending.leaves.len()
            || pending.leaves.iter().any(|prior| prior == leaf)
            || !pending.bytes.len().is_multiple_of(CHUNK_BYTES)
            || payload.len() > MAX_ENTRY_BYTES.saturating_sub(pending.bytes.len())
        {
            return Err(corrupt(
                "command chunk order, owner or aggregate bound changed",
            ));
        }
        pending.bytes.extend_from_slice(&payload);
        pending.leaves.push(leaf.to_owned());
        Ok(())
    }
    fn entry(&mut self, reference: CommandEntryReference) -> Result<()> {
        let pending = self
            .pending
            .take()
            .ok_or_else(|| corrupt("command entry lacks its bytes"))?;
        if reference.schema_version != 1
            || reference.invocation_id != pending.invocation
            || reference.entry_id != pending.entry
            || reference.chunks != pending.leaves
            || reference.bytes != pending.bytes.len() as u64
            || reference.digest != bytes_digest(&pending.bytes)
            || !self.entries.insert(reference.entry_id)
        {
            return Err(corrupt(
                "command entry differs from its exact retained chunks",
            ));
        }
        let entry: CommandJournalEntry = serde_json::from_slice(&pending.bytes)
            .map_err(|_| corrupt("command entry does not decode strictly"))?;
        self.transition(entry, reference)
    }
    fn transition(
        &mut self,
        entry: CommandJournalEntry,
        reference: CommandEntryReference,
    ) -> Result<()> {
        let (invocation, operation, digest) = match &entry {
            CommandJournalEntry::Prepared {
                invocation_id,
                operation_id,
                preparation_digest,
                ..
            }
            | CommandJournalEntry::DispatchRequested {
                invocation_id,
                operation_id,
                preparation_digest,
                ..
            }
            | CommandJournalEntry::Started {
                invocation_id,
                operation_id,
                preparation_digest,
                ..
            }
            | CommandJournalEntry::Completed {
                invocation_id,
                operation_id,
                preparation_digest,
                ..
            }
            | CommandJournalEntry::Uncertain {
                invocation_id,
                operation_id,
                preparation_digest,
                ..
            }
            | CommandJournalEntry::NotStarted {
                invocation_id,
                operation_id,
                preparation_digest,
                ..
            } => (*invocation_id, *operation_id, preparation_digest.clone()),
        };
        if invocation != reference.invocation_id
            || operation != self.admission.operation_id
            || invocation == operation
        {
            return Err(corrupt(
                "command entry crosses its invocation or original operation",
            ));
        }
        if let CommandJournalEntry::Prepared { preparation, .. } = entry {
            self.preparation(&preparation)?;
            if preparation.invocation_id != invocation
                || preparation.digest()? != digest
                || self.invocations.contains_key(&invocation)
            {
                return Err(corrupt("command preparation changed or was repeated"));
            }
            self.invocations.insert(
                invocation,
                Invocation {
                    preparation,
                    digest,
                    stage: Stage::Prepared,
                    dispatched_at: None,
                    started_at: None,
                    completed: None,
                },
            );
            return Ok(());
        }
        let prior = self
            .invocations
            .get_mut(&invocation)
            .ok_or_else(|| corrupt("command preparation prefix is missing"))?;
        if prior.digest != digest {
            return Err(corrupt("command transition changed preparation identity"));
        }
        match entry {
            CommandJournalEntry::DispatchRequested { at, .. } if prior.stage == Stage::Prepared => {
                if at < prior.preparation.prepared_at || at >= prior.preparation.deadline_at {
                    return Err(corrupt(
                        "command dispatch exceeds original preparation window",
                    ));
                }
                prior.dispatched_at = Some(at);
                prior.stage = Stage::Dispatched;
            }
            CommandJournalEntry::Started { at, .. } if prior.stage == Stage::Dispatched => {
                if Some(at) < prior.dispatched_at {
                    return Err(corrupt("command start precedes dispatch"));
                }
                prior.started_at = Some(at);
                prior.stage = Stage::Started;
            }
            CommandJournalEntry::Completed { record, .. } if prior.stage == Stage::Started => {
                validate_completion(&record, prior)?;
                prior.completed = Some((
                    reference.clone(),
                    super::command::command_result(&record, &reference)?,
                ));
                prior.stage = Stage::Completed;
            }
            CommandJournalEntry::NotStarted { .. } if prior.stage == Stage::Prepared => {
                prior.stage = Stage::NotStarted
            }
            CommandJournalEntry::Uncertain { .. }
                if matches!(
                    prior.stage,
                    Stage::Prepared | Stage::Dispatched | Stage::Started
                ) =>
            {
                prior.stage = Stage::Uncertain
            }
            _ => {
                return Err(corrupt(
                    "command lifecycle is duplicated, reordered or clears uncertainty",
                ));
            }
        }
        Ok(())
    }
    fn preparation(&self, p: &CommandPreparationRecord) -> Result<()> {
        let original = &p.original_binding;
        let binding = &p.binding;
        original.work.validate()?;
        original.repository.validate()?;
        original.revision.validate()?;
        binding.work.validate()?;
        if p.limits.command_timeout_ms == 0 || p.limits.command_timeout_ms > 86_400_000 {
            return Err(corrupt("command history has an invalid original timeout"));
        }
        let deadline = p
            .prepared_at
            .checked_add_signed(chrono::Duration::milliseconds(
                p.limits.command_timeout_ms as i64,
            ));
        if p.schema_version != 1
            || p.invocation_id.is_nil()
            || original.runtime_id != self.admission.runtime_id
            || original.operation_id != self.admission.operation_id
            || original.work.digest()? != self.manifest.work_generation
            || original.isolation_digest != self.manifest.permissions.enforcement_profile
            || self.manifest.repository_revision.as_deref() != Some(&original.revision.commit)
            || !self.manifest.permissions.repository_code_execution
            || !self.manifest.role.permits_repository_execution()
            || binding.runtime_id != original.runtime_id
            || binding.operation_id != original.operation_id
            || binding.repository != original.repository
            || binding.revision != original.revision
            || binding.candidate_vector_digest != original.candidate_vector_digest
            || binding.isolation_digest != original.isolation_digest
            || deadline != Some(p.deadline_at)
            || p.before.revision != original.revision
            || p.before.digest()? != p.before_digest
        {
            return Err(corrupt(
                "command preparation differs from admitted original boundary",
            ));
        }
        match &p.correction {
            None if original != binding => {
                return Err(corrupt("changed command work lacks correction provenance"));
            }
            Some(correction) => {
                if original.work.child.is_none()
                    || correction.original_binding_digest != canonical_digest(1, original)?
                    || correction.watermark == 0
                    || binding.work.child != original.work.child
                    || binding.work.parent != original.work.parent
                    || binding.work.linear_workspace != original.work.linear_workspace
                    || binding.work.linear_endpoint != original.work.linear_endpoint
                    || binding.work.generation.authorization
                        != original.work.generation.authorization
                    || binding.work.generation.definition != original.work.generation.definition
                    || binding.work.generation.scope != original.work.generation.scope
                {
                    return Err(corrupt("command correction crosses original ownership"));
                }
                crate::evidence::validate_digest(&correction.transition_digest)?;
                crate::evidence::validate_digest(&correction.input_digest)?;
            }
            None => {}
        }
        let observation = &p.observation;
        let argv_bytes = p
            .request
            .argv
            .iter()
            .try_fold(0usize, |n, arg| n.checked_add(arg.len() + 1));
        if p.request.argv.is_empty()
            || p.request.argv.len() > 128
            || p.request.argv[0].is_empty()
            || p.request
                .argv
                .iter()
                .any(|arg| arg.len() > 4096 || arg.contains('\0'))
            || argv_bytes.is_none_or(|bytes| bytes > 65536)
            || p.request.cwd.is_empty()
            || p.request.cwd.len() > 4096
            || p.request.cwd.contains('\0')
            || observation.schema_version != 1
            || observation.requested_argv != p.request.argv
            || observation.original_isolation_digest != original.isolation_digest
            || !observation.cwd.is_absolute()
            || self
                .manifest
                .repository_root
                .as_ref()
                .is_none_or(|root| !observation.cwd.starts_with(root))
            || !observation.selected_executable.is_absolute()
            || !observation.canonical_executable.path.is_absolute()
            || !observation.launcher.path.is_absolute()
        {
            return Err(corrupt("command invocation observation is inconsistent"));
        }
        for digest in [
            &observation.isolation_digest,
            &observation.launch_digest,
            &observation.canonical_executable.digest,
            &observation.launcher.digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        // Domain policy identity has its own schema, unlike the schema-1
        // invocation/evidence digests. Preserve it without promoting old policy.
        let policy = observation
            .backend_policy_digest
            .strip_prefix("sha256:v5:")
            .ok_or_else(|| corrupt("command backend policy schema is incompatible"))?;
        if policy.len() != 64
            || !policy
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(corrupt("command backend policy digest is malformed"));
        }
        Ok(())
    }
    fn effect(&mut self, effect: CommandEffect) -> Result<()> {
        if self.pending.is_some() || effect.schema_version != 1 {
            return Err(corrupt(
                "command effect interrupts history or has invalid schema",
            ));
        }
        let prior = self
            .invocations
            .get_mut(&effect.invocation_id)
            .ok_or_else(|| corrupt("command effect lacks original preparation"))?;
        let (reference, value) = prior
            .completed
            .as_ref()
            .ok_or_else(|| corrupt("command effect precedes completion"))?;
        let receipt = &effect.receipt;
        let expected = BTreeMap::from([
            (
                "instruction_digest".to_owned(),
                canonical_digest(1, self.manifest)?,
            ),
            (
                "tool_contract_digest".to_owned(),
                self.tool_contract.to_owned(),
            ),
            (
                "admission_operation".to_owned(),
                self.admission.operation_id.to_string(),
            ),
            (
                "admitted_scope".to_owned(),
                self.admission
                    .expected
                    .get("scope")
                    .cloned()
                    .ok_or_else(|| corrupt("admission lacks scope"))?,
            ),
            (
                "admitted_work".to_owned(),
                self.admission
                    .canonical_targets
                    .first()
                    .cloned()
                    .ok_or_else(|| corrupt("admission lacks work"))?,
            ),
        ]);
        let payload_digest = canonical_digest(
            1,
            &json!({
                "kind":"worker_command_run", "targets":self.admission.canonical_targets,
                "expected":expected, "payload":prior.preparation.request, "principal":null
            }),
        )?;
        if prior.stage != Stage::Completed
            || canonical_digest(1, reference)? != canonical_digest(1, &effect.completed_entry)?
            || receipt.operation_id != effect.invocation_id
            || receipt.request_id != effect.invocation_id
            || receipt.runtime_id != self.admission.runtime_id
            || receipt.owner_generation != self.admission.owner_generation
            || receipt.actor != format!("worker:{}", self.admission.operation_id)
            || receipt.kind != "worker_command_run"
            || receipt.phase != ReceiptPhase::Acknowledged
            || receipt.canonical_targets != self.admission.canonical_targets
            || receipt.expected != expected
            || receipt.payload_digest != payload_digest
            || receipt.observed_result.as_ref() != Some(value)
            || !receipt.remote_ids.is_empty()
        {
            return Err(corrupt(
                "command effect is not the matching original acknowledged result",
            ));
        }
        prior.stage = Stage::Settled;
        Ok(())
    }
    pub(super) fn ensure_settled(&self) -> Result<()> {
        if self.pending.is_some()
            || self
                .invocations
                .values()
                .any(|entry| !matches!(entry.stage, Stage::Settled | Stage::NotStarted))
        {
            return Err(Error::new(
                "reconciliation_required",
                "retained command history is incomplete or uncertain; do not replay it",
            ));
        }
        Ok(())
    }
}
fn validate_completion(record: &CommandExecutionRecord, prior: &Invocation) -> Result<()> {
    let p = &record.preparation;
    let outcome = &record.outcome;
    if record.schema_version != 1
        || p.digest()? != prior.digest
        || record.preparation_digest != prior.digest
        || Some(record.dispatch_requested_at) != prior.dispatched_at
        || Some(record.started_at) != prior.started_at
        || record.completed_at < record.started_at
        || record.after.revision != p.original_binding.revision
        || record.after.digest()? != record.after_digest
        || outcome.schema_version != 1
        || outcome.argv != p.request.argv
        || outcome.cwd != p.observation.cwd
        || outcome.isolation_digest != p.observation.isolation_digest
        || outcome.termination_signal.is_some_and(|signal| signal <= 0)
        || (outcome.termination_signal.is_some() && outcome.exit_code.is_some())
    {
        return Err(corrupt(
            "command completion changed original preparation or terminal facts",
        ));
    }
    Ok(())
}
