//! Read-only, serialized observation of one exact parent preparation/model pair.
//! No receipt data recreates an index writer, permit, process or joined worker.
use super::*;

/// Serialized comparison data only. A decoder cannot create an observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum OwnerEffectHistory {
    LegacyRetainedScan,
    FrozenControlIndexV1 {
        owner: Uuid,
        owner_request: String,
        root: String,
        count: u32,
        tip: String,
        frozen: String,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParentEffectHistoryBinding {
    preparation: OwnerEffectHistory,
    model: OwnerEffectHistory,
}
impl ParentEffectHistoryBinding {
    pub(crate) fn has_indexed_owner(&self) -> bool {
        !matches!(self.preparation, OwnerEffectHistory::LegacyRetainedScan)
            || !matches!(self.model, OwnerEffectHistory::LegacyRetainedScan)
    }
}
/// Borrowed only during the actual Control callback. No Deserialize/constructor.
/// Frozen membership includes reservations, including known not-sent outcomes.
pub(crate) struct ParentEffectHistoryObservation {
    preparation: Receipt,
    model: Receipt,
    effects: Vec<Receipt>,
    binding: ParentEffectHistoryBinding,
}
impl ParentEffectHistoryObservation {
    pub(crate) fn preparation(&self) -> &Receipt {
        &self.preparation
    }
    pub(crate) fn model(&self) -> &Receipt {
        &self.model
    }
    pub(crate) fn effects(&self) -> &[Receipt] {
        &self.effects
    }
    pub(crate) fn binding(&self) -> &ParentEffectHistoryBinding {
        &self.binding
    }
}

fn closed_owner(receipt: &Receipt) -> Result<()> {
    if receipt.kind != "admission"
        || receipt.phase != ReceiptPhase::Reconciled
        || receipt.observed_result.as_ref().is_none_or(|value| {
            value["local_execution"] != "exited"
                || value["unresolved_effects"] != serde_json::json!([])
        })
    {
        return Err(invalid(
            "parent effect history requires closed owner observations",
        ));
    }
    Ok(())
}

impl ControlHost {
    /// Replaces (never nests inside) serialize_observation at the parent reader.
    /// Collect one bounded batch under the actual mutex; callback must not await
    /// or reenter Control. All values are observation data, never cold authority.
    pub(crate) fn with_parent_effect_history<T>(
        &self,
        preparation: Uuid,
        model: Uuid,
        observe: impl FnOnce(&ControlSnapshot, &ParentEffectHistoryObservation) -> Result<T>,
    ) -> Result<T> {
        let state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        let history = self.parent_effect_history(&state, preparation, model)?;
        observe(&state.snapshot(), &history)
    }

    fn parent_effect_history(
        &self,
        state: &State,
        preparation: Uuid,
        model: Uuid,
    ) -> Result<ParentEffectHistoryObservation> {
        if preparation == model || preparation.is_nil() || model.is_nil() {
            return Err(invalid(
                "parent effect history needs two distinct actual owners",
            ));
        }
        // Each protected inventory is bounded once for the whole pair. Reuse
        // these exact receipt objects in selection, seal creation and readback.
        let all = receipts(&self.receipts)?;
        let names = index_names(&self.root)?;
        let get = |operation: Uuid| {
            all.get(&operation).cloned().ok_or_else(|| {
                Error::new(
                    "record_missing",
                    "parent effect history owner receipt is missing",
                )
            })
        };
        let preparation = get(preparation)?;
        let model = get(model)?;
        let prep = PreparationReference::from_receipt(&preparation)?
            .ok_or_else(|| invalid("parent effect history preparation identity is missing"))?;
        let run = crate::control::run_reference_from_receipt(&model)?
            .ok_or_else(|| invalid("parent effect history model identity is missing"))?;
        if prep.worker_operation != model.operation_id
            || prep.phase != run.phase
            || prep.workspace != run.workspace
            || prep.account != run.account
            || preparation.runtime_id != model.runtime_id
            || model.runtime_id != state.intent.runtime_id
            || preparation.owner_generation != model.owner_generation
            || preparation.actor != model.actor
            || !matches!(
                run.phase.phase.as_str(),
                "parent_integrated_validation"
                    | "parent_integrated_review"
                    | "parent_evidence_review"
            )
        {
            return Err(invalid(
                "parent effect history owner pair differs from typed preparation",
            ));
        }
        closed_owner(&preparation)?;
        closed_owner(&model)?;
        let mut selected = BTreeMap::new();
        let mut bind = |owner: &Receipt| -> Result<OwnerEffectHistory> {
            let owner_id = owner.operation_id;
            if state.effect_history.members.contains_key(&owner_id) {
                return Err(invalid("parent owner reuses an indexed effect identity"));
            }
            let owned: BTreeSet<_> = all
                .values()
                .filter(|r| r.expected.get("admission_operation") == Some(&owner_id.to_string()))
                .map(|r| r.operation_id)
                .collect();
            let (provenance, members) = if required(owner)? {
                let inventory = read_owned(&self.root, owner, &all, &names, &owned)?;
                let frozen = inventory
                    .frozen
                    .as_ref()
                    .ok_or_else(|| invalid("parent owner membership is not frozen"))?;
                if let Some(live) = state.effect_indexes.get(&owner_id)
                    && (live.root != inventory.root
                        || live.cursor != inventory.cursor
                        || !live.frozen)
                {
                    return Err(invalid(
                        "parent membership differs from actual retained high-water",
                    ));
                }
                if let Some(anchor) = state.effect_history.owners.get(&owner_id)
                    && (anchor.root != inventory.root
                        || anchor.cursor != inventory.cursor
                        || !anchor.frozen)
                {
                    return Err(invalid(
                        "parent membership differs from retained historical identity",
                    ));
                }
                for (operation, member) in &state.effect_history.members {
                    if member.owner == owner_id
                        && inventory.members.get(operation) != Some(&member.identity)
                    {
                        return Err(invalid("parent membership lost a known effect identity"));
                    }
                }
                let binding = OwnerEffectHistory::FrozenControlIndexV1 {
                    owner: owner_id,
                    owner_request: inventory.root.admission_digest.clone(),
                    root: canonical_digest(1, &inventory.root)?,
                    count: frozen.count,
                    tip: frozen.tip.clone(),
                    frozen: canonical_digest(1, frozen)?,
                };
                (binding, inventory.members.keys().copied().collect())
            } else {
                let prefix = format!("{PREFIX}{owner_id}-");
                if state.effect_indexes.contains_key(&owner_id)
                    || state.effect_history.owners.contains_key(&owner_id)
                    || state
                        .effect_history
                        .members
                        .values()
                        .any(|m| m.owner == owner_id)
                    || names
                        .range(prefix.clone()..)
                        .next()
                        .is_some_and(|n| n.starts_with(&prefix))
                {
                    return Err(invalid(
                        "known indexed parent owner cannot become legacy history",
                    ));
                }
                (OwnerEffectHistory::LegacyRetainedScan, owned)
            };
            for operation in members {
                let receipt = all.get(&operation).ok_or_else(|| {
                    Error::new("record_missing", "parent indexed effect receipt is missing")
                })?;
                if receipt.runtime_id != owner.runtime_id
                    || receipt.owner_generation != owner.owner_generation
                    || receipt.expected.get("admission_operation") != Some(&owner_id.to_string())
                    || receipt.expected.get("admitted_work") != owner.canonical_targets.first()
                    || receipt.expected.get("admitted_scope") != owner.expected.get("scope")
                    || !matches!(
                        receipt.phase,
                        ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                    )
                    || receipt.observed_result.is_none()
                    || selected.insert(operation, receipt.clone()).is_some()
                {
                    return Err(invalid(
                        "parent effect is unresolved, duplicated or crosses ownership",
                    ));
                }
            }
            Ok(provenance)
        };
        let binding = ParentEffectHistoryBinding {
            preparation: bind(&preparation)?,
            model: bind(&model)?,
        };
        let mut operations: BTreeSet<_> = selected.keys().copied().collect();
        operations.extend([preparation.operation_id, model.operation_id]);
        if operations.iter().any(|id| {
            state.permits.contains_key(id)
                || state.holds.contains_key(id)
                || state.pending_external_effects.contains(id)
                || state.local_processes.contains(id)
        }) {
            return Err(invalid(
                "parent effect history retains active or unresolved ownership",
            ));
        }
        Ok(ParentEffectHistoryObservation {
            preparation,
            model,
            effects: selected.into_values().collect(),
            binding,
        })
    }
}

#[cfg(test)]
#[path = "control_parent_effect_history_tests.rs"]
mod tests;
