//! Observation-only complete journal closure. This does not restore a broker,
//! check execution, resume candidate, or process-exit capability.
use super::*;
use crate::control::Receipt;
use crate::worker_run::RunCompletion;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParentJournalHistory {
    files: BTreeMap<String, String>,
    session_digest: String,
    check_digests: Vec<String>,
}
impl ParentJournalHistory {
    pub(crate) fn validate(&self, records: &PrivateDir, receipt: &Receipt) -> Result<()> {
        let journal = PrivateDir::open(&records.path().join("journal"))?;
        let actual = journal.records_bounded(65_537)?;
        let expected: Vec<_> = self
            .files
            .keys()
            .filter(|n| n.ends_with(".json"))
            .cloned()
            .collect();
        if actual != expected {
            return Err(invalid("parent journal record inventory changed"));
        }
        for (leaf, digest) in &self.files {
            if bytes_digest(&journal.read_bytes(leaf, 4 * 1024 * 1024)?) != *digest {
                return Err(invalid("parent journal/artifact bytes changed"));
            }
        }
        let recovered = recover_session_records(records, receipt, None, true)?
            .ok_or_else(|| invalid("parent session journal disappeared"))?;
        if canonical_digest(1, &recovered.state)? != self.session_digest
            || recovered.pending_method.is_some()
        {
            return Err(invalid("parent final session changed"));
        }
        crate::worker_run::verify_native_history(records, receipt.operation_id, &recovered.state)?;
        Ok(())
    }
}
fn invalid(message: &str) -> Error {
    Error::new("parent_retirement", message)
}

pub(crate) fn capture_parent_journal(
    records: &PrivateDir,
    receipt: &Receipt,
    completion: &RunCompletion,
    _effects: &[Receipt],
) -> Result<ParentJournalHistory> {
    let contract = completion
        .contract
        .as_deref()
        .ok_or_else(|| invalid("actual parent contract missing"))?;
    let journal = PrivateDir::open(&records.path().join("journal"))?;
    Journal::verify_scope(&journal, contract)?;
    let recovered = recover_session_records(records, receipt, None, true)?
        .ok_or_else(|| invalid("actual parent session journal missing"))?;
    let origin = recovered
        .resume_origin
        .as_ref()
        .ok_or_else(|| invalid("parent final journal binding missing"))?;
    if canonical_digest(1, &recovered.state)? != canonical_digest(1, &completion.state)?
        || recovered.pending_method.is_some()
        || !origin.after_hook_is_latest
        || origin.after_hook_error != completion.hook_error.is_some()
    {
        return Err(invalid(
            "actual parent completion differs from final protected journal",
        ));
    }
    let mut wanted = Vec::new();
    for check in &completion.checks {
        wanted.push(canonical_digest(1, check.production_evidence()?)?);
    }
    for check in &completion.failed_checks {
        if check.record().evidence_class != crate::execution::ExecutionEvidenceClass::CodexExecution
        {
            return Err(Error::new(
                "unqualified_fixture",
                "parent retirement cannot qualify fixture check history",
            ));
        }
        wanted.push(canonical_digest(1, check.record())?);
    }
    wanted.sort();
    let mut files = BTreeMap::new();
    let mut actual = Vec::new();
    let mut observed = BTreeMap::<(String, String, String, String), String>::new();
    let mut after_hooks = 0usize;
    for leaf in journal.records_bounded(65_537)? {
        let bytes = journal.read_bytes(&leaf, 4 * 1024 * 1024)?;
        let (digest, event) = captured_journal_record(&leaf, &bytes)?;
        files.insert(leaf.clone(), digest);
        let Some(event) = event else {
            continue;
        };
        if event["kind"] == "after_hook" {
            after_hooks += 1;
            if event["record"]["error"]
                != serde_json::to_value(&completion.hook_error)
                    .map_err(|_| invalid("hook error encoding"))?
            {
                return Err(invalid(
                    "actual after-hook error differs from protected history",
                ));
            }
        }
        if event["kind"] == "check" {
            return Err(Error::new(
                "legacy_execution_evidence",
                "custom command journals require fresh Codex result reconciliation",
            ));
        }
        if event["kind"] == "codex_execution_observed" {
            let native = &event["record"];
            if native["item"]["type"] != "commandExecution" {
                continue;
            }
            let read = |key: &str| -> Result<String> {
                native[key]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("native execution observation has incomplete identity"))
            };
            if read("operation_id")? != receipt.operation_id.to_string() {
                return Err(invalid("native result belongs to another operation"));
            }
            let phase = read("phase")?;
            if !matches!(phase.as_str(), "started" | "completed") {
                return Err(invalid("unknown native observation phase"));
            }
            let digest = read("observation_digest")?;
            crate::evidence::validate_digest(&digest)?;
            let key = (
                read("thread_id")?,
                read("turn_id")?,
                read("item_id")?,
                phase,
            );
            if observed.insert(key, digest).is_some() {
                return Err(invalid("native result identity appears more than once"));
            }
            continue;
        }
        if event["kind"] != "codex_validation_recorded" {
            continue;
        }
        let checked: crate::execution::CheckExecutionRecord =
            serde_json::from_value(event["record"].clone())
                .map_err(|_| invalid("malformed native validation record"))?;
        checked.validate_shape()?;
        let native = checked
            .codex
            .as_ref()
            .ok_or_else(|| invalid("native validation provenance is absent"))?;
        if checked.binding.operation_id != receipt.operation_id
            || checked.evidence_class != crate::execution::ExecutionEvidenceClass::CodexExecution
            || completion.state.thread_id.as_deref() != Some(&native.thread_id)
            || !completion
                .state
                .completed_turns
                .contains_key(&native.turn_id)
        {
            return Err(invalid(
                "validation differs from the actual completed operation/thread/turn",
            ));
        }
        for (phase, digest) in [
            ("started", &native.started_item_digest),
            ("completed", &native.completed_item_digest),
        ] {
            let key = (
                native.thread_id.clone(),
                native.turn_id.clone(),
                native.item_id.clone(),
                phase.into(),
            );
            if observed.get(&key) != Some(digest) {
                return Err(invalid(
                    "native validation lacks its original observed command lifecycle",
                ));
            }
        }
        actual.push(canonical_digest(1, &checked)?);
        for artifact in &checked.artifacts {
            if artifact.bytes > 256 * 1024 * 1024 {
                return Err(invalid("retained check artifact exceeds bound"));
            }
            crate::evidence::validate_digest(&artifact.digest)?;
            let mut output = Vec::new();
            for index in 0..artifact.bytes.div_ceil(4 * 1024 * 1024) {
                let leaf = format!("artifact-{}-{index}.bin", &artifact.digest[7..]);
                let bytes = journal.read_bytes(&leaf, 4 * 1024 * 1024)?;
                files.insert(leaf, bytes_digest(&bytes));
                output.extend(bytes);
            }
            if output.len() as u64 != artifact.bytes || bytes_digest(&output) != artifact.digest {
                return Err(invalid("retained check output changed"));
            }
        }
    }
    actual.sort();
    if wanted != actual || after_hooks != 1 {
        return Err(invalid("parent check/after-hook history is incomplete"));
    }
    let result = ParentJournalHistory {
        files,
        session_digest: canonical_digest(1, &completion.state)?,
        check_digests: actual,
    };
    result.validate(records, receipt)?;
    Ok(result)
}

// The same bounded bytes determine both retained history and interpreted event.
// Binding verification remains Journal::verify_scope above; event records keep
// the previous PrivateDir::read limit in addition to its version/digest checks.
fn captured_journal_record(leaf: &str, bytes: &[u8]) -> Result<(String, Option<Value>)> {
    let event = if leaf == "binding.json" {
        None
    } else {
        if bytes.len() > 256 * 1024 {
            return Err(Error::new(
                "record_corrupt",
                "private file exceeds its size bound",
            ));
        }
        Some(kegari_storage::decode_record(bytes)?)
    };
    Ok((bytes_digest(bytes), event))
}

#[cfg(test)]
#[path = "worker_retirement_tests.rs"]
mod tests;
