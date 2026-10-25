//! Local acknowledgement of an already observed operator retry publication.
//! This never dispatches work or reconciles a successor's unknown effects.
use super::*;
use crate::decisions::VerifiedRetryGrant;

impl ControlHost {
    pub(crate) fn record_retry_completion(
        &self,
        request: MutationRequest,
        parent: &Receipt,
        grant: &VerifiedRetryGrant,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Receipt> {
        let record = grant.record();
        if request.kind != "decision/Retry"
            || request.runtime_id != record.runtime_id
            || request.actor.identity != record.control_actor
            || request.actor.principal.as_ref() != Some(&record.principal)
            || request.expected.get("parent_command_id") != Some(&parent.operation_id.to_string())
            || request.expected.get("parent_payload_digest") != Some(&parent.payload_digest)
            || record.command_id != parent.operation_id
            || request.canonical_targets != parent.canonical_targets
            || !request
                .canonical_targets
                .contains(&record.previous_phase.work)
        {
            return Err(Error::new(
                "scope_violation",
                "retry completion lacks its exact command and verified grant",
            ));
        }
        self.mutate_synchronous(
            request.clone(), None, Some("recover"),
            |snapshot| {
                let command = self.receipts.get(parent.operation_id)?;
                let publication = self.receipts.get(record.operation_id)?;
                if command.runtime_id != request.runtime_id
                    || command.kind != "command/Retry"
                    || command.phase != ReceiptPhase::Prepared
                    || command.actor != request.actor.identity
                    || command.payload_digest != parent.payload_digest
                    || !snapshot.background_commands.contains(&command.operation_id)
                    || publication.runtime_id != request.runtime_id
                    || publication.kind != "linear_comment"
                    || !matches!(publication.phase, ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled)
                    || publication.expected.get("parent_command_id") != Some(&parent.operation_id.to_string())
                    || publication.expected.get("parent_payload_digest") != Some(&parent.payload_digest)
                    || publication.expected.get("provider_writer") != Some(&record.provider_writer)
                    || publication.remote_ids.get("comment_id") != Some(&grant.provenance().record_id)
                    || !publication.canonical_targets.contains(&format!(
                        "linear:{}:{}:{}",
                        record.work.linear_endpoint,
                        record.work.linear_workspace,
                        record.work.child.unwrap_or(record.work.parent)
                    ))
                {
                    return Err(Error::new("missing_evidence", "retry acknowledgement lost exact observed publication lineage"));
                }
                fresh(snapshot)
            },
            |_| Ok((BTreeMap::new(), serde_json::json!({"retry_grant":record.operation_id,"grant_digest":grant.digest(),
                "publication_operation":record.operation_id,"comment_id":grant.provenance().record_id,
                "budget":record.budget,"phase":record.previous_phase.phase}))),
            true,
        )
    }
}
