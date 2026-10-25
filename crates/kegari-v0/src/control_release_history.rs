//! Read-only historical release references. These helpers return actual closed
//! receipts, never an owner, permit, decision or authorization capability.
use super::*;

const MAX_RESUME_LINKS: usize = 256;

struct RecoveryCommandLink<'a> {
    command_id: Uuid,
    command_digest: &'a str,
    authorization_id: Uuid,
    authorization_digest: &'a str,
}

fn invalid(message: &str) -> Error {
    Error::new("reconciliation_required", message)
}
fn terminal_admission(receipt: &Receipt) -> Result<()> {
    if receipt.kind != "admission"
        || receipt.operation_id != receipt.request_id
        || receipt.canonical_targets.len() != 1
        || receipt.phase != ReceiptPhase::Reconciled
        || receipt.observed_result.as_ref().is_none_or(|v| {
            v["local_execution"] != "exited"
                || v["unresolved_effects"]
                    .as_array()
                    .is_none_or(|v| !v.is_empty())
        })
        || [
            "run",
            "metadata",
            "continuation",
            "source_advanced",
            "worker_preparation",
            "session_resume",
            "session_recovery_preparation",
        ]
        .iter()
        .any(|key| receipt.expected.contains_key(*key))
    {
        return Err(invalid(
            "release history lacks an exact closed release admission",
        ));
    }
    validate_digest(&receipt.payload_digest)?;
    validate_digest(
        receipt
            .expected
            .get("scope")
            .ok_or_else(|| invalid("release admission scope is missing"))?,
    )
}
fn same_request(a: &Receipt, b: &Receipt) -> bool {
    a.operation_id == b.operation_id
        && a.request_id == b.request_id
        && a.runtime_id == b.runtime_id
        && a.owner_generation == b.owner_generation
        && a.actor == b.actor
        && a.kind == b.kind
        && a.canonical_targets == b.canonical_targets
        && a.payload_digest == b.payload_digest
        && a.expected == b.expected
}
fn retain(records: &mut BTreeMap<Uuid, Receipt>, receipt: Receipt) -> Result<()> {
    if let Some(prior) = records.get(&receipt.operation_id)
        && canonical_digest(1, prior)? != canonical_digest(1, &receipt)?
    {
        return Err(invalid("release history changed during collection"));
    }
    records.insert(receipt.operation_id, receipt);
    Ok(())
}
fn expected_id(receipt: &Receipt, key: &str) -> Result<Uuid> {
    receipt
        .expected
        .get(key)
        .and_then(|v| Uuid::parse_str(v).ok())
        .filter(|id| !id.is_nil())
        .ok_or_else(|| invalid("release recovery reference is missing or invalid"))
}

impl ControlHost {
    fn release_history_receipt(&self, id: Uuid) -> Result<Receipt> {
        let receipt = self.receipts.get(id)?;
        if id.is_nil()
            || receipt.operation_id != id
            || receipt.request_id != id
            || receipt.runtime_id.is_nil()
        {
            return Err(invalid(
                "release history receipt differs from its requested identity",
            ));
        }
        validate_digest(&receipt.payload_digest)?;
        Ok(receipt)
    }
    /// Safe inside a serialized observation: reads receipts without taking the
    /// Control state lock. The caller must retain/recheck the returned whole
    /// receipt hashes before projecting a historical completion.
    pub(crate) fn closed_release_resume_receipts(
        &self,
        admission_id: Uuid,
    ) -> Result<Vec<Receipt>> {
        let latest = self.release_history_receipt(admission_id)?;
        terminal_admission(&latest)?;
        let Some(last) = ReleaseResumeReference::from_receipt(&latest)? else {
            return Ok(vec![latest]);
        };
        if last.ordinal as usize > MAX_RESUME_LINKS {
            return Err(invalid("release resume history exceeds its finite bound"));
        }
        let original = self.release_history_receipt(last.plan_operation_id)?;
        terminal_admission(&original)?;
        if ReleaseResumeReference::from_receipt(&original)?.is_some()
            || original.runtime_id != last.runtime_id
            || original.payload_digest != last.original_admission_digest
            || original.expected.get("scope") != Some(&last.plan_digest)
            || original.canonical_targets != vec![last.work_key.clone()]
        {
            return Err(invalid(
                "release resume origin differs from its immutable plan",
            ));
        }
        let mut records = BTreeMap::new();
        retain(&mut records, original.clone())?;
        let mut seen = BTreeSet::new();
        let mut current = latest;
        let mut ordinal = last.ordinal;
        loop {
            if !seen.insert(current.operation_id) || seen.len() > MAX_RESUME_LINKS {
                return Err(invalid(
                    "release resume history is cyclic or exceeds its bound",
                ));
            }
            terminal_admission(&current)?;
            let reference = ReleaseResumeReference::from_receipt(&current)?
                .ok_or_else(|| invalid("release resume history lost an intermediate admission"))?;
            if reference.ordinal != ordinal
                || reference.runtime_id != last.runtime_id
                || reference.plan_operation_id != last.plan_operation_id
                || reference.plan_digest != last.plan_digest
                || reference.original_admission_digest != last.original_admission_digest
                || reference.work_key != last.work_key
            {
                return Err(invalid(
                    "release resume history changes identity or skips an ordinal",
                ));
            }
            self.retain_closed_recovery_command(
                &original,
                &current,
                RecoveryCommandLink {
                    command_id: reference.recovery_command_id,
                    command_digest: &reference.recovery_command_digest,
                    authorization_id: reference.authorization_id,
                    authorization_digest: &reference.authorization_digest,
                },
                &mut records,
            )?;
            let previous = self.release_history_receipt(reference.previous_admission_id)?;
            terminal_admission(&previous)?;
            if previous.runtime_id != last.runtime_id
                || previous.payload_digest != reference.previous_admission_digest
                || previous.expected.get("scope") != Some(&reference.previous_scope)
                || previous.canonical_targets != original.canonical_targets
            {
                return Err(invalid(
                    "release resume predecessor differs from its exact reference",
                ));
            }
            retain(&mut records, current)?;
            if ordinal == 1 {
                if canonical_digest(1, &previous)? != canonical_digest(1, &original)? {
                    return Err(invalid(
                        "first resumed admission does not follow the original release",
                    ));
                }
                break;
            }
            ordinal -= 1;
            current = previous;
        }
        Ok(records.into_values().collect())
    }

    /// Include acknowledged local transitions as well as actual resumed
    /// admissions recorded by one exact original release attempt.
    pub(crate) fn closed_release_recovery_receipts(
        &self,
        original_id: Uuid,
        plan_digest: &str,
        recovery_id: Uuid,
    ) -> Result<Vec<Receipt>> {
        validate_digest(plan_digest)?;
        let original = self.release_history_receipt(original_id)?;
        terminal_admission(&original)?;
        if ReleaseResumeReference::from_receipt(&original)?.is_some()
            || original.expected.get("scope").map(String::as_str) != Some(plan_digest)
        {
            return Err(invalid("recovery history identifies another original plan"));
        }
        let recovery = self.release_history_receipt(recovery_id)?;
        if let Some(reference) = ReleaseResumeReference::from_receipt(&recovery)? {
            if reference.plan_operation_id != original_id || reference.plan_digest != plan_digest {
                return Err(invalid("recovery admission belongs to another release"));
            }
            return self.closed_release_resume_receipts(recovery_id);
        }
        let action = recovery.expected.get("recovery_action").map(String::as_str);
        if recovery.kind != "release_recovery_transition"
            || recovery.runtime_id != original.runtime_id
            || recovery.canonical_targets != original.canonical_targets
            || !matches!(
                recovery.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || recovery.expected.get("plan_operation_id") != Some(&original_id.to_string())
            || recovery.expected.get("plan_digest").map(String::as_str) != Some(plan_digest)
            || recovery.expected.get("original_admission_digest") != Some(&original.payload_digest)
            || !matches!(action, Some("abandon" | "resolve" | "supersede"))
            || recovery.observed_result.as_ref().is_none_or(|v| {
                v["release_operation"] != json!(original_id)
                    || v["action"].as_str() != action
                    || v["released"] != false
            })
        {
            return Err(invalid(
                "release recovery transition is not exactly acknowledged",
            ));
        }
        let mut records = BTreeMap::new();
        self.retain_closed_recovery_command(
            &original,
            &recovery,
            RecoveryCommandLink {
                command_id: expected_id(&recovery, "parent_command_id")?,
                command_digest: recovery
                    .expected
                    .get("parent_payload_digest")
                    .ok_or_else(|| invalid("recovery command digest missing"))?,
                authorization_id: expected_id(&recovery, "authorization_id")?,
                authorization_digest: recovery
                    .expected
                    .get("authorization_digest")
                    .ok_or_else(|| invalid("recovery authorization digest missing"))?,
            },
            &mut records,
        )?;
        retain(&mut records, original)?;
        retain(&mut records, recovery)?;
        Ok(records.into_values().collect())
    }

    fn retain_closed_recovery_command(
        &self,
        original: &Receipt,
        backend: &Receipt,
        link: RecoveryCommandLink<'_>,
        records: &mut BTreeMap<Uuid, Receipt>,
    ) -> Result<()> {
        let RecoveryCommandLink {
            command_id,
            command_digest,
            authorization_id,
            authorization_digest,
        } = link;
        validate_digest(command_digest)?;
        validate_digest(authorization_digest)?;
        let command = self.release_history_receipt(command_id)?;
        let authorization = self.release_history_receipt(authorization_id)?;
        if command.runtime_id != original.runtime_id
            || command.kind != "command/ReleaseRecover"
            || command.payload_digest != command_digest
            || command.phase != ReceiptPhase::Reconciled
            || !command
                .canonical_targets
                .contains(&original.canonical_targets[0])
            || authorization.runtime_id != original.runtime_id
            || authorization.kind != "decision/ReleaseRecover"
            || authorization.actor != command.actor
            || authorization.payload_digest != authorization_digest
            || !matches!(
                authorization.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || authorization.expected.get("parent_command_id") != Some(&command_id.to_string())
            || authorization.expected.get("parent_payload_digest") != Some(&command.payload_digest)
            || !authorization
                .canonical_targets
                .contains(&original.canonical_targets[0])
            || authorization
                .observed_result
                .as_ref()
                .is_none_or(|v| v["decision"] != "authorized" || v["command"] != "release_recover")
        {
            return Err(invalid(
                "release recovery lacks its exact completed command and authorization",
            ));
        }
        let result = command
            .observed_result
            .as_ref()
            .ok_or_else(|| invalid("recovery command has no result"))?;
        let effect_id = result["effect_operation_id"]
            .as_str()
            .and_then(|v| Uuid::parse_str(v).ok())
            .ok_or_else(|| invalid("recovery command lacks its exact completion reference"))?;
        let effect = self.release_history_receipt(effect_id)?;
        let recorded: Receipt = serde_json::from_value(result["receipt"].clone())
            .map_err(|_| invalid("recovery command completion reference is malformed"))?;
        if !same_request(&recorded, &effect)
            || recorded.observed_result != effect.observed_result
            || recorded.remote_ids != effect.remote_ids
            || effect.runtime_id != original.runtime_id
            || effect.kind != "decision/ReleaseRecover"
            || effect.actor != command.actor
            || effect.operation_id == authorization_id
            || !matches!(
                effect.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || !matches!(
                recorded.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || effect.expected.get("parent_command_id") != Some(&command_id.to_string())
            || effect.expected.get("parent_payload_digest") != Some(&command.payload_digest)
            || !effect
                .canonical_targets
                .contains(&original.canonical_targets[0])
        {
            return Err(invalid(
                "recovery command completion differs from its actual receipt",
            ));
        }
        let outcome = effect
            .observed_result
            .as_ref()
            .ok_or_else(|| invalid("recovery completion has no outcome"))?;
        let recorded_backend: Receipt = serde_json::from_value(outcome["receipt"].clone())
            .map_err(|_| invalid("recovery completion lacks its original action receipt"))?;
        if outcome["release_operation"] != json!(backend.operation_id)
            || !same_request(&recorded_backend, backend)
            || (backend.kind == "release_recovery_transition"
                && (!matches!(
                    recorded_backend.phase,
                    ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                ) || recorded_backend.observed_result != backend.observed_result
                    || recorded_backend.remote_ids != backend.remote_ids))
        {
            return Err(invalid("recovery completion identifies another action"));
        }
        retain(records, command)?;
        retain(records, authorization)?;
        retain(records, effect)
    }
}
