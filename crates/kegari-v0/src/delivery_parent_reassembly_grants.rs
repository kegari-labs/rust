//! Exact native grant ancestry for the parent-only reassembly lane.
use super::*;
use crate::control::ParentGrantReference;
use kegari_types::run::PhaseKey;

pub(super) fn capture(
    phase: &PhaseKey,
    authority: &FreshGate,
    runtime: Uuid,
    receipts: &[Receipt],
    current: &[VerifiedRetryGrant],
) -> Result<(Vec<VerifiedRetryGrant>, Vec<ParentGrantReference>)> {
    capture_for_subject(
        phase,
        authority.work(),
        authority.workspace_id(),
        runtime,
        receipts,
        current,
    )
}
pub(super) fn capture_for_subject(
    phase: &PhaseKey,
    work: &WorkIdentity,
    workspace: Uuid,
    runtime: Uuid,
    receipts: &[Receipt],
    current: &[VerifiedRetryGrant],
) -> Result<(Vec<VerifiedRetryGrant>, Vec<ParentGrantReference>)> {
    let mut selected = Vec::new();
    let mut cursor = phase.clone();
    let mut seen = BTreeSet::new();
    while let Some(id) = cursor.retry_grant {
        if !seen.insert(id) {
            return Err(blocked("parent grant history is cyclic"));
        }
        let matches = current
            .iter()
            .filter(|g| g.record().operation_id == id)
            .collect::<Vec<_>>();
        let [grant] = matches.as_slice() else {
            return Err(blocked("parent exact native grant missing or duplicated"));
        };
        crate::runtime_workflow::require_current_grant(grant, current)?;
        if grant.phase_key() != cursor
            || grant.work() != work
            || grant.record().runtime_id != runtime
            || grant.record().workspace_id != workspace
            || current
                .iter()
                .filter(|g| g.phase() == grant.phase())
                .count()
                != 1
        {
            return Err(blocked(
                "parent native grant ancestry changes subject or forks",
            ));
        }
        selected.push((*grant).clone());
        cursor = grant.phase().clone();
    }
    if current.iter().any(|g| {
        g.work() == work
            && g.phase().work == phase.work
            && g.phase().generation == phase.generation
            && g.phase().phase == phase.phase
            && !seen.contains(&g.record().operation_id)
    }) {
        return Err(blocked(
            "parent phase has a newer or unrelated native grant",
        ));
    }
    selected.reverse();
    let mut references = Vec::new();
    for grant in &selected {
        let record = grant.record();
        let publication = unique(receipts, record.operation_id)?;
        let command = unique(receipts, record.command_id)?;
        if command.kind != "command/Retry"
            || command.runtime_id != runtime
            || command.actor != record.control_actor
        {
            return Err(blocked(
                "parent grant lost its actual authenticated command",
            ));
        }
        let target = format!(
            "linear:{}:{}:{}",
            grant.work().linear_endpoint,
            grant.work().linear_workspace,
            grant.work().child.unwrap_or(grant.work().parent)
        );
        if publication.canonical_targets != [target.clone()]
            || publication.remote_ids.get("comment_id") != Some(&grant.provenance().record_id)
            || publication.expected.get("provider_writer") != Some(&record.provider_writer)
            || publication.expected.get("parent_command_id") != Some(&record.command_id.to_string())
            || publication.expected.get("parent_payload_digest") != Some(&command.payload_digest)
        {
            return Err(blocked(
                "parent native grant differs from actual publication",
            ));
        }
        references.push(ParentGrantReference {
            previous_phase: grant.phase().clone(),
            publication: ReviewReceiptReference {
                operation: publication.operation_id,
                receipt_digest: canonical_digest(1, publication)?,
            },
            grant_digest: grant.digest().into(),
            native_target: target,
            comment_id: grant.provenance().record_id.clone(),
            provider_writer: record.provider_writer.clone(),
            body_digest: publication
                .expected
                .get("provider_body_digest")
                .cloned()
                .ok_or_else(|| blocked("parent grant has no acknowledged native body"))?,
            command: record.command_id,
            command_payload_digest: command.payload_digest.clone(),
            command_actor: command.actor.clone(),
            prior_admissions: record.prior_admissions.clone(),
            max_attempts: record.budget.max_attempts,
            max_turns: record.budget.max_turns,
            max_review_rounds: record.budget.max_review_rounds,
        });
    }
    Ok((selected, references))
}
fn unique(receipts: &[Receipt], id: Uuid) -> Result<&Receipt> {
    let matches = receipts
        .iter()
        .filter(|r| r.operation_id == id)
        .collect::<Vec<_>>();
    let [receipt] = matches.as_slice() else {
        return Err(blocked("parent lineage receipt missing or duplicated"));
    };
    Ok(*receipt)
}
pub(super) fn history(receipts: &[Receipt], phase: &PhaseKey) -> Result<BTreeMap<Uuid, String>> {
    let mut retained = BTreeMap::new();
    let mut add = |id, expected: Option<&str>| -> Result<()> {
        let receipt = unique(receipts, id)?;
        let digest = canonical_digest(1, receipt)?;
        if expected.is_some_and(|expected| expected != digest) {
            return Err(blocked("consumed parent ancestry changed"));
        }
        if retained
            .insert(id, digest.clone())
            .is_some_and(|old| old != digest)
        {
            return Err(blocked("parent ancestry contains conflicting receipts"));
        }
        Ok(())
    };
    for receipt in receipts {
        if let Some(run) = crate::control::run_reference_from_receipt(receipt)?
            && run.phase.work == phase.work
            && run.phase.generation == phase.generation
            && run.phase.phase == phase.phase
        {
            add(receipt.operation_id, None)?;
            if let Some(prior) = ParentReassemblyReference::from_receipt(receipt)? {
                for retained in [
                    &prior.source,
                    &prior.predecessor,
                    &prior.original_parent,
                    &prior.original_acceptance,
                ] {
                    add(retained.operation, Some(&retained.receipt_digest))?;
                }
                for (id, digest) in &prior.history {
                    add(*id, Some(digest))?;
                }
                for grant in &prior.grants {
                    add(
                        grant.publication.operation,
                        Some(&grant.publication.receipt_digest),
                    )?;
                }
                let prep: ReviewReceiptReference = serde_json::from_str(
                    receipt
                        .expected
                        .get("parent_reassembly_preparation")
                        .ok_or_else(|| blocked("marked parent predecessor lost preparation"))?,
                )
                .map_err(|_| blocked("marked parent preparation is invalid"))?;
                add(prep.operation, Some(&prep.receipt_digest))?;
            }
        }
    }
    Ok(retained)
}
