//! Host-owned provider observation cache and identity-only recovery references.
//! Refresh performs provider reads; it cannot create workers or mutate providers.
use crate::{
    app::ConfigurationControl,
    authority::IssueKey,
    configuration::{ConfigurationInputs, NativeConfigurationInputs},
    connections::ConnectionRegistry,
    control::{ControlHost, Receipt, ReceiptPhase},
    digest::canonical_digest,
    evidence::{AttestationKey, Publication, decode_signed_publication},
    reconcile::{
        AuthorizationSnapshot, Blocker, DefinitionCorrection, ParentProjection, ParentRefs,
        ReconcileRefs, Reconciler, RuntimeProjection, VerifiedAuthorization,
        VerifiedDefinitionCorrection,
    },
    secrets::SecretStore,
    storage::PrivateDir,
    workflow::Workflow,
};
use chrono::{DateTime, Utc};
use kegari_config::Config;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, RwLock},
};
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrackedParent {
    runtime_id: Uuid,
    workspace_id: Uuid,
    parent: IssueKey,
    required_children: BTreeSet<Uuid>,
    disappeared_children: BTreeMap<Uuid, DateTime<Utc>>,
    #[serde(default)]
    correction_invalidations: crate::authority::CorrectionInvalidations,
}
/// No descriptions, prompts, result assertions or acceptance state are persisted.
pub struct ReferenceStore {
    root: PrivateDir,
    runtime: Uuid,
}
impl ReferenceStore {
    pub fn new(root: PrivateDir, runtime: Uuid) -> Result<Self> {
        if runtime.is_nil() {
            return Err(Error::invalid("reference store requires runtime identity"));
        }
        Ok(Self { root, runtime })
    }
    pub fn restore(&self, config: &Config) -> Result<ReconcileRefs> {
        if config.runtime.id != self.runtime {
            return Err(Error::new(
                "identity_mismatch",
                "reference store belongs to another runtime",
            ));
        }
        let mut refs = ReconcileRefs::new();
        for file in self.root.records()? {
            let record: TrackedParent = self.root.read(&file)?;
            if record.runtime_id != self.runtime
                || record.workspace_id.is_nil()
                || record.parent.issue.is_nil()
                || record.required_children.iter().any(Uuid::is_nil)
                || file != tracking_leaf(record.workspace_id, record.parent.issue)
            {
                return Err(Error::new(
                    "record_corrupt",
                    "retained parent reference identity is invalid",
                ));
            }
            let Some(workspace) = config
                .workspaces
                .values()
                .find(|w| w.id == record.workspace_id)
            else {
                continue;
            };
            let endpoint = config.connections[&workspace.linear_connection]
                .endpoint
                .as_deref()
                .ok_or_else(|| Error::invalid("Linear endpoint missing"))?;
            if record.parent
                != IssueKey::new(endpoint, workspace.linear_workspace_id, record.parent.issue)?
            {
                return Err(Error::new(
                    "identity_mismatch",
                    "retained work differs from configured provider mapping",
                ));
            }
            refs.entry(record.workspace_id).or_default().insert(
                record.parent.issue,
                ParentRefs {
                    required_child_ids: record.required_children,
                    disappeared_children: record.disappeared_children,
                    corrections: crate::authority::Corrections::from_invalidations(
                        record.correction_invalidations,
                    )?,
                    ..Default::default()
                },
            );
        }
        Ok(refs)
    }
    pub fn retain(&self, projection: &RuntimeProjection) -> Result<()> {
        if projection.runtime_id != self.runtime {
            return Err(Error::new(
                "identity_mismatch",
                "observations belong to another runtime",
            ));
        }
        for workspace in projection.workspaces.values().filter(|w| w.complete) {
            for parent in workspace.parents.values() {
                self.root.write(
                    &tracking_leaf(workspace.workspace_id, parent.key.issue),
                    &TrackedParent {
                        runtime_id: self.runtime,
                        workspace_id: workspace.workspace_id,
                        parent: parent.key.clone(),
                        required_children: parent.required_children.clone(),
                        disappeared_children: parent.disappeared_children.clone(),
                        correction_invalidations: parent.corrections.invalidations(),
                    },
                )?;
            }
        }
        Ok(())
    }
}
fn tracking_leaf(workspace: Uuid, parent: Uuid) -> String {
    format!("{workspace}-{parent}.json")
}

struct CachedObservation {
    configuration: Config,
    workflows: BTreeMap<Uuid, Workflow>,
    projection: RuntimeProjection,
    health: Value,
}

pub struct ObservationCache {
    control: Arc<ControlHost>,
    configuration: Arc<dyn ConfigurationControl>,
    secrets: Arc<dyn SecretStore>,
    key: Arc<AttestationKey>,
    inputs: Arc<dyn ConfigurationInputs>,
    references: ReferenceStore,
    retained: Mutex<ReconcileRefs>,
    current: RwLock<CachedObservation>,
    refresh: AsyncMutex<()>,
    corrective_publications: Mutex<BTreeMap<Uuid, CorrectivePublication>>,
    active: Mutex<BTreeMap<String, std::sync::Weak<ActiveSnapshotInner>>>,
}
impl ObservationCache {
    pub async fn new(
        configuration: Arc<dyn ConfigurationControl>,
        secrets: Arc<dyn SecretStore>,
        key: Arc<AttestationKey>,
        root: PrivateDir,
        control: Arc<ControlHost>,
    ) -> Result<Self> {
        Self::with_inputs(
            configuration,
            secrets,
            key,
            root,
            Arc::new(NativeConfigurationInputs),
            control,
        )
        .await
    }
    pub async fn with_inputs(
        configuration: Arc<dyn ConfigurationControl>,
        secrets: Arc<dyn SecretStore>,
        key: Arc<AttestationKey>,
        root: PrivateDir,
        inputs: Arc<dyn ConfigurationInputs>,
        control: Arc<ControlHost>,
    ) -> Result<Self> {
        let config = configuration.snapshot()?;
        if control.snapshot()?.intent.runtime_id != config.runtime.id {
            return Err(Error::new(
                "identity_mismatch",
                "observation receipts belong to another runtime",
            ));
        }
        let references = ReferenceStore::new(root, config.runtime.id)?;
        let retained = references.restore(&config)?;
        // Missing clients create explicit failed workspace observations, including
        // retained child IDs. Startup never emits a fabricated empty success.
        let current = Reconciler::default()
            .refresh(&config, &BTreeMap::new(), &BTreeMap::new(), &retained)
            .await?;
        Ok(Self {
            control,
            configuration,
            secrets,
            key,
            inputs,
            references,
            retained: Mutex::new(retained),
            current: RwLock::new(CachedObservation {
                configuration: config,
                workflows: BTreeMap::new(),
                projection: current,
                health: json!({"status":"observation_pending","observed_at":null}),
            }),
            refresh: AsyncMutex::new(()),
            corrective_publications: Mutex::new(BTreeMap::new()),
            active: Mutex::new(BTreeMap::new()),
        })
    }
    pub fn snapshot(&self) -> Result<RuntimeProjection> {
        self.current
            .read()
            .map(|v| v.projection.clone())
            .map_err(|_| {
                Error::new(
                    "observation_unavailable",
                    "provider observation cache lock failed",
                )
            })
    }
    pub fn health(&self) -> Result<Value> {
        self.current
            .read()
            .map(|v| v.health.clone())
            .map_err(|_| Error::new("observation_unavailable", "provider health lock failed"))
    }
    pub fn references(&self) -> Result<ReconcileRefs> {
        self.retained
            .lock()
            .map(|v| v.clone())
            .map_err(|_| Error::new("observation_unavailable", "reference cache lock failed"))
    }
    pub async fn refresh(&self) -> Result<RuntimeProjection> {
        let _guard = self.refresh.lock().await;
        let result = self.refresh_inner().await;
        if let Err(error) = &result {
            self.record_refresh_failure(error)?;
        }
        result
    }

    /// Bound a read-only observation attempt, including serialization wait.
    /// A timeout invalidates current and retained gates before returning; it
    /// cannot turn the previous successful observation into a new one.
    pub async fn refresh_bounded(&self, bound: std::time::Duration) -> Result<RuntimeProjection> {
        if bound.is_zero() || bound > std::time::Duration::from_secs(300) {
            return Err(Error::invalid(
                "observation refresh bound must be positive and at most 300 seconds",
            ));
        }
        match tokio::time::timeout(bound, self.refresh()).await {
            Ok(result) => result,
            Err(_) => {
                let error = Error::new(
                    "timeout",
                    "complete provider observation exceeded its finite bound",
                );
                self.record_refresh_failure(&error)?;
                Err(error)
            }
        }
    }

    pub(crate) fn record_refresh_failure(&self, error: &Error) -> Result<()> {
        self.control.serialize_observation(|_| {
            self.current
                .write()
                .map_err(|_| Error::new("observation_unavailable", "health cache lock failed"))?
                .health =
                json!({"status":"observation_failed","observed_at":Utc::now(),"error":error});
            let active: Vec<_> = self
                .active
                .lock()
                .map_err(|_| {
                    Error::new(
                        "observation_unavailable",
                        "active snapshot registry unavailable",
                    )
                })?
                .values()
                .filter_map(std::sync::Weak::upgrade)
                .collect();
            for snapshot in active {
                snapshot
                    .current
                    .write()
                    .map_err(|_| {
                        Error::new("observation_unavailable", "active snapshot lock failed")
                    })?
                    .projection = Err(Error::new(
                    "observation_unavailable",
                    "latest complete provider observation failed",
                )
                .detail(json!({"cause":error.code})));
            }
            Ok(())
        })
    }
    pub fn admission_snapshot(&self) -> Result<RuntimeProjection> {
        let current = self
            .current
            .read()
            .map_err(|_| Error::new("observation_unavailable", "observation cache lock failed"))?;
        if current.health["status"] == "observation_failed"
            || current.health["status"] == "observation_pending"
        {
            return Err(Error::new(
                "observation_unavailable",
                "latest reconciliation did not complete",
            ));
        }
        Ok(current.projection.clone())
    }
    async fn refresh_inner(&self) -> Result<RuntimeProjection> {
        let config = self.configuration.snapshot()?;
        let mut retained = self.references()?;
        // Reconstruct newly configured mappings from retained identity records;
        // never silently reinterpret an old workspace under a different account.
        for (workspace, parents) in self.references.restore(&config)? {
            for (id, reference) in parents {
                retained
                    .entry(workspace)
                    .or_default()
                    .entry(id)
                    .or_insert(reference);
            }
        }
        let (workflows, workflow_errors) = load_workflows(&config);
        let transports = config
            .connections
            .keys()
            .filter_map(|name| {
                self.inputs
                    .transport(name)
                    .map(|transport| (name.clone(), transport))
            })
            .collect();
        let clients = ConnectionRegistry::bind_with_transports(
            &config,
            self.secrets.as_ref(),
            |name| self.inputs.environment(name),
            &transports,
        )?;
        let mut projection = Reconciler::default()
            .refresh(&config, clients.clients(&config)?, &workflows, &retained)
            .await?;
        let authority_revision = publication_revision(&projection)?;
        let previous_host_ids = host_comment_revision(&retained)?;
        // Capture one receipt epoch against actual serialized replacements.
        // Reconstruction below and active reads consume these owned bytes; no
        // provider I/O runs while the Control gate is held.
        let receipts = reconstruction_receipts(&self.control)?;
        restore_host_comment_ids(&projection, &mut retained, &receipts)?;
        let (changed, mut publication_errors) =
            restore_publications(&config, &projection, &mut retained, self.key.as_ref())?;
        let (explicit_changed, explicit_errors) =
            restore_explicit_corrections(&config, &projection, &mut retained, self.key.as_ref())?;
        publication_errors.extend(explicit_errors);
        let (classified_changed, classified_errors) = crate::corrections::restore_classifications(
            &config,
            &projection,
            &mut retained,
            &self.key,
            &receipts,
        )?;
        publication_errors.extend(classified_errors);
        if changed
            || explicit_changed
            || classified_changed
            || previous_host_ids != host_comment_revision(&retained)?
        {
            projection = Reconciler::default()
                .refresh(&config, clients.clients(&config)?, &workflows, &retained)
                .await?;
            if publication_revision(&projection)? != authority_revision {
                return Err(Error::new(
                    "unstable_observation",
                    "authority publications changed during reconstruction",
                ));
            }
        }
        let fences = self.configuration.authority_fences()?;
        for workspace in projection.workspaces.values_mut() {
            if let Err(error) = fences.check_workspace(workspace.workspace_id) {
                workspace.complete = false;
                workspace.error = Some(error);
            }
            if let Some(error) = workflow_errors
                .get(&workspace.workspace_id)
                .or_else(|| clients.blocked().get(&workspace.workspace_id))
            {
                workspace.complete = false;
                workspace.error = Some(error.clone());
            }
            for parent in workspace.parents.values_mut() {
                if let Some(error) = publication_errors.get(&parent.key) {
                    parent.blockers.push(Blocker {
                        code: error.code.clone(),
                        reason: error.message.clone(),
                        affected: parent.key.clone(),
                        impacted_gates: vec!["all".into()],
                        last_verified_evidence: None,
                        attempted_recovery: "Observed canonical signed provider records".into(),
                        next_action: "Reconcile changed or conflicting published authority".into(),
                        prerequisite: None,
                    });
                }
            }
        }
        let mut active = self
            .refresh_active(&config, &workflows, &retained, &receipts)
            .await?;
        self.control.serialize_observation(|_| {
        if crate::config::config_revision(&self.configuration.snapshot()?)? != crate::config::config_revision(&config)? {
            return Err(Error::new(
                "stale_config",
                "configuration changed during complete provider observation",
            ));
        }
        self.apply_corrective_publications(&config,&mut projection)?;
        for (_,observed) in &mut active {if let Ok(observed)=observed {self.apply_corrective_publications(&config,observed)?;}}
        self.references.retain(&projection)?;
        projection.retain_references(&mut retained);
        *self
            .retained
            .lock()
            .map_err(|_| Error::new("observation_unavailable", "reference cache lock failed"))? =
            retained;
        let complete = projection.workspaces.values().all(|w| w.complete);
        *self
            .current
            .write()
            .map_err(|_| Error::new("observation_unavailable", "projection cache lock failed"))? =
            CachedObservation {
                configuration: config.clone(),
                workflows: workflows.clone(),
                projection: projection.clone(),
                health: json!({"status":if complete{"observed"}else{"provider_blocked"},"observed_at":projection.observed_at,"config_revision":projection.config_revision,"runtime_build":crate::BUILD_ID}),
            };
            for (snapshot, result) in active {
                *snapshot.current.write().map_err(|_| Error::new("observation_unavailable", "active snapshot lock failed"))? = ActiveObservation { configuration_revision: crate::config::config_revision(&config)?, projection: result };
            }
            Ok(())
        })?;
        Ok(projection)
    }
}
fn publication_revision(projection: &RuntimeProjection) -> Result<String> {
    let mut records = Vec::new();
    for workspace in projection.workspaces.values() {
        for parent in workspace.parents.values() {
            for observation in parent.observations.values() {
                for comment in &observation.comments {
                    records.push((
                        &parent.key,
                        comment.issue_id,
                        comment.id,
                        &comment.body_digest,
                        comment.user_id,
                        comment.bot,
                        comment.delegated,
                    ));
                }
            }
        }
    }
    records.sort_by(|a, b| a.0.cmp(b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    canonical_digest(1, &records)
}
fn host_comment_revision(refs: &ReconcileRefs) -> Result<String> {
    let records: Vec<_> = refs
        .iter()
        .flat_map(|(workspace, parents)| {
            parents
                .iter()
                .map(move |(parent, reference)| (workspace, parent, &reference.host_comment_ids))
        })
        .collect();
    canonical_digest(1, &records)
}
/// Only call outside an existing Control gate. The returned records are data,
/// not authority, and remain immutable while read-only reconstruction runs.
fn reconstruction_receipts(control: &ControlHost) -> Result<Vec<Receipt>> {
    control.serialize_observation(|_| control.operations())
}

fn restore_host_comment_ids(
    projection: &RuntimeProjection,
    refs: &mut ReconcileRefs,
    receipts: &[Receipt],
) -> Result<()> {
    for workspace in projection.workspaces.values().filter(|w| w.complete) {
        for parent in workspace.parents.values() {
            let prior = refs
                .entry(workspace.workspace_id)
                .or_default()
                .entry(parent.key.issue)
                .or_default();
            // Rebuild this exception from current native records. Edited text must
            // re-enter classification; ownership of an old ID does not freeze it.
            prior.host_comment_ids.clear();
            for receipt in receipts {
                if receipt.runtime_id != projection.runtime_id
                    || receipt.kind != "linear_comment"
                    || !matches!(
                        receipt.phase,
                        ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                    )
                    || receipt
                        .expected
                        .get("explicit_correction")
                        .is_some_and(|v| v == "true")
                {
                    continue;
                }
                let (Some(id), Some(body), Some(writer)) = (
                    receipt.remote_ids.get("comment_id"),
                    receipt.expected.get("provider_body_digest"),
                    receipt.expected.get("provider_writer"),
                ) else {
                    continue;
                };
                let id = Uuid::parse_str(id).map_err(|_| {
                    Error::new(
                        "record_corrupt",
                        "host comment receipt has invalid provider ID",
                    )
                })?;
                for observation in parent.observations.values() {
                    for comment in &observation.comments {
                        if comment.id == id
                            && &comment.body_digest == body
                            && !comment.bot
                            && !comment.delegated
                            && comment.user_id.is_some_and(|id| id.to_string() == *writer)
                            && receipt.canonical_targets.contains(&format!(
                                "linear:{}:{}:{}",
                                parent.key.endpoint, parent.key.workspace, comment.issue_id
                            ))
                        {
                            prior.host_comment_ids.insert(id);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
fn load_workflows(config: &Config) -> (BTreeMap<Uuid, Workflow>, BTreeMap<Uuid, Error>) {
    let mut loaded = BTreeMap::new();
    let mut errors = BTreeMap::new();
    for (name, workspace) in &config.workspaces {
        let mut scoped = config.clone();
        scoped.workspaces.retain(|key, _| key == name);
        match crate::config::load_workflows(&scoped) {
            Ok(workflows) => loaded.extend(workflows),
            Err(error) => {
                errors.insert(workspace.id, error);
            }
        }
    }
    (loaded, errors)
}

fn restore_publications(
    config: &Config,
    projection: &RuntimeProjection,
    refs: &mut ReconcileRefs,
    key: &AttestationKey,
) -> Result<(bool, BTreeMap<IssueKey, Error>)> {
    let mut changed = false;
    let mut errors = BTreeMap::new();
    for workspace in projection.workspaces.values().filter(|w| w.complete) {
        let configured = config
            .workspaces
            .values()
            .find(|w| w.id == workspace.workspace_id)
            .ok_or_else(|| {
                Error::new(
                    "identity_mismatch",
                    "observed workspace is no longer configured",
                )
            })?;
        let writer = &config.connections[&configured.linear_connection].account_id;
        for parent in workspace.parents.values() {
            let Some(observation) = parent.observations.get(&parent.key.issue) else {
                continue;
            };
            let prior = refs
                .entry(workspace.workspace_id)
                .or_default()
                .entry(parent.key.issue)
                .or_default();
            let mut authorizations = Vec::new();
            let mut corrections = Vec::new();
            let mut owned_ids = BTreeSet::new();
            let result = (|| -> Result<()> {
                for comment in &observation.comments {
                    if comment.user_id.map(|id| id.to_string()).as_ref() != Some(writer)
                        || comment.bot
                        || comment.delegated
                        || comment.issue_id != parent.key.issue
                    {
                        continue;
                    }
                    let purpose = if comment
                        .body
                        .starts_with("<!-- kegari:v0:parent_authorization -->")
                    {
                        "parent_authorization"
                    } else if comment
                        .body
                        .starts_with("<!-- kegari:v0:in_scope_definition_correction -->")
                    {
                        "in_scope_definition_correction"
                    } else {
                        continue;
                    };
                    if purpose == "parent_authorization" {
                        let signed = decode_signed_publication::<AuthorizationSnapshot>(
                            &comment.body,
                            purpose,
                            key,
                        )?;
                        if signed.payload.parent != parent.key {
                            return Err(Error::new(
                                "scope_violation",
                                "authorization publication names a different parent",
                            ));
                        }
                        let publication = Publication {
                            provider: "linear".into(),
                            record_id: comment.id.to_string(),
                            writer_account_id: writer.clone(),
                            payload_digest: canonical_digest(1, &signed)?,
                            observed_at: projection.observed_at,
                            url: comment.url.clone(),
                        };
                        authorizations.push((
                            signed.payload.event.occurred_at,
                            VerifiedAuthorization::verify(key, &signed, &publication, writer)?,
                        ));
                    } else {
                        let signed = decode_signed_publication::<DefinitionCorrection>(
                            &comment.body,
                            purpose,
                            key,
                        )?;
                        if signed.payload.parent != parent.key {
                            return Err(Error::new(
                                "scope_violation",
                                "definition correction publication names a different parent",
                            ));
                        }
                        let publication = Publication {
                            provider: "linear".into(),
                            record_id: comment.id.to_string(),
                            writer_account_id: writer.clone(),
                            payload_digest: canonical_digest(1, &signed)?,
                            observed_at: projection.observed_at,
                            url: comment.url.clone(),
                        };
                        corrections.push((
                            signed.payload.correction_watermark,
                            VerifiedDefinitionCorrection::verify(
                                key,
                                &signed,
                                &publication,
                                writer,
                            )?,
                        ));
                    }
                    owned_ids.insert(comment.id);
                }
                authorizations.sort_by_key(|(time, _)| *time);
                corrections.sort_by_key(|(watermark, _)| *watermark);
                if authorizations.windows(2).any(|w| w[0].0 == w[1].0)
                    || corrections.windows(2).any(|w| w[0].0 == w[1].0)
                {
                    return Err(Error::new(
                        "ownership_conflict",
                        "multiple published authority records claim the same generation",
                    ));
                }
                Ok(())
            })();
            if let Err(error) = result {
                prior.authorization = None;
                prior.definition_correction = None;
                changed = true;
                errors.insert(parent.key.clone(), error);
                continue;
            }
            let next = authorizations.pop().map(|(_, proof)| proof);
            changed |= prior.authorization.as_ref().map(|a| a.digest())
                != next.as_ref().map(|a| a.digest());
            // Correction state is always reconstructed from a verified record;
            // the second pass rebinds it to current authorizing scope/comments.
            changed |= prior.definition_correction.is_some() || !corrections.is_empty();
            prior.authorization = next;
            prior.definition_correction = corrections.pop().map(|(_, proof)| proof);
            prior.host_comment_ids.extend(owned_ids);
        }
    }
    Ok((changed, errors))
}

fn restore_explicit_corrections(
    config: &Config,
    projection: &RuntimeProjection,
    refs: &mut ReconcileRefs,
    key: &AttestationKey,
) -> Result<(bool, BTreeMap<IssueKey, Error>)> {
    let mut changed = false;
    let mut errors = BTreeMap::new();
    for workspace in projection.workspaces.values().filter(|w| w.complete) {
        let configured = config
            .workspaces
            .values()
            .find(|w| w.id == workspace.workspace_id)
            .ok_or_else(|| Error::new("identity_mismatch", "correction workspace was removed"))?;
        let writer = &config.connections[&configured.linear_connection].account_id;
        for parent in workspace.parents.values() {
            let prior = refs
                .entry(workspace.workspace_id)
                .or_default()
                .entry(parent.key.issue)
                .or_default();
            let before: Vec<_> = prior
                .explicit_corrections
                .iter()
                .map(|(id, p)| (*id, p.body_digest().to_owned()))
                .collect();
            let mut current = BTreeMap::new();
            let mut commands = BTreeSet::new();
            if let Some(observed) = parent.observations.get(&parent.key.issue) {
                for comment in &observed.comments {
                    if !comment
                        .body
                        .contains("\n\n<!-- kegari:v0:explicit_correction -->\n")
                        || comment.user_id.map(|id| id.to_string()).as_ref() != Some(writer)
                    {
                        continue;
                    }
                    match crate::decisions::VerifiedExplicitCorrection::verify_comment(
                        comment,
                        &parent.key,
                        config.runtime.id,
                        workspace.workspace_id,
                        &configured.actors.correctors,
                        writer,
                        key,
                    ) {
                        Ok(proof) => {
                            if !commands.insert(proof.record().command_id) {
                                errors.insert(
                                    parent.key.clone(),
                                    Error::new(
                                        "ownership_conflict",
                                        "multiple explicit correction records claim one command",
                                    ),
                                );
                            }
                            current.insert(comment.id, proof);
                        }
                        Err(error) => {
                            errors.insert(parent.key.clone(), error);
                        }
                    }
                }
            }
            let after: Vec<_> = current
                .iter()
                .map(|(id, p)| (*id, p.body_digest().to_owned()))
                .collect();
            changed |= before != after;
            prior.explicit_corrections = current;
        }
    }
    Ok((changed, errors))
}

/// Retain this handle only while its admitted operation is alive. The registry
/// keeps weak references; finished snapshots disappear without persisted prompts.
#[derive(Clone)]
pub struct ActiveSnapshot {
    inner: Arc<ActiveSnapshotInner>,
}
impl ActiveSnapshot {
    pub fn identity(&self) -> &str {
        &self.inner.identity
    }
}
struct ActiveSnapshotInner {
    identity: String,
    workspace: Uuid,
    configuration: Config,
    workflow: Workflow,
    current: RwLock<ActiveObservation>,
}
struct ActiveObservation {
    configuration_revision: String,
    projection: Result<RuntimeProjection>,
}
fn active_key(workspace: Uuid, config: &str, workflow: &str) -> Result<String> {
    canonical_digest(
        1,
        &json!({"workspace":workspace,"configuration":config,"workflow":workflow}),
    )
}
impl ObservationCache {
    pub fn capture_active(
        &self,
        workspace: Uuid,
        configuration_revision: &str,
        workflow_digest: &str,
    ) -> Result<ActiveSnapshot> {
        self.admission_snapshot()?;
        if crate::config::config_revision(&self.configuration.snapshot()?)?
            != configuration_revision
        {
            return Err(Error::new(
                "stale_config",
                "admission configuration changed before snapshot retention",
            ));
        }
        self.configuration
            .authority_fences()?
            .check_workspace(workspace)?;
        let snapshot = self
            .current
            .read()
            .map_err(|_| Error::new("observation_unavailable", "snapshot lock failed"))?;
        if crate::config::config_revision(&snapshot.configuration)? != configuration_revision
            || snapshot.projection.config_revision != configuration_revision
            || !snapshot
                .projection
                .workspaces
                .get(&workspace)
                .is_some_and(|w| w.complete)
        {
            return Err(Error::new(
                "stale_config",
                "only a current complete admitted snapshot can be retained",
            ));
        }
        let workflow = snapshot
            .workflows
            .get(&workspace)
            .filter(|w| w.digest == workflow_digest)
            .ok_or_else(|| Error::new("stale_result", "admitted workflow snapshot is unavailable"))?
            .clone();
        let identity = active_key(workspace, configuration_revision, workflow_digest)?;
        let mut registry = self.active.lock().map_err(|_| {
            Error::new(
                "observation_unavailable",
                "active snapshot registry unavailable",
            )
        })?;
        registry.retain(|_, value| value.strong_count() > 0);
        if let Some(inner) = registry.get(&identity).and_then(std::sync::Weak::upgrade) {
            return Ok(ActiveSnapshot { inner });
        }
        let inner = Arc::new(ActiveSnapshotInner {
            identity: identity.clone(),
            workspace,
            configuration: snapshot.configuration.clone(),
            workflow,
            current: RwLock::new(ActiveObservation {
                configuration_revision: configuration_revision.into(),
                projection: Ok(snapshot.projection.clone()),
            }),
        });
        registry.insert(identity, Arc::downgrade(&inner));
        Ok(ActiveSnapshot { inner })
    }
    fn active_projection(
        &self,
        workspace: Uuid,
        configuration_revision: &str,
        workflow_digest: &str,
    ) -> Result<RuntimeProjection> {
        self.configuration
            .authority_fences()?
            .check_workspace(workspace)?;
        self.admission_snapshot()?;
        let key = active_key(workspace, configuration_revision, workflow_digest)?;
        let registry = self.active.lock().map_err(|_| {
            Error::new(
                "observation_unavailable",
                "active snapshot registry unavailable",
            )
        })?;
        let snapshot = registry
            .get(&key)
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                Error::new(
                    "ownership_conflict",
                    "operation has no retained admitted snapshot",
                )
            })?;
        let current = snapshot.current.read().map_err(|_| {
            Error::new(
                "observation_unavailable",
                "active authority state unavailable",
            )
        })?;
        if crate::config::config_revision(&self.configuration.snapshot()?)?
            != current.configuration_revision
        {
            return Err(Error::new(
                "stale_config",
                "active authority has not observed current permissions",
            ));
        }
        current.projection.clone()
    }
    pub fn active_gate(
        &self,
        expected: &crate::reconcile::FreshGate,
        outcomes: &crate::reconcile::DependencyOutcomes,
    ) -> Result<crate::reconcile::FreshGate> {
        self.active_parent(expected, outcomes)?.retain_active_gate(
            expected,
            outcomes,
            Utc::now(),
            chrono::Duration::seconds(30),
        )
    }
    /// Read current native correction facts through an existing admitted
    /// snapshot. This returns no execution or transition authority.
    pub(crate) fn active_correction_context(
        &self,
        admitted: &crate::reconcile::FreshGate,
    ) -> Result<(Config, ParentProjection)> {
        let projection = self.active_projection(
            admitted.workspace_id(),
            admitted.config_revision(),
            admitted.workflow_digest(),
        )?;
        let identity = active_key(
            admitted.workspace_id(),
            admitted.config_revision(),
            admitted.workflow_digest(),
        )?;
        let snapshot = self
            .active
            .lock()
            .map_err(|_| {
                Error::new(
                    "observation_unavailable",
                    "active snapshot registry unavailable",
                )
            })?
            .get(&identity)
            .and_then(std::sync::Weak::upgrade)
            .ok_or_else(|| {
                Error::new(
                    "ownership_conflict",
                    "correction has no retained admitted snapshot",
                )
            })?;
        let current = self.configuration.snapshot()?;
        let adjusted =
            active_configuration(&current, &snapshot.configuration, admitted.workspace_id())?;
        if crate::config::config_revision(&adjusted)? != projection.config_revision
            || !projection
                .workspaces
                .get(&admitted.workspace_id())
                .is_some_and(|workspace| workspace.complete)
        {
            return Err(Error::new(
                "stale_config",
                "correction observation does not cover the current active permissions",
            ));
        }
        let parent = projection
            .parent(admitted.workspace_id(), admitted.work().parent)?
            .clone();
        Ok((adjusted, parent))
    }
    pub(crate) fn active_corrected_gate(
        &self,
        admitted: &crate::reconcile::FreshGate,
        routes: &crate::corrections::VerifiedCorrectionRoutes,
        outcomes: &crate::reconcile::DependencyOutcomes,
    ) -> Result<crate::reconcile::FreshGate> {
        let (_, parent) = self.active_correction_context(admitted)?;
        parent.retain_corrected_gate(
            admitted,
            routes,
            outcomes,
            Utc::now(),
            chrono::Duration::seconds(30),
        )
    }
    /// The parent and its validation come from one retained observation. Callers
    /// cannot relabel a new projection with the old configuration/workflow to
    /// continue admitted work after an ordinary settings reload.
    pub fn active_parent(
        &self,
        expected: &crate::reconcile::FreshGate,
        outcomes: &crate::reconcile::DependencyOutcomes,
    ) -> Result<crate::reconcile::ParentProjection> {
        let projection = self.active_projection(
            expected.workspace_id(),
            expected.config_revision(),
            expected.workflow_digest(),
        )?;
        if !projection
            .workspaces
            .get(&expected.workspace_id())
            .is_some_and(|w| w.complete)
        {
            return Err(Error::new(
                "observation_unavailable",
                "active workspace provider observation is incomplete",
            ));
        }
        let parent = projection.parent(expected.workspace_id(), expected.work().parent)?;
        parent.retain_active_gate(
            expected,
            outcomes,
            Utc::now(),
            chrono::Duration::seconds(30),
        )?;
        Ok(parent.clone())
    }
    pub fn active_independent_gate(
        &self,
        expected: &crate::reconcile::IndependentReviewGate,
    ) -> Result<crate::reconcile::IndependentReviewGate> {
        let projection = self.active_projection(
            expected.workspace_id(),
            expected.config_revision(),
            expected.workflow_digest(),
        )?;
        let current = projection.independent_gate(
            expected.workspace_id(),
            expected.repository_id(),
            expected.pull_request().number,
            Utc::now(),
            chrono::Duration::seconds(30),
        )?;
        expected.retain_active(current, Utc::now(), chrono::Duration::seconds(30))
    }
    async fn refresh_active(
        &self,
        current: &Config,
        workflows: &BTreeMap<Uuid, Workflow>,
        retained: &ReconcileRefs,
        receipts: &[Receipt],
    ) -> Result<Vec<(Arc<ActiveSnapshotInner>, Result<RuntimeProjection>)>> {
        let active: Vec<_> = self
            .active
            .lock()
            .map_err(|_| {
                Error::new(
                    "observation_unavailable",
                    "active snapshot registry unavailable",
                )
            })?
            .values()
            .filter_map(std::sync::Weak::upgrade)
            .collect();
        let mut refreshed = Vec::new();
        for snapshot in active {
            let result = async {
                self.configuration
                    .authority_fences()?
                    .check_workspace(snapshot.workspace)?;
                let adjusted =
                    active_configuration(current, &snapshot.configuration, snapshot.workspace)?;
                let transports = adjusted
                    .connections
                    .keys()
                    .filter_map(|name| self.inputs.transport(name).map(|t| (name.clone(), t)))
                    .collect();
                let clients = ConnectionRegistry::bind_with_transports(
                    &adjusted,
                    self.secrets.as_ref(),
                    |name| self.inputs.environment(name),
                    &transports,
                )?;
                let mut pinned = workflows.clone();
                pinned.insert(snapshot.workspace, snapshot.workflow.clone());
                let mut refs = retained.clone();
                let mut observed = Reconciler::default()
                    .refresh(&adjusted, clients.clients(&adjusted)?, &pinned, &refs)
                    .await?;
                let before = publication_revision(&observed)?;
                restore_host_comment_ids(&observed, &mut refs, receipts)?;
                let (_, mut errors) =
                    restore_publications(&adjusted, &observed, &mut refs, &self.key)?;
                let (_, explicit) =
                    restore_explicit_corrections(&adjusted, &observed, &mut refs, &self.key)?;
                errors.extend(explicit);
                let (_, classified) = crate::corrections::restore_classifications(
                    &adjusted, &observed, &mut refs, &self.key, receipts,
                )?;
                errors.extend(classified);
                observed = Reconciler::default()
                    .refresh(&adjusted, clients.clients(&adjusted)?, &pinned, &refs)
                    .await?;
                if before != publication_revision(&observed)? {
                    return Err(Error::new(
                        "unstable_observation",
                        "active authority changed during reconstruction",
                    ));
                }
                for workspace in observed.workspaces.values_mut() {
                    for parent in workspace.parents.values_mut() {
                        if let Some(error) = errors.get(&parent.key) {
                            parent.blockers.push(Blocker {
                                code: error.code.clone(),
                                reason: error.message.clone(),
                                affected: parent.key.clone(),
                                impacted_gates: vec!["active_authority".into()],
                                last_verified_evidence: None,
                                attempted_recovery: "Fresh active authority reconciliation".into(),
                                next_action: "Reconcile published authority".into(),
                                prerequisite: None,
                            });
                        }
                    }
                }
                Ok(observed)
            }
            .await;
            refreshed.push((snapshot, result));
        }
        Ok(refreshed)
    }
}
/// Preserve execution choices only after current permissions and path identities
/// are checked. The resulting config is used solely for registered active work.
fn active_configuration(current: &Config, admitted: &Config, workspace: Uuid) -> Result<Config> {
    let (old_name, old) = admitted
        .workspaces
        .iter()
        .find(|(_, w)| w.id == workspace)
        .ok_or_else(|| Error::new("scope_violation", "admitted workspace absent"))?;
    let (name, new) = current
        .workspaces
        .iter()
        .find(|(_, w)| w.id == workspace)
        .ok_or_else(|| Error::new("access_denied", "admitted workspace was removed"))?;
    if name != old_name
        || current.runtime.id != admitted.runtime.id
        || current.runtime.host_id != admitted.runtime.host_id
        || current.runtime.execution_root != admitted.runtime.execution_root
        || old.linear_workspace_id != new.linear_workspace_id
        || old.linear_connection != new.linear_connection
        || old.github_connection != new.github_connection
        || old.codex_connection != new.codex_connection
    {
        return Err(Error::new(
            "identity_mismatch",
            "active runtime/workspace/account binding changed",
        ));
    }
    for connection in [
        &old.linear_connection,
        &old.github_connection,
        &old.codex_connection,
    ] {
        let a = &admitted.connections[connection];
        let b = current
            .connections
            .get(connection)
            .ok_or_else(|| Error::new("access_denied", "active account removed"))?;
        if a.kind != b.kind
            || a.account_id != b.account_id
            || a.endpoint != b.endpoint
            || a.auth_home != b.auth_home
            || a.argv != b.argv
        {
            return Err(Error::new(
                "identity_mismatch",
                "active connection authority changed",
            ));
        }
    }
    for (id, repository) in &old.repositories {
        let next = new
            .repositories
            .get(id)
            .ok_or_else(|| Error::new("access_denied", "active repository was removed"))?;
        if repository.source != next.source
            || repository.hostname != next.hostname
            || repository.main != next.main
            || repository.staging != next.staging
            || canonical_digest(1, &repository.deployment)?
                != canonical_digest(1, &next.deployment)?
        {
            return Err(Error::new(
                "scope_violation",
                "active repository path/ref/deployment authority changed",
            ));
        }
    }
    for (id, resource) in &old.test_resources {
        let next = new
            .test_resources
            .get(id)
            .ok_or_else(|| Error::new("access_denied", "active test resource was removed"))?;
        if resource.path != next.path
            || resource.writable && !next.writable
            || resource.disposable && !next.disposable
        {
            return Err(Error::new(
                "access_denied",
                "active test resource permission was reduced",
            ));
        }
    }
    let mut adjusted = current.clone();
    let mut pinned = old.clone();
    pinned.actors = new.actors.clone();
    pinned.linear = new.linear.clone();
    pinned.github = new.github.clone();
    pinned.github.pr_authors = old.github.pr_authors.clone();
    adjusted.workspaces.insert(name.clone(), pinned);
    adjusted.validate()?;
    Ok(adjusted)
}

#[derive(Clone)]
struct CorrectivePublication {
    workspace: Uuid,
    parent: IssueKey,
    operation: Uuid,
    body_digest: String,
    writer: String,
}
impl ObservationCache {
    /// Called inside the serialized final provider gate, before dispatch. It is
    /// synchronous, idempotent and never reenters ControlHost. Every old work
    /// snapshot is fenced until the actual corrective input is reconstructed.
    pub fn fence_explicit_correction(
        &self,
        workspace: Uuid,
        parent: &IssueKey,
        operation: Uuid,
        body_digest: &str,
        writer: &str,
    ) -> Result<()> {
        crate::evidence::validate_digest(body_digest)?;
        if operation.is_nil() || workspace.is_nil() || writer.is_empty() {
            return Err(Error::invalid(
                "corrective publication needs exact provenance",
            ));
        }
        let entry = CorrectivePublication {
            workspace,
            parent: parent.clone(),
            operation,
            body_digest: body_digest.into(),
            writer: writer.into(),
        };
        let mut publications = self.corrective_publications.lock().map_err(|_| {
            Error::new(
                "observation_unavailable",
                "corrective publication fence unavailable",
            )
        })?;
        if publications.get(&operation).is_some_and(|old| {
            old.workspace != workspace
                || old.parent != *parent
                || old.body_digest != body_digest
                || old.writer != writer
        }) {
            return Err(Error::new(
                "request_conflict",
                "corrective publication fence differs",
            ));
        }
        publications.insert(operation, entry.clone());
        add_corrective_blocker(
            &mut self
                .current
                .write()
                .map_err(|_| Error::new("observation_unavailable", "projection unavailable"))?
                .projection,
            &entry,
        );
        for snapshot in self
            .active
            .lock()
            .map_err(|_| Error::new("observation_unavailable", "active snapshots unavailable"))?
            .values()
            .filter_map(std::sync::Weak::upgrade)
        {
            if let Ok(projection) = &mut snapshot
                .current
                .write()
                .map_err(|_| Error::new("observation_unavailable", "active authority unavailable"))?
                .projection
            {
                add_corrective_blocker(projection, &entry);
            }
        }
        Ok(())
    }
    fn apply_corrective_publications(
        &self,
        config: &Config,
        projection: &mut RuntimeProjection,
    ) -> Result<()> {
        let mut publications = self.corrective_publications.lock().map_err(|_| {
            Error::new(
                "observation_unavailable",
                "corrective publication fence unavailable",
            )
        })?;
        let receipts: BTreeMap<_, _> = self
            .control
            .operations()?
            .into_iter()
            .map(|r| (r.request_id, r))
            .collect();
        publications.retain(|id, _| {
            !receipts.get(id).is_some_and(|r| {
                r.phase == ReceiptPhase::Reconciled
                    && r.observed_result
                        .as_ref()
                        .is_some_and(|v| v["effect"] == "not_sent")
            })
        });
        for entry in publications.values() {
            let observed = projection
                .workspaces
                .get(&entry.workspace)
                .filter(|w| w.complete)
                .and_then(|w| w.parents.get(&entry.parent.issue));
            let current = observed
                .and_then(|p| p.observations.get(&entry.parent.issue))
                .and_then(|o| {
                    let matches: Vec<_> = o
                        .comments
                        .iter()
                        .filter(|c| {
                            c.body_digest == entry.body_digest
                                && c.issue_id == entry.parent.issue
                                && !c.bot
                                && !c.delegated
                                && c.user_id.map(|id| id.to_string()).as_deref()
                                    == Some(&entry.writer)
                        })
                        .collect();
                    (matches.len() == 1).then(|| matches[0])
                });
            let reconstructed = current.is_some_and(|comment| {
                config
                    .workspaces
                    .values()
                    .find(|w| w.id == entry.workspace)
                    .is_some_and(|workspace| {
                        crate::decisions::VerifiedExplicitCorrection::verify_comment(
                            comment,
                            &entry.parent,
                            config.runtime.id,
                            entry.workspace,
                            &workspace.actors.correctors,
                            &entry.writer,
                            &self.key,
                        )
                        .is_ok_and(|proof| proof.record().operation_id == entry.operation)
                    })
            });
            if !reconstructed {
                add_corrective_blocker(projection, entry);
            }
        }
        Ok(())
    }
}
fn add_corrective_blocker(projection: &mut RuntimeProjection, entry: &CorrectivePublication) {
    if let Some(parent) = projection
        .workspaces
        .get_mut(&entry.workspace)
        .and_then(|w| w.parents.get_mut(&entry.parent.issue))
        && !parent
            .blockers
            .iter()
            .any(|b| b.code == "corrective_publication_pending")
    {
        parent.blockers.push(Blocker {
            code: "corrective_publication_pending".into(),
            reason: "An authenticated corrective publication is pending exact authoritative intake"
                .into(),
            affected: entry.parent.clone(),
            impacted_gates: vec!["all".into()],
            last_verified_evidence: None,
            attempted_recovery: "Await complete current native comment observation".into(),
            next_action: "Reconcile the exact corrective publication before continuing work".into(),
            prerequisite: None,
        });
    }
}

#[cfg(test)]
#[path = "observations_receipt_tests.rs"]
mod receipt_tests;
