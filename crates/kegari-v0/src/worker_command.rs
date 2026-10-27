//! Original worker command invocations. These records describe execution; they
//! cannot manufacture a check, repair capability, native qualification or grant.
use super::*;
use crate::execution::{CommandExecutionRecord, CommandJournalEntry, CommandRequest};
use std::io::Write;

pub(super) const MAX_HISTORY_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRY_BYTES: usize = 32 * 1024 * 1024;
const CHUNK_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommandEntryReference {
    pub schema_version: u32,
    pub invocation_id: Uuid,
    pub entry_id: Uuid,
    pub digest: String,
    pub bytes: u64,
    pub chunks: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommandChunk {
    pub schema_version: u32,
    pub invocation_id: Uuid,
    pub entry_id: Uuid,
    pub index: u32,
    pub payload_hex: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommandEffect {
    pub schema_version: u32,
    pub invocation_id: Uuid,
    pub completed_entry: CommandEntryReference,
    pub receipt: crate::control::Receipt,
}
struct EntryBytes(Vec<u8>);
impl Write for EntryBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_ENTRY_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("command journal entry exceeds bound"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Journal {
    // Caller holds `next`. All event kinds share this monotone budget. Failed
    // writes keep their reservation and poison append rather than refill it.
    pub(super) fn reserve_bytes(&self, count: usize) -> Result<()> {
        let prior = self.bytes.load(Ordering::SeqCst);
        let next = prior
            .checked_add(count)
            .filter(|next| *next <= MAX_HISTORY_BYTES)
            .ok_or_else(|| {
                Error::new(
                    "journal_limit",
                    "worker history exceeds its finite recovery bound",
                )
            })?;
        self.bytes.store(next, Ordering::SeqCst);
        Ok(())
    }
    pub(super) fn append_command(
        &self,
        invocation_id: Uuid,
        entry: &CommandJournalEntry,
    ) -> Result<CommandEntryReference> {
        let mut bytes = EntryBytes(Vec::new());
        serde_json::to_writer(&mut bytes, entry).map_err(|_| {
            Error::new(
                "journal_limit",
                "command entry cannot be encoded within its byte bound",
            )
        })?;
        let bytes = bytes.0;
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
        let count = bytes.len().div_ceil(CHUNK_BYTES);
        if *seq + count as u64 + 1 > 65_536 {
            return Err(Error::new(
                "journal_limit",
                "command entry exceeds remaining journal inventory",
            ));
        }
        let entry_id = Uuid::new_v4();
        let chunks: Vec<_> = (0..count)
            .map(|index| format!("event-{:08}.json", *seq + index as u64))
            .collect();
        let reference = CommandEntryReference {
            schema_version: 1,
            invocation_id,
            entry_id,
            digest: bytes_digest(&bytes),
            bytes: bytes.len() as u64,
            chunks,
        };
        let at = Utc::now();
        let mut events = Vec::with_capacity(count + 1);
        for (index, payload) in bytes.chunks(CHUNK_BYTES).enumerate() {
            let chunk = CommandChunk {
                schema_version: 1,
                invocation_id,
                entry_id,
                index: index as u32,
                payload_hex: hex::encode(payload),
            };
            events.push(json!({"kind":"command_chunk","record":chunk,"observed_at":at}));
        }
        events.push(json!({"kind":"command_entry","record":reference,"observed_at":at}));
        let total = events.iter().try_fold(0usize, |total, event| {
            let size = serde_json::to_vec(event)
                .map_err(|_| Error::invalid("command event encoding"))?
                .len();
            total
                .checked_add(size)
                .ok_or_else(|| Error::new("journal_limit", "command event size overflow"))
        })?;
        // Reserve the entire sequence before its first write. This is the same
        // 64 MiB journal budget used by ordinary events and recovery, not a new
        // allowance per command or per entry. Hex overhead is included.
        self.reserve_bytes(total)?;
        for event in events {
            let leaf = format!("event-{:08}.json", *seq);
            if let Err(error) = self.dir.write(&leaf, &event) {
                self.failed.store(true, Ordering::SeqCst);
                return Err(error);
            }
            *seq += 1;
        }
        Ok(reference)
    }
}
fn preview(text: &str) -> &str {
    let mut end = text.len().min(4096);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
pub(super) fn command_result(
    record: &CommandExecutionRecord,
    reference: &CommandEntryReference,
) -> Result<Value> {
    bounded_value(json!({
        "schema_version":1,
        "invocation_id":record.preparation.invocation_id,
        "evidence_class":record.preparation.evidence_class,
        "completed_entry":{"entry_id":reference.entry_id,"digest":reference.digest,"bytes":reference.bytes},
        "preparation_digest":record.preparation_digest,
        "before_digest":record.preparation.before_digest,
        "after_digest":record.after_digest,
        "outcome":{
            "exit_code":record.outcome.exit_code,"termination_signal":record.outcome.termination_signal,
            "timed_out":record.outcome.timed_out,"elapsed_ms":record.outcome.elapsed_ms,
            "output_truncated":record.outcome.output_truncated,
            "stdout_preview":preview(&record.outcome.stdout),"stderr_preview":preview(&record.outcome.stderr),
            "stdout_bytes":record.outcome.stdout.len(),"stderr_bytes":record.outcome.stderr.len(),
            "stdout_digest":bytes_digest(record.outcome.stdout.as_bytes()),
            "stderr_digest":bytes_digest(record.outcome.stderr.as_bytes())
        }
    }))
}
impl ScopedWorker {
    pub(super) fn command_history(&self) -> Result<Value> {
        // Bounded newest references, with the omitted prefix explicit. The
        // original complete records remain in the finite protected journal.
        let mut entries = Vec::new();
        for record in self.command_records.iter().rev() {
            let mut candidate = entries.clone();
            candidate.push(record.clone());
            if serde_json::to_vec(&candidate)
                .map_err(|_| Error::invalid("command history encoding"))?
                .len()
                > MAX_TOOL_BYTES / 2
            {
                break;
            }
            entries = candidate;
        }
        entries.reverse();
        bounded_value(
            json!({"schema_version":1,"total":self.command_records.len(),
            "omitted":self.command_records.len()-entries.len(),"entries":entries}),
        )
    }
    pub(super) async fn run_command(&mut self, call: &ToolCall, id: Uuid) -> Result<Value> {
        let request: CommandRequest = parse(&call.arguments)?;
        let role = self.contract.assembled.manifest.role;
        if !role.permits_repository_execution()
            || !self
                .contract
                .assembled
                .manifest
                .permissions
                .repository_code_execution
            || self.contract.isolation.plan().managed_execution.is_none()
            || !matches!(&self.contract.authority, WorkerAuthority::Authorized(_))
        {
            return Err(Error::new(
                "scope_violation",
                "command requires actual repository execution authority",
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
            &contract.authority,
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
                "command requires exact acknowledged correction and current source binding",
            ));
        }
        let journal = self.journal.clone();
        let control = self.control.clone();
        let host = self.host.clone();
        let permit = self.permit.clone();
        let mutation = self.mutation(id, "worker_command_run", call.arguments.clone());
        let executions = self.executions.clone();
        let handle = tokio::runtime::Handle::current();
        let result = tokio::task::spawn_blocking(move || {
            let executor = execution.executor(&contract.isolation);
            let completed_entry = Mutex::new(None);
            let persist = |entry: &CommandJournalEntry| {
                let reference = journal.append_command(id, entry)?;
                if matches!(entry, CommandJournalEntry::Completed { .. }) {
                    *completed_entry.lock().map_err(|_| {
                        Error::new("journal_failed", "command completion reference lock failed")
                    })? = Some(reference);
                }
                Ok(())
            };
            let binding = effective
                .as_ref()
                .map_or(&execution.binding, |binding| binding.binding());
            let prepared =
                executor.prepare_command(binding, &request, id, effective.as_ref(), persist)?;
            if let Err(error) = executor.preflight_command(&prepared) {
                executor.refuse_command(prepared, error.clone(), persist)?;
                return Err(error);
            }
            let current = || {
                host.verify_owner(&permit.lease)?;
                expected_authority.compare(&effective_authority(
                    host.as_ref(),
                    permit.operation_id,
                    &contract.authority,
                )?)
            };
            let mut prepared = Some(prepared);
            let began = control.begin_effect(mutation, &permit, current, |receipt| {
                let prepared = prepared.take().ok_or_else(|| {
                    Error::new("ownership_conflict", "command preparation already consumed")
                })?;
                if let Err(error) = executor.preflight_command(&prepared) {
                    executor.refuse_command(prepared, error.clone(), persist)?;
                    return Err(error);
                }
                let reservation =
                    match executions.reserve_command(permit.operation_id, receipt.operation_id) {
                        Ok(reservation) => reservation,
                        Err(error) => {
                            executor.refuse_command(prepared, error.clone(), persist)?;
                            return Err(error);
                        }
                    };
                let running = executor.start_command(prepared, persist)?;
                reservation.attach(running.cancellation())?;
                Ok(running)
            });
            let (effect, running) = match began {
                Ok(value) => value,
                Err(error) => {
                    // The callback never consumed the actual preparation. No
                    // launch was attempted; record that original refusal.
                    if let Some(prepared) = prepared {
                        executor.refuse_command(prepared, error.clone(), persist)?;
                    }
                    return Err(error);
                }
            };
            let record = handle.block_on(executor.complete_command(running, persist))?;
            let reference = completed_entry
                .into_inner()
                .map_err(|_| {
                    Error::new("journal_failed", "command completion reference lock failed")
                })?
                .ok_or_else(|| {
                    Error::new("journal_failed", "command lacks persisted completion entry")
                })?;
            let value = command_result(&record, &reference)?;
            let receipt = control.complete_effect(effect, &permit, current, value.clone())?;
            journal.append(
                "command_effect",
                &CommandEffect {
                    schema_version: 1,
                    invocation_id: id,
                    completed_entry: reference,
                    receipt,
                },
            )?;
            // Process closure alone cannot prune this owner before durable
            // command history plus the original current-authority ACK.
            executions.remove(permit.operation_id, id)?;
            Ok::<_, Error>(value)
        })
        .await
        .map_err(|_| Error::new("operation_uncertain", "command dispatcher interrupted"))??;
        self.evidence.push(canonical_digest(1, &result)?);
        self.command_records.push(result.clone());
        self.effects.push("command_run".into());
        Ok(result)
    }
}
