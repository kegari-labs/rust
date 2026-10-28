//! Fresh release admission after explicit recovery. Serializable references are
//! provenance only; they cannot reconstruct an execution permit.
use super::*;
use crate::{decisions::AuthorizedDecision, evidence::validate_digest};
use kegari_types::api::CommandName;
use serde_json::json;

#[path = "control_release_history.rs"]
mod history;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseResumeReference {
    schema_version: u32,
    runtime_id: Uuid,
    plan_operation_id: Uuid,
    plan_digest: String,
    original_admission_digest: String,
    previous_admission_id: Uuid,
    previous_admission_digest: String,
    previous_scope: String,
    recovery_command_id: Uuid,
    recovery_command_digest: String,
    authorization_id: Uuid,
    authorization_digest: String,
    ordinal: u32,
    work_key: String,
}
impl ReleaseResumeReference {
    pub fn plan_operation_id(&self) -> Uuid {
        self.plan_operation_id
    }
    pub fn plan_digest(&self) -> &str {
        &self.plan_digest
    }
    pub fn previous_admission_id(&self) -> Uuid {
        self.previous_admission_id
    }
    pub fn recovery_command_id(&self) -> Uuid {
        self.recovery_command_id
    }
    pub fn ordinal(&self) -> u32 {
        self.ordinal
    }
    pub fn request_id(&self) -> Result<Uuid> {
        let digest = canonical_digest(
            1,
            &json!({"kind":"release_resume_admission","runtime":self.runtime_id,"plan":self.plan_operation_id,"command":self.recovery_command_id}),
        )?;
        let bytes = hex::decode(digest.rsplit(':').next().unwrap())
            .map_err(|_| Error::invalid("resume identity digest"))?;
        let mut id: [u8; 16] = bytes[..16]
            .try_into()
            .map_err(|_| Error::invalid("resume identity length"))?;
        id[6] = (id[6] & 0x0f) | 0x80;
        id[8] = (id[8] & 0x3f) | 0x80;
        Ok(Uuid::from_bytes(id))
    }
    pub fn scope_digest(&self) -> Result<String> {
        self.validate()?;
        canonical_digest(1, &json!({"kind":"release_resume_scope","reference":self}))
    }
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.ordinal == 0
            || self.work_key.is_empty()
            || [
                self.runtime_id,
                self.plan_operation_id,
                self.previous_admission_id,
                self.recovery_command_id,
                self.authorization_id,
            ]
            .iter()
            .any(Uuid::is_nil)
            || self.recovery_command_id == self.plan_operation_id
            || self.authorization_id == self.recovery_command_id
        {
            return Err(Error::new(
                "record_corrupt",
                "release resume identity is invalid",
            ));
        }
        for digest in [
            &self.plan_digest,
            &self.original_admission_digest,
            &self.previous_admission_digest,
            &self.previous_scope,
            &self.recovery_command_digest,
            &self.authorization_digest,
        ] {
            validate_digest(digest)?;
        }
        Ok(())
    }
    pub fn from_receipt(receipt: &Receipt) -> Result<Option<Self>> {
        let Some(encoded) = receipt.expected.get("release_resume") else {
            return Ok(None);
        };
        let reference: Self = serde_json::from_str(encoded)
            .map_err(|_| Error::new("record_corrupt", "release resume reference is malformed"))?;
        reference.validate()?;
        if receipt.kind != "admission"
            || receipt.runtime_id != reference.runtime_id
            || receipt.operation_id != reference.request_id()?
            || receipt.request_id != receipt.operation_id
            || receipt.canonical_targets != vec![reference.work_key.clone()]
            || receipt.expected.get("scope") != Some(&reference.scope_digest()?)
            || [
                "run",
                "metadata",
                "continuation",
                "source_advanced",
                "worker_preparation",
                "review_round",
                "review_round_preparation",
                "review_round_resume",
                "review_round_recovery",
                "session_resume",
                "session_recovery_preparation",
            ]
            .iter()
            .any(|key| receipt.expected.contains_key(*key))
        {
            return Err(Error::new(
                "record_corrupt",
                "release resume reference differs from its actual admission",
            ));
        }
        Ok(Some(reference))
    }
}

/// Issued from an authenticated decision and actual retained admission lineage.
/// The release backend must also prove current provider/plan/queue facts inside
/// the final serialized callback. This capability alone never grants dispatch.
pub(crate) struct VerifiedReleaseResume {
    reference: ReleaseResumeReference,
    actor: String,
}
impl VerifiedReleaseResume {
    pub(crate) fn for_decision(
        original: &Receipt,
        previous: &Receipt,
        plan_digest: &str,
        decision: &AuthorizedDecision,
    ) -> Result<Self> {
        let authorization = decision.authorization_receipt()?;
        let command = decision.parent_command();
        let target = decision.request().args["operation"]
            .as_str()
            .and_then(|v| Uuid::parse_str(v).ok());
        if decision.request().command != CommandName::ReleaseRecover
            || decision.request().args["action"] != "resume"
            || target != Some(original.operation_id)
            || decision.work().child.is_some()
            || decision.actor().principal.is_none()
            || original.expected.get("scope").map(String::as_str) != Some(plan_digest)
            || original.canonical_targets != vec![decision.parent().identity()?]
            || original.runtime_id != decision.request().runtime_id
            || ReleaseResumeReference::from_receipt(original)?.is_some()
            || [
                "run",
                "metadata",
                "continuation",
                "source_advanced",
                "worker_preparation",
                "review_round",
                "review_round_preparation",
                "review_round_resume",
                "review_round_recovery",
                "session_resume",
                "session_recovery_preparation",
            ]
            .iter()
            .any(|key| original.expected.contains_key(*key))
        {
            return Err(Error::new(
                "scope_violation",
                "resume decision does not identify the original immutable release plan",
            ));
        }
        decision.actor().require("recover")?;
        let ordinal = if previous.operation_id == original.operation_id {
            1
        } else {
            let prior = ReleaseResumeReference::from_receipt(previous)?.ok_or_else(|| {
                Error::new(
                    "ownership_conflict",
                    "previous operation is not an actual resume admission",
                )
            })?;
            if prior.plan_operation_id != original.operation_id
                || prior.plan_digest != plan_digest
                || prior.original_admission_digest != original.payload_digest
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "resume predecessor belongs to another plan",
                ));
            }
            prior
                .ordinal
                .checked_add(1)
                .ok_or_else(|| Error::new("bounded_input", "resume ordinal exhausted"))?
        };
        let reference = ReleaseResumeReference {
            schema_version: 1,
            runtime_id: original.runtime_id,
            plan_operation_id: original.operation_id,
            plan_digest: plan_digest.into(),
            original_admission_digest: original.payload_digest.clone(),
            previous_admission_id: previous.operation_id,
            previous_admission_digest: previous.payload_digest.clone(),
            previous_scope: previous.expected.get("scope").cloned().ok_or_else(|| {
                Error::new(
                    "ownership_conflict",
                    "resume predecessor lacks admission scope",
                )
            })?,
            recovery_command_id: command.operation_id,
            recovery_command_digest: command.payload_digest.clone(),
            authorization_id: authorization.operation_id,
            authorization_digest: authorization.payload_digest.clone(),
            ordinal,
            work_key: decision.parent().identity()?,
        };
        reference.validate()?;
        Ok(Self {
            reference,
            actor: decision.actor().identity.clone(),
        })
    }
    pub(crate) fn reference(&self) -> &ReleaseResumeReference {
        &self.reference
    }
}

impl ControlHost {
    pub(crate) fn admit_release_resume(
        &self,
        lease: &Lease,
        verified: &VerifiedReleaseResume,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        let reference = &verified.reference;
        self.admit_inner(
            lease,
            reference.request_id()?,
            reference.work_key.clone(),
            reference.scope_digest()?,
            Some(AdmissionReference::ReleaseResume(reference.clone())),
            |snapshot| {
                self.check_release_resume(reference, &verified.actor, snapshot)?;
                fresh(snapshot)
            },
        )
    }
    fn check_release_resume(
        &self,
        reference: &ReleaseResumeReference,
        actor: &str,
        snapshot: &ControlSnapshot,
    ) -> Result<()> {
        reference.validate()?;
        if reference.runtime_id != snapshot.intent.runtime_id || snapshot.service_stopping {
            return Err(Error::new(
                "ownership_conflict",
                "resume belongs to another runtime or stopping host",
            ));
        }
        let original = self.receipts.get(reference.plan_operation_id)?;
        let previous = self.receipts.get(reference.previous_admission_id)?;
        for (receipt, digest, scope) in [
            (
                &original,
                &reference.original_admission_digest,
                &reference.plan_digest,
            ),
            (
                &previous,
                &reference.previous_admission_digest,
                &reference.previous_scope,
            ),
        ] {
            if receipt.runtime_id != reference.runtime_id
                || receipt.kind != "admission"
                || &receipt.payload_digest != digest
                || receipt.expected.get("scope") != Some(scope)
                || receipt.canonical_targets != vec![reference.work_key.clone()]
                || receipt.phase != ReceiptPhase::Reconciled
                || receipt.observed_result.as_ref().is_none_or(|v| {
                    v["local_execution"] != "exited"
                        || v["unresolved_effects"]
                            .as_array()
                            .is_none_or(|v| !v.is_empty())
                })
                || snapshot
                    .active_operations
                    .iter()
                    .any(|p| p.operation_id == receipt.operation_id)
                || snapshot
                    .pending_external_effects
                    .contains(&receipt.operation_id)
                || snapshot
                    .pending_local_processes
                    .contains(&receipt.operation_id)
                || snapshot.background_commands.contains(&receipt.operation_id)
                || snapshot
                    .unresolved_operations
                    .contains_key(&receipt.operation_id)
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "original release ownership and effects are not conclusively retired",
                ));
            }
        }
        if ReleaseResumeReference::from_receipt(&original)?.is_some() {
            return Err(Error::new(
                "ownership_conflict",
                "release origin cannot itself be a resume",
            ));
        }
        if [
            "run",
            "metadata",
            "continuation",
            "source_advanced",
            "worker_preparation",
            "review_round",
            "review_round_preparation",
            "review_round_resume",
            "review_round_recovery",
            "session_resume",
            "session_recovery_preparation",
        ]
        .iter()
        .any(|key| original.expected.contains_key(*key))
        {
            return Err(Error::new(
                "ownership_conflict",
                "release origin is another admission kind",
            ));
        }
        if previous.operation_id == original.operation_id {
            if reference.ordinal != 1 {
                return Err(Error::new(
                    "ownership_conflict",
                    "first resume ordinal differs",
                ));
            }
        } else {
            let prior = ReleaseResumeReference::from_receipt(&previous)?
                .ok_or_else(|| Error::new("ownership_conflict", "resume predecessor is missing"))?;
            if prior.plan_operation_id != reference.plan_operation_id
                || prior.plan_digest != reference.plan_digest
                || prior.original_admission_digest != reference.original_admission_digest
                || prior.ordinal.checked_add(1) != Some(reference.ordinal)
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "resume chain is not the exact next admission",
                ));
            }
        }
        // A second command cannot fork an already consumed predecessor, even
        // after the first resumed execution has itself retired.
        for receipt in self.receipts.all()? {
            if let Some(prior) = ReleaseResumeReference::from_receipt(&receipt)?
                && prior.plan_operation_id == reference.plan_operation_id
                && (prior.ordinal >= reference.ordinal
                    || prior.recovery_command_id == reference.recovery_command_id)
            {
                return Err(Error::new(
                    "operation_uncertain",
                    "resume lineage already has this or a later admission",
                ));
            }
        }
        let command = self.receipts.get(reference.recovery_command_id)?;
        let authorization = self.receipts.get(reference.authorization_id)?;
        if command.runtime_id != reference.runtime_id
            || command.kind != "command/ReleaseRecover"
            || command.actor != actor
            || command.payload_digest != reference.recovery_command_digest
            || command.phase != ReceiptPhase::Prepared
            || !snapshot.background_commands.contains(&command.operation_id)
            || authorization.runtime_id != reference.runtime_id
            || authorization.kind != "decision/ReleaseRecover"
            || authorization.actor != actor
            || authorization.payload_digest != reference.authorization_digest
            || !matches!(
                authorization.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || authorization.expected.get("parent_command_id")
                != Some(&command.operation_id.to_string())
            || authorization.expected.get("parent_payload_digest") != Some(&command.payload_digest)
            || !authorization
                .canonical_targets
                .contains(&reference.work_key)
            || authorization
                .observed_result
                .as_ref()
                .is_none_or(|v| v["decision"] != "authorized" || v["command"] != "release_recover")
        {
            return Err(Error::new(
                "access_denied",
                "resume lacks its actual active operator command and acknowledged authorization",
            ));
        }
        Ok(())
    }

    /// Commit only local release-recovery state. The callback may write the
    /// authenticated attempt/queue record; it must not reenter ControlHost,
    /// invoke providers or issue/adopt execution authority. Unknown effects
    /// remain represented by their original holds and receipts.
    pub(crate) fn record_release_recovery_transition(
        &self,
        decision: &AuthorizedDecision,
        original: &Receipt,
        plan_digest: &str,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
        persist_local: impl FnOnce(&Receipt) -> Result<(BTreeMap<String, String>, Value)>,
    ) -> Result<Receipt> {
        let authorization = decision.authorization_receipt()?;
        let command = decision.parent_command();
        let action = decision.request().args["action"].as_str().unwrap_or("");
        let operation = decision.request().args["operation"]
            .as_str()
            .and_then(|v| Uuid::parse_str(v).ok());
        let work_key = decision.parent().identity()?;
        if decision.request().command != CommandName::ReleaseRecover
            || !["abandon", "resolve", "supersede"].contains(&action)
            || operation != Some(original.operation_id)
            || original.runtime_id != decision.request().runtime_id
            || original.kind != "admission"
            || original.expected.get("scope").map(String::as_str) != Some(plan_digest)
            || [
                "run",
                "metadata",
                "continuation",
                "source_advanced",
                "worker_preparation",
                "review_round",
                "review_round_preparation",
                "review_round_resume",
                "review_round_recovery",
                "session_resume",
                "session_recovery_preparation",
            ]
            .iter()
            .any(|key| original.expected.contains_key(*key))
            || ReleaseResumeReference::from_receipt(original)?.is_some()
            || original.canonical_targets != vec![work_key.clone()]
        {
            return Err(Error::new(
                "scope_violation",
                "local recovery transition is not the exact original release decision",
            ));
        }
        validate_digest(plan_digest)?;
        let identity = canonical_digest(
            1,
            &json!({"kind":"release_recovery_transition","command":command.operation_id,"plan":original.operation_id}),
        )?;
        let bytes = hex::decode(identity.rsplit(':').next().unwrap())
            .map_err(|_| Error::invalid("recovery identity digest"))?;
        let mut id: [u8; 16] = bytes[..16]
            .try_into()
            .map_err(|_| Error::invalid("recovery identity length"))?;
        id[6] = (id[6] & 0x0f) | 0x80;
        id[8] = (id[8] & 0x3f) | 0x80;
        let request = MutationRequest {
            request_id: Uuid::from_bytes(id),
            runtime_id: original.runtime_id,
            actor: decision.actor().clone(),
            kind: "release_recovery_transition".into(),
            canonical_targets: vec![work_key],
            expected: BTreeMap::from([
                ("parent_command_id".into(), command.operation_id.to_string()),
                (
                    "parent_payload_digest".into(),
                    command.payload_digest.clone(),
                ),
                (
                    "authorization_id".into(),
                    authorization.operation_id.to_string(),
                ),
                (
                    "authorization_digest".into(),
                    authorization.payload_digest.clone(),
                ),
                (
                    "plan_operation_id".into(),
                    original.operation_id.to_string(),
                ),
                ("plan_digest".into(), plan_digest.into()),
                ("recovery_action".into(), action.into()),
                (
                    "original_admission_digest".into(),
                    original.payload_digest.clone(),
                ),
            ]),
            payload: decision.request().args.clone(),
        };
        self.mutate_synchronous(
            request,
            None,
            Some("recover"),
            |snapshot| {
                let stored = self.receipts.get(command.operation_id)?;
                let actual_authorization = self.receipts.get(authorization.operation_id)?;
                let actual_original = self.receipts.get(original.operation_id)?;
                if stored.runtime_id != original.runtime_id
                    || stored.kind != "command/ReleaseRecover"
                    || stored.actor != decision.actor().identity
                    || stored.payload_digest != command.payload_digest
                    || stored.phase != ReceiptPhase::Prepared
                    || !snapshot.background_commands.contains(&stored.operation_id)
                    || actual_authorization.runtime_id != original.runtime_id
                    || actual_authorization.kind != "decision/ReleaseRecover"
                    || actual_authorization.actor != decision.actor().identity
                    || actual_authorization.payload_digest != authorization.payload_digest
                    || !matches!(
                        actual_authorization.phase,
                        ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                    )
                    || actual_authorization.expected.get("parent_command_id")
                        != Some(&stored.operation_id.to_string())
                    || actual_authorization.expected.get("parent_payload_digest")
                        != Some(&stored.payload_digest)
                    || actual_original.runtime_id != original.runtime_id
                    || actual_original.kind != original.kind
                    || actual_original.payload_digest != original.payload_digest
                    || actual_original.expected != original.expected
                {
                    return Err(Error::new(
                        "access_denied",
                        "local recovery transition lost exact durable authorization",
                    ));
                }
                fresh(snapshot)
            },
            persist_local,
            true,
        )
    }

    /// Acknowledge a recovery result only from its actual completed local
    /// transition or actual resumed admission. This copies durable metadata;
    /// no provider operation or unknown-effect reconciliation occurs here.
    pub(crate) fn record_release_recovery_completion(
        &self,
        request: MutationRequest,
        parent: &Receipt,
        result: Value,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Receipt> {
        let backend: Receipt = serde_json::from_value(result["receipt"].clone()).map_err(|_| {
            Error::new(
                "missing_evidence",
                "recovery completion needs its actual action receipt",
            )
        })?;
        let action = request.payload["action"].as_str().unwrap_or("").to_owned();
        let operation = request.payload["operation"]
            .as_str()
            .and_then(|v| Uuid::parse_str(v).ok());
        if request.kind != "decision/ReleaseRecover"
            || !["resume", "abandon", "resolve", "supersede"].contains(&action.as_str())
            || operation.is_none()
            || result["release_operation"] != json!(backend.operation_id)
            || request.expected.get("parent_command_id") != Some(&parent.operation_id.to_string())
            || request.expected.get("parent_payload_digest") != Some(&parent.payload_digest)
        {
            return Err(Error::new(
                "scope_violation",
                "completion is not an exact release recovery result",
            ));
        }
        self.mutate_synchronous(
            request.clone(),
            None,
            Some("recover"),
            |snapshot| {
                let command = self.receipts.get(parent.operation_id)?;
                let actual = self.receipts.get(backend.operation_id)?;
                if command.runtime_id != request.runtime_id
                    || command.kind != "command/ReleaseRecover"
                    || command.actor != request.actor.identity
                    || command.payload_digest != parent.payload_digest
                    || command.phase != ReceiptPhase::Prepared
                    || !snapshot.background_commands.contains(&command.operation_id)
                    || actual.runtime_id != request.runtime_id
                    || canonical_digest(1, &actual)? != canonical_digest(1, &backend)?
                    || !actual
                        .canonical_targets
                        .iter()
                        .any(|v| request.canonical_targets.contains(v))
                {
                    return Err(Error::new(
                        "ownership_conflict",
                        "recovery result does not match the actual owned action receipt",
                    ));
                }
                if action == "resume" {
                    let reference =
                        ReleaseResumeReference::from_receipt(&actual)?.ok_or_else(|| {
                            Error::new(
                                "missing_evidence",
                                "resume completion lacks actual admission lineage",
                            )
                        })?;
                    if Some(reference.plan_operation_id) != operation
                        || reference.recovery_command_id != command.operation_id
                        || reference.recovery_command_digest != command.payload_digest
                    {
                        return Err(Error::new(
                            "ownership_conflict",
                            "resume completion belongs to another plan or command",
                        ));
                    }
                } else if actual.kind != "release_recovery_transition"
                    || !matches!(
                        actual.phase,
                        ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                    )
                    || actual.expected.get("parent_command_id")
                        != Some(&command.operation_id.to_string())
                    || actual.expected.get("parent_payload_digest") != Some(&command.payload_digest)
                    || actual.expected.get("plan_operation_id")
                        != Some(&operation.unwrap().to_string())
                    || actual.expected.get("recovery_action") != Some(&action)
                    || actual.observed_result.is_none()
                {
                    return Err(Error::new(
                        "missing_evidence",
                        "recovery completion requires its exact observed local transition",
                    ));
                }
                fresh(snapshot)
            },
            |_| {
                Ok((
                    BTreeMap::new(),
                    json!({"release_operation":backend.operation_id,"receipt":backend}),
                ))
            },
            true,
        )
    }

    /// Record local recovery intent despite an unresolved effect. This method
    /// has no caller-supplied effect closure and does not clear holds, issue a
    /// permit, call a provider, or claim that any remote outcome is known.
    pub(crate) fn record_release_recovery_intent(
        &self,
        request: MutationRequest,
        parent: &Receipt,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Receipt> {
        let operation = request.payload["operation"]
            .as_str()
            .and_then(|v| Uuid::parse_str(v).ok())
            .ok_or_else(|| Error::invalid("recovery intent needs an operation"))?;
        let action = request.payload["action"]
            .as_str()
            .ok_or_else(|| Error::invalid("recovery action missing"))?;
        if request.kind != "decision/ReleaseRecover"
            || !["resume", "abandon", "resolve", "supersede"].contains(&action)
            || request.expected.get("parent_command_id") != Some(&parent.operation_id.to_string())
            || request.expected.get("parent_payload_digest") != Some(&parent.payload_digest)
        {
            return Err(Error::new(
                "scope_violation",
                "only exact local release recovery intent may bypass an effect hold",
            ));
        }
        self.mutate_synchronous(
            request.clone(),
            None,
            Some("recover"),
            |snapshot| {
                let stored = self.receipts.get(parent.operation_id)?;
                let target = self.receipts.get(operation)?;
                if stored.runtime_id != request.runtime_id
                    || stored.kind != "command/ReleaseRecover"
                    || stored.actor != request.actor.identity
                    || stored.payload_digest != parent.payload_digest
                    || stored.phase != ReceiptPhase::Prepared
                    || !snapshot.background_commands.contains(&stored.operation_id)
                    || target.runtime_id != request.runtime_id
                    || target.kind != "admission"
                    || !target
                        .canonical_targets
                        .iter()
                        .any(|v| request.canonical_targets.contains(v))
                {
                    return Err(Error::new(
                        "access_denied",
                        "recovery intent lacks exact original command/operation ownership",
                    ));
                }
                fresh(snapshot)
            },
            |_| {
                Ok((
                    BTreeMap::new(),
                    json!({"decision":"authorized","command":"release_recover"}),
                ))
            },
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kegari_control_api::identity::Principal;
    fn actor() -> Actor {
        Actor {
            identity: "fixture-operator".into(),
            roles: ["recover".into(), "lifecycle".into()].into(),
            principal: Some(Principal::Unix { uid: 501 }),
        }
    }
    fn request(runtime: Uuid, kind: &str, targets: Vec<String>, payload: Value) -> MutationRequest {
        MutationRequest {
            request_id: Uuid::new_v4(),
            runtime_id: runtime,
            actor: actor(),
            kind: kind.into(),
            canonical_targets: targets,
            expected: BTreeMap::new(),
            payload,
        }
    }
    fn setup() -> (tempfile::TempDir, ControlHost, Lease, Permit) {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let host = ControlHost::setup(
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("control")).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        host.lifecycle(
            request(runtime, "lifecycle", vec![runtime.to_string()], Value::Null),
            Lifecycle::Start,
            0,
        )
        .unwrap();
        let lease = Lease {
            runtime_id: runtime,
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        host.attach_reconciled(lease.clone()).unwrap();
        let permit = host
            .admit(
                &lease,
                Uuid::new_v4(),
                "fixture-parent".into(),
                canonical_digest(1, &"immutable release plan").unwrap(),
                || Ok(()),
            )
            .unwrap();
        (temp, host, lease, permit)
    }
    fn authorize(host: &ControlHost, original: &Permit, action: &str) -> (Receipt, Receipt) {
        let payload = json!({"operation":original.operation_id,"action":action,"reason":"explicit fixture recovery"});
        let command = request(
            original.lease.runtime_id,
            "command/ReleaseRecover",
            vec![original.work_key.clone()],
            json!({"arguments":payload,"secret_digest":null}),
        );
        let (parent, _) = host.queue_command(command, "recover", || Ok(())).unwrap();
        let mut stage = request(
            original.lease.runtime_id,
            "decision/ReleaseRecover",
            vec![original.work_key.clone()],
            payload,
        );
        stage
            .expected
            .insert("parent_command_id".into(), parent.operation_id.to_string());
        stage.expected.insert(
            "parent_payload_digest".into(),
            parent.payload_digest.clone(),
        );
        let authorization = host
            .record_release_recovery_intent(stage, &parent, |_| Ok(()))
            .unwrap();
        (parent, authorization)
    }
    fn completion(original: &Permit, parent: &Receipt, action: &str) -> MutationRequest {
        let mut result = request(
            original.lease.runtime_id,
            "decision/ReleaseRecover",
            vec![original.work_key.clone()],
            json!({"operation":original.operation_id,"action":action,"reason":"fixture recovery"}),
        );
        result
            .expected
            .insert("parent_command_id".into(), parent.operation_id.to_string());
        result.expected.insert(
            "parent_payload_digest".into(),
            parent.payload_digest.clone(),
        );
        result
    }
    // This unit fixture checks the control layer using actual receipts. Product
    // construction additionally requires the opaque AuthorizedDecision above.
    fn reference(
        original: &Receipt,
        previous: &Receipt,
        parent: &Receipt,
        authorization: &Receipt,
        ordinal: u32,
    ) -> VerifiedReleaseResume {
        VerifiedReleaseResume {
            actor: actor().identity,
            reference: ReleaseResumeReference {
                schema_version: 1,
                runtime_id: original.runtime_id,
                plan_operation_id: original.operation_id,
                plan_digest: original.expected["scope"].clone(),
                original_admission_digest: original.payload_digest.clone(),
                previous_admission_id: previous.operation_id,
                previous_admission_digest: previous.payload_digest.clone(),
                previous_scope: previous.expected["scope"].clone(),
                recovery_command_id: parent.operation_id,
                recovery_command_digest: parent.payload_digest.clone(),
                authorization_id: authorization.operation_id,
                authorization_digest: authorization.payload_digest.clone(),
                ordinal,
                work_key: original.canonical_targets[0].clone(),
            },
        }
    }
    fn closed_resume_fixture(
        host: &ControlHost,
        lease: &Lease,
        original: &Permit,
        origin: &Receipt,
        previous: &Receipt,
        ordinal: u32,
    ) -> Receipt {
        let (command, authorization) = authorize(host, original, "resume");
        let verified = reference(origin, previous, &command, &authorization, ordinal);
        let resumed = host
            .admit_release_resume(lease, &verified, |_| Ok(()))
            .unwrap();
        assert!(
            host.closed_release_resume_receipts(resumed.operation_id)
                .is_err()
        );
        let backend = host.operation(resumed.operation_id).unwrap();
        let effect = host
            .record_release_recovery_completion(
                completion(original, &command, "resume"),
                &command,
                json!({"release_operation":backend.operation_id,"receipt":backend}),
                |_| Ok(()),
            )
            .unwrap();
        let closed = host
            .retire_local(
                resumed.operation_id,
                json!({"fixture":"closed actual Control admission; no provider execution"}),
            )
            .unwrap();
        // A retired model/step admission does not prove that its operator
        // command completed with a known acknowledged result.
        assert!(
            host.closed_release_resume_receipts(closed.operation_id)
                .is_err()
        );
        host.complete_command(
            command.operation_id,
            Ok(json!({"effect_operation_id":effect.operation_id,"receipt":effect})),
        )
        .unwrap();
        closed
    }

    #[test]
    fn closed_release_history_retains_every_resume_command_and_authorization() {
        let (_temp, host, lease, original) = setup();
        let origin = host
            .retire_local(
                original.operation_id,
                json!({"fixture":"known original exit"}),
            )
            .unwrap();
        assert_eq!(
            host.closed_release_resume_receipts(origin.operation_id)
                .unwrap()
                .len(),
            1
        );
        let first = closed_resume_fixture(&host, &lease, &original, &origin, &origin, 1);
        let second = closed_resume_fixture(&host, &lease, &original, &origin, &first, 2);
        let records = host
            .closed_release_resume_receipts(second.operation_id)
            .unwrap();
        assert_eq!(records.len(), 9);
        let ids: BTreeSet<_> = records.iter().map(|r| r.operation_id).collect();
        assert_eq!(ids.len(), records.len());
        let linked = host
            .closed_release_recovery_receipts(
                origin.operation_id,
                &original.scope_digest,
                second.operation_id,
            )
            .unwrap();
        assert_eq!(
            canonical_digest(1, &linked).unwrap(),
            canonical_digest(1, &records).unwrap()
        );
        assert!(
            host.closed_release_recovery_receipts(
                origin.operation_id,
                &canonical_digest(1, &"wrong plan").unwrap(),
                second.operation_id
            )
            .is_err()
        );
        for record in &records {
            let path = host
                .receipts
                .directory
                .path()
                .join(format!("{}.json", record.operation_id));
            std::fs::remove_file(&path).unwrap();
            assert!(
                host.closed_release_resume_receipts(second.operation_id)
                    .is_err(),
                "missing {} {}",
                record.kind,
                record.operation_id
            );
            host.receipts.save(record).unwrap();
        }
        for record in &records {
            let mut changed = record.clone();
            if changed.kind == "admission" {
                changed.observed_result.as_mut().unwrap()["unresolved_effects"] =
                    json!(["unconfirmed"]);
            } else if changed.kind == "command/ReleaseRecover" {
                changed.observed_result = Some(json!({"error":{"code":"operation_uncertain"}}));
            } else {
                changed.payload_digest =
                    canonical_digest(1, &"changed protected decision").unwrap();
            }
            host.receipts.save(&changed).unwrap();
            assert!(
                host.closed_release_resume_receipts(second.operation_id)
                    .is_err(),
                "changed {} {}",
                record.kind,
                record.operation_id
            );
            host.receipts.save(record).unwrap();
        }
        let mut broken = first.clone();
        let mut reference = ReleaseResumeReference::from_receipt(&broken)
            .unwrap()
            .unwrap();
        reference.ordinal = 2;
        broken.expected.insert(
            "release_resume".into(),
            serde_json::to_string(&reference).unwrap(),
        );
        broken
            .expected
            .insert("scope".into(), reference.scope_digest().unwrap());
        host.receipts.save(&broken).unwrap();
        assert!(
            host.closed_release_resume_receipts(second.operation_id)
                .is_err()
        );
        host.receipts.save(&first).unwrap();
        // The read-only collector must not reenter the Control state mutex.
        host.serialize_observation(|_| {
            assert_eq!(
                host.closed_release_resume_receipts(second.operation_id)?
                    .len(),
                9
            );
            Ok(())
        })
        .unwrap();
        assert!(host.snapshot().unwrap().active_operations.is_empty());
    }

    #[test]
    fn resume_uses_new_actual_admission_and_cannot_fork_consumed_lineage() {
        let (_temp, host, lease, original) = setup();
        let origin = host
            .retire_local(
                original.operation_id,
                json!({"fixture":"known terminal local phase"}),
            )
            .unwrap();
        let (parent, authorization) = authorize(&host, &original, "resume");
        let verified = reference(&origin, &origin, &parent, &authorization, 1);
        let resumed = host
            .admit_release_resume(&lease, &verified, |_| Ok(()))
            .unwrap();
        assert_ne!(resumed.operation_id, original.operation_id);
        assert_ne!(resumed.scope_digest, original.scope_digest);
        let receipt = host.operation(resumed.operation_id).unwrap();
        let retained = ReleaseResumeReference::from_receipt(&receipt)
            .unwrap()
            .unwrap();
        assert_eq!(retained, verified.reference);
        for key in [
            "run",
            "metadata",
            "continuation",
            "source_advanced",
            "worker_preparation",
            "review_round",
            "review_round_preparation",
            "review_round_resume",
            "review_round_recovery",
            "session_resume",
            "session_recovery_preparation",
        ] {
            let mut mixed = receipt.clone();
            mixed.expected.insert(key.into(), "{}".into());
            assert_eq!(
                ReleaseResumeReference::from_receipt(&mixed)
                    .unwrap_err()
                    .code,
                "record_corrupt",
                "{key}"
            );
        }
        assert_eq!(retained.plan_operation_id(), original.operation_id);
        assert_eq!(retained.ordinal(), 1);
        let acknowledged = host
            .record_release_recovery_completion(
                completion(&original, &parent, "resume"),
                &parent,
                json!({"release_operation":receipt.operation_id,"receipt":receipt}),
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(acknowledged.phase, ReceiptPhase::Acknowledged);
        assert_eq!(
            acknowledged.observed_result.as_ref().unwrap()["release_operation"],
            json!(resumed.operation_id)
        );
        host.retire_local(resumed.operation_id, json!({"fixture":"known local exit"}))
            .unwrap();
        let (second, second_authorization) = authorize(&host, &original, "resume");
        let fork = reference(&origin, &origin, &second, &second_authorization, 1);
        assert_eq!(
            host.admit_release_resume(&lease, &fork, |_| Ok(()))
                .unwrap_err()
                .code,
            "operation_uncertain"
        );
        assert_eq!(
            host.operation(fork.reference.request_id().unwrap())
                .unwrap_err()
                .code,
            "record_missing"
        );
        let previous = host.operation(resumed.operation_id).unwrap();
        let next = reference(&origin, &previous, &second, &second_authorization, 2);
        let next_permit = host
            .admit_release_resume(&lease, &next, |_| Ok(()))
            .unwrap();
        assert_ne!(next_permit.operation_id, resumed.operation_id);
        let next_receipt = host.operation(next_permit.operation_id).unwrap();
        assert_eq!(
            ReleaseResumeReference::from_receipt(&next_receipt)
                .unwrap()
                .unwrap()
                .ordinal(),
            2
        );
        assert_eq!(
            host.operation(original.operation_id)
                .unwrap()
                .payload_digest,
            origin.payload_digest
        );
        let mut transplanted = next_receipt;
        transplanted
            .expected
            .insert("scope".into(), original.scope_digest);
        assert_eq!(
            ReleaseResumeReference::from_receipt(&transplanted)
                .unwrap_err()
                .code,
            "record_corrupt"
        );
    }
    #[test]
    fn retired_worker_preparation_cannot_become_an_original_release_admission() {
        let (_temp, host, lease, original) = setup();
        host.retire_local(
            original.operation_id,
            json!({"fixture":"prior work retired"}),
        )
        .unwrap();
        let preparation = PreparationReference {
            worker_operation: Uuid::new_v4(),
            workspace: Uuid::new_v4(),
            account: "fixture-codex".into(),
            phase: crate::scheduler::PhaseKey {
                work: original.work_key.clone(),
                generation: canonical_digest(1, &"fixture-generation").unwrap(),
                phase: "implementation".into(),
                retry_grant: None,
            },
        };
        let permit = host
            .admit_preparation(
                &lease,
                PreparationAdmission {
                    request_id: preparation.operation_id().unwrap(),
                    work_key: original.work_key.clone(),
                    scope_digest: original.scope_digest.clone(),
                    reference: preparation,
                },
                || Ok(()),
            )
            .unwrap();
        let retired = host
            .retire_local(
                permit.operation_id,
                json!({"fixture":"metadata preparation exited"}),
            )
            .unwrap();
        assert!(
            PreparationReference::from_receipt(&retired)
                .unwrap()
                .is_some()
        );
        let (command, authorization) = authorize(&host, &permit, "resume");
        let verified = reference(&retired, &retired, &command, &authorization, 1);
        assert_eq!(
            host.admit_release_resume(&lease, &verified, |_| Ok(()))
                .unwrap_err()
                .code,
            "ownership_conflict"
        );
        assert_eq!(
            host.operation(verified.reference.request_id().unwrap())
                .unwrap_err()
                .code,
            "record_missing"
        );
        assert!(host.snapshot().unwrap().active_operations.is_empty());
        assert_eq!(
            host.operation(retired.operation_id).unwrap().payload_digest,
            retired.payload_digest
        );
    }

    #[test]
    fn unknown_effect_allows_only_local_recovery_intent_and_keeps_hold() {
        let (_temp, host, lease, original) = setup();
        let effect = request(
            lease.runtime_id,
            "fixture_external_effect",
            vec![original.work_key.clone()],
            Value::Null,
        );
        let effect_id = effect.request_id;
        let (_pending, ()) = host
            .begin_effect(effect, &original, || Ok(()), |_| Ok(()))
            .unwrap();
        let origin = host
            .retire_local(
                original.operation_id,
                json!({"fixture":"local exit, remote unknown"}),
            )
            .unwrap();
        let before = host.operation(effect_id).unwrap();
        let (parent, authorization) = authorize(&host, &original, "abandon");
        assert_eq!(authorization.phase, ReceiptPhase::Acknowledged);
        assert_eq!(
            host.record_release_recovery_completion(
                completion(&original, &parent, "abandon"),
                &parent,
                json!({"release_operation":authorization.operation_id,"receipt":authorization}),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "missing_evidence"
        );
        assert_eq!(host.operation(effect_id).unwrap().phase, before.phase);
        assert!(
            host.snapshot()
                .unwrap()
                .unresolved_operations
                .contains_key(&effect_id)
        );
        assert!(host.snapshot().unwrap().active_operations.is_empty());
        // Even an otherwise valid acknowledged resume request cannot consume
        // the still-unknown original action or erase its retained receipt.
        let (resume_parent, resume_authorization) = authorize(&host, &original, "resume");
        let verified = reference(&origin, &origin, &resume_parent, &resume_authorization, 1);
        assert_eq!(
            host.admit_release_resume(&lease, &verified, |_| Ok(()))
                .unwrap_err()
                .code,
            "reconciliation_required"
        );
        assert_eq!(
            host.operation(verified.reference.request_id().unwrap())
                .unwrap_err()
                .code,
            "record_missing"
        );
        let mut wrong = request(
            lease.runtime_id,
            "linear_comment",
            vec![original.work_key.clone()],
            json!({"operation":original.operation_id,"action":"abandon"}),
        );
        wrong
            .expected
            .insert("parent_command_id".into(), parent.operation_id.to_string());
        wrong.expected.insert(
            "parent_payload_digest".into(),
            parent.payload_digest.clone(),
        );
        assert_eq!(
            host.record_release_recovery_intent(wrong, &parent, |_| Ok(()))
                .unwrap_err()
                .code,
            "scope_violation"
        );
        assert_eq!(
            host.operation(effect_id).unwrap().payload_digest,
            before.payload_digest
        );
    }
    #[test]
    fn operation_reconciliation_completion_preserves_a_different_runtime_wide_hold() {
        let (_temp, host, lease, original) = setup();
        host.retire_local(original.operation_id, json!({"fixture":"not started"}))
            .unwrap();
        let target_request = request(
            lease.runtime_id,
            "fixture_effect",
            vec!["fixture-target".into()],
            Value::Null,
        );
        let target = target_request.request_id;
        assert_eq!(
            host.mutate_observed(
                target_request,
                None,
                Some("recover"),
                |_| Ok(()),
                |_| { Err(Error::new("operation_uncertain", "fixture lost reply")) }
            )
            .unwrap_err()
            .code,
            "operation_uncertain"
        );
        let unknown = host.operation(target).unwrap();
        let other_request = request(
            lease.runtime_id,
            "fixture_other_effect",
            vec![lease.runtime_id.to_string()],
            Value::Null,
        );
        let other = other_request.request_id;
        assert_eq!(
            host.mutate_observed(
                other_request,
                None,
                Some("recover"),
                |_| Ok(()),
                |_| {
                    Err(Error::new(
                        "operation_uncertain",
                        "different fixture lost reply",
                    ))
                }
            )
            .unwrap_err()
            .code,
            "operation_uncertain"
        );
        let other_before = host.operation(other).unwrap();
        host.lifecycle(
            request(
                lease.runtime_id,
                "lifecycle",
                vec![lease.runtime_id.to_string()],
                Value::Null,
            ),
            Lifecycle::Stop,
            1,
        )
        .unwrap();
        let (parent, _) = host
            .queue_command(
                request(
                    lease.runtime_id,
                    "command/OperationReconcile",
                    vec![target.to_string()],
                    json!({"arguments":{"operation":target},"secret_digest":null}),
                ),
                "recover",
                || Ok(()),
            )
            .unwrap();
        let completion = || {
            let mut effect = request(
                lease.runtime_id,
                "decision/OperationReconcile",
                vec![target.to_string()],
                json!({"operation":target}),
            );
            effect
                .expected
                .insert("parent_command_id".into(), parent.operation_id.to_string());
            effect.expected.insert(
                "parent_payload_digest".into(),
                parent.payload_digest.clone(),
            );
            effect
        };
        assert_eq!(
            host.record_operation_reconciliation_completion(
                completion(),
                &parent,
                json!({"reconciled_operation":target,"receipt":unknown}),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "missing_evidence"
        );
        let reconciled = host
            .reconcile_operation(&actor(), target, |receipt| {
                Ok(Reconciliation::UniqueVerified {
                    remote_ids: BTreeMap::new(),
                    result: json!({"fixture":"exact observed outcome"}),
                    payload_digest: receipt.payload_digest.clone(),
                    actor: receipt.actor.clone(),
                })
            })
            .unwrap();
        assert_eq!(
            host.mutate_observed(
                completion(),
                None,
                Some("recover"),
                |_| Ok(()),
                |_| { panic!("generic completion must remain blocked by the other runtime hold") }
            )
            .unwrap_err()
            .code,
            "reconciliation_required"
        );
        let mut altered = reconciled.clone();
        altered.observed_result = Some(json!({"invented":"different result"}));
        assert_eq!(
            host.record_operation_reconciliation_completion(
                completion(),
                &parent,
                json!({"reconciled_operation":target,"receipt":altered}),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "ownership_conflict"
        );
        let result = host
            .record_operation_reconciliation_completion(
                completion(),
                &parent,
                json!({"reconciled_operation":target,"receipt":reconciled}),
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(result.phase, ReceiptPhase::Acknowledged);
        assert_eq!(
            result.observed_result.as_ref().unwrap()["receipt"]["operation_id"],
            json!(target)
        );
        let after = host.snapshot().unwrap();
        assert_eq!(after.intent.desired, Intent::Stopped);
        assert!(after.active_operations.is_empty());
        assert!(after.unresolved_operations.contains_key(&other));
        assert!(!after.unresolved_operations.contains_key(&target));
        assert_eq!(
            canonical_digest(1, &host.operation(other).unwrap()).unwrap(),
            canonical_digest(1, &other_before).unwrap()
        );
    }

    #[test]
    fn resume_requires_actual_terminal_ownership_current_intent_and_fresh_gate() {
        let (_temp, host, lease, original) = setup();
        assert_eq!(host.active_lease().unwrap(), lease);
        let (parent, authorization) = authorize(&host, &original, "resume");
        let live = host.operation(original.operation_id).unwrap();
        let verified = reference(&live, &live, &parent, &authorization, 1);
        assert_eq!(
            host.admit_release_resume(&lease, &verified, |_| Ok(()))
                .unwrap_err()
                .code,
            "ownership_conflict"
        );
        let origin = host
            .retire_local(
                original.operation_id,
                json!({"fixture":"actual local retirement"}),
            )
            .unwrap();
        let verified = reference(&origin, &origin, &parent, &authorization, 1);
        assert_eq!(
            host.admit_release_resume(&lease, &verified, |_| Err(Error::new(
                "stale_result",
                "acceptance changed"
            )))
            .unwrap_err()
            .code,
            "stale_result"
        );
        assert_eq!(
            host.operation(verified.reference.request_id().unwrap())
                .unwrap_err()
                .code,
            "record_missing"
        );
        host.lifecycle(
            request(
                lease.runtime_id,
                "lifecycle",
                vec![lease.runtime_id.to_string()],
                Value::Null,
            ),
            Lifecycle::Pause,
            1,
        )
        .unwrap();
        assert_eq!(
            host.admit_release_resume(&lease, &verified, |_| Ok(()))
                .unwrap_err()
                .code,
            "admission_closed"
        );
        assert!(host.snapshot().unwrap().active_operations.is_empty());
        assert_eq!(host.active_lease().unwrap_err().code, "admission_closed");
    }
}
