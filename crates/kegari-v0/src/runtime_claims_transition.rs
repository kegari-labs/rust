//! Historical correction provenance only. Recovery never turns these records
//! into a live transition capability; native gates must authorize further work.
use super::*;
use kegari_digest::bytes_digest;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransitionRecord {
    schema_version: u32,
    original: Value,
    original_digest: String,
    current: Value,
    current_digest: String,
    previous: Option<String>,
    routes: Value,
    routes_digest: String,
    input: String,
    input_digest: String,
    digest: String,
    depth: u32,
}
struct Retained {
    record: TransitionRecord,
    original: FreshGate,
    current: FreshGate,
}
pub(super) fn original_authority(
    key: &AttestationKey,
    claims: &PrivateDir,
    admission: &Receipt,
    outcome: &WorkerOutcome,
    authority: &FreshGate,
) -> Result<FreshGate> {
    let processed = outcome.processed_correction.as_ref();
    if outcome.schema_version == 1 {
        if processed.is_some() {
            return Err(invalid(
                "legacy outcome cannot acquire correction-transition fields",
            ));
        }
        return Ok(authority.clone());
    }
    if outcome.schema_version != 2 {
        return Err(invalid("unknown worker outcome schema"));
    }
    if processed.is_none() && authority.correction_watermark() > 0 {
        return Err(invalid("corrected worker omitted the processed watermark"));
    }
    let Some(processed) = processed else {
        return Ok(authority.clone());
    };
    if processed.watermark != authority.correction_watermark()
        || processed.watermark != outcome.session.correction_consumed
        || processed.watermark != outcome.session.correction_required
    {
        return Err(invalid(
            "processed watermark differs from actual effective authority",
        ));
    }
    let Some(endpoint) = &processed.transition_digest else {
        if processed.input_digest != outcome.instructions.context_digest {
            return Err(invalid(
                "initial correction report differs from original assembled context",
            ));
        }
        return Ok(authority.clone());
    };
    let prefix = format!("worker-transition-{}-", admission.operation_id);
    let mut retained = BTreeMap::new();
    let mut total = 0usize;
    for leaf in claims
        .records_bounded(65_536)?
        .into_iter()
        .filter(|leaf| leaf.starts_with(&prefix))
    {
        if retained.len() >= 128 {
            return Err(invalid("correction lineage exceeds finite bound"));
        }
        let bytes = claims.read_bytes(&leaf, 8 * 1024 * 1024)?;
        total = total.saturating_add(bytes.len());
        if total > 64 * 1024 * 1024 {
            return Err(invalid("correction lineage exceeds byte bound"));
        }
        let signed: Signed<Value> =
            serde_json::from_slice(&bytes).map_err(|_| invalid("correction record is invalid"))?;
        key.verify("worker_correction_transition", &signed)?;
        let payload = &signed.payload;
        if payload.as_object().is_none_or(|value| value.len() != 8)
            || payload["schema_version"] != 1
            || payload["runtime"] != serde_json::json!(admission.runtime_id)
            || payload["operation"] != serde_json::json!(admission.operation_id)
            || payload["work"].as_str() != admission.canonical_targets.first().map(String::as_str)
            || payload["scope"].as_str() != admission.expected.get("scope").map(String::as_str)
            || payload["instructions"].as_str() != Some(&outcome.instruction_digest)
            || payload["tool_contract"].as_str() != Some(&outcome.tool_contract_digest)
        {
            return Err(invalid(
                "correction record crosses its immutable admission, scope or assembly",
            ));
        }
        let record: TransitionRecord = serde_json::from_value(payload["transition"].clone())
            .map_err(|_| invalid("correction transition record schema differs"))?;
        let original = FreshGate::recover_correction_record(
            key,
            &signed,
            "original",
            &record.original_digest,
        )?;
        let current =
            FreshGate::recover_correction_record(key, &signed, "current", &record.current_digest)?;
        if record.schema_version != 1
            || record.depth == 0
            || record.depth > 128
            || leaf != format!("{prefix}{}.json", current.correction_watermark())
            || record.input.len() > 65_536
            || bytes_digest(record.input.as_bytes()) != record.input_digest
            || canonical_digest(1, &record.routes)? != record.routes_digest
            || canonical_digest(
                1,
                &serde_json::json!({"original":record.original_digest,"previous":record.previous,"current":record.current_digest,"routes":record.routes_digest,"input":record.input_digest}),
            )? != record.digest
            || canonical_digest(1, &record.original)?
                != canonical_digest(
                    1,
                    &serde_json::to_value(&original).map_err(|_| invalid("authority encoding"))?,
                )?
            || canonical_digest(1, &record.current)?
                != canonical_digest(
                    1,
                    &serde_json::to_value(&current).map_err(|_| invalid("authority encoding"))?,
                )?
        {
            return Err(invalid(
                "correction transition digests or immutable file identity differ",
            ));
        }
        let digest = record.digest.clone();
        if retained
            .insert(
                digest,
                Retained {
                    record,
                    original,
                    current,
                },
            )
            .is_some()
        {
            return Err(invalid("correction transition identity is duplicated"));
        }
    }
    let endpoint_record = retained
        .get(endpoint)
        .ok_or_else(|| invalid("effective outcome lacks its retained correction transition"))?;
    if endpoint_record.current.digest()? != authority.digest()?
        || endpoint_record.record.input_digest != processed.input_digest
    {
        return Err(invalid(
            "effective outcome names another correction input or authority",
        ));
    }
    let original = endpoint_record.original.clone();
    let mut cursor = endpoint.as_str();
    let mut depth = endpoint_record.record.depth;
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(cursor.to_owned()) {
            return Err(invalid("correction chain cycles"));
        }
        let item = retained
            .get(cursor)
            .ok_or_else(|| invalid("correction predecessor record is missing"))?;
        if item.record.depth != depth || item.original.digest()? != original.digest()? {
            return Err(invalid(
                "correction chain changes its original authority or ordinal",
            ));
        }
        let prior = match item.record.previous.as_deref() {
            Some(previous) => {
                &retained
                    .get(previous)
                    .ok_or_else(|| invalid("correction predecessor is unavailable"))?
                    .current
            }
            None => &original,
        };
        validate_link(prior, &item.current, &item.record)?;
        match item.record.previous.as_deref() {
            Some(previous) if depth > 1 => {
                cursor = previous;
                depth -= 1;
            }
            None if depth == 1 => break,
            _ => {
                return Err(invalid(
                    "correction chain depth differs from retained history",
                ));
            }
        }
    }
    Ok(original)
}
fn validate_link(prior: &FreshGate, current: &FreshGate, record: &TransitionRecord) -> Result<()> {
    let history: BTreeSet<String> = serde_json::from_value(record.routes["history"].clone())
        .map_err(|_| invalid("correction history is invalid"))?;
    let checkpoints: BTreeMap<String, BTreeSet<String>> =
        serde_json::from_value(record.routes["checkpoints"].clone())
            .map_err(|_| invalid("correction checkpoints are invalid"))?;
    let predecessor = checkpoints
        .get(&prior.work().generation.correction)
        .ok_or_else(|| invalid("exact correction predecessor is absent"))?;
    let before = prior.work();
    let after = current.work();
    if canonical_digest(1, predecessor)? != before.generation.correction
        || predecessor.len() as u64 != prior.correction_watermark()
        || !predecessor.is_subset(&history)
        || predecessor == &history
        || canonical_digest(1, &history)? != after.generation.correction
        || history.len() as u64 != current.correction_watermark()
        || before.linear_endpoint != after.linear_endpoint
        || before.linear_workspace != after.linear_workspace
        || before.parent != after.parent
        || before.child != after.child
        || before.child.is_none()
        || before.generation.authorization != after.generation.authorization
        || before.generation.scope != after.generation.scope
        || prior.workspace_id() != current.workspace_id()
        || prior.config_revision() != current.config_revision()
        || prior.workflow_digest() != current.workflow_digest()
        || canonical_digest(1, prior.profile())? != canonical_digest(1, current.profile())?
        || prior.child_scope_digest() != current.child_scope_digest()
        || prior.required_children() != current.required_children()
        || prior.dependency_digest() != current.dependency_digest()
        || record.routes["work"] != serde_json::json!(after)
        || record.routes["watermark"].as_u64() != Some(current.correction_watermark())
        || record.routes["scope"].as_str() != current.child_scope_digest()
    {
        return Err(invalid(
            "retained correction link changes authority, scope, settings, dependencies or exact history",
        ));
    }
    let input: Value = serde_json::from_str(&record.input)
        .map_err(|_| invalid("retained correction input is invalid"))?;
    if input["work"] != serde_json::json!(after)
        || input["watermark"].as_u64() != Some(current.correction_watermark())
    {
        return Err(invalid(
            "retained correction input differs from effective work",
        ));
    }
    Ok(())
}
fn invalid(message: &str) -> Error {
    Error::new("worker_binding", message)
}
