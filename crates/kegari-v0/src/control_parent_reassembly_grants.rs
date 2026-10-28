//! Data-only native grant closure. Opaque callers separately require the exact
//! current VerifiedRetryGrant objects; serialized fields never authorize reads.
use super::*;
use kegari_types::run::PhaseKey;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParentGrantReference {
    #[serde(with = "strict_phase")]
    pub(crate) previous_phase: PhaseKey,
    pub(crate) publication: ReviewReceiptReference,
    pub(crate) grant_digest: String,
    pub(crate) native_target: String,
    pub(crate) comment_id: String,
    pub(crate) provider_writer: String,
    pub(crate) body_digest: String,
    pub(crate) command: Uuid,
    pub(crate) command_payload_digest: String,
    pub(crate) command_actor: String,
    pub(crate) prior_admissions: BTreeMap<Uuid, String>,
    pub(crate) max_attempts: u32,
    pub(crate) max_turns: u32,
    pub(crate) max_review_rounds: u32,
}
impl ParentGrantReference {
    pub(crate) fn phase(&self) -> PhaseKey {
        let mut phase = self.previous_phase.clone();
        phase.retry_grant = Some(self.publication.operation);
        phase
    }
}
pub(super) fn validate_shape(reference: &ParentReassemblyReference) -> Result<()> {
    let mut phase = reference.phase.clone();
    phase.retry_grant = None;
    let target = format!(
        "linear:{}:{}:{}",
        reference.worker_work.linear_endpoint,
        reference.worker_work.linear_workspace,
        reference
            .worker_work
            .child
            .unwrap_or(reference.worker_work.parent)
    );
    let mut seen = BTreeSet::new();
    for grant in &reference.grants {
        if grant.previous_phase != phase
            || grant.publication.operation.is_nil()
            || !seen.insert(grant.publication.operation)
            || grant.native_target != target
            || grant.comment_id.parse::<Uuid>().is_err()
            || grant.provider_writer.is_empty()
            || grant.command.is_nil()
            || grant.command_actor.is_empty()
            || grant.prior_admissions.is_empty()
            || [grant.max_attempts, grant.max_turns, grant.max_review_rounds].contains(&0)
        {
            return Err(invalid(
                "parent grant ancestry is disconnected or changes exact subject",
            ));
        }
        for digest in [
            &grant.publication.receipt_digest,
            &grant.grant_digest,
            &grant.body_digest,
            &grant.command_payload_digest,
        ] {
            validate_digest(digest)?;
        }
        for (id, digest) in &grant.prior_admissions {
            if id.is_nil() {
                return Err(invalid("parent grant prior admission is nil"));
            }
            validate_digest(digest)?;
        }
        phase = grant.phase();
    }
    if phase != reference.phase {
        return Err(invalid(
            "parent phase has no complete retained grant ancestry",
        ));
    }
    if let Some(grant) = reference.grants.last()
        && (reference.max_attempts > grant.max_attempts
            || reference.max_turns > grant.max_turns
            || reference.max_review_rounds > grant.max_review_rounds)
    {
        return Err(invalid(
            "parent readmission exceeds its actual granted budget",
        ));
    }
    Ok(())
}
pub(super) fn inventory(
    receipts: &[Receipt],
    reference: &ParentReassemblyReference,
) -> Result<String> {
    validate_shape(reference)?;
    let namespaces: BTreeSet<_> = std::iter::once(None)
        .chain(
            reference
                .grants
                .iter()
                .map(|g| Some(g.publication.operation)),
        )
        .collect();
    let mut inventory: BTreeMap<Option<Uuid>, BTreeMap<u32, (Uuid, String)>> = BTreeMap::new();
    let mut payloads: BTreeMap<Option<Uuid>, BTreeMap<Uuid, String>> = BTreeMap::new();
    for receipt in receipts {
        if let Some(run) = crate::control::run_reference_from_receipt(receipt)?
            && run.phase.work == reference.phase.work
            && run.phase.phase == reference.phase.phase
            && run.phase.generation == reference.phase.generation
        {
            if receipt.runtime_id != reference.runtime
                || run.workspace != reference.workspace
                || run.account != reference.account
                || !namespaces.contains(&run.phase.retry_grant)
                || inventory
                    .entry(run.phase.retry_grant)
                    .or_default()
                    .insert(
                        run.attempt,
                        (receipt.operation_id, canonical_digest(1, receipt)?),
                    )
                    .is_some()
            {
                return Err(invalid(
                    "parent phase has ambiguous ownership or retry-grant history",
                ));
            }
            payloads
                .entry(run.phase.retry_grant)
                .or_default()
                .insert(receipt.operation_id, receipt.payload_digest.clone());
        }
    }
    for values in inventory.values() {
        if values.keys().copied().ne(1..=u32::try_from(values.len())
            .map_err(|_| invalid("parent inventory exceeds ordinal encoding"))?)
        {
            return Err(invalid("parent phase history has an ordinal gap"));
        }
    }
    let count = inventory
        .get(&reference.phase.retry_grant)
        .map_or(0, BTreeMap::len);
    if u32::try_from(count).ok().and_then(|n| n.checked_add(1)) != Some(reference.attempt) {
        return Err(invalid(
            "parent predecessor is not the latest attempt in its exact budget",
        ));
    }
    for grant in &reference.grants {
        if payloads.get(&grant.previous_phase.retry_grant) != Some(&grant.prior_admissions) {
            return Err(invalid(
                "parent native grant does not cover the complete retired namespace",
            ));
        }
    }
    // Preserve old schema-one digest for the original ungranted namespace.
    if reference.grants.is_empty() {
        return canonical_digest(1, &inventory.remove(&None).unwrap_or_default());
    }
    // Optional namespaces are values, not JSON object keys. BTreeMap order
    // makes the sequence stable regardless of receipt observation order.
    canonical_digest(1, &inventory.into_iter().collect::<Vec<_>>())
}
pub(super) fn verify_receipts(
    snapshot: &ControlSnapshot,
    reference: &ParentReassemblyReference,
    receipts: &[Receipt],
) -> Result<()> {
    for receipt in receipts {
        if let Some(run) = crate::control::run_reference_from_receipt(receipt)?
            && run.phase.work == reference.phase.work
            && run.phase.phase == reference.phase.phase
            && run.phase.generation == reference.phase.generation
        {
            closed(snapshot, receipt)?;
        }
    }
    if !reference.history.is_empty() || !reference.grants.is_empty() {
        for receipt in receipts {
            let Some(run) = crate::control::run_reference_from_receipt(receipt)? else {
                continue;
            };
            if run.phase.work != reference.phase.work
                || run.phase.generation != reference.phase.generation
                || run.phase.phase != reference.phase.phase
            {
                continue;
            }
            require_history(reference, receipt)?;
            if let Some(prior) = ParentReassemblyReference::from_receipt(receipt)? {
                if !reference.grants.starts_with(&prior.grants) {
                    return Err(invalid("parent ancestry changes a consumed grant"));
                }
                for retained in [
                    &prior.source,
                    &prior.predecessor,
                    &prior.original_parent,
                    &prior.original_acceptance,
                ] {
                    require_history(reference, exact(receipts, retained)?)?;
                }
                for (id, hash) in &prior.history {
                    if reference.history.get(id) != Some(hash) {
                        return Err(invalid("parent grant drops consumed source ancestry"));
                    }
                }
                let prep: ReviewReceiptReference = serde_json::from_str(
                    receipt
                        .expected
                        .get(PREPARATION)
                        .ok_or_else(|| invalid("parent ancestor preparation absent"))?,
                )
                .map_err(|_| invalid("parent ancestor preparation invalid"))?;
                let preparation = exact(receipts, &prep)?;
                require_successful_parent_preparation(preparation)?;
                closed(snapshot, preparation)?;
                if preparation.actor != receipt.actor
                    || preparation.owner_generation != receipt.owner_generation
                    || ParentReassemblyReference::from_receipt(preparation)?.as_ref()
                        != Some(&prior)
                {
                    return Err(invalid(
                        "parent ancestor preparation differs from actual model",
                    ));
                }
                require_history(reference, preparation)?;
            }
        }
    }
    for grant in &reference.grants {
        let receipt = exact(receipts, &grant.publication)?;
        if receipt.runtime_id != reference.runtime
            || receipt.kind != "linear_comment"
            || !matches!(
                receipt.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || receipt.canonical_targets != [grant.native_target.clone()]
            || receipt.remote_ids.get("comment_id") != Some(&grant.comment_id)
            || receipt.expected.get("provider_writer") != Some(&grant.provider_writer)
            || receipt.expected.get("provider_body_digest") != Some(&grant.body_digest)
            || receipt.expected.get("parent_command_id") != Some(&grant.command.to_string())
            || receipt.expected.get("parent_payload_digest") != Some(&grant.command_payload_digest)
        {
            return Err(invalid(
                "parent grant native publication differs from retained authority",
            ));
        }
        let commands: Vec<_> = receipts
            .iter()
            .filter(|r| r.operation_id == grant.command)
            .collect();
        let [command] = commands.as_slice() else {
            return Err(invalid("parent grant command is missing or duplicated"));
        };
        // A published grant may be consumed while command finalization is still
        // running. Bind its immutable authenticated command, never freeze ACK time.
        if command.kind != "command/Retry"
            || command.runtime_id != reference.runtime
            || command.actor != grant.command_actor
            || command.payload_digest != grant.command_payload_digest
        {
            return Err(invalid("parent grant authenticated command changed"));
        }
    }
    Ok(())
}

fn require_history(reference: &ParentReassemblyReference, receipt: &Receipt) -> Result<()> {
    if reference.history.get(&receipt.operation_id) != Some(&canonical_digest(1, receipt)?) {
        return Err(invalid(
            "parent source/run/preparation ancestry was omitted or changed",
        ));
    }
    Ok(())
}
