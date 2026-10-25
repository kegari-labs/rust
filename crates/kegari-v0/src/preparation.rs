//! Bounded host publication of human-authorized parent scope. This is separate
//! from agent execution and cannot grant authorization from an agent's output.
use crate::{
    app::ConfigurationControl,
    configuration::{ConfigurationInputs, NativeConfigurationInputs},
    connections::ConnectionRegistry,
    control::{
        Actor, ControlHost, Lease, MetadataAdmission, MetadataReference, MutationRequest, Receipt,
        ReceiptPhase, Reconciliation,
    },
    digest::{bytes_digest, canonical_digest},
    error::{Error, Result},
    evidence::{AttestationKey, Publication, decode_signed_publication, encode_signed_publication},
    observations::ObservationCache,
    providers::{LinearClient, ProviderBroker, ProviderMutation},
    reconcile::{AuthorizationGate, VerifiedAuthorization},
    secrets::SecretStore,
};
use chrono::{Duration, Utc};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;

pub type OwnerCheck = Arc<dyn Fn(&Lease) -> Result<()> + Send + Sync>;

pub struct AuthorizationPublisher {
    configuration: Arc<dyn ConfigurationControl>,
    control: Arc<ControlHost>,
    observations: Arc<ObservationCache>,
    secrets: Arc<dyn SecretStore>,
    key: Arc<AttestationKey>,
    inputs: Arc<dyn ConfigurationInputs>,
    owner: OwnerCheck,
}
impl AuthorizationPublisher {
    /// Stable next request identity, derived from protected admission history.
    /// A new identity is possible only after an exited, known-not-sent retryable
    /// attempt. Unknown and acknowledged effects always keep their original ID.
    pub fn next_publication(&self, gate: &AuthorizationGate) -> Result<Uuid> {
        Ok(self.plan(gate)?.operation)
    }
    fn plan(&self, gate: &AuthorizationGate) -> Result<PublicationAttempt> {
        let config = self.configuration.snapshot()?;
        publication_attempt(
            config.runtime.id,
            &canonical_digest(1, gate.snapshot())?,
            &gate.work_key()?,
            &self.control.operations()?,
            &config.runtime,
            Utc::now(),
        )
    }
    pub fn new(
        configuration: Arc<dyn ConfigurationControl>,
        control: Arc<ControlHost>,
        observations: Arc<ObservationCache>,
        secrets: Arc<dyn SecretStore>,
        key: Arc<AttestationKey>,
        owner: OwnerCheck,
    ) -> Self {
        Self::with_inputs(
            configuration,
            control,
            observations,
            secrets,
            key,
            Arc::new(NativeConfigurationInputs),
            owner,
        )
    }
    pub fn with_inputs(
        configuration: Arc<dyn ConfigurationControl>,
        control: Arc<ControlHost>,
        observations: Arc<ObservationCache>,
        secrets: Arc<dyn SecretStore>,
        key: Arc<AttestationKey>,
        inputs: Arc<dyn ConfigurationInputs>,
        owner: OwnerCheck,
    ) -> Self {
        Self {
            configuration,
            control,
            observations,
            secrets,
            key,
            inputs,
            owner,
        }
    }
    fn current(&self, gate: &AuthorizationGate, lease: &Lease) -> Result<()> {
        (self.owner)(lease)?;
        self.configuration
            .authority_fences()?
            .check_workspace(gate.workspace_id())?;
        if self.configuration.snapshot()?.revision()? != gate.config_revision() {
            return Err(Error::new(
                "stale_config",
                "configuration changed before authorization publication",
            ));
        }
        let projection = self.observations.admission_snapshot()?;
        let current = projection
            .parent(gate.workspace_id(), gate.snapshot().parent.issue)?
            .authorization_gate(Utc::now(), Duration::seconds(30))?;
        gate.compare_current(&current, Utc::now(), Duration::seconds(30))
    }
    fn client(&self, gate: &AuthorizationGate) -> Result<(LinearClient, String, Uuid)> {
        let config = self.configuration.snapshot()?;
        if config.revision()? != gate.config_revision() {
            return Err(Error::new(
                "stale_config",
                "authorization preparation configuration changed",
            ));
        }
        self.client_for_workspace(gate.workspace_id())
    }
    fn client_for_workspace(&self, workspace_id: Uuid) -> Result<(LinearClient, String, Uuid)> {
        let config = self.configuration.snapshot()?;
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == workspace_id)
            .ok_or_else(|| Error::new("scope_violation", "authorization workspace was removed"))?;
        let transports = config
            .connections
            .keys()
            .filter_map(|name| {
                self.inputs
                    .transport(name)
                    .map(|transport| (name.clone(), transport))
            })
            .collect();
        let registry = ConnectionRegistry::bind_with_transports(
            &config,
            self.secrets.as_ref(),
            |name| self.inputs.environment(name),
            &transports,
        )?;
        let client = registry
            .clients(&config)?
            .get(&workspace.id)
            .ok_or_else(|| {
                Error::new(
                    "missing_secret",
                    "authorization workspace provider connection is unavailable",
                )
            })?
            .linear
            .clone();
        Ok((
            client,
            config.connections[&workspace.linear_connection]
                .account_id
                .clone(),
            config.runtime.id,
        ))
    }
    /// Recover an already transmitted authorization publication from its exact
    /// current native record. This creates no authorization/admission and never
    /// resends a missing result. The host supplies its actual control-owner check;
    /// it must not reenter ControlHost, because the final check is serialized.
    pub async fn recover_receipt(
        &self,
        operation: Uuid,
        current_owner: impl Fn() -> Result<()> + Send + Sync,
    ) -> Result<VerifiedAuthorization> {
        current_owner()?;
        let receipt = self.control.operation(operation)?;
        let config = self.configuration.snapshot()?;
        let parse_id = |name: &str| {
            receipt
                .expected
                .get(name)
                .and_then(|s| Uuid::parse_str(s).ok())
                .filter(|id| !id.is_nil())
                .ok_or_else(|| {
                    Error::new(
                        "record_corrupt",
                        format!("authorization receipt lacks {name} identity"),
                    )
                })
        };
        let workspace_id = parse_id("workspace")?;
        let parent_id = parse_id("parent")?;
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == workspace_id)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "authorization recovery workspace is unavailable",
                )
            })?;
        let connection = &config.connections[&workspace.linear_connection];
        let endpoint = connection
            .endpoint
            .as_deref()
            .ok_or_else(|| Error::invalid("Linear endpoint is unavailable"))?;
        let expected_target = format!(
            "linear:{endpoint}:{}:{parent_id}",
            workspace.linear_workspace_id
        );
        let (linear, writer, runtime) = self.client_for_workspace(workspace_id)?;
        if receipt.runtime_id != runtime
            || receipt.kind != "linear_comment"
            || receipt.expected.get("preparation").map(String::as_str)
                != Some("parent_authorization")
            || receipt.expected.get("writer") != Some(&writer)
            || receipt.expected.get("provider_writer") != Some(&writer)
            || receipt.canonical_targets != vec![expected_target]
            || receipt.expected.get("body_digest") != receipt.expected.get("provider_body_digest")
            || !receipt.expected.contains_key("body_digest")
            || receipt.observed_result.as_ref().is_some_and(|r| {
                r.get("effect").and_then(serde_json::Value::as_str) == Some("not_sent")
            })
        {
            return Err(Error::new(
                "ownership_conflict",
                "receipt is not the exact owned authorization publication",
            ));
        }
        let comments = linear.comments(parent_id).await?;
        let matches = comments
            .iter()
            .filter(|comment| {
                comment.issue_id == parent_id
                    && !comment.bot
                    && !comment.delegated
                    && comment.user_id.is_some_and(|id| id.to_string() == writer)
                    && receipt.expected.get("body_digest")
                        == Some(&bytes_digest(comment.body.as_bytes()))
            })
            .collect::<Vec<_>>();
        let [comment] = matches.as_slice() else {
            return Err(Error::new(
                if matches.is_empty() {
                    "operation_uncertain"
                } else {
                    "ownership_conflict"
                },
                "authorization recovery requires exactly one unchanged native record; no record will be resent",
            ));
        };
        let signed = decode_signed_publication::<crate::reconcile::AuthorizationSnapshot>(
            &comment.body,
            "parent_authorization",
            &self.key,
        )?;
        let snapshot_digest = canonical_digest(1, &signed.payload)?;
        let admission_id = parse_id("admission_operation")?;
        let parent_key =
            crate::authority::IssueKey::new(endpoint, workspace.linear_workspace_id, parent_id)?;
        let admission = self.control.operation(admission_id)?;
        let attempt = MetadataReference::from_receipt(&admission)?;
        let ordinal = attempt.as_ref().map_or(1, |reference| reference.attempt);
        if signed.payload.parent != parent_key
            || receipt.expected.get("snapshot") != Some(&snapshot_digest)
            || operation != attempt_id(runtime, &snapshot_digest, "publication", ordinal)?
            || admission_id != attempt_id(runtime, &snapshot_digest, "admission", ordinal)?
            || attempt
                .as_ref()
                .is_some_and(|reference| reference.generation != snapshot_digest)
            || attempt.as_ref().is_some_and(|reference| {
                receipt.expected.get("metadata_attempt") != Some(&reference.attempt.to_string())
                    || receipt.expected.get("metadata_max_attempts")
                        != Some(&reference.max_attempts.to_string())
            })
            || admission.kind != "admission"
            || admission.runtime_id != runtime
            || admission.actor != receipt.actor
            || admission.owner_generation != receipt.owner_generation
            || admission.canonical_targets != vec![parent_key.identity()?]
            || admission.expected.contains_key("run")
            || !admission.expected.contains_key("scope")
            || receipt.expected.get("admission_operation") != Some(&admission_id.to_string())
            || receipt.expected.get("admitted_scope") != admission.expected.get("scope")
            || receipt
                .remote_ids
                .get("comment_id")
                .is_some_and(|id| id != &comment.id.to_string())
        {
            return Err(Error::new(
                "ownership_conflict",
                "authorization native record differs from its original admission",
            ));
        }
        let publication = Publication {
            provider: "linear".into(),
            record_id: comment.id.to_string(),
            writer_account_id: writer.clone(),
            payload_digest: canonical_digest(1, &signed)?,
            observed_at: Utc::now(),
            url: comment.url.clone(),
        };
        let verified = VerifiedAuthorization::verify(&self.key, &signed, &publication, &writer)?;
        self.control.reconcile_owned_checked(&receipt,Reconciliation::UniqueVerified {
            remote_ids:BTreeMap::from([("comment_id".into(),comment.id.to_string())]),
            result:json!({"publication":publication,"authorization_digest":verified.digest(),"recovery":"observed_existing_native_record"}),
            payload_digest:receipt.payload_digest.clone(),actor:receipt.actor.clone(),
        },||{
            current_owner()?;
            if self.configuration.snapshot()?.revision()?!=config.revision()? {
                return Err(Error::new("stale_config","authorization recovery configuration changed"));
            }
            Ok(())
        })?;
        // This admission only owned the finite publication task (no model or
        // native process). Its exact side effect is now known. Active tasks are
        // rejected by reconcile_owned_checked; unknown effects are never resent.
        if admission.phase != crate::control::ReceiptPhase::Reconciled {
            self.control.reconcile_owned_checked(&admission,Reconciliation::UniqueVerified {
                remote_ids:BTreeMap::from([("publication_operation".into(),operation.to_string())]),
                result:json!({"metadata_task":"authorization_publication_verified","publication_operation":operation,"authorization_digest":verified.digest()}),
                payload_digest:admission.payload_digest.clone(),actor:admission.actor.clone(),
            },&current_owner)?;
        }
        // Historical publication recovery cannot restore current authorization.
        // Ordinary observation reconstruction decides present scope separately.
        self.observations.refresh().await?;
        Ok(verified)
    }
    /// Called only by the RUNNING supervisor after complete provider refresh.
    /// Stable request IDs preserve uncertainty across repeat polls and restart.
    pub async fn publish(
        self: &Arc<Self>,
        lease: &Lease,
        gate: AuthorizationGate,
    ) -> Result<VerifiedAuthorization> {
        let (linear, writer, runtime) = self.client(&gate)?;
        let signed = self.key.sign("parent_authorization", gate.snapshot())?;
        let body = encode_signed_publication("parent_authorization", &signed)?;
        let plan = self.plan(&gate)?;
        let operation = plan.operation;
        let admission = plan.admission;
        let prior = self
            .control
            .operations()?
            .into_iter()
            .find(|r| r.operation_id == operation);
        if let Some(receipt) = prior {
            // Recovery reads are safe in PAUSED/STOPPED. They can adopt one exact
            // record, but neither a missing result nor a replay sends another.
            return self
                .recover_publication(&linear, &writer, &gate, &body, &signed, receipt)
                .await;
        }
        self.current(&gate, lease)?;
        let permit = self.control.admit_metadata(
            lease,
            MetadataAdmission {
                request_id: admission,
                work_key: gate.work_key()?,
                scope_digest: gate.digest()?,
                reference: plan.reference.clone(),
            },
            || self.current(&gate, lease),
        )?;
        let mut expected = BTreeMap::from([
            ("preparation".into(), "parent_authorization".into()),
            ("workspace".into(), gate.workspace_id().to_string()),
            ("parent".into(), gate.snapshot().parent.issue.to_string()),
            ("snapshot".into(), canonical_digest(1, gate.snapshot())?),
            ("body_digest".into(), bytes_digest(body.as_bytes())),
            ("writer".into(), writer.clone()),
            (
                "metadata_attempt".into(),
                plan.reference.attempt.to_string(),
            ),
            (
                "metadata_max_attempts".into(),
                plan.reference.max_attempts.to_string(),
            ),
        ]);
        if let Some(previous) = plan.previous {
            expected.insert("previous_publication".into(), previous.to_string());
        }
        let request = MutationRequest {
            request_id: operation,
            runtime_id: runtime,
            actor: Actor {
                identity: format!("owner:{}", lease.owner_id),
                roles: Default::default(),
                principal: None,
            },
            kind: "parent_authorization".into(),
            canonical_targets: vec![gate.work_key()?],
            expected,
            payload: json!({}),
        };
        let this = self.clone();
        let checked_gate = gate.clone();
        let checked_lease = lease.clone();
        let result = ProviderBroker {
            linear: Some(linear.clone()),
            github: None,
        }
        .mutate(
            self.control.clone(),
            request,
            Some(permit),
            None,
            ProviderMutation::LinearComment {
                issue: gate.snapshot().parent.issue,
                body: body.clone(),
            },
            move || this.current(&checked_gate, &checked_lease),
        )
        .await;
        // The owned broker future has returned; remote uncertainty does not keep
        // this local preparation task falsely alive or clear its effect hold.
        self.control.retire_local(admission,json!({"phase":"authorization_preparation","result":match &result{Ok(_)=>"provider_acknowledged",Err(_)=>"blocked"},"error":result.as_ref().err()}))?;
        let receipt = result?;
        self.recover_publication(&linear, &writer, &gate, &body, &signed, receipt)
            .await
    }
    async fn recover_publication(
        &self,
        linear: &LinearClient,
        writer: &str,
        gate: &AuthorizationGate,
        body: &str,
        signed: &crate::evidence::Signed<crate::reconcile::AuthorizationSnapshot>,
        receipt: Receipt,
    ) -> Result<VerifiedAuthorization> {
        if let Some(error) = receipt
            .observed_result
            .as_ref()
            .and_then(|value| value.get("error"))
        {
            return Err(serde_json::from_value(error.clone()).map_err(|_| {
                Error::new(
                    "record_corrupt",
                    "authorization preparation rejection is invalid",
                )
            })?);
        }
        if receipt.runtime_id != self.configuration.snapshot()?.runtime.id
            || receipt.kind != "linear_comment"
            || receipt.expected.get("preparation").map(String::as_str)
                != Some("parent_authorization")
            || receipt.expected.get("snapshot") != Some(&canonical_digest(1, gate.snapshot())?)
            || receipt.expected.get("body_digest") != Some(&bytes_digest(body.as_bytes()))
            || receipt.expected.get("writer").map(String::as_str) != Some(writer)
        {
            return Err(Error::new(
                "ownership_conflict",
                "authorization publication receipt differs from exact current snapshot",
            ));
        }
        let matches: Vec<_> = linear
            .comments(gate.snapshot().parent.issue)
            .await?
            .into_iter()
            .filter(|comment| {
                comment.issue_id == gate.snapshot().parent.issue
                    && comment.body == body
                    && !comment.bot
                    && !comment.delegated
                    && comment.user_id.is_some_and(|id| id.to_string() == writer)
            })
            .collect();
        if matches.len() != 1 {
            return Err(Error::new(
                if matches.is_empty() {
                    "operation_uncertain"
                } else {
                    "ownership_conflict"
                },
                "authorization publication requires one independently observed exact provider record; it will not be resent",
            ));
        }
        let comment = &matches[0];
        if receipt
            .remote_ids
            .get("comment_id")
            .is_some_and(|id| id != &comment.id.to_string())
        {
            return Err(Error::new(
                "ownership_conflict",
                "acknowledged authorization record differs from current provider observation",
            ));
        }
        let publication = Publication {
            provider: "linear".into(),
            record_id: comment.id.to_string(),
            writer_account_id: writer.into(),
            payload_digest: canonical_digest(1, signed)?,
            observed_at: Utc::now(),
            url: comment.url.clone(),
        };
        let verified = VerifiedAuthorization::verify(&self.key, signed, &publication, writer)?;
        self.control.reconcile_owned(
            &receipt,
            Reconciliation::UniqueVerified {
                remote_ids: BTreeMap::from([("comment_id".into(), comment.id.to_string())]),
                result: json!({"publication":publication,"authorization_digest":verified.digest()}),
                payload_digest: receipt.payload_digest.clone(),
                actor: receipt.actor.clone(),
            },
        )?;
        // Publication is not admission. Reconstruct the current provider scope
        // again so a raced correction/removal cannot inherit this old proof.
        let current = self.observations.refresh().await?;
        let parent = current.parent(gate.workspace_id(), gate.snapshot().parent.issue)?;
        if !parent.authorization_published
            || parent
                .authorization_candidate
                .as_ref()
                .map(|s| canonical_digest(1, s))
                .transpose()?
                .as_deref()
                != Some(canonical_digest(1, gate.snapshot())?.as_str())
        {
            return Err(Error::new(
                "stale_authorization",
                "published record is historical after current provider reconciliation",
            ));
        }
        Ok(verified)
    }
}
pub(crate) struct PublicationAttempt {
    pub(crate) operation: Uuid,
    pub(crate) admission: Uuid,
    pub(crate) reference: MetadataReference,
    pub(crate) previous: Option<Uuid>,
}
fn publication_attempt(
    runtime: Uuid,
    snapshot: &str,
    work: &str,
    receipts: &[Receipt],
    config: &crate::config::RuntimeConfig,
    now: chrono::DateTime<Utc>,
) -> Result<PublicationAttempt> {
    metadata_publication_attempt(
        "parent_authorization",
        runtime,
        snapshot,
        work,
        receipts,
        config,
        now,
    )
}
pub(crate) fn metadata_publication_attempt(
    purpose: &str,
    runtime: Uuid,
    snapshot: &str,
    work: &str,
    receipts: &[Receipt],
    config: &crate::config::RuntimeConfig,
    now: chrono::DateTime<Utc>,
) -> Result<PublicationAttempt> {
    if !matches!(purpose, "parent_authorization" | "no_release_required") {
        return Err(Error::invalid(
            "unknown finite metadata publication purpose",
        ));
    }
    let mut history = BTreeMap::new();
    let mut cap = config.max_attempts_per_phase;
    for receipt in receipts {
        if let Some(reference) = MetadataReference::from_receipt(receipt)?
            && reference.purpose == purpose
            && reference.generation == snapshot
        {
            if receipt.runtime_id != runtime
                || receipt.canonical_targets != vec![work.to_string()]
                || receipt.operation_id
                    != metadata_attempt_id(
                        purpose,
                        runtime,
                        snapshot,
                        "admission",
                        reference.attempt,
                    )?
                || history.insert(reference.attempt, receipt).is_some()
            {
                return Err(Error::new(
                    "record_corrupt",
                    "authorization metadata history has conflicting ownership",
                ));
            }
            cap = cap.min(reference.max_attempts);
        }
    }
    if history.keys().copied().ne(1..=history.len() as u32) {
        return Err(Error::new(
            "record_corrupt",
            "authorization metadata history has a gap",
        ));
    }
    let make = |attempt, previous| -> Result<PublicationAttempt> {
        Ok(PublicationAttempt {
            operation: metadata_attempt_id(purpose, runtime, snapshot, "publication", attempt)?,
            admission: metadata_attempt_id(purpose, runtime, snapshot, "admission", attempt)?,
            reference: MetadataReference {
                purpose: purpose.into(),
                generation: snapshot.into(),
                attempt,
                max_attempts: cap,
            },
            previous,
        })
    };
    let mut previous = None;
    for (attempt, admission) in &history {
        let plan = make(*attempt, previous)?;
        let effect = receipts
            .iter()
            .find(|receipt| receipt.operation_id == plan.operation);
        if let Some(effect) = effect {
            if effect.runtime_id != runtime
                || effect.kind != "linear_comment"
                || effect.expected.get("admission_operation")
                    != Some(&admission.operation_id.to_string())
                || effect.expected.get("snapshot").map(String::as_str) != Some(snapshot)
            {
                return Err(Error::new(
                    "record_corrupt",
                    "authorization publication differs from its metadata admission",
                ));
            }
            if effect.phase != ReceiptPhase::Reconciled
                || effect
                    .observed_result
                    .as_ref()
                    .is_none_or(|value| value["effect"] != "not_sent")
            {
                return Ok(plan);
            }
        }
        let exited = admission.phase == ReceiptPhase::Reconciled
            && admission.observed_result.as_ref().is_some_and(|value| {
                value["local_execution"] == "exited"
                    && value["unresolved_effects"]
                        .as_array()
                        .is_some_and(Vec::is_empty)
            });
        if !exited {
            return Ok(plan);
        }
        let value = effect
            .and_then(|receipt| receipt.observed_result.as_ref())
            .and_then(|value| value.get("error"))
            .or_else(|| {
                admission
                    .observed_result
                    .as_ref()
                    .and_then(|value| value.pointer("/observation/error"))
            });
        let error: Error = value
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "retired authorization preparation has no classified outcome",
                )
            })?;
        if !retryable_preparation(&error) {
            return Err(error);
        }
        let updated = effect.map_or(admission.updated_at, |receipt| {
            receipt.updated_at.max(admission.updated_at)
        });
        let retry_after_ms = error
            .details
            .get("retry_after_ms")
            .and_then(serde_json::Value::as_u64)
            .into_iter()
            .chain(
                error
                    .details
                    .pointer("/rate_limit/retry_after_ms")
                    .and_then(serde_json::Value::as_u64),
            )
            .max();
        let mut retry_at =
            crate::scheduler::retry_at(updated, *attempt, config.retry_cap_ms, retry_after_ms)?;
        if let Some(reset) = error
            .details
            .pointer("/rate_limit/reset_at")
            .filter(|value| !value.is_null())
        {
            let reset: chrono::DateTime<Utc> = serde_json::from_value(reset.clone())
                .map_err(|_| Error::new("record_corrupt", "provider reset bound is invalid"))?;
            retry_at = retry_at.max(reset);
        }
        if now < retry_at {
            return Err(Error::new(
                "retry_wait",
                "authorization publication retry is waiting for its bounded backoff",
            )
            .detail(json!({"retry_at":retry_at,"attempt":attempt})));
        }
        previous = Some(plan.operation);
    }
    let attempt = history.len() as u32 + 1;
    if attempt > cap {
        return Err(Error::new(
            "attempts_exhausted",
            "authorization preparation exhausted its durable attempt budget",
        ));
    }
    make(attempt, previous)
}
fn retryable_preparation(error: &Error) -> bool {
    matches!(error.code.as_str(), "provider_transport" | "timeout")
        || (error.retryable && matches!(error.code.as_str(), "provider_status" | "rate_limited"))
}
fn attempt_id(runtime: Uuid, snapshot: &str, phase: &str, attempt: u32) -> Result<Uuid> {
    metadata_attempt_id("parent_authorization", runtime, snapshot, phase, attempt)
}
pub(crate) fn metadata_attempt_id(
    purpose: &str,
    runtime: Uuid,
    snapshot: &str,
    phase: &str,
    attempt: u32,
) -> Result<Uuid> {
    let phase = if purpose == "parent_authorization" {
        phase.to_owned()
    } else {
        format!("{purpose}:{phase}")
    };
    publication_id(
        runtime,
        snapshot,
        &if attempt == 1 {
            phase
        } else {
            format!("{phase}:{attempt}")
        },
    )
}
fn publication_id(runtime: Uuid, snapshot: &str, phase: &str) -> Result<Uuid> {
    let digest = canonical_digest(
        1,
        &json!({"namespace":"kegari-v0/authorization-preparation","runtime":runtime,"snapshot":snapshot,"phase":phase}),
    )?;
    let digest = digest
        .strip_prefix("sha256:v1:")
        .ok_or_else(|| Error::invalid("publication digest algorithm"))?;
    let mut bytes = [0; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digest[index * 2..index * 2 + 2], 16)
            .map_err(|_| Error::invalid("publication digest encoding"))?;
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> crate::config::RuntimeConfig {
        serde_json::from_value(json!({"id":Uuid::from_u128(1),"host_id":"fixture","max_agents":3}))
            .unwrap()
    }
    fn history(attempt: u32, cap: u32, sent: bool) -> Vec<Receipt> {
        let config = config();
        let snapshot = canonical_digest(1, &"snapshot").unwrap();
        let admission = attempt_id(config.id, &snapshot, "admission", attempt).unwrap();
        let publication = attempt_id(config.id, &snapshot, "publication", attempt).unwrap();
        let reference = MetadataReference {
            purpose: "parent_authorization".into(),
            generation: snapshot.clone(),
            attempt,
            max_attempts: cap,
        };
        let mut error = Error::new(
            "provider_transport",
            "fixture preflight failed before dispatch",
        );
        error.retryable = true;
        let first = Receipt {
            operation_id: admission,
            request_id: admission,
            runtime_id: config.id,
            owner_generation: 1,
            actor: "fixture".into(),
            kind: "admission".into(),
            canonical_targets: vec!["work".into()],
            payload_digest: snapshot.clone(),
            expected: BTreeMap::from([(
                "metadata".into(),
                serde_json::to_string(&reference).unwrap(),
            )]),
            phase: ReceiptPhase::Reconciled,
            remote_ids: BTreeMap::new(),
            observed_result: Some(
                json!({"local_execution":"exited","unresolved_effects":[],"observation":{"error":error}}),
            ),
            cancellation: "none".into(),
            updated_at: Utc::now() - Duration::minutes(5),
        };
        let effect = Receipt {
            operation_id: publication,
            request_id: publication,
            kind: "linear_comment".into(),
            canonical_targets: vec!["linear:fixture".into()],
            expected: BTreeMap::from([
                ("admission_operation".into(), admission.to_string()),
                ("snapshot".into(), snapshot),
            ]),
            phase: if sent {
                ReceiptPhase::Sent
            } else {
                ReceiptPhase::Reconciled
            },
            observed_result: if sent {
                None
            } else {
                Some(json!({"effect":"not_sent","error":error}))
            },
            ..first.clone()
        };
        vec![first, effect]
    }
    fn planned(
        receipts: &[Receipt],
        config: &crate::config::RuntimeConfig,
    ) -> Result<PublicationAttempt> {
        publication_attempt(
            config.id,
            &canonical_digest(1, &"snapshot").unwrap(),
            "work",
            receipts,
            config,
            Utc::now(),
        )
    }
    #[test]
    fn authorization_retries_only_known_no_effect_with_durable_budget() {
        let mut config = config();
        let initial = planned(&[], &config).unwrap();
        let first = history(1, 2, false);
        let retry = planned(&first, &config).unwrap();
        assert_ne!(initial.operation, retry.operation);
        assert_eq!(retry.reference.attempt, 2);
        assert_eq!(retry.previous, Some(initial.operation));
        let mut exhausted = first.clone();
        exhausted.extend(history(2, 2, false));
        config.max_attempts_per_phase = 100;
        assert_eq!(
            planned(&exhausted, &config).err().unwrap().code,
            "attempts_exhausted"
        );
        config.max_attempts_per_phase = 1;
        assert_eq!(
            planned(&first, &config).err().unwrap().code,
            "attempts_exhausted"
        );
        let unknown = history(1, 3, true);
        assert_eq!(
            planned(&unknown, &config).unwrap().operation,
            initial.operation
        );
        let mut no_receipt = history(1, 3, false);
        no_receipt.pop();
        config.max_attempts_per_phase = 3;
        assert_eq!(planned(&no_receipt, &config).unwrap().reference.attempt, 2);
    }
    #[test]
    fn authorization_retry_preserves_holds_backoff_and_permanent_failures() {
        let config = config();
        let initial = planned(&[], &config).unwrap();
        let mut held = history(1, 3, false);
        held[0].observed_result.as_mut().unwrap()["unresolved_effects"] = json!([Uuid::new_v4()]);
        assert_eq!(
            planned(&held, &config).unwrap().operation,
            initial.operation
        );
        let mut delayed = history(1, 3, false);
        delayed[1].updated_at = Utc::now();
        assert_eq!(planned(&delayed, &config).err().unwrap().code, "retry_wait");
        delayed[1].updated_at = Utc::now() - Duration::minutes(5);
        delayed[1].observed_result.as_mut().unwrap()["error"]["details"] =
            json!({"retry_after_ms":600000});
        assert_eq!(planned(&delayed, &config).err().unwrap().code, "retry_wait");
        delayed[1].observed_result.as_mut().unwrap()["error"]["details"] =
            json!({"rate_limit":{"retry_after_ms":600000}});
        assert_eq!(planned(&delayed, &config).err().unwrap().code, "retry_wait");
        delayed[1].observed_result.as_mut().unwrap()["error"]["details"] =
            json!({"rate_limit":{"reset_at":Utc::now()+Duration::minutes(10)}});
        assert_eq!(planned(&delayed, &config).err().unwrap().code, "retry_wait");
        let mut blocked = history(1, 3, false);
        blocked[1].observed_result.as_mut().unwrap()["error"] =
            json!(Error::new("access_denied", "fixture"));
        assert_eq!(
            planned(&blocked, &config).err().unwrap().code,
            "access_denied"
        );
        blocked[1].observed_result.as_mut().unwrap()["error"]["retryable"] = json!(true);
        assert_eq!(
            planned(&blocked, &config).err().unwrap().code,
            "access_denied"
        );
        let mut ambiguous = history(1, 3, false);
        ambiguous[1].observed_result = Some(json!({"effect":"unknown"}));
        assert_eq!(
            planned(&ambiguous, &config).unwrap().operation,
            initial.operation
        );
        let mut gap = history(2, 3, false);
        assert_eq!(planned(&gap, &config).err().unwrap().code, "record_corrupt");
        gap = history(1, 3, false);
        gap[1]
            .expected
            .insert("admission_operation".into(), Uuid::new_v4().to_string());
        assert_eq!(planned(&gap, &config).err().unwrap().code, "record_corrupt");
    }
    #[test]
    fn no_release_metadata_retries_are_separate_bounded_and_never_replay_unknowns() {
        let mut config = config();
        let snapshot = canonical_digest(1, &"snapshot").unwrap();
        let make = |receipts: &[Receipt], config: &crate::config::RuntimeConfig| {
            metadata_publication_attempt(
                "no_release_required",
                config.id,
                &snapshot,
                "work",
                receipts,
                config,
                Utc::now(),
            )
        };
        let initial = make(&[], &config).unwrap();
        assert_ne!(initial.operation, planned(&[], &config).unwrap().operation);
        assert_ne!(initial.admission, planned(&[], &config).unwrap().admission);
        let mut retained = history(1, 2, false);
        let mut reference = initial.reference.clone();
        reference.max_attempts = 2;
        retained[0].operation_id = initial.admission;
        retained[0].request_id = initial.admission;
        retained[0].expected.insert(
            "metadata".into(),
            serde_json::to_string(&reference).unwrap(),
        );
        retained[1].operation_id = initial.operation;
        retained[1].request_id = initial.operation;
        retained[1]
            .expected
            .insert("admission_operation".into(), initial.admission.to_string());
        let retry = make(&retained, &config).unwrap();
        assert_eq!(retry.reference.attempt, 2);
        assert_eq!(retry.reference.max_attempts, 2);
        assert_eq!(retry.previous, Some(initial.operation));
        config.max_attempts_per_phase = 99;
        assert_eq!(make(&retained, &config).unwrap().reference.max_attempts, 2);
        retained[1].phase = ReceiptPhase::Sent;
        retained[1].observed_result = None;
        assert_eq!(
            make(&retained, &config).unwrap().operation,
            initial.operation
        );
        retained[1].phase = ReceiptPhase::Acknowledged;
        retained[1].observed_result = Some(json!({"comment_id":Uuid::new_v4()}));
        assert_eq!(
            make(&retained, &config).unwrap().operation,
            initial.operation
        );
        retained[1].phase = ReceiptPhase::Reconciled;
        retained[1].observed_result =
            Some(json!({"effect":"not_sent","error":Error::new("timeout", "fixture")}));
        retained[1].updated_at = Utc::now();
        assert_eq!(make(&retained, &config).err().unwrap().code, "retry_wait");
    }
}
