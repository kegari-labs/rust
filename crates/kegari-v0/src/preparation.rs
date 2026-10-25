//! Bounded host publication of human-authorized parent scope. This is separate
//! from agent execution and cannot grant authorization from an agent's output.
use crate::{
    app::ConfigurationControl,
    configuration::{ConfigurationInputs, NativeConfigurationInputs},
    connections::ConnectionRegistry,
    control::{Actor, ControlHost, Lease, MutationRequest, Receipt, Reconciliation},
    digest::{bytes_digest, canonical_digest},
    error::{Error, Result},
    evidence::{AttestationKey, Publication, encode_signed_publication},
    observations::ObservationCache,
    providers::{LinearClient, ProviderBroker, ProviderMutation},
    reconcile::{AuthorizationGate, VerifiedAuthorization},
    secrets::SecretStore,
};
use chrono::{Duration, Utc};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};
use uuid::Uuid;

pub struct AuthorizationPublisher {
    configuration: Arc<dyn ConfigurationControl>,
    control: Arc<ControlHost>,
    observations: Arc<ObservationCache>,
    secrets: Arc<dyn SecretStore>,
    key: Arc<AttestationKey>,
    inputs: Arc<dyn ConfigurationInputs>,
}
impl AuthorizationPublisher {
    pub fn new(
        configuration: Arc<dyn ConfigurationControl>,
        control: Arc<ControlHost>,
        observations: Arc<ObservationCache>,
        secrets: Arc<dyn SecretStore>,
        key: Arc<AttestationKey>,
    ) -> Self {
        Self::with_inputs(
            configuration,
            control,
            observations,
            secrets,
            key,
            Arc::new(NativeConfigurationInputs),
        )
    }
    pub fn with_inputs(
        configuration: Arc<dyn ConfigurationControl>,
        control: Arc<ControlHost>,
        observations: Arc<ObservationCache>,
        secrets: Arc<dyn SecretStore>,
        key: Arc<AttestationKey>,
        inputs: Arc<dyn ConfigurationInputs>,
    ) -> Self {
        Self {
            configuration,
            control,
            observations,
            secrets,
            key,
            inputs,
        }
    }
    fn current(&self, gate: &AuthorizationGate) -> Result<()> {
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
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == gate.workspace_id())
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
        let operation = publication_id(
            runtime,
            &canonical_digest(1, gate.snapshot())?,
            "publication",
        )?;
        let admission =
            publication_id(runtime, &canonical_digest(1, gate.snapshot())?, "admission")?;
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
        self.current(&gate)?;
        let permit =
            self.control
                .admit(lease, admission, gate.work_key()?, gate.digest()?, || {
                    self.current(&gate)
                })?;
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
            expected: BTreeMap::from([
                ("preparation".into(), "parent_authorization".into()),
                ("workspace".into(), gate.workspace_id().to_string()),
                ("parent".into(), gate.snapshot().parent.issue.to_string()),
                ("snapshot".into(), canonical_digest(1, gate.snapshot())?),
                ("body_digest".into(), bytes_digest(body.as_bytes())),
                ("writer".into(), writer.clone()),
            ]),
            payload: json!({}),
        };
        let this = self.clone();
        let checked_gate = gate.clone();
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
            move || this.current(&checked_gate),
        )
        .await;
        // The owned broker future has returned; remote uncertainty does not keep
        // this local preparation task falsely alive or clear its effect hold.
        self.control.retire_local(admission,json!({"phase":"authorization_preparation","result":match &result{Ok(_)=>"provider_acknowledged",Err(_)=>"blocked"}}))?;
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
