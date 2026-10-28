//! Completed read-only coordination claims become authority only after exact
//! current native sources, scoped child routes and final intake records verify.
use crate::{
    authority::{CommentClass, CommentRevision, Corrections, IssueKey},
    control::{ControlHost, Receipt, ReceiptPhase},
    evidence::{AttestationKey, Signed, WorkIdentity, decode_signed_publication},
    reconcile::{ParentProjection, ReconcileRefs, RuntimeProjection},
};
use chrono::{DateTime, Utc};
use kegari_config::{Config, Workspace};
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_providers::Comment;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[path = "correction_transition.rs"]
mod transition;
pub use transition::{VerifiedCorrectionRoutes, VerifiedCorrectionTransition};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassificationProposal {
    pub kind: String,
    pub classifications: Vec<ProposedClassification>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedClassification {
    pub source_comment: Uuid,
    pub source_revision: String,
    pub class: CommentClass,
    pub rationale: String,
    pub routes: Vec<ProposedRoute>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedRoute {
    pub child: Uuid,
    pub instructions: String,
    pub validation_expectations: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRevision {
    issue: Uuid,
    comment: Uuid,
    author: Uuid,
    corrector: Uuid,
    body_digest: String,
    edited_at: Option<DateTime<Utc>>,
}
impl SourceRevision {
    fn revision(&self) -> CommentRevision {
        CommentRevision {
            id: self.comment,
            author: Some(self.author),
            body_digest: self.body_digest.clone(),
            edited_at: self.edited_at,
            observed_revision: 0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteRecord {
    schema_version: u32,
    runtime: Uuid,
    workspace: Uuid,
    parent: IssueKey,
    worker_operation: Uuid,
    proposal: Uuid,
    outcome_digest: String,
    publisher_operation: Uuid,
    writer: String,
    work: WorkIdentity,
    source: SourceRevision,
    child: Uuid,
    child_scope_digest: String,
    instructions: String,
    validation_expectations: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteReference {
    child: Uuid,
    operation: Uuid,
    comment: Uuid,
    body_digest: String,
    signed_digest: String,
    scope_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassifiedRevision {
    source: SourceRevision,
    class: CommentClass,
    rationale: String,
    routes: Vec<RouteReference>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassificationRecord {
    schema_version: u32,
    runtime: Uuid,
    workspace: Uuid,
    parent: IssueKey,
    worker_operation: Uuid,
    proposal: Uuid,
    outcome_digest: String,
    publisher_operation: Uuid,
    writer: String,
    before: WorkIdentity,
    sequence: u64,
    previous: Option<String>,
    prior_history: BTreeSet<String>,
    history: BTreeSet<String>,
    classifications: Vec<ClassifiedRevision>,
}
#[derive(Clone, Debug)]
pub struct CorrectionPublication {
    operation: Uuid,
    comment: Uuid,
    digest: String,
    generation: String,
    watermark: u64,
    remaining_pending: Vec<Uuid>,
}
impl CorrectionPublication {
    pub fn operation_id(&self) -> Uuid {
        self.operation
    }
    pub fn comment_id(&self) -> Uuid {
        self.comment
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn generation(&self) -> &str {
        &self.generation
    }
    pub fn watermark(&self) -> u64 {
        self.watermark
    }
    pub fn remaining_pending(&self) -> &[Uuid] {
        &self.remaining_pending
    }
}
/// Verified existing native stages only; this is progress, never authority to
/// create a missing stage or retry an unknown write.
pub struct CorrectionRecovery {
    publication: Option<CorrectionPublication>,
    stages: BTreeMap<Uuid, String>,
    stage_limit: usize,
}
impl CorrectionRecovery {
    pub fn publication(&self) -> Option<&CorrectionPublication> {
        self.publication.as_ref()
    }
    pub fn stages(&self) -> &BTreeMap<Uuid, String> {
        &self.stages
    }
    pub fn stage_limit(&self) -> usize {
        self.stage_limit
    }
    pub fn fingerprint(&self) -> Result<String> {
        canonical_digest(1, &self.stages)
    }
}
struct VerifiedIntake {
    signed: Signed<ClassificationRecord>,
    comment: Uuid,
}
fn invalid(message: &str) -> Error {
    Error::new("correction_publication", message)
}
fn text(value: &str, maximum: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > maximum || value.contains('\0') {
        return Err(invalid(
            "correction instructions/rationale must be concrete and bounded",
        ));
    }
    Ok(())
}
fn validate_proposal(value: &serde_json::Value) -> Result<ClassificationProposal> {
    let proposal: ClassificationProposal = serde_json::from_value(value.clone()).map_err(|_| {
        invalid("coordination result must use the exact correction_classification schema")
    })?;
    if proposal.kind != "correction_classification"
        || proposal.classifications.is_empty()
        || proposal.classifications.len() > 128
        || serde_json::to_vec(&proposal)
            .map_err(|_| invalid("claim cannot be encoded"))?
            .len()
            > 512 * 1024
    {
        return Err(invalid(
            "classification claim is absent, unsupported or exceeds finite bounds",
        ));
    }
    let mut sources = BTreeSet::new();
    for item in &proposal.classifications {
        if item.source_comment.is_nil() || !sources.insert(item.source_comment) {
            return Err(invalid(
                "classification source IDs must be nonnil and unique",
            ));
        }
        crate::evidence::validate_digest(&item.source_revision)?;
        text(&item.rationale, 4096)?;
        if (item.class == CommentClass::Corrective) != !item.routes.is_empty()
            || item.routes.len() > 128
        {
            return Err(invalid(
                "only corrective classifications have nonempty finite child routes",
            ));
        }
        let mut children = BTreeSet::new();
        for route in &item.routes {
            if route.child.is_nil()
                || !children.insert(route.child)
                || route.validation_expectations.is_empty()
                || route.validation_expectations.len() > 64
            {
                return Err(invalid(
                    "each route needs one existing child and concrete validation expectations",
                ));
            }
            text(&route.instructions, 16_384)?;
            for validation in &route.validation_expectations {
                text(validation, 4096)?;
            }
        }
    }
    Ok(proposal)
}
fn source(
    parent: &ParentProjection,
    configured: &Workspace,
    writer: &str,
    key: &AttestationKey,
    runtime: Uuid,
    id: Uuid,
) -> Result<SourceRevision> {
    let matches: Vec<_> = parent
        .observations
        .values()
        .filter(|o| {
            o.issue.id == parent.key.issue || parent.required_children.contains(&o.issue.id)
        })
        .flat_map(|o| &o.comments)
        .filter(|c| c.id == id)
        .collect();
    if matches.len() != 1 {
        return Err(invalid(
            "source comment is absent or duplicated in complete scoped provider observations",
        ));
    }
    let comment = matches[0];
    let author = comment
        .user_id
        .filter(|_| !comment.bot && !comment.delegated)
        .ok_or_else(|| {
            invalid("source comment does not have a native nondelegated human identity")
        })?;
    if comment.body_digest != bytes_digest(comment.body.as_bytes()) {
        return Err(invalid(
            "source comment bytes differ from native revision digest",
        ));
    }
    let corrector = if comment
        .body
        .contains("\n\n<!-- kegari:v0:explicit_correction -->\n")
    {
        crate::decisions::VerifiedExplicitCorrection::verify_comment(
            comment,
            &parent.key,
            runtime,
            configured.id,
            &configured.actors.correctors,
            writer,
            key,
        )?
        .record()
        .linear_actor
    } else {
        if !configured.actors.correctors.contains(&author) {
            return Err(invalid(
                "source author is not a current permitted corrector",
            ));
        }
        author
    };
    Ok(SourceRevision {
        issue: comment.issue_id,
        comment: id,
        author,
        corrector,
        body_digest: comment.body_digest.clone(),
        edited_at: comment.edited_at.or(comment.updated_at),
    })
}
fn native<'a>(
    parent: &'a ParentProjection,
    issue: Uuid,
    id: Uuid,
    digest: &str,
    writer: &str,
) -> Result<&'a Comment> {
    let observation = parent
        .observations
        .get(&issue)
        .ok_or_else(|| invalid("published issue is absent from complete scope observation"))?;
    let matching: Vec<_> = observation
        .comments
        .iter()
        .filter(|c| c.body_digest == digest)
        .collect();
    let comment = matching
        .first()
        .copied()
        .filter(|_| matching.len() == 1)
        .ok_or_else(|| invalid("publication requires one unique exact current provider record"))?;
    if comment.id != id
        || comment.issue_id != issue
        || comment.bot
        || comment.delegated
        || comment.user_id.map(|u| u.to_string()).as_deref() != Some(writer)
        || comment.body_digest != bytes_digest(comment.body.as_bytes())
    {
        return Err(invalid(
            "publication native identity, bytes or target changed",
        ));
    }
    Ok(comment)
}
fn receipt<'a>(
    receipts: &'a [Receipt],
    runtime: Uuid,
    parent: &IssueKey,
    operation: Uuid,
    comment: &Comment,
    writer: &str,
    kind: &str,
) -> Result<&'a Receipt> {
    let matches: Vec<_> = receipts
        .iter()
        .filter(|r| r.operation_id == operation)
        .collect();
    let receipt = matches
        .first()
        .copied()
        .filter(|_| matches.len() == 1)
        .ok_or_else(|| invalid("publication has no unique owned operation receipt"))?;
    if receipt.runtime_id != runtime
        || receipt.kind != "linear_comment"
        || !matches!(
            receipt.phase,
            ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
        )
        || receipt.expected.get("correction_parent") != Some(&parent.identity()?)
        || receipt.expected.get("correction_kind").map(String::as_str) != Some(kind)
        || receipt.expected.get("provider_body_digest") != Some(&comment.body_digest)
        || receipt.expected.get("provider_writer").map(String::as_str) != Some(writer)
        || receipt.remote_ids.get("comment_id") != Some(&comment.id.to_string())
        || !receipt.canonical_targets.contains(&format!(
            "linear:{}:{}:{}",
            parent.endpoint, parent.workspace, comment.issue_id
        ))
    {
        return Err(invalid(
            "publication receipt does not acknowledge its exact current native record",
        ));
    }
    Ok(receipt)
}
fn read_chain(
    config: &Config,
    configured: &Workspace,
    parent: &ParentProjection,
    key: &AttestationKey,
    receipts: &[Receipt],
) -> Result<Vec<VerifiedIntake>> {
    let writer = &config.connections[&configured.linear_connection].account_id;
    let mut chain = Vec::new();
    if let Some(observed) = parent.observations.get(&parent.key.issue) {
        for comment in &observed.comments {
            if !comment
                .body
                .starts_with("<!-- kegari:v0:correction_classification -->\n")
            {
                continue;
            }
            // A quoted or foreign author's copy is untrusted issue context, not a host record.
            if comment.user_id.map(|id| id.to_string()).as_deref() != Some(writer) {
                continue;
            }
            let signed: Signed<ClassificationRecord> =
                decode_signed_publication(&comment.body, "correction_classification", key)?;
            let record = &signed.payload;
            native(
                parent,
                parent.key.issue,
                comment.id,
                &comment.body_digest,
                writer,
            )?;
            receipt(
                receipts,
                config.runtime.id,
                &parent.key,
                record.publisher_operation,
                comment,
                writer,
                "final",
            )?;
            if record.schema_version != 1
                || record.runtime != config.runtime.id
                || record.workspace != configured.id
                || record.parent != parent.key
                || record.writer != *writer
                || record.before.parent != parent.key.issue
                || record.before.child.is_some()
                || record.before.linear_workspace != parent.key.workspace
                || record.before.linear_endpoint != parent.key.endpoint
                || record.sequence == 0
                || record.classifications.is_empty()
                || record.classifications.len() > 128
                || record.history.len() > 16_384
                || !record.prior_history.is_subset(&record.history)
            {
                return Err(invalid(
                    "classification record crosses parent scope or its finite history contract",
                ));
            }
            record.before.validate()?;
            for digest in record.history.iter().chain([&record.outcome_digest]) {
                crate::evidence::validate_digest(digest)?;
            }
            let mut expected = record.prior_history.clone();
            let mut ids = BTreeSet::new();
            for item in &record.classifications {
                if !ids.insert(item.source.comment)
                    || !configured
                        .actors
                        .correctors
                        .contains(&item.source.corrector)
                    || (item.class == CommentClass::Corrective) != !item.routes.is_empty()
                {
                    return Err(invalid("classification source or role is ambiguous"));
                }
                text(&item.rationale, 4096)?;
                if item.class == CommentClass::Corrective {
                    expected.insert(item.source.revision().identity_digest()?);
                }
            }
            if expected != record.history {
                return Err(invalid(
                    "classification history does not match its actual corrective revisions",
                ));
            }
            chain.push(VerifiedIntake {
                signed,
                comment: comment.id,
            });
        }
    }
    chain.sort_by_key(|r| r.signed.payload.sequence);
    let mut previous = None;
    let mut prior_history = BTreeSet::new();
    for (index, item) in chain.iter().enumerate() {
        let record = &item.signed.payload;
        if record.sequence != index as u64 + 1
            || record.previous != previous
            || !prior_history.is_subset(&record.prior_history)
        {
            return Err(invalid(
                "classification chain has a missing, conflicting or regressing publication",
            ));
        }
        previous = Some(item.signed.digest()?);
        prior_history = record.history.clone();
    }
    // A previously acknowledged final record cannot vanish and resurrect an
    // older classification after restart. Durable receipts are invalidation only.
    let parent_identity = parent.key.identity()?;
    let present: BTreeSet<_> = chain
        .iter()
        .map(|c| c.signed.payload.publisher_operation)
        .collect();
    for r in receipts.iter().filter(|r| {
        r.runtime_id == config.runtime.id
            && r.expected.get("correction_parent") == Some(&parent_identity)
            && r.expected.get("correction_kind").map(String::as_str) == Some("final")
            && matches!(
                r.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            && r.remote_ids.contains_key("comment_id")
    }) {
        if !present.contains(&r.operation_id) {
            return Err(invalid(
                "an acknowledged classification publication is missing or changed",
            ));
        }
    }
    Ok(chain)
}
fn verify_route(
    config: &Config,
    parent: &ParentProjection,
    intake: &ClassificationRecord,
    item: &ClassifiedRevision,
    reference: &RouteReference,
    key: &AttestationKey,
    receipts: &[Receipt],
) -> Result<()> {
    let native = native(
        parent,
        reference.child,
        reference.comment,
        &reference.body_digest,
        &intake.writer,
    )?;
    receipt(
        receipts,
        config.runtime.id,
        &parent.key,
        reference.operation,
        native,
        &intake.writer,
        "route",
    )?;
    let signed: Signed<RouteRecord> =
        decode_signed_publication(&native.body, "correction_route", key)?;
    let route = &signed.payload;
    if signed.digest()? != reference.signed_digest
        || route.schema_version != 1
        || route.runtime != intake.runtime
        || route.workspace != intake.workspace
        || route.parent != intake.parent
        || route.writer != intake.writer
        || route.proposal != intake.proposal
        || route.worker_operation != intake.worker_operation
        || route.outcome_digest != intake.outcome_digest
        || route.publisher_operation != reference.operation
        || route.work != intake.before
        || route.source != item.source
        || route.child != reference.child
        || route.child_scope_digest != reference.scope_digest
        || route.validation_expectations.is_empty()
    {
        return Err(invalid(
            "signed child route differs from final classification/source provenance",
        ));
    }
    text(&route.instructions, 16_384)?;
    for validation in &route.validation_expectations {
        text(validation, 4096)?;
    }
    Ok(())
}

/// Rebuild clearance only from a complete current native signed chain and routes.
/// Protected local references contribute monotone invalidation, never success.
pub(crate) fn restore_classifications(
    config: &Config,
    projection: &RuntimeProjection,
    refs: &mut ReconcileRefs,
    key: &AttestationKey,
    receipts: &[Receipt],
) -> Result<(bool, BTreeMap<IssueKey, Error>)> {
    let mut changed = false;
    let mut errors = BTreeMap::new();
    for workspace in projection.workspaces.values().filter(|w| w.complete) {
        let configured = config
            .workspaces
            .values()
            .find(|w| w.id == workspace.workspace_id)
            .ok_or_else(|| invalid("classification workspace disappeared"))?;
        let writer = &config.connections[&configured.linear_connection].account_id;
        for parent in workspace.parents.values() {
            let prior = refs
                .entry(workspace.workspace_id)
                .or_default()
                .entry(parent.key.issue)
                .or_default();
            let before = canonical_digest(1, &prior.corrections)?;
            let mut invalidations = parent.corrections.invalidations();
            invalidations.pending.retain(|id, _| {
                !prior.host_comment_ids.contains(id) || prior.explicit_corrections.contains_key(id)
            });
            let chain = read_chain(config, configured, parent, key, receipts);
            if let Ok(chain) = &chain {
                for intake in chain {
                    invalidations
                        .corrective_revisions
                        .extend(intake.signed.payload.history.clone());
                    invalidations.corrective_comments.extend(
                        intake
                            .signed
                            .payload
                            .classifications
                            .iter()
                            .filter(|c| c.class == CommentClass::Corrective)
                            .map(|c| c.source.comment),
                    );
                }
            }
            let mut rebuilt = Corrections::from_invalidations(invalidations)?;
            for observed in parent.observations.values().filter(|o| {
                o.issue.id == parent.key.issue || parent.required_children.contains(&o.issue.id)
            }) {
                for comment in &observed.comments {
                    if comment.bot || comment.delegated || comment.user_id.is_none() {
                        continue;
                    }
                    let explicit = prior
                        .explicit_corrections
                        .get(&comment.id)
                        .filter(|c| c.body_digest() == comment.body_digest);
                    rebuilt.observe(
                        CommentRevision {
                            id: comment.id,
                            author: comment.user_id,
                            body_digest: comment.body_digest.clone(),
                            edited_at: comment.edited_at.or(comment.updated_at),
                            observed_revision: parent.observation_revision,
                        },
                        &configured.actors.correctors,
                        &prior.host_comment_ids,
                        explicit.map(|c| c.record().linear_actor),
                    )?;
                }
            }
            let applied = (|| -> Result<()> {
                let chain = chain?;
                let mut latest = BTreeMap::new();
                for intake in &chain {
                    for item in &intake.signed.payload.classifications {
                        for route in &item.routes {
                            verify_route(
                                config,
                                parent,
                                &intake.signed.payload,
                                item,
                                route,
                                key,
                                receipts,
                            )?;
                        }
                        latest.insert(item.source.comment, (intake, item));
                    }
                }
                for (id, (intake, item)) in latest {
                    let current =
                        match source(parent, configured, writer, key, config.runtime.id, id) {
                            Ok(current) => current,
                            Err(error)
                                if item.class == CommentClass::Informational
                                    && !parent
                                        .observations
                                        .values()
                                        .any(|o| o.comments.iter().any(|c| c.id == id)) =>
                            {
                                let _ = error;
                                continue;
                            }
                            Err(error) => return Err(error),
                        };
                    if current != item.source {
                        continue;
                    }
                    let Some(work) = parent.work.as_ref() else {
                        continue;
                    };
                    if work.generation.authorization
                        != intake.signed.payload.before.generation.authorization
                        || work.generation.scope != intake.signed.payload.before.generation.scope
                        || work.generation.definition
                            != intake.signed.payload.before.generation.definition
                    {
                        continue;
                    }
                    if prior.explicit_corrections.contains_key(&id)
                        && item.class != CommentClass::Corrective
                    {
                        return Err(invalid(
                            "explicit native CLI correction cannot become informational or ambiguous",
                        ));
                    }
                    for route in &item.routes {
                        let child = parent
                            .children
                            .get(&route.child)
                            .filter(|_| parent.required_children.contains(&route.child))
                            .ok_or_else(|| {
                                invalid("routed child left the current required inventory")
                            })?;
                        if child
                            .scope
                            .as_ref()
                            .map(|scope| canonical_digest(1, scope))
                            .transpose()?
                            .as_ref()
                            != Some(&route.scope_digest)
                        {
                            return Err(invalid("routed child scope changed since classification"));
                        }
                    }
                    rebuilt.classify(id, &current.body_digest, item.class)?;
                }
                Ok(())
            })();
            if let Err(error) = applied {
                errors.insert(parent.key.clone(), error);
            }
            changed |= before != canonical_digest(1, &rebuilt)?;
            prior.corrections = rebuilt;
        }
    }
    Ok((changed, errors))
}

pub struct CorrectionDependencies {
    pub configuration: std::sync::Arc<dyn crate::app::ConfigurationControl>,
    pub control: std::sync::Arc<ControlHost>,
    pub observations: std::sync::Arc<crate::observations::ObservationCache>,
    pub secrets: std::sync::Arc<dyn crate::secrets::SecretStore>,
    pub key: std::sync::Arc<AttestationKey>,
    pub inputs: std::sync::Arc<dyn crate::configuration::ConfigurationInputs>,
    pub owner_check: crate::preparation::OwnerCheck,
    pub control_owner_check: std::sync::Arc<dyn Fn() -> Result<()> + Send + Sync>,
}
pub struct CorrectionPublisher {
    services: CorrectionDependencies,
}
#[derive(Clone)]
struct PublicationSpec {
    operation: Uuid,
    issue: Uuid,
    kind: &'static str,
    body: String,
}
#[derive(Clone)]
struct PublicationPlan {
    gate: crate::reconcile::FreshGate,
    sources: BTreeMap<Uuid, SourceRevision>,
    previous: Option<String>,
    routes: Vec<(RouteReference, RouteRecord)>,
}
impl CorrectionPublisher {
    pub fn new(services: CorrectionDependencies) -> Self {
        Self { services }
    }
    fn client(&self, config: &Config, workspace: Uuid) -> Result<kegari_providers::LinearClient> {
        let transports = config
            .connections
            .keys()
            .filter_map(|name| {
                self.services
                    .inputs
                    .transport(name)
                    .map(|wire| (name.clone(), wire))
            })
            .collect();
        let clients = crate::connections::ConnectionRegistry::bind_with_transports(
            config,
            self.services.secrets.as_ref(),
            |name| self.services.inputs.environment(name),
            &transports,
        )?;
        clients
            .clients(config)?
            .get(&workspace)
            .map(|client| client.linear.clone())
            .ok_or_else(|| invalid("correction workspace has no current bound provider client"))
    }
    fn current(&self, plan: &PublicationPlan, lease: &crate::control::Lease) -> Result<()> {
        (self.services.owner_check)(lease)?;
        self.services
            .configuration
            .authority_fences()?
            .check_workspace(plan.gate.workspace_id())?;
        let config = self.services.configuration.snapshot()?;
        if crate::config::config_revision(&config)? != plan.gate.config_revision() {
            return Err(invalid(
                "configuration changed during correction publication",
            ));
        }
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == plan.gate.workspace_id())
            .ok_or_else(|| invalid("correction workspace was removed"))?;
        let projection = self.services.observations.admission_snapshot()?;
        let parent = projection.parent(workspace.id, plan.gate.work().parent)?;
        let gate = parent.fresh_gate(
            None,
            crate::reconcile::GatePurpose::Coordination,
            &crate::reconcile::DependencyOutcomes::default(),
            Utc::now(),
            chrono::Duration::seconds(30),
        )?;
        plan.gate
            .compare_current(&gate, Utc::now(), chrono::Duration::seconds(30))?;
        let writer = &config.connections[&workspace.linear_connection].account_id;
        for (id, expected) in &plan.sources {
            if source(
                parent,
                workspace,
                writer,
                &self.services.key,
                config.runtime.id,
                *id,
            )? != *expected
            {
                return Err(invalid(
                    "native correction source changed before publication",
                ));
            }
        }
        let chain = read_chain(
            &config,
            workspace,
            parent,
            &self.services.key,
            &self.services.control.operations()?,
        )?;
        let receipts = self.services.control.operations()?;
        for (reference, expected) in &plan.routes {
            let comment = native(
                parent,
                reference.child,
                reference.comment,
                &reference.body_digest,
                writer,
            )?;
            receipt(
                &receipts,
                config.runtime.id,
                &parent.key,
                reference.operation,
                comment,
                writer,
                "route",
            )?;
            let signed: Signed<RouteRecord> =
                decode_signed_publication(&comment.body, "correction_route", &self.services.key)?;
            if signed.digest()? != reference.signed_digest
                || canonical_digest(1, &signed.payload)? != canonical_digest(1, expected)?
            {
                return Err(invalid(
                    "an acknowledged child route changed before final intake publication",
                ));
            }
        }
        if chain.last().map(|last| last.signed.digest()).transpose()? != plan.previous {
            return Err(invalid(
                "another classification changed the exact intake chain",
            ));
        }
        Ok(())
    }
    /// Observe only existing deterministic stages. No runtime lease, permit,
    /// stage creation or provider write is used by this recovery path.
    pub async fn recover(
        self: &std::sync::Arc<Self>,
        completed: &crate::runtime::VerifiedCoordinationOutcome,
    ) -> Result<Option<CorrectionPublication>> {
        Ok(self.recover_progress(completed).await?.publication)
    }
    pub async fn recover_progress(
        self: &std::sync::Arc<Self>,
        completed: &crate::runtime::VerifiedCoordinationOutcome,
    ) -> Result<CorrectionRecovery> {
        (self.services.control_owner_check)()?;
        let outcome = completed.outcome();
        let proposal = validate_proposal(&outcome.claim)?;
        let config = self.services.configuration.snapshot()?;
        if config.runtime.id != outcome.runtime_id {
            return Err(invalid("correction recovery belongs to another runtime"));
        }
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == completed.original_authority().workspace_id())
            .ok_or_else(|| invalid("correction recovery workspace is no longer configured"))?;
        let client = self.client(&config, workspace.id)?;
        let writer = &config.connections[&workspace.linear_connection].account_id;
        let work = completed.original_authority().work();
        let parent = IssueKey::new(&work.linear_endpoint, work.linear_workspace, work.parent)?;
        let mut stages = Vec::new();
        for item in &proposal.classifications {
            for route in &item.routes {
                stages.push((
                    operation_id(
                        outcome.runtime_id,
                        outcome.proposal_id,
                        "route",
                        Some(item.source_comment),
                        Some(route.child),
                    )?,
                    route.child,
                    "route",
                    Some((item, route)),
                ));
            }
        }
        let final_operation =
            operation_id(outcome.runtime_id, outcome.proposal_id, "final", None, None)?;
        stages.push((final_operation, parent.issue, "final", None));
        let stage_limit = stages.len();
        let mut verified_stages = BTreeMap::new();
        let mut final_observed = false;
        for (operation, issue, kind, expected) in stages {
            let owned = match self.services.control.operation(operation) {
                Ok(receipt) => receipt,
                Err(error) if error.code == "record_missing" => continue,
                Err(error) => return Err(error),
            };
            let comments = client.comments(issue).await?;
            let matches: Vec<_> = comments
                .iter()
                .filter(|c| owned.expected.get("provider_body_digest") == Some(&c.body_digest))
                .collect();
            let [comment] = matches.as_slice() else {
                return Err(invalid(
                    "existing correction stage is missing or ambiguous; no recovery write is permitted",
                ));
            };
            if let Some((item, expected)) = expected {
                let signed: Signed<RouteRecord> = decode_signed_publication(
                    &comment.body,
                    "correction_route",
                    &self.services.key,
                )?;
                let route = &signed.payload;
                if route.schema_version != 1
                    || route.runtime != outcome.runtime_id
                    || route.workspace != workspace.id
                    || route.parent != parent
                    || route.worker_operation != outcome.operation_id
                    || route.proposal != outcome.proposal_id
                    || route.outcome_digest != completed.digest()
                    || route.publisher_operation != operation
                    || route.writer != *writer
                    || &route.work != work
                    || route.child != expected.child
                    || route.source.comment != item.source_comment
                    || route.source.body_digest != item.source_revision
                    || route.instructions != expected.instructions
                    || route.validation_expectations != expected.validation_expectations
                {
                    return Err(invalid(
                        "existing route does not bind the exact completed coordination proposal",
                    ));
                }
            } else {
                let signed: Signed<ClassificationRecord> = decode_signed_publication(
                    &comment.body,
                    "correction_classification",
                    &self.services.key,
                )?;
                let record = &signed.payload;
                if record.schema_version != 1
                    || record.runtime != outcome.runtime_id
                    || record.workspace != workspace.id
                    || record.parent != parent
                    || record.worker_operation != outcome.operation_id
                    || record.proposal != outcome.proposal_id
                    || record.outcome_digest != completed.digest()
                    || record.publisher_operation != operation
                    || record.writer != *writer
                    || &record.before != work
                {
                    return Err(invalid(
                        "existing final intake does not bind the completed coordination proposal",
                    ));
                }
            }
            self.observe_stage(
                &client,
                writer,
                &parent,
                &PublicationSpec {
                    operation,
                    issue,
                    kind,
                    body: comment.body.clone(),
                },
                owned,
            )
            .await?;
            verified_stages.insert(operation, canonical_digest(1, &serde_json::json!({
                "comment":comment.id,"body_digest":comment.body_digest,"writer":comment.user_id,
                "issue":issue
            }))?);
            final_observed |= kind == "final";
        }
        let projection = self.services.observations.refresh().await?;
        let publication = if final_observed {
            Some(self.published(
                &config,
                workspace,
                projection.parent(workspace.id, parent.issue)?,
                completed,
                final_operation,
            )?)
        } else {
            None
        };
        Ok(CorrectionRecovery {
            publication,
            stages: verified_stages,
            stage_limit,
        })
    }
    /// The claim has actual completed-turn provenance, but remains an untrusted
    /// semantic proposal until fresh source/scope checks and all publications pass.
    pub async fn publish(
        self: &std::sync::Arc<Self>,
        lease: &crate::control::Lease,
        completed: &crate::runtime::VerifiedCoordinationOutcome,
    ) -> Result<CorrectionPublication> {
        (self.services.control_owner_check)()?;
        let outcome = completed.outcome();
        let proposal = validate_proposal(&outcome.claim)?;
        let config = self.services.configuration.snapshot()?;
        if config.runtime.id != outcome.runtime_id {
            return Err(invalid("coordination outcome belongs to another runtime"));
        }
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == completed.original_authority().workspace_id())
            .ok_or_else(|| invalid("completed coordination workspace is no longer configured"))?;
        let writer = config.connections[&workspace.linear_connection]
            .account_id
            .clone();
        let client = self.client(&config, workspace.id)?;
        let final_operation =
            operation_id(outcome.runtime_id, outcome.proposal_id, "final", None, None)?;
        let projection = self.services.observations.refresh().await?;
        // Already existing final records take the metadata-only path, including
        // reconciliation of any earlier finite route admissions.
        match self.services.control.operation(final_operation) {
            Ok(_) => {
                return self.recover(completed).await?.ok_or_else(|| {
                    invalid("existing final correction intake could not be recovered")
                });
            }
            Err(error) if error.code == "record_missing" => {}
            Err(error) => return Err(error),
        }
        let parent =
            projection.parent(workspace.id, completed.original_authority().work().parent)?;
        let gate = parent.fresh_gate(
            None,
            crate::reconcile::GatePurpose::Coordination,
            &crate::reconcile::DependencyOutcomes::default(),
            Utc::now(),
            chrono::Duration::seconds(30),
        )?;
        completed.original_authority().compare_current(
            &gate,
            Utc::now(),
            chrono::Duration::seconds(30),
        )?;
        let chain = read_chain(
            &config,
            workspace,
            parent,
            &self.services.key,
            &self.services.control.operations()?,
        )?;
        let previous = chain.last().map(|v| v.signed.digest()).transpose()?;
        let references = self.services.observations.references()?;
        let refs = references
            .get(&workspace.id)
            .and_then(|p| p.get(&parent.key.issue))
            .cloned()
            .unwrap_or_default();
        let pending: BTreeSet<_> = parent.corrections.pending_ids().into_iter().collect();
        let mut sources = BTreeMap::new();
        for item in &proposal.classifications {
            if !pending.contains(&item.source_comment)
                || refs.host_comment_ids.contains(&item.source_comment)
                    && !refs.explicit_corrections.contains_key(&item.source_comment)
            {
                return Err(invalid(
                    "classification source is not pending permitted native input",
                ));
            }
            let native = source(
                parent,
                workspace,
                &writer,
                &self.services.key,
                config.runtime.id,
                item.source_comment,
            )?;
            if native.body_digest != item.source_revision
                || refs.explicit_corrections.contains_key(&item.source_comment)
                    && item.class != CommentClass::Corrective
            {
                return Err(invalid(
                    "classification differs from the exact current corrective source",
                ));
            }
            for route in &item.routes {
                let child = parent
                    .children
                    .get(&route.child)
                    .filter(|_| parent.required_children.contains(&route.child))
                    .ok_or_else(|| {
                        invalid(
                            "correction may route only to existing authorized required children",
                        )
                    })?;
                if child.scope.is_none() || child.work.is_none() || !child.blockers.is_empty() {
                    return Err(invalid(
                        "routed child has no complete current scoped authority",
                    ));
                }
            }
            sources.insert(item.source_comment, native);
        }
        let mut plan = PublicationPlan {
            gate: gate.clone(),
            sources,
            previous: previous.clone(),
            routes: Vec::new(),
        };
        self.current(&plan, lease)?;
        let mut history = parent.corrections.invalidations().corrective_revisions;
        let prior_history = history.clone();
        let mut classified = Vec::new();
        let mut ordered = proposal.classifications;
        ordered.sort_by_key(|item| item.source_comment);
        for item in ordered {
            let native = plan.sources[&item.source_comment].clone();
            let mut routes = Vec::new();
            let mut ordered_routes = item.routes;
            ordered_routes.sort_by_key(|r| r.child);
            for route in ordered_routes {
                let scope = parent.children[&route.child]
                    .scope
                    .as_ref()
                    .ok_or_else(|| invalid("child scope disappeared"))?;
                let operation = operation_id(
                    outcome.runtime_id,
                    outcome.proposal_id,
                    "route",
                    Some(item.source_comment),
                    Some(route.child),
                )?;
                let record = RouteRecord {
                    schema_version: 1,
                    runtime: outcome.runtime_id,
                    workspace: workspace.id,
                    parent: parent.key.clone(),
                    worker_operation: outcome.operation_id,
                    proposal: outcome.proposal_id,
                    outcome_digest: completed.digest().into(),
                    publisher_operation: operation,
                    writer: writer.clone(),
                    work: gate.work().clone(),
                    source: native.clone(),
                    child: route.child,
                    child_scope_digest: canonical_digest(1, scope)?,
                    instructions: route.instructions,
                    validation_expectations: route.validation_expectations,
                };
                let signed = self.services.key.sign("correction_route", &record)?;
                let body = crate::evidence::encode_signed_publication("correction_route", &signed)?;
                self.services.observations.refresh().await?;
                let comment = self
                    .stage(
                        lease,
                        &plan,
                        &client,
                        &writer,
                        PublicationSpec {
                            operation,
                            issue: route.child,
                            kind: "route",
                            body,
                        },
                    )
                    .await?;
                let reference = RouteReference {
                    child: route.child,
                    operation,
                    comment: comment.id,
                    body_digest: comment.body_digest,
                    signed_digest: signed.digest()?,
                    scope_digest: record.child_scope_digest.clone(),
                };
                plan.routes.push((reference.clone(), record));
                routes.push(reference);
            }
            if item.class == CommentClass::Corrective {
                history.insert(native.revision().identity_digest()?);
            }
            classified.push(ClassifiedRevision {
                source: native,
                class: item.class,
                rationale: item.rationale,
                routes,
            });
        }
        self.services.observations.refresh().await?;
        self.current(&plan, lease)?;
        let record = ClassificationRecord {
            schema_version: 1,
            runtime: outcome.runtime_id,
            workspace: workspace.id,
            parent: parent.key.clone(),
            worker_operation: outcome.operation_id,
            proposal: outcome.proposal_id,
            outcome_digest: completed.digest().into(),
            publisher_operation: final_operation,
            writer: writer.clone(),
            before: gate.work().clone(),
            sequence: chain.len() as u64 + 1,
            previous,
            prior_history,
            history,
            classifications: classified,
        };
        let signed = self
            .services
            .key
            .sign("correction_classification", &record)?;
        let body =
            crate::evidence::encode_signed_publication("correction_classification", &signed)?;
        self.stage(
            lease,
            &plan,
            &client,
            &writer,
            PublicationSpec {
                operation: final_operation,
                issue: parent.key.issue,
                kind: "final",
                body,
            },
        )
        .await?;
        let after = self.services.observations.refresh().await?;
        self.published(
            &config,
            workspace,
            after.parent(workspace.id, parent.key.issue)?,
            completed,
            final_operation,
        )
    }
    fn published(
        &self,
        config: &Config,
        workspace: &Workspace,
        parent: &ParentProjection,
        completed: &crate::runtime::VerifiedCoordinationOutcome,
        operation: Uuid,
    ) -> Result<CorrectionPublication> {
        if crate::config::config_revision(&self.services.configuration.snapshot()?)?
            != crate::config::config_revision(config)?
        {
            return Err(invalid(
                "configuration changed after classification publication",
            ));
        }
        let chain = read_chain(
            config,
            workspace,
            parent,
            &self.services.key,
            &self.services.control.operations()?,
        )?;
        let published = chain
            .iter()
            .find(|r| r.signed.payload.publisher_operation == operation)
            .ok_or_else(|| invalid("final classification is not currently published"))?;
        let record = &published.signed.payload;
        if record.outcome_digest != completed.digest()
            || record.proposal != completed.outcome().proposal_id
        {
            return Err(invalid(
                "final classification has another completed outcome lineage",
            ));
        }
        let receipts = self.services.control.operations()?;
        for item in &record.classifications {
            if source(
                parent,
                workspace,
                &record.writer,
                &self.services.key,
                config.runtime.id,
                item.source.comment,
            )? != item.source
            {
                return Err(invalid("correction source changed after final publication"));
            }
            for route in &item.routes {
                verify_route(
                    config,
                    parent,
                    record,
                    item,
                    route,
                    &self.services.key,
                    &receipts,
                )?;
            }
            if item.class != CommentClass::Ambiguous
                && parent
                    .corrections
                    .pending_ids()
                    .contains(&item.source.comment)
            {
                return Err(invalid(
                    "authoritative restoration has not cleared the exact published classification",
                ));
            }
        }
        Ok(CorrectionPublication {
            operation,
            comment: published.comment,
            digest: published.signed.digest()?,
            generation: canonical_digest(1, &record.history)?,
            watermark: record.history.len() as u64,
            remaining_pending: parent.corrections.pending_ids(),
        })
    }
    async fn stage(
        self: &std::sync::Arc<Self>,
        lease: &crate::control::Lease,
        plan: &PublicationPlan,
        client: &kegari_providers::LinearClient,
        writer: &str,
        publication: PublicationSpec,
    ) -> Result<Comment> {
        let PublicationSpec {
            operation,
            issue,
            kind,
            ref body,
        } = publication;
        self.current(plan, lease)?;
        let parent = IssueKey::new(
            &plan.gate.work().linear_endpoint,
            plan.gate.work().linear_workspace,
            plan.gate.work().parent,
        )?;
        let receipt = if let Some(existing) = self
            .services
            .control
            .operations()?
            .into_iter()
            .find(|r| r.operation_id == operation)
        {
            existing
        } else {
            let admission = operation_id(lease.runtime_id, operation, "admission", None, None)?;
            let scope = canonical_digest(
                1,
                &serde_json::json!({"purpose":"correction_publication","gate":plan.gate.digest()?,"operation":operation,"issue":issue,"body":bytes_digest(body.as_bytes())}),
            )?;
            let permit = self.services.control.admit(
                lease,
                admission,
                plan.gate.work_key()?,
                scope,
                || self.current(plan, lease),
            )?;
            let request = crate::control::MutationRequest {
                request_id: operation,
                runtime_id: lease.runtime_id,
                actor: crate::control::Actor {
                    identity: format!("owner:{}", lease.owner_id),
                    principal: None,
                    roles: BTreeSet::new(),
                },
                kind: "correction_publication".into(),
                canonical_targets: vec![parent.identity()?],
                expected: BTreeMap::from([
                    ("correction_parent".into(), parent.identity()?),
                    ("correction_kind".into(), kind.into()),
                ]),
                payload: serde_json::Value::Null,
            };
            let this = self.clone();
            let checked = plan.clone();
            let owner = lease.clone();
            let result = crate::providers::ProviderBroker {
                linear: Some(client.clone()),
                github: None,
            }
            .mutate(
                self.services.control.clone(),
                request,
                Some(permit),
                None,
                crate::providers::ProviderMutation::LinearComment {
                    issue,
                    body: body.clone(),
                },
                move || this.current(&checked, &owner),
            )
            .await;
            self.services.control.retire_local(admission,serde_json::json!({"phase":"correction_publication","result":if result.is_ok(){"provider_acknowledged"}else{"blocked"}}))?;
            result?
        };
        self.observe_stage(client, writer, &parent, &publication, receipt)
            .await
    }
    async fn observe_stage(
        &self,
        client: &kegari_providers::LinearClient,
        writer: &str,
        parent: &IssueKey,
        publication: &PublicationSpec,
        owned: Receipt,
    ) -> Result<Comment> {
        let PublicationSpec {
            operation,
            kind,
            body,
            ..
        } = publication;
        (self.services.control_owner_check)()?;
        let configuration =
            crate::config::config_revision(&self.services.configuration.snapshot()?)?;
        if owned.operation_id != *operation
            || owned.runtime_id != self.services.configuration.snapshot()?.runtime.id
            || owned.kind != "linear_comment"
            || owned.expected.get("correction_parent") != Some(&parent.identity()?)
            || owned.expected.get("correction_kind").map(String::as_str) != Some(*kind)
            || owned.expected.get("provider_body_digest") != Some(&bytes_digest(body.as_bytes()))
            || owned.expected.get("provider_writer").map(String::as_str) != Some(writer)
        {
            return Err(invalid(
                "existing correction operation is bound to another immutable payload",
            ));
        }
        if owned
            .observed_result
            .as_ref()
            .is_some_and(|v| v["effect"] == "not_sent")
        {
            return Err(invalid("correction publication was rejected before send"));
        }
        let target = owned
            .canonical_targets
            .iter()
            .find_map(|target| {
                target.strip_prefix(&format!("linear:{}:{}:", parent.endpoint, parent.workspace))
            })
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(|| invalid("correction receipt target is not canonical"))?;
        let comments = client.comments(target).await?;
        let matches: Vec<_> = comments
            .into_iter()
            .filter(|c| {
                c.body == *body
                    && c.issue_id == target
                    && !c.bot
                    && !c.delegated
                    && c.user_id.map(|id| id.to_string()).as_deref() == Some(writer)
            })
            .collect();
        if matches.len() != 1 {
            return Err(invalid(
                "correction publication is absent or duplicated; it will not be resent",
            ));
        }
        let comment = matches
            .into_iter()
            .next()
            .ok_or_else(|| invalid("correction publication disappeared"))?;
        if owned
            .remote_ids
            .get("comment_id")
            .is_some_and(|id| id != &comment.id.to_string())
        {
            return Err(invalid(
                "correction acknowledgement names another provider record",
            ));
        }
        self.services.control.reconcile_owned_checked(&owned,crate::control::Reconciliation::UniqueVerified{
            remote_ids:BTreeMap::from([("comment_id".into(),comment.id.to_string())]),
            result:serde_json::json!({"correction_publication":kind,"comment_id":comment.id,"body_digest":comment.body_digest}),
            payload_digest:owned.payload_digest.clone(),actor:owned.actor.clone()}, || {
            (self.services.control_owner_check)()?;
            if crate::config::config_revision(&self.services.configuration.snapshot()?)? != configuration { return Err(invalid("configuration changed before correction receipt reconciliation")); }
            Ok(())
        })?;
        // This exact admission owned only the finite metadata publication, never
        // a worker or subprocess. A recovered native effect can retire that
        // metadata hold; active local dispatches remain rejected by ControlHost.
        let admission_id = operation_id(owned.runtime_id, *operation, "admission", None, None)?;
        let admission = self.services.control.operation(admission_id)?;
        if admission.kind != "admission"
            || admission.runtime_id != owned.runtime_id
            || admission.actor != owned.actor
            || admission.owner_generation != owned.owner_generation
            || admission.canonical_targets != vec![parent.identity()?]
            || admission.expected.contains_key("run")
            || owned.expected.get("admission_operation") != Some(&admission_id.to_string())
            || owned.expected.get("admitted_scope") != admission.expected.get("scope")
            || !admission.expected.contains_key("scope")
        {
            return Err(invalid(
                "correction recovery differs from original finite metadata admission",
            ));
        }
        if admission.phase != ReceiptPhase::Reconciled {
            self.services.control.reconcile_owned_checked(&admission,crate::control::Reconciliation::UniqueVerified {
                remote_ids:BTreeMap::from([("publication_operation".into(),operation.to_string())]),
                result:serde_json::json!({"metadata_task":"correction_publication_verified","publication_operation":operation}),
                payload_digest:admission.payload_digest.clone(),actor:admission.actor.clone(),
            },|| (self.services.control_owner_check)())?;
        }
        Ok(comment)
    }
}
fn operation_id(
    runtime: Uuid,
    proposal: Uuid,
    kind: &str,
    source: Option<Uuid>,
    child: Option<Uuid>,
) -> Result<Uuid> {
    let digest = canonical_digest(
        1,
        &serde_json::json!({"namespace":"kegari-v0/correction-publication","runtime":runtime,"proposal":proposal,"kind":kind,"source":source,"child":child}),
    )?;
    let bytes = hex::decode(
        digest
            .rsplit(':')
            .next()
            .ok_or_else(|| invalid("operation digest unavailable"))?,
    )
    .map_err(|_| invalid("operation digest malformed"))?;
    let mut id = [0; 16];
    id.copy_from_slice(&bytes[..16]);
    id[6] = (id[6] & 0x0f) | 0x80;
    id[8] = (id[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(id))
}

#[cfg(test)]
#[path = "../tests/support/correction_pipeline.rs"]
mod tests;
#[cfg(test)]
#[path = "../tests/support/correction_wire.rs"]
pub(crate) mod wire_fixture;

#[cfg(test)]
pub(crate) use tests::{test_pending_transition_fixture, test_transition_fixture};
