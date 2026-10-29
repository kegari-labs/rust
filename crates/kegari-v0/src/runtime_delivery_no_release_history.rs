//! Immutable no-release completion with explicit owned or external target provenance.
//! A retained seal grants no current baseline, permit, acceptance or mutation.
use super::*;
use crate::{
    authority::IssueKey,
    control::{ControlHost, MetadataReference, ReceiptPhase},
    delivery::{AcceptedOriginIdentity, AcceptedOriginRef, RepositoryContract},
    deployment::{
        ExternalTargetObservationRef, OwnedTargetObservationRef, ProductionObserver,
        VerifiedProductionBaseline,
    },
    evidence::{VerifiedNoReleaseRequired, WorkIdentity},
    runtime_delivery::completion_binding::DependencyBinding,
};
use chrono::{DateTime, Duration, Utc};
use kegari_git::{RepositoryId, Revision};
use kegari_secrets::AttestationKey;
use kegari_storage::PrivateDir;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};

const PURPOSE: &str = "no_release_completion";
const MAX_REFERENCES: usize = 4096;

#[derive(Default)]
pub(in crate::runtime_delivery::release) struct NoReleaseHistory {
    diagnostics: Mutex<BTreeMap<Uuid, Error>>,
    // A missing file cannot downgrade a seal this service actually observed
    // into an unsealed intent or authorize a second initial mint.
    anchors: Mutex<BTreeMap<Uuid, VerifiedNoReleaseCompletion>>,
}

pub(super) struct NoReleaseSealInput<'a> {
    pub workspace: &'a WorkspaceDelivery,
    pub gate: &'a FreshGate,
    pub parent: &'a VerifiedParent,
    pub acceptance: &'a VerifiedAcceptance,
    pub carry: Option<&'a crate::delivery::PublishedCarry>,
    pub intent: &'a NoReleaseIntent,
    pub proof: &'a VerifiedNoReleaseRequired,
    pub observed: &'a [VerifiedProductionBaseline],
}

pub(crate) struct NoReleaseCompletionInventory {
    pub proofs: Vec<VerifiedNoReleaseCompletion>,
    /// Authenticated immutable identities, including proofs with unavailable
    /// references. Consumers must not derive credit without full validation.
    pub anchors: Vec<VerifiedNoReleaseCompletion>,
    pub errors: BTreeMap<Uuid, Error>,
    pub rows: Vec<crate::inspection::NoReleaseInspectionRow>,
    /// Actual authenticated seal identities, including previously observed
    /// anchors whose required file/reference has since disappeared.
    pub sealed_operations: BTreeSet<Uuid>,
}

fn invalid(message: &str) -> Error {
    Error::new("no_release_completion", message)
}
fn completion_leaf(operation: Uuid) -> String {
    format!("no-release-completion-{operation}.json")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptRef {
    operation: Uuid,
    digest: String,
}
impl ReceiptRef {
    fn read(&self, control: &ControlHost, runtime: Uuid) -> Result<Receipt> {
        let receipt = control.operation(self.operation)?;
        if self.operation.is_nil()
            || receipt.operation_id != self.operation
            || receipt.request_id != self.operation
            || receipt.runtime_id != runtime
            || canonical_digest(1, &receipt)? != self.digest
        {
            return Err(invalid(
                "retained no-release receipt changed identity or bytes",
            ));
        }
        Ok(receipt)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataAttemptRef {
    ordinal: u32,
    admission: ReceiptRef,
    // Explicit absence protects a failed-before-transmit prior attempt.
    effect: Option<ReceiptRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationIdentity {
    comment: Uuid,
    writer: Uuid,
    body_digest: String,
    url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservedSource {
    repository: RepositoryId,
    revision: Revision,
    contract_digest: String,
    observed_at: DateTime<Utc>,
}

/// Constructed only from actual opaque provider baseline observations.
/// This is preparation data, not a completed outcome or current dispatch grant.
pub(super) struct VerifiedNoReleaseObservation {
    owned_targets: Vec<OwnedTargetObservationRef>,
    external_targets: Vec<ExternalTargetObservationRef>,
    sources: Vec<ObservedSource>,
    observed_at: DateTime<Utc>,
}

pub(super) fn prepare_deployment_free_observation(
    proof: &VerifiedNoReleaseRequired,
    parent: &VerifiedParent,
    contracts: &[RepositoryContract],
    observed: &[VerifiedProductionBaseline],
    now: DateTime<Utc>,
) -> Result<VerifiedNoReleaseObservation> {
    let record = proof.record();
    let manifest = parent.manifest();
    if record.work != manifest.work
        || record.evidence_token != parent.token()
        || record.baselines != manifest.baselines
        || manifest.baselines.is_empty()
        || manifest.candidates.iter().any(|candidate| {
            !candidate.ordered_delta_digests.is_empty()
                || candidate.result.as_ref() != Some(&candidate.production)
        })
    {
        return Err(invalid(
            "history preparation differs from exact unchanged parent evidence",
        ));
    }
    if !record.targets.is_empty()
        || !manifest.production_targets.is_empty()
        || manifest
            .baselines
            .iter()
            .any(|baseline| !baseline.deployment_free)
    {
        return Err(Error::new(
            "historical_baseline_proof_unavailable",
            "deployed-target history requires its actual immutable observation provenance",
        ));
    }
    let repositories: BTreeSet<_> = manifest
        .baselines
        .iter()
        .map(|baseline| baseline.repository.clone())
        .collect();
    if repositories.len() != manifest.baselines.len()
        || repositories.len() != manifest.candidates.len()
        || repositories.len() != observed.len()
        || manifest
            .candidates
            .iter()
            .map(|c| c.repository.clone())
            .collect::<BTreeSet<_>>()
            != repositories
    {
        return Err(invalid(
            "deployment-free history has incomplete repository inventory",
        ));
    }
    let mut sources = BTreeMap::new();
    for current in observed {
        let baseline = current.baseline();
        let matches: Vec<_> = contracts
            .iter()
            .filter(|contract| contract.repository == baseline.repository)
            .collect();
        let [contract] = matches.as_slice() else {
            return Err(invalid(
                "baseline lacks one exact configured repository contract",
            ));
        };
        if !matches!(contract.configuration.deployment_policy()?,
            kegari_config::Deployment::None(value) if value == "none")
            || !baseline.deployment_free
            || !current.targets().is_empty()
            || !baseline.target_artifacts.is_empty()
            || !baseline.target_revisions.is_empty()
            || baseline.deployed_revision.is_some()
            || baseline.deployed_artifact.is_some()
        {
            return Err(Error::new(
                "historical_baseline_proof_unavailable",
                "deployed-target history requires its actual immutable observation provenance",
            ));
        }
        if !manifest.baselines.contains(baseline)
            || current.contract_digest() != contract.digest()?
            || current.observed_at() > now + Duration::seconds(5)
            || now - current.observed_at() > Duration::seconds(30)
            || sources.contains_key(&baseline.repository)
        {
            return Err(invalid(
                "actual deployment-free production observation is stale or changed",
            ));
        }
        sources.insert(
            baseline.repository.clone(),
            ObservedSource {
                repository: baseline.repository.clone(),
                revision: baseline.production.clone(),
                contract_digest: current.contract_digest().into(),
                observed_at: current.observed_at(),
            },
        );
    }
    if sources.keys().cloned().collect::<BTreeSet<_>>() != repositories {
        return Err(invalid(
            "actual production observation omitted a selected repository",
        ));
    }
    Ok(VerifiedNoReleaseObservation {
        owned_targets: vec![],
        external_targets: vec![],
        sources: sources.into_values().collect(),
        observed_at: now,
    })
}

fn prepare_no_release_observation(
    proof: &VerifiedNoReleaseRequired,
    parent: &VerifiedParent,
    contracts: &[RepositoryContract],
    observed: &[VerifiedProductionBaseline],
    observer: &ProductionObserver,
    now: DateTime<Utc>,
) -> Result<VerifiedNoReleaseObservation> {
    if proof.record().targets.is_empty()
        && proof
            .record()
            .baselines
            .iter()
            .all(|baseline| baseline.deployment_free)
    {
        return prepare_deployment_free_observation(proof, parent, contracts, observed, now);
    }
    let record = proof.record();
    let manifest = parent.manifest();
    let repositories: BTreeSet<_> = manifest
        .baselines
        .iter()
        .map(|baseline| baseline.repository.clone())
        .collect();
    if record.work != manifest.work
        || record.evidence_token != parent.token()
        || record.baselines != manifest.baselines
        || repositories.is_empty()
        || repositories.len() != manifest.baselines.len()
        || repositories.len() != manifest.candidates.len()
        || repositories.len() != observed.len()
        || manifest
            .candidates
            .iter()
            .map(|candidate| candidate.repository.clone())
            .collect::<BTreeSet<_>>()
            != repositories
        || manifest.candidates.iter().any(|candidate| {
            !candidate.ordered_delta_digests.is_empty()
                || candidate.result.as_ref() != Some(&candidate.production)
        })
        || record.targets.len() != manifest.production_targets.len()
        || record
            .targets
            .iter()
            .map(|target| target.target.clone())
            .collect::<BTreeSet<_>>()
            != manifest.production_targets
    {
        return Err(invalid(
            "owned target history differs from exact unchanged complete parent",
        ));
    }
    let mut sources = BTreeMap::new();
    let mut targets = Vec::new();
    let mut owned_targets = Vec::new();
    let mut external_targets = Vec::new();
    for current in observed {
        let baseline = current.baseline();
        let matching: Vec<_> = contracts
            .iter()
            .filter(|contract| contract.repository == baseline.repository)
            .collect();
        let [contract] = matching.as_slice() else {
            return Err(invalid(
                "owned baseline lacks one exact configured contract",
            ));
        };
        baseline.validate()?;
        if !manifest.baselines.contains(baseline)
            || baseline.successor_recovery_operation.is_some()
            || current.contract_digest() != contract.digest()?
            || current.observed_at() > now + Duration::seconds(5)
            || now - current.observed_at() > Duration::seconds(30)
            || sources.contains_key(&baseline.repository)
        {
            return Err(invalid(
                "owned baseline current observation is stale, duplicated or changed",
            ));
        }
        // This reader consumes private caps retained by the actual observer;
        // external provider JSON without a supported proof cannot reach here.
        owned_targets.extend(observer.owned_baseline_references(contract, current)?);
        external_targets.extend(observer.external_baseline_references(contract, current)?);
        targets.extend_from_slice(current.targets());
        sources.insert(
            baseline.repository.clone(),
            ObservedSource {
                repository: baseline.repository.clone(),
                revision: baseline.production.clone(),
                contract_digest: current.contract_digest().into(),
                observed_at: current.observed_at(),
            },
        );
    }
    if sources.keys().cloned().collect::<BTreeSet<_>>() != repositories
        || !same_targets(&record.targets, &targets)
        || owned_targets.len() + external_targets.len() != targets.len()
    {
        return Err(invalid(
            "owned target proof inventory differs from current complete no-release facts",
        ));
    }
    owned_targets.sort_by(|a, b| a.target().target.cmp(&b.target().target));
    external_targets.sort_by(|a, b| a.target().target.cmp(&b.target().target));
    Ok(VerifiedNoReleaseObservation {
        sources: sources.into_values().collect(),
        owned_targets,
        external_targets,
        observed_at: now,
    })
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NoReleaseCompletionRecord {
    schema_version: u32,
    runtime: Uuid,
    workspace: Uuid,
    work: WorkIdentity,
    operation: Uuid,
    publication_operation: Uuid,
    // This is the original signed NoReleaseRequired digest, not this seal's hash.
    outcome_digest: String,
    evidence_token: String,
    acceptance_digest: String,
    intent_digest: String,
    config_revision: String,
    workflow_digest: String,
    metadata: Vec<MetadataAttemptRef>,
    publication: PublicationIdentity,
    origin: AcceptedOriginRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    carried: Option<crate::delivery::CarriedNoReleaseOrigin>,
    binding: DependencyBinding,
    sources: Vec<ObservedSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    owned_targets: Vec<OwnedTargetObservationRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    external_targets: Vec<ExternalTargetObservationRef>,
    outcome: Signed<NoReleaseRequired>,
    display: ReleaseDisplay,
    recorded_at: DateTime<Utc>,
    sealed_at: DateTime<Utc>,
}

impl NoReleaseCompletionRecord {
    fn valid_schema(&self) -> bool {
        matches!(
            (self.schema_version, self.external_targets.is_empty()),
            (1, true) | (2, false)
        )
    }
    fn target_origins(
        &self,
    ) -> impl Iterator<Item = (&evidence::ProductionTargetBaseline, &str, Uuid)> {
        self.owned_targets
            .iter()
            .map(|target| {
                (
                    target.target(),
                    target.contract_digest(),
                    target.runtime_id(),
                )
            })
            .chain(self.external_targets.iter().map(|target| {
                (
                    target.target(),
                    target.contract_digest(),
                    target.runtime_id(),
                )
            }))
    }
}

struct HistoryServices {
    runtime: Uuid,
    records: Arc<PrivateDir>,
    control: Arc<ControlHost>,
    key: Arc<AttestationKey>,
}

/// No Deserialize or public constructor. Recovery reads the actual signed seal
/// from its canonical file and rechecks every immutable reference.
#[derive(Clone)]
pub(crate) struct VerifiedNoReleaseCompletion {
    signed: Signed<NoReleaseCompletionRecord>,
    digest: String,
    services: Arc<HistoryServices>,
}
impl std::fmt::Debug for VerifiedNoReleaseCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedNoReleaseCompletion")
            .field("operation", &self.signed.payload.operation)
            .field("digest", &self.digest)
            .finish()
    }
}
impl NoReleaseHistory {
    fn anchor(&self, operation: Uuid) -> Result<Option<VerifiedNoReleaseCompletion>> {
        Ok(self
            .anchors
            .lock()
            .map_err(|_| invalid("no-release anchor lock failed"))?
            .get(&operation)
            .cloned())
    }
    fn anchors(&self) -> Result<BTreeMap<Uuid, VerifiedNoReleaseCompletion>> {
        Ok(self
            .anchors
            .lock()
            .map_err(|_| invalid("no-release anchor lock failed"))?
            .clone())
    }
    // Caller has authenticated the actual seal and its identity, or just
    // committed it from current opaque evidence. No raw caller data enters.
    fn remember(&self, proof: &VerifiedNoReleaseCompletion) -> Result<()> {
        let mut anchors = self
            .anchors
            .lock()
            .map_err(|_| invalid("no-release anchor lock failed"))?;
        if let Some(previous) = anchors.get(&proof.operation()) {
            if previous.digest() != proof.digest() {
                return Err(invalid(
                    "previously observed immutable no-release seal changed",
                ));
            }
            return Ok(());
        }
        if anchors.len() >= MAX_REFERENCES {
            return Err(invalid("no-release seal anchors exceed finite bound"));
        }
        anchors.insert(proof.operation(), proof.clone());
        Ok(())
    }
}

impl VerifiedNoReleaseCompletion {
    pub(crate) fn runtime_id(&self) -> Uuid {
        self.signed.payload.runtime
    }
    pub(crate) fn work(&self) -> &WorkIdentity {
        &self.signed.payload.work
    }
    pub(crate) fn workspace_id(&self) -> Uuid {
        self.signed.payload.workspace
    }
    pub(crate) fn operation(&self) -> Uuid {
        self.signed.payload.operation
    }
    pub(crate) fn publication_operation(&self) -> Uuid {
        self.signed.payload.publication_operation
    }
    pub(crate) fn outcome_digest(&self) -> &str {
        &self.signed.payload.outcome_digest
    }
    pub(crate) fn record(&self) -> &NoReleaseRequired {
        &self.signed.payload.outcome.payload
    }
    pub(crate) fn dependency_repositories(&self) -> Result<&BTreeSet<RepositoryId>> {
        Ok(&self.signed.payload.binding.repositories)
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn successful_history(&self) -> Result<crate::release::SuccessfulRelease> {
        self.validate()?;
        let record = &self.signed.payload;
        Ok(crate::release::SuccessfulRelease {
            operation_id: record.operation,
            parent_id: record.work.parent,
            generation: record.work.generation.clone(),
            outcome_digest: record.outcome_digest.clone(),
            resources: record
                .binding
                .repositories
                .iter()
                .cloned()
                .map(|repository| crate::release::ResourceKey::Repository { repository })
                .chain(record.outcome.payload.targets.iter().map(|target| {
                    crate::release::ResourceKey::Target {
                        provider: target.provider.clone(),
                        id: target.target.clone(),
                    }
                }))
                .collect(),
            completed_at: record.recorded_at,
            spacing_seconds: 0,
            no_release_required: true,
        })
    }
    pub(crate) fn validate_for_configuration(
        &self,
        config: &Config,
    ) -> Result<crate::release::SuccessfulRelease> {
        let successful = self.successful_history()?;
        let record = &self.signed.payload;
        if config.runtime.id != record.runtime
            || DependencyBinding::configured(
                config,
                record.workspace,
                record.binding.repositories.clone(),
            )? != record.binding
        {
            return Err(invalid(
                "completed no-release provider/workspace configuration changed",
            ));
        }
        Ok(successful)
    }
    fn read_authenticated(services: Arc<HistoryServices>, operation: Uuid) -> Result<Self> {
        let signed: Signed<NoReleaseCompletionRecord> =
            services.records.read(&completion_leaf(operation))?;
        services.key.verify(PURPOSE, &signed)?;
        validate_inspection_identity(&signed.payload, services.runtime, &services.key)?;
        if signed.payload.operation != operation {
            return Err(invalid(
                "completion filename differs from its signed operation",
            ));
        }
        let digest = signed.digest()?;
        Ok(Self {
            signed,
            digest,
            services,
        })
    }

    /// The outer seal authenticates the scope used for queue isolation. Missing
    /// known seals are handled by strict validation; malformed or unauthenticated
    /// present envelopes are inventory failures even for disjoint candidates.
    pub(crate) fn validate_queue_envelope(&self) -> Result<()> {
        let current: Signed<NoReleaseCompletionRecord> = match self
            .services
            .records
            .read(&completion_leaf(self.operation()))
        {
            Ok(signed) => signed,
            Err(error) if error.code == "record_missing" => return Ok(()),
            Err(error) => return Err(error),
        };
        self.services.key.verify(PURPOSE, &current)?;
        validate_inspection_identity(&current.payload, self.services.runtime, &self.services.key)?;
        if current.payload.operation != self.operation() {
            return Err(invalid("no-release seal filename identity changed"));
        }
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let record = &self.signed.payload;
        let current: Signed<NoReleaseCompletionRecord> = self
            .services
            .records
            .read(&completion_leaf(record.operation))?;
        self.services.key.verify(PURPOSE, &current)?;
        if current.digest()? != self.signed.digest()? {
            return Err(invalid("immutable no-release completion was replaced"));
        }
        self.validate_references()
    }

    fn validate_references(&self) -> Result<()> {
        let record = &self.signed.payload;
        self.services.key.verify(PURPOSE, &self.signed)?;
        record.work.validate()?;
        if !record.valid_schema()
            || record.runtime != self.services.runtime
            || record.runtime.is_nil()
            || record.workspace.is_nil()
            || record.operation.is_nil()
            || record.publication_operation.is_nil()
            || record.work.child.is_some()
            || record.publication.comment.is_nil()
            || record.publication.writer.is_nil()
            || record.metadata.is_empty()
            || record.metadata.len() > MAX_REFERENCES
            || record.sources.is_empty()
            || record.sources.len() > MAX_REFERENCES
            || record.recorded_at > record.sealed_at
            || record.sealed_at > Utc::now() + Duration::seconds(5)
        {
            return Err(invalid(
                "no-release completion has invalid bounded identity",
            ));
        }
        for digest in [
            &record.outcome_digest,
            &record.evidence_token,
            &record.acceptance_digest,
            &record.intent_digest,
            &record.config_revision,
            &record.workflow_digest,
            &record.publication.body_digest,
        ] {
            evidence::validate_digest(digest)?;
        }
        let signed_intent: Signed<NoReleaseIntent> =
            self.services.records.read(&intent_leaf(record.operation))?;
        self.services
            .key
            .verify("runtime_no_release_intent", &signed_intent)?;
        self.services
            .key
            .verify("no_release_required", &signed_intent.payload.record)?;
        let intent = &signed_intent.payload;
        let outcome = &intent.record.payload;
        self.services
            .key
            .verify("no_release_required", &record.outcome)?;
        if record.outcome.digest()? != record.outcome_digest || record.outcome.payload != *outcome {
            return Err(invalid(
                "completion outcome differs from original signed payload",
            ));
        }
        if signed_intent.digest()? != record.intent_digest
            || intent.schema_version != 1
            || outcome.operation_id != record.operation
            || intent.workspace != record.workspace
            || outcome.runtime_id != record.runtime
            || outcome.work != record.work
            || intent.effect()? != record.publication_operation
            || intent.record.digest()? != record.outcome_digest
            || outcome.evidence_token != record.evidence_token
            || outcome.acceptance_digest != record.acceptance_digest
            || outcome.recorded_at != record.recorded_at
            || intent.config_revision != record.config_revision
            || intent.workflow_digest != record.workflow_digest
            || outcome.baselines.len() != record.sources.len()
            || intent.contracts.len() != record.sources.len()
        {
            return Err(invalid(
                "completion lost its exact original no-release intent",
            ));
        }
        let mut repositories = BTreeSet::new();
        for source in &record.sources {
            source.revision.validate()?;
            evidence::validate_digest(&source.contract_digest)?;
            if !repositories.insert(source.repository.clone())
                || source.observed_at > record.sealed_at + Duration::seconds(5)
                || record.sealed_at - source.observed_at > Duration::seconds(30)
                || !outcome.baselines.iter().any(|baseline| {
                    baseline.repository == source.repository
                        && baseline.production == source.revision
                        && baseline.successor_recovery_operation.is_none()
                        && if baseline.deployment_free {
                            baseline.target_artifacts.is_empty()
                                && baseline.target_revisions.is_empty()
                                && baseline.deployed_revision.is_none()
                                && baseline.deployed_artifact.is_none()
                                && !record
                                    .target_origins()
                                    .any(|(target, _, _)| target.repository == source.repository)
                        } else {
                            let origins: Vec<_> = record
                                .target_origins()
                                .filter(|(target, _, _)| target.repository == source.repository)
                                .collect();
                            let ids: BTreeSet<_> = origins
                                .iter()
                                .map(|(target, _, _)| target.target.clone())
                                .collect();
                            !origins.is_empty()
                                && ids.len() == origins.len()
                                && ids == baseline.target_artifacts.keys().cloned().collect()
                                && ids == baseline.target_revisions.keys().cloned().collect()
                                && origins.iter().all(|(target, contract_digest, runtime)| {
                                    *contract_digest == source.contract_digest
                                        && *runtime == record.runtime
                                        && target.source == source.revision
                                        && baseline.artifact_for_target(&target.target)
                                            == Some(target.artifact_digest.as_str())
                                        && baseline.source_for_target(&target.target)
                                            == Some(target.source.commit.as_str())
                                })
                        }
                })
                || intent
                    .contracts
                    .iter()
                    .filter(|(repository, digest)| {
                        repository == &source.repository && digest == &source.contract_digest
                    })
                    .count()
                    != 1
            {
                return Err(invalid(
                    "retained no-release production inventory is incomplete",
                ));
            }
        }
        if record.owned_targets.len() + record.external_targets.len() > MAX_REFERENCES
            || !same_targets(
                &outcome.targets,
                &record
                    .target_origins()
                    .map(|(target, _, _)| target.clone())
                    .collect::<Vec<_>>(),
            )
        {
            return Err(invalid(
                "completion lost exact complete target observation inventory",
            ));
        }
        if !record.owned_targets.is_empty() || !record.external_targets.is_empty() {
            let deployment = PrivateDir::open(
                &self
                    .services
                    .records
                    .path()
                    .join(record.workspace.to_string())
                    .join("deployment"),
            )?;
            for target in &record.external_targets {
                target.validate_retained(&deployment, &self.services.key)?;
            }
            for (target, _, _) in record.target_origins() {
                if target.observed_at > record.sealed_at + Duration::seconds(5)
                    || record.sealed_at - target.observed_at > Duration::seconds(30)
                {
                    return Err(invalid(
                        "target initial observation was not current at sealing",
                    ));
                }
            }
            for target in &record.owned_targets {
                target.validate_retained(
                    &deployment,
                    &self.services.control,
                    &self.services.key,
                )?;
                if target.target().observed_at > record.sealed_at + Duration::seconds(5)
                    || record.sealed_at - target.target().observed_at > Duration::seconds(30)
                {
                    return Err(invalid(
                        "owned target initial observation was not current at sealing",
                    ));
                }
            }
        }
        record.binding.validate_work(&record.work, &repositories)?;
        if record.binding.linear.account != record.publication.writer.to_string() {
            return Err(invalid(
                "completion native writer differs from recorded account binding",
            ));
        }
        self.validate_metadata(intent)?;
        // The opaque helper validates exact native publication receipt and
        // workspace-vector refs. It opens existing namespaces; no provider read.
        match (&record.carried, &intent.carried) {
            (None, None) => record.origin.validate_retained(
                &self.services.records,
                &self.services.control,
                &self.services.key,
                AcceptedOriginIdentity {
                    runtime: record.runtime,
                    workspace: record.workspace,
                    work: &record.work,
                    token: &record.evidence_token,
                    acceptance_digest: &record.acceptance_digest,
                },
            )?,
            (Some(carried), Some(binding)) if binding.matches_origin(carried) => {
                carried.validate_retained(
                    &self.services.records,
                    &self.services.control,
                    &self.services.key,
                    outcome,
                    record.workspace,
                    &record.origin,
                )?;
            }
            _ => {
                return Err(invalid(
                    "no-release completion lost exact carried acceptance origin",
                ));
            }
        }
        Ok(())
    }

    fn validate_metadata(&self, intent: &NoReleaseIntent) -> Result<()> {
        let record = &self.signed.payload;
        if intent.attempt.purpose != "no_release_required"
            || record.metadata.len() != intent.attempt.attempt as usize
        {
            return Err(invalid("no-release metadata history is not complete"));
        }
        let work = IssueKey::new(
            &record.work.linear_endpoint,
            record.work.linear_workspace,
            record.work.parent,
        )?
        .identity()?;
        for (index, reference) in record.metadata.iter().enumerate() {
            let ordinal =
                u32::try_from(index + 1).map_err(|_| invalid("metadata ordinal exceeds bound"))?;
            let admission_id = crate::preparation::metadata_attempt_id(
                "no_release_required",
                record.runtime,
                &intent.attempt.generation,
                "admission",
                ordinal,
            )?;
            let effect_id = crate::preparation::metadata_attempt_id(
                "no_release_required",
                record.runtime,
                &intent.attempt.generation,
                "publication",
                ordinal,
            )?;
            let admission = reference
                .admission
                .read(&self.services.control, record.runtime)?;
            let metadata = MetadataReference::from_receipt(&admission)?
                .ok_or_else(|| invalid("completion owner is not a metadata admission"))?;
            if reference.ordinal != ordinal
                || admission.operation_id != admission_id
                || admission.canonical_targets != vec![work.clone()]
                || admission.phase != ReceiptPhase::Reconciled
                || metadata.purpose != intent.attempt.purpose
                || metadata.generation != intent.attempt.generation
                || metadata.attempt != ordinal
                || metadata.max_attempts < intent.attempt.max_attempts
                || admission.observed_result.as_ref().is_none_or(|result| {
                    result["local_execution"] != "exited"
                        || result["unresolved_effects"]
                            .as_array()
                            .is_none_or(|effects| !effects.is_empty())
                })
            {
                return Err(invalid(
                    "no-release admission ancestry changed or remains uncertain",
                ));
            }
            let effect = match &reference.effect {
                Some(effect) => {
                    if effect.operation != effect_id {
                        return Err(invalid(
                            "metadata publication has another deterministic identity",
                        ));
                    }
                    Some(effect.read(&self.services.control, record.runtime)?)
                }
                None => match self.services.control.operation(effect_id) {
                    Err(error) if error.code == "record_missing" => None,
                    Err(error) => return Err(error),
                    Ok(_) => return Err(invalid("previously absent metadata effect appeared")),
                },
            };
            if effect.as_ref().is_some_and(|effect| {
                effect.kind != "linear_comment"
                    || effect.actor != admission.actor
                    || effect.owner_generation != admission.owner_generation
                    || effect.canonical_targets != vec![provider_target(&intent.record.payload)]
                    || effect.expected.get("admission_operation") != Some(&admission_id.to_string())
                    || effect.expected.get("admitted_scope") != admission.expected.get("scope")
                    || effect.expected.get("snapshot") != Some(&intent.attempt.generation)
            }) {
                return Err(invalid(
                    "metadata effect belongs to another admitted publication",
                ));
            }
            if ordinal < intent.attempt.attempt {
                if effect.as_ref().is_some_and(|effect| {
                    effect.phase != ReceiptPhase::Reconciled
                        || effect
                            .observed_result
                            .as_ref()
                            .is_none_or(|v| v["effect"] != "not_sent")
                }) {
                    return Err(invalid(
                        "earlier metadata attempt did not prove no transmission",
                    ));
                }
                continue;
            }
            let effect = effect.ok_or_else(|| invalid("final publication receipt is missing"))?;
            let body = evidence::encode_signed_publication("no_release_required", &intent.record)?;
            if admission_id != record.operation
                || effect_id != record.publication_operation
                || admission.expected.get("scope") != Some(&intent.scope()?)
                || canonical_digest(1, &metadata)? != canonical_digest(1, &intent.attempt)?
                || effect.kind != "linear_comment"
                || !matches!(
                    effect.phase,
                    ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                )
                || effect.actor != admission.actor
                || effect.owner_generation != admission.owner_generation
                || effect.canonical_targets != vec![provider_target(&intent.record.payload)]
                || effect.expected.get("admission_operation") != Some(&record.operation.to_string())
                || effect.expected.get("admitted_scope") != admission.expected.get("scope")
                || effect.expected.get("no_release_digest") != Some(&record.outcome_digest)
                || effect.expected.get("provider_writer")
                    != Some(&record.publication.writer.to_string())
                || record.publication.writer != intent.parent_publication.writer
                || effect.expected.get("provider_body_digest")
                    != Some(&record.publication.body_digest)
                || record.publication.body_digest != bytes_digest(body.as_bytes())
                || effect.remote_ids.get("comment_id")
                    != Some(&record.publication.comment.to_string())
            {
                return Err(invalid(
                    "no-release publication lost exact actual ownership/readback",
                ));
            }
        }
        Ok(())
    }
}

fn metadata_references(
    control: &ControlHost,
    intent: &NoReleaseIntent,
) -> Result<Vec<MetadataAttemptRef>> {
    if intent.attempt.attempt == 0 || intent.attempt.attempt as usize > MAX_REFERENCES {
        return Err(invalid("metadata history exceeds its finite bound"));
    }
    let mut refs = Vec::new();
    for ordinal in 1..=intent.attempt.attempt {
        let id = |phase| {
            crate::preparation::metadata_attempt_id(
                "no_release_required",
                intent.record.payload.runtime_id,
                &intent.attempt.generation,
                phase,
                ordinal,
            )
        };
        let admission = control.operation(id("admission")?)?;
        let effect = match control.operation(id("publication")?) {
            Ok(value) => Some(ReceiptRef {
                operation: value.operation_id,
                digest: canonical_digest(1, &value)?,
            }),
            Err(error) if error.code == "record_missing" => None,
            Err(error) => return Err(error),
        };
        refs.push(MetadataAttemptRef {
            ordinal,
            admission: ReceiptRef {
                operation: admission.operation_id,
                digest: canonical_digest(1, &admission)?,
            },
            effect,
        });
    }
    Ok(refs)
}

impl RuntimeDelivery {
    fn seed_no_release_history_anchors(&self, runtime: Uuid) -> Result<()> {
        // Clone runtime anchors before taking the delivery history lock. These
        // were actual sealed capabilities, not caller records or inferred intent.
        let known = self.runtime.known_no_release_completions()?;
        if known.len() > MAX_REFERENCES {
            return Err(invalid("runtime no-release anchors exceed finite bound"));
        }
        for proof in known {
            if proof.runtime_id() != runtime {
                return Err(invalid("known no-release seal belongs to another runtime"));
            }
            self.releases.no_release_history.remember(&proof)?;
        }
        Ok(())
    }

    /// Caller owns Control serialization and holds no release-state guard.
    /// Remember the shared original before the facade copy, so a later failure
    /// cannot reopen initial minting through a new facade in this runtime.
    fn remember_no_release_history_anchor(
        &self,
        proof: &VerifiedNoReleaseCompletion,
    ) -> Result<()> {
        self.runtime
            .remember_no_release_completion_anchors(vec![proof.clone()])?;
        self.releases.no_release_history.remember(proof)
    }

    /// Failure-only publication while the caller owns Control and no release
    /// state guard. The original error is returned by the caller unchanged.
    fn invalidate_no_release_history_inventory(&self, error: &Error) -> Result<()> {
        self.runtime
            .invalidate_no_release_inventory(error.clone())?;
        let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
        state.historical_no_release.fatal_error = Some(error.clone());
        state.queue.invalidate_after_rebuild(error.clone());
        Ok(())
    }

    /// This boundary reads only the actual outer seal. Missing remains scoped;
    /// malformed, unauthenticated or invalid-identity present data is global.
    fn read_no_release_history_anchor(
        &self,
        services: Arc<HistoryServices>,
        operation: Uuid,
        expected_digest: Option<&str>,
    ) -> Result<VerifiedNoReleaseCompletion> {
        let proof = match VerifiedNoReleaseCompletion::read_authenticated(services, operation) {
            Ok(proof) => proof,
            Err(error) => {
                if error.code != "record_missing" {
                    self.invalidate_no_release_history_inventory(&error)?;
                }
                return Err(error);
            }
        };
        if expected_digest.is_some_and(|expected| proof.digest() != expected) {
            return Err(invalid("immutable no-release completion was replaced"));
        }
        self.remember_no_release_history_anchor(&proof)?;
        Ok(proof)
    }

    /// Do not infer a global outer-envelope failure from an inner reference's
    /// generic authentication error. Unsafe storage is global at either level.
    fn check_no_release_history_storage<T>(&self, result: Result<T>) -> Result<T> {
        if let Err(error) = &result
            && error.code == "unsafe_storage"
        {
            self.invalidate_no_release_history_inventory(error)?;
        }
        result
    }

    fn validate_no_release_history_anchor(
        &self,
        proof: &VerifiedNoReleaseCompletion,
    ) -> Result<()> {
        // Exactly one actual outer read is classified by origin. Do not follow
        // it with validate(), which would perform a second unclassified read.
        let observed = self.read_no_release_history_anchor(
            proof.services.clone(),
            proof.operation(),
            Some(proof.digest()),
        )?;
        if observed.digest() != proof.digest() {
            return Err(invalid("immutable no-release completion was replaced"));
        }
        self.check_no_release_history_storage(proof.validate_references())
    }

    /// Deny-only shared-scope gate. Caller holds Control and no ReleaseState
    /// guard. A newly sealed earlier parent in this same hydration pass may not
    /// yet have entered the facade queue, so check the actual retained anchors.
    pub(in crate::runtime_delivery::release) fn validate_shared_no_release_resources(
        &self,
        record: &NoReleaseRequired,
    ) -> Result<()> {
        self.runtime.registration.verify_control_owner()?;
        self.runtime.require_no_release_inventory_available()?;
        record.work.validate()?;
        let runtime = self.runtime.configuration.snapshot()?.runtime.id;
        if record.runtime_id != runtime || record.work.child.is_some() {
            return Err(invalid(
                "no-release resource check changed runtime or parent identity",
            ));
        }
        let parent = IssueKey::new(
            &record.work.linear_endpoint,
            record.work.linear_workspace,
            record.work.parent,
        )?;
        let mut resources: BTreeSet<_> = record
            .baselines
            .iter()
            .map(|baseline| crate::release::ResourceKey::Repository {
                repository: baseline.repository.clone(),
            })
            .collect();
        resources.extend(
            record
                .targets
                .iter()
                .map(|target| crate::release::ResourceKey::Target {
                    provider: target.provider.clone(),
                    id: target.target.clone(),
                }),
        );
        let anchors = self.runtime.known_no_release_completions()?;
        if anchors.len() > MAX_REFERENCES {
            return Err(invalid("runtime no-release anchors exceed finite bound"));
        }
        // Check every actual outer envelope before any resource/parent filter.
        // Missing known seals remain scoped; unknown present envelopes do not.
        for anchor in &anchors {
            if let Err(error) = anchor.validate_queue_envelope() {
                self.invalidate_no_release_history_inventory(&error)?;
                return Err(error);
            }
        }
        for anchor in anchors {
            let scope = crate::release::CompletionScope::no_release(&anchor)?;
            if let Err(error) = self.validate_no_release_history_anchor(&anchor) {
                // The origin-aware read may have seen a new global error since
                // the first pass. It already latched the exact shared failure.
                self.runtime.require_no_release_inventory_available()?;
                let original = IssueKey::new(
                    &scope.work().linear_endpoint,
                    scope.work().linear_workspace,
                    scope.work().parent,
                )?;
                if original == parent || !scope.resources().is_disjoint(&resources) {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn no_release_history_services(&self) -> Result<Arc<HistoryServices>> {
        Ok(Arc::new(HistoryServices {
            runtime: self.runtime.configuration.snapshot()?.runtime.id,
            records: self.records.clone(),
            control: self.runtime.control.clone(),
            key: self.runtime.services.key.clone(),
        }))
    }

    pub(super) async fn observe_no_release_history(
        &self,
        input: NoReleaseSealInput<'_>,
    ) -> Result<()> {
        let operation = input.intent.record.payload.operation_id;
        let result = self.seal_deployment_free_no_release(&input).await;
        let known_sealed = self
            .releases
            .no_release_history
            .anchor(operation)?
            .is_some();
        let mut diagnostics = self
            .releases
            .no_release_history
            .diagnostics
            .lock()
            .map_err(|_| invalid("no-release history diagnostic lock failed"))?;
        match result {
            Ok(_) => {
                diagnostics.remove(&operation);
            }
            Err(error) if error.code == "unsafe_storage" => return Err(error),
            Err(error) => {
                if !diagnostics.contains_key(&operation) && diagnostics.len() >= MAX_REFERENCES {
                    return Err(invalid(
                        "no-release history diagnostics exceed their finite bound",
                    ));
                }
                diagnostics.insert(operation, error.clone());
                if known_sealed
                    || !matches!(
                        error.code.as_str(),
                        "historical_baseline_proof_unavailable" | "no_release_completion_pending"
                    )
                {
                    return Err(error);
                }
            }
        }
        // Only never-sealed unsupported/pending history remains diagnostic.
        // A failed known seal cannot fall back to ephemeral current evidence.
        Ok(())
    }

    async fn seal_deployment_free_no_release(
        &self,
        input: &NoReleaseSealInput<'_>,
    ) -> Result<VerifiedNoReleaseCompletion> {
        let services = self.no_release_history_services()?;
        let operation = input.intent.record.payload.operation_id;
        let existing = self.runtime.control.serialize_observation(|_| {
            self.runtime.registration.verify_control_owner()?;
            self.seed_no_release_history_anchors(services.runtime)?;
            if let Some(proof) = self.releases.no_release_history.anchor(operation)? {
                self.remember_no_release_history_anchor(&proof)?;
                // Missing own seals remain fatal even when all proposed bytes
                // would be identical. A known identity is never reminted.
                self.validate_no_release_history_anchor(&proof)?;
                if proof.outcome_digest() != input.proof.digest()
                    || proof.work() != input.gate.work()
                {
                    return Err(invalid(
                        "known no-release seal belongs to another exact outcome",
                    ));
                }
                return Ok(Some(proof));
            }
            match self.read_no_release_history_anchor(services.clone(), operation, None) {
                Ok(proof) => {
                    self.check_no_release_history_storage(proof.validate_references())?;
                    if proof.outcome_digest() != input.proof.digest()
                        || proof.work() != input.gate.work()
                    {
                        return Err(invalid(
                            "immutable no-release operation names another verified outcome",
                        ));
                    }
                    Ok(Some(proof))
                }
                Err(error) if error.code == "record_missing" => Ok(None),
                Err(error) => Err(error),
            }
        })?;
        if let Some(proof) = existing {
            return Ok(proof);
        }
        let observation = prepare_no_release_observation(
            input.proof,
            input.parent,
            &input.workspace.contracts,
            input.observed,
            &input.workspace.observer,
            Utc::now(),
        )?;
        Self::check_no_release_intent(
            &self.runtime,
            input.intent,
            input.gate,
            input.parent,
            input.acceptance,
            input.carry,
        )?;
        let (origin, carried, origin_observed_at) = match input.carry {
            Some(carry) => {
                let (origin, carried) = carry.no_release_origin(
                    &input.workspace.configuration,
                    &input.intent.record.payload,
                )?;
                (origin, Some(carried), None)
            }
            None => {
                let origin = input
                    .workspace
                    .coordinator
                    .accepted_origin_references(
                        input.gate,
                        input.parent,
                        input.acceptance,
                        input.intent.parent_publication.id,
                        input.intent.acceptance_publication.id,
                    )
                    .await?;
                (origin.reference().clone(), None, Some(origin.observed_at()))
            }
        };
        let broker =
            self.provider_broker(&input.workspace.configuration, input.gate.workspace_id())?;
        let linear = broker.linear.as_ref().ok_or_else(|| {
            Error::new(
                "prerequisite",
                "no-release history Linear reader unavailable",
            )
        })?;
        let receipt = self.runtime.control.operation(input.intent.effect()?)?;
        let id: Uuid = receipt
            .remote_ids
            .get("comment_id")
            .ok_or_else(|| invalid("acknowledged no-release publication lacks its native ID"))?
            .parse()
            .map_err(|_| invalid("no-release publication ID is not canonical"))?;
        let comments = linear.comments(input.gate.work().parent).await?;
        let matching: Vec<_> = comments.iter().filter(|comment| comment.id == id).collect();
        let [comment] = matching.as_slice() else {
            return Err(invalid(
                "native no-release publication is missing or duplicated",
            ));
        };
        let publication = self.no_release_publication(input.intent, comment, &receipt)?;
        if publication.payload_digest != canonical_digest(1, &input.intent.record)? {
            return Err(invalid(
                "native no-release publication has another signed payload",
            ));
        }
        let current = self.runtime.observations.admission_snapshot()?;
        let parent = current.parent(input.gate.workspace_id(), input.gate.work().parent)?;
        let display = ReleaseDisplay::capture(&input.workspace.configuration, parent)?;
        let signed_intent: Signed<NoReleaseIntent> = self.records.read(&intent_leaf(operation))?;
        self.runtime
            .services
            .key
            .verify("runtime_no_release_intent", &signed_intent)?;
        if canonical_digest(1, &signed_intent.payload)? != canonical_digest(1, input.intent)? {
            return Err(invalid(
                "no-release intent changed during history observation",
            ));
        }
        let repositories = observation
            .sources
            .iter()
            .map(|source| source.repository.clone())
            .collect();
        let binding = DependencyBinding::configured(
            &input.workspace.configuration,
            input.gate.workspace_id(),
            repositories,
        )?;
        let prepared = NoReleaseCompletionRecord {
            schema_version: if observation.external_targets.is_empty() {
                1
            } else {
                2
            },
            runtime: input.intent.record.payload.runtime_id,
            workspace: input.gate.workspace_id(),
            work: input.gate.work().clone(),
            operation,
            publication_operation: input.intent.effect()?,
            outcome_digest: input.proof.digest().into(),
            evidence_token: input.parent.token().into(),
            acceptance_digest: input.acceptance.digest().into(),
            intent_digest: signed_intent.digest()?,
            config_revision: input.intent.config_revision.clone(),
            workflow_digest: input.intent.workflow_digest.clone(),
            metadata: metadata_references(&self.runtime.control, input.intent)?,
            publication: PublicationIdentity {
                comment: comment.id,
                writer: input.intent.parent_publication.writer,
                body_digest: comment.body_digest.clone(),
                url: (!comment.url.is_empty()).then(|| comment.url.clone()),
            },
            origin,
            carried,
            binding,
            sources: observation.sources,
            owned_targets: observation.owned_targets,
            external_targets: observation.external_targets,
            outcome: input.intent.record.clone(),
            display,
            recorded_at: input.intent.record.payload.recorded_at,
            sealed_at: Utc::now(),
        };
        let signed = self.runtime.services.key.sign(PURPOSE, &prepared)?;
        let digest = signed.digest()?;
        let proof = VerifiedNoReleaseCompletion {
            signed,
            digest,
            services,
        };
        self.runtime.control.serialize_observation(|snapshot| {
            self.runtime.registration.verify_control_owner()?;
            let now = Utc::now();
            if crate::config::config_revision(&self.runtime.configuration.snapshot()?)?
                != prepared.config_revision
                || now - observation.observed_at > Duration::seconds(30)
                || origin_observed_at.is_some_and(|observed| now - observed > Duration::seconds(30))
                || now - prepared.sealed_at > Duration::seconds(30)
            {
                return Err(invalid(
                    "initial history observations or configuration became stale",
                ));
            }
            self.runtime
                .configuration
                .authority_fences()?
                .check_workspace(prepared.workspace)?;
            Self::check_no_release_intent(
                &self.runtime,
                input.intent,
                input.gate,
                input.parent,
                input.acceptance,
                input.carry,
            )?;
            let work = input.gate.work_key()?;
            if snapshot.unresolved_operations.values().any(|targets| {
                targets.contains(&work) || targets.contains(&prepared.runtime.to_string())
            }) {
                return Err(invalid(
                    "no-release history still has unresolved related effects",
                ));
            }
            self.check_no_release_history_storage(proof.validate_references())?;
            // Re-seed after every provider await, inside the final write gate.
            // Another facade may have learned the original identity meanwhile.
            self.seed_no_release_history_anchors(prepared.runtime)?;
            let _section = self.check_no_release_history_storage(
                self.records.section_lock("no-release-completions.lock"),
            )?;
            if let Some(prior) = self.releases.no_release_history.anchor(operation)? {
                self.remember_no_release_history_anchor(&prior)?;
                self.validate_no_release_history_anchor(&prior)?;
                if prior.signed.payload.intent_digest != prepared.intent_digest
                    || prior.outcome_digest() != proof.outcome_digest()
                {
                    return Err(invalid(
                        "known no-release seal names another immutable intent",
                    ));
                }
                return Ok(prior);
            }
            match self.read_no_release_history_anchor(proof.services.clone(), operation, None) {
                Ok(prior) => {
                    self.check_no_release_history_storage(prior.validate_references())?;
                    if prior.signed.payload.intent_digest != prepared.intent_digest
                        || prior.outcome_digest() != proof.outcome_digest()
                    {
                        return Err(invalid(
                            "concurrent no-release seal names different immutable intent",
                        ));
                    }
                    Ok(prior)
                }
                Err(error) if error.code == "record_missing" => {
                    self.runtime.require_released_inventory_available()?;
                    self.runtime.require_no_release_inventory_available()?;
                    self.validate_shared_no_release_resources(&prepared.outcome.payload)?;
                    // A failed global inventory cannot establish an empty
                    // namespace or authorize another initial seal. The guard is
                    // dropped before shared runtime/local anchor registration.
                    {
                        let state = self.releases.state.lock().map_err(|_| unavailable())?;
                        if let Some(error) = &state.completion_inventory_error {
                            return Err(error.clone());
                        }
                        state.historical_no_release.require_available()?;
                        state
                            .queue
                            .check_no_release_resources(&prepared.outcome.payload)?;
                    }
                    self.check_no_release_history_storage(
                        self.records
                            .write(&completion_leaf(operation), &proof.signed),
                    )?;
                    self.remember_no_release_history_anchor(&proof)?;
                    self.validate_no_release_history_anchor(&proof)?;
                    Ok(proof.clone())
                }
                Err(error) => Err(error),
            }
        })
    }

    /// Local immutable evidence only. Runtime consumers separately apply current
    /// configuration, permissions and complete native prerequisite visibility.
    pub(crate) fn no_release_completion_inventory(&self) -> Result<NoReleaseCompletionInventory> {
        self.runtime
            .control
            .serialize_observation(|_| self.no_release_completion_inventory_in_observation())
    }

    /// Release sync already owns Control; inspection uses the wrapper above.
    /// Reads, first-anchor learning and failure latching form one observation.
    pub(in crate::runtime_delivery::release) fn no_release_completion_inventory_in_observation(
        &self,
    ) -> Result<NoReleaseCompletionInventory> {
        self.runtime.registration.verify_control_owner()?;
        let result = self
            .records
            .records_bounded(65_536)
            .and_then(|names| self.no_release_completion_inventory_locked(names));
        if let Err(error) = &result {
            // Preserve an incomplete inventory before releasing the gate;
            // inspection-only failures must close fresh minting as well.
            self.invalidate_no_release_history_inventory(error)?;
        }
        result
    }

    fn no_release_completion_inventory_locked(
        &self,
        names: Vec<String>,
    ) -> Result<NoReleaseCompletionInventory> {
        if names.len() > 65_536 {
            return Err(invalid("no-release inventory exceeds finite storage bound"));
        }
        let services = self.no_release_history_services()?;
        self.seed_no_release_history_anchors(services.runtime)?;
        let anchors = self.releases.no_release_history.anchors()?;
        let mut identities = BTreeMap::<Uuid, (bool, bool)>::new();
        for operation in anchors.keys() {
            identities.insert(*operation, (false, false));
        }
        for name in names {
            let parsed = if let Some(id) = name
                .strip_prefix("no-release-completion-")
                .and_then(|n| n.strip_suffix(".json"))
            {
                Some((id, true))
            } else if !name.starts_with("no-release-unadmitted-") {
                name.strip_prefix("no-release-")
                    .and_then(|n| n.strip_suffix(".json"))
                    .map(|id| (id, false))
            } else {
                None
            };
            let Some((raw, seal)) = parsed else { continue };
            let id: Uuid = raw
                .parse()
                .map_err(|_| invalid("no-release inventory filename is invalid"))?;
            if id.is_nil() || id.to_string() != raw {
                return Err(invalid("no-release filename is not canonical"));
            }
            let entry = identities.entry(id).or_default();
            if seal {
                entry.0 = true;
            } else {
                entry.1 = true;
            }
            if identities.len() > MAX_REFERENCES {
                return Err(invalid("no-release inventory exceeds operation bound"));
            }
        }
        let remembered = self
            .releases
            .no_release_history
            .diagnostics
            .lock()
            .map_err(|_| invalid("no-release diagnostics lock failed"))?
            .clone();
        let mut retained_anchors = anchors.clone();
        let mut inventory = NoReleaseCompletionInventory {
            proofs: Vec::new(),
            anchors: Vec::new(),
            errors: BTreeMap::new(),
            rows: Vec::new(),
            sealed_operations: anchors.keys().copied().collect(),
        };
        for (operation, (has_seal, has_intent)) in identities {
            let anchor = anchors.get(&operation);
            if has_seal || anchor.is_some() {
                // Only an absent known seal or a different authenticated seal
                // can be quarantined by its retained original scope. A present
                // malformed/unauthenticated envelope is a global inventory failure.
                let signed: Result<Signed<NoReleaseCompletionRecord>> =
                    match self.records.read(&completion_leaf(operation)) {
                        Ok(signed) => Ok(signed),
                        Err(error) if error.code == "record_missing" => Err(error),
                        Err(error) => return Err(error),
                    };
                if let Ok(signed) = &signed {
                    self.runtime.services.key.verify(PURPOSE, signed)?;
                    validate_inspection_identity(
                        &signed.payload,
                        services.runtime,
                        &self.runtime.services.key,
                    )?;
                    if signed.payload.operation != operation {
                        return Err(invalid("no-release seal filename identity changed"));
                    }
                }
                let read = (|| -> Result<VerifiedNoReleaseCompletion> {
                    let signed = signed?;
                    let digest = signed.digest()?;
                    if anchor.is_some_and(|prior| prior.digest() != digest) {
                        return Err(invalid(
                            "previously observed immutable no-release seal changed",
                        ));
                    }
                    Ok(VerifiedNoReleaseCompletion {
                        signed,
                        digest,
                        services: services.clone(),
                    })
                })();
                let proof = match read {
                    Ok(proof) => proof,
                    Err(error) if error.code == "unsafe_storage" => return Err(error),
                    Err(error) if anchor.is_some() => {
                        inventory.errors.insert(operation, error.clone());
                        inventory
                            .rows
                            .push(sealed_row(anchor.expect("checked anchor"), Some(error))?);
                        continue;
                    }
                    Err(error) if error.code == "record_missing" => {
                        inventory.errors.insert(operation, error);
                        if has_intent {
                            self.append_unsealed_history(operation, &remembered, &mut inventory)?;
                        }
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                // Only an authenticated actual seal or prior authenticated
                // anchor classifies this operation as formerly completed.
                self.remember_no_release_history_anchor(&proof)?;
                retained_anchors.insert(operation, proof.clone());
                inventory.sealed_operations.insert(operation);
                let diagnostic = match proof.validate() {
                    Ok(()) => None,
                    Err(error) if error.code == "unsafe_storage" => return Err(error),
                    Err(error) => Some(error),
                };
                let row = sealed_row(&proof, diagnostic.clone())?;
                if let Some(error) = diagnostic {
                    inventory.errors.insert(operation, error);
                } else {
                    inventory.proofs.push(proof);
                }
                inventory.rows.push(row);
            } else if has_intent {
                self.append_unsealed_history(operation, &remembered, &mut inventory)?;
            }
        }
        inventory.rows.sort_by(|a, b| {
            (a.workspace, &a.parent, a.admission).cmp(&(b.workspace, &b.parent, b.admission))
        });
        inventory.anchors = retained_anchors.into_values().collect();
        Ok(inventory)
    }

    fn append_unsealed_history(
        &self,
        operation: Uuid,
        remembered: &BTreeMap<Uuid, Error>,
        inventory: &mut NoReleaseCompletionInventory,
    ) -> Result<()> {
        let signed: Signed<NoReleaseIntent> = match self.records.read(&intent_leaf(operation)) {
            Ok(value) => value,
            Err(error) if error.code == "record_missing" => {
                inventory.errors.insert(operation, error);
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.runtime
            .services
            .key
            .verify("runtime_no_release_intent", &signed)?;
        self.runtime
            .services
            .key
            .verify("no_release_required", &signed.payload.record)?;
        let intent = signed.payload;
        let record = &intent.record.payload;
        record.work.validate()?;
        if intent.schema_version != 1
            || record.schema_version != 1
            || record.operation_id != operation
            || record.runtime_id != self.runtime.configuration.snapshot()?.runtime.id
            || intent.workspace.is_nil()
            || record.work.child.is_some()
            || intent.attempt.purpose != "no_release_required"
            || intent.attempt.attempt == 0
            || intent.attempt.attempt > intent.attempt.max_attempts
        {
            return Err(invalid(
                "authenticated no-release intent has invalid display identity",
            ));
        }
        let diagnostic = inventory.errors.get(&operation).cloned().or_else(|| remembered.get(&operation).cloned())
            .unwrap_or_else(|| if !record.targets.is_empty() || record.baselines.iter().any(|b| !b.deployment_free) {
                Error::new("historical_baseline_proof_unavailable", "deployed no-release history lacks its actual immutable observation provenance")
            } else { Error::new("no_release_completion_pending", "no immutable observed no-release completion is retained") });
        // A signed intent is a historical claim, not proof it was published.
        // Only the sealed row below exposes a native comment identity/URL.
        inventory
            .rows
            .push(crate::inspection::NoReleaseInspectionRow {
                workspace: intent.workspace,
                parent: IssueKey::new(
                    &record.work.linear_endpoint,
                    record.work.linear_workspace,
                    record.work.parent,
                )?,
                work: record.work.clone(),
                admission: operation,
                publication: crate::inspection::NoReleasePublicationInspection {
                    effect: intent.effect()?,
                    comment: None,
                    url: None,
                },
                record: record.clone(),
                record_digest: intent.record.digest()?,
                completion: None,
                diagnostic: Some(diagnostic.clone()),
                display: crate::inspection::ParentDisplay::default(),
            });
        inventory.errors.insert(operation, diagnostic);
        Ok(())
    }

    pub(crate) fn no_release_inspection_rows(
        &self,
    ) -> Result<Vec<crate::inspection::NoReleaseInspectionRow>> {
        Ok(self.no_release_completion_inventory()?.rows)
    }
}

fn validate_inspection_identity(
    record: &NoReleaseCompletionRecord,
    runtime: Uuid,
    key: &AttestationKey,
) -> Result<()> {
    key.verify("no_release_required", &record.outcome)?;
    record.work.validate()?;
    let outcome = &record.outcome.payload;
    if !record.valid_schema()
        || record.runtime != runtime
        || record.workspace.is_nil()
        || record.operation.is_nil()
        || record.publication_operation.is_nil()
        || record.work.child.is_some()
        || outcome.schema_version != 1
        || outcome.runtime_id != runtime
        || outcome.operation_id != record.operation
        || outcome.work != record.work
        || outcome.recorded_at != record.recorded_at
        || record.outcome.digest()? != record.outcome_digest
        || outcome.evidence_token != record.evidence_token
        || outcome.acceptance_digest != record.acceptance_digest
    {
        return Err(invalid(
            "authenticated no-release seal has inconsistent inspection identity",
        ));
    }
    Ok(())
}

fn sealed_row(
    proof: &VerifiedNoReleaseCompletion,
    diagnostic: Option<Error>,
) -> Result<crate::inspection::NoReleaseInspectionRow> {
    let record = &proof.signed.payload;
    Ok(crate::inspection::NoReleaseInspectionRow {
        workspace: record.workspace,
        parent: IssueKey::new(
            &record.work.linear_endpoint,
            record.work.linear_workspace,
            record.work.parent,
        )?,
        work: record.work.clone(),
        admission: record.operation,
        publication: crate::inspection::NoReleasePublicationInspection {
            effect: proof.publication_operation(),
            comment: Some(record.publication.comment),
            url: record.publication.url.clone(),
        },
        record: proof.record().clone(),
        record_digest: record.outcome_digest.clone(),
        completion: diagnostic.is_none().then_some(
            crate::inspection::NoReleaseCompletionInspection {
                completed_at: record.recorded_at,
            },
        ),
        diagnostic,
        display: crate::inspection::ParentDisplay {
            workspace_name: Some(record.display.workspace_name.clone()),
            identifier: Some(record.display.identifier.clone()),
            title: Some(record.display.title.clone()),
            url: Some(record.display.url.clone()),
        },
    })
}
