//! Provider-derived authorization and correction reconciliation. Names and agent
//! statements never confer mutation authority.
use crate::digest::canonical_digest;
use chrono::{DateTime, Utc};
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueKey {
    pub endpoint: String,
    pub workspace: Uuid,
    pub issue: Uuid,
}
impl IssueKey {
    pub fn new(endpoint: &str, workspace: Uuid, issue: Uuid) -> Result<Self> {
        let mut url =
            url::Url::parse(endpoint).map_err(|_| Error::invalid("invalid Linear endpoint"))?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_some()
        {
            return Err(Error::invalid(
                "Linear endpoint must be a canonical credential-free HTTPS URL",
            ));
        }
        if url.path().is_empty() {
            url.set_path("/");
        }
        if workspace.is_nil() || issue.is_nil() {
            return Err(Error::invalid("canonical issue IDs cannot be nil"));
        }
        Ok(Self {
            endpoint: url.to_string(),
            workspace,
            issue,
        })
    }
    pub fn identity(&self) -> Result<String> {
        canonical_digest(1, self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Responsibility {
    Implementation,
    Testing,
    Documentation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildScope {
    pub schema_version: u32,
    pub kind: String,
    pub responsibility: Responsibility,
    pub write_repository: Option<String>,
    pub write_paths: Vec<String>,
    pub read_repositories: Vec<String>,
    pub test_resources: Vec<String>,
}
impl ChildScope {
    pub fn validate(
        &self,
        allowed_repositories: &BTreeSet<String>,
        allowed_resources: &BTreeSet<String>,
    ) -> Result<()> {
        if self.schema_version != 1 || self.kind != "child" {
            return Err(Error::invalid("invalid child metadata schema/kind"));
        }
        if let Some(repo) = &self.write_repository {
            if !allowed_repositories.contains(repo) || self.write_paths.is_empty() {
                return Err(Error::new(
                    "access_denied",
                    "child write repository or paths lack explicit authorization",
                ));
            }
        } else if !self.write_paths.is_empty() {
            return Err(Error::invalid(
                "read-only child cannot declare source write paths",
            ));
        }
        for path in &self.write_paths {
            validate_relative_boundary(path)?;
        }
        for repo in &self.read_repositories {
            if !allowed_repositories.contains(repo) {
                return Err(Error::new(
                    "access_denied",
                    "child read repository is outside configured scope",
                ));
            }
        }
        for resource in &self.test_resources {
            if !allowed_resources.contains(resource) {
                return Err(Error::new(
                    "access_denied",
                    "child test resource is not configured",
                ));
            }
        }
        if self.write_paths.iter().collect::<BTreeSet<_>>().len() != self.write_paths.len() {
            return Err(Error::invalid("duplicate child write boundary"));
        }
        Ok(())
    }
    pub fn permits_path(&self, path: &str) -> bool {
        if validate_relative_boundary(path).is_err() || path == "." {
            return false;
        }
        self.write_repository.is_some()
            && self.write_paths.iter().any(|allowed| {
                allowed == "."
                    || allowed == path
                    || (allowed.ends_with('/') && path.starts_with(allowed))
            })
    }
}

pub fn validate_relative_boundary(path: &str) -> Result<()> {
    if path == "." {
        return Ok(());
    }
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\0')
        || path.contains('\\')
        || path.contains(['*', '?', '[', ']'])
        || path
            .trim_end_matches('/')
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == ".." || c.eq_ignore_ascii_case(".git"))
    {
        return Err(Error::invalid(
            "write paths must be explicit relative files/directory prefixes; traversal, globs and Git internals are forbidden",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentMetadata {
    pub schema_version: u32,
    pub kind: String,
    pub required_children: Vec<Uuid>,
    pub e2e_child: Uuid,
    pub visual: bool,
    pub production_targets: Vec<String>,
}

/// Preserve prose separately; this parser does not claim prose is semantically prepared.
pub fn metadata<T: serde::de::DeserializeOwned>(description: &str) -> Result<T> {
    let lines: Vec<_> = description.lines().collect();
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim() == "```kegari" {
            index += 1;
            let start = index;
            while index < lines.len() && lines[index].trim() != "```" {
                index += 1;
            }
            if index == lines.len() {
                return Err(Error::new(
                    "incompletely_prepared",
                    "unterminated kegari metadata block",
                ));
            }
            blocks.push(lines[start..index].join("\n"));
        }
        index += 1;
    }
    if blocks.len() != 1 {
        return Err(Error::new(
            "incompletely_prepared",
            "description must contain exactly one kegari JSON metadata block",
        ));
    }
    // Strict duplicate-key detection is handled before typed deserialization.
    let value: StrictJson = serde_json::from_str(&blocks[0]).map_err(|_| {
        Error::new(
            "incompletely_prepared",
            "metadata must be valid JSON without duplicate keys",
        )
    })?;
    serde_json::from_value(value.0).map_err(|_| {
        Error::new(
            "incompletely_prepared",
            "metadata has missing, unknown or incompatible fields",
        )
    })
}

struct StrictJson(serde_json::Value);
impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = StrictJson;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("JSON without duplicate keys")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<StrictJson, A::Error> {
                let mut out = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, StrictJson>()? {
                    if out.insert(key, value.0).is_some() {
                        return Err(serde::de::Error::custom("duplicate key"));
                    }
                }
                Ok(StrictJson(out.into()))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<StrictJson, A::Error> {
                let mut out = Vec::new();
                while let Some(value) = seq.next_element::<StrictJson>()? {
                    out.push(value.0);
                }
                Ok(StrictJson(out.into()))
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                v: bool,
            ) -> std::result::Result<StrictJson, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<StrictJson, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<StrictJson, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> std::result::Result<StrictJson, E> {
                Err(E::custom("floating metadata values unsupported"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<StrictJson, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_none<E: serde::de::Error>(self) -> std::result::Result<StrictJson, E> {
                Ok(StrictJson(serde_json::Value::Null))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<StrictJson, E> {
                Ok(StrictJson(serde_json::Value::Null))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

pub fn required_children(
    metadata: &ParentMetadata,
    enumerated: &BTreeSet<Uuid>,
    previously_authorized: &BTreeSet<Uuid>,
    authorized_removals: &BTreeSet<Uuid>,
) -> Result<BTreeSet<Uuid>> {
    if metadata.schema_version != 1 || metadata.kind != "parent" || metadata.e2e_child.is_nil() {
        return Err(Error::new(
            "incompletely_prepared",
            "parent metadata/E2E identity invalid",
        ));
    }
    if metadata
        .required_children
        .iter()
        .collect::<BTreeSet<_>>()
        .len()
        != metadata.required_children.len()
    {
        return Err(Error::new(
            "incompletely_prepared",
            "duplicate declared child",
        ));
    }
    let mut required: BTreeSet<_> = metadata
        .required_children
        .iter()
        .copied()
        .chain(enumerated.iter().copied())
        .collect();
    required.extend(
        previously_authorized
            .difference(authorized_removals)
            .copied(),
    );
    if required.is_empty()
        || !required.contains(&metadata.e2e_child)
        || required.iter().any(Uuid::is_nil)
    {
        return Err(Error::new(
            "incompletely_prepared",
            "parent requires children and an identifiable required E2E child",
        ));
    }
    Ok(required)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LabelTransition {
    pub event_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub actor: Option<Uuid>,
    pub added: BTreeSet<Uuid>,
    pub removed: BTreeSet<Uuid>,
    pub unambiguous: bool,
}

pub fn latest_authorization(
    label: Uuid,
    present: &BTreeSet<Uuid>,
    history: &[LabelTransition],
    history_complete: bool,
    authorizers: &BTreeSet<Uuid>,
) -> Result<LabelTransition> {
    let unavailable = || {
        Error::new(
            "authorization_provenance_unavailable",
            "current label presence needs a complete, unambiguous latest permitted authorizer event",
        )
    };
    if !present.contains(&label) {
        return Err(Error::new(
            "unauthorized",
            "parent authorization label is absent",
        ));
    }
    if !history_complete {
        return Err(unavailable());
    }
    let mut relevant: Vec<_> = history
        .iter()
        .filter(|event| event.added.contains(&label) || event.removed.contains(&label))
        .collect();
    relevant.sort_by_key(|event| event.occurred_at);
    let latest = relevant.last().ok_or_else(unavailable)?;
    if relevant.len() > 1 && relevant[relevant.len() - 2].occurred_at == latest.occurred_at {
        return Err(unavailable());
    }
    if !latest.unambiguous
        || latest.removed.contains(&label)
        || !latest.added.contains(&label)
        || latest
            .actor
            .is_none_or(|actor| !authorizers.contains(&actor))
    {
        return Err(unavailable());
    }
    Ok((*latest).clone())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeSnapshot {
    pub child_scopes: BTreeMap<Uuid, String>,
    pub child_definitions: BTreeMap<Uuid, String>,
    pub parent_definition: String,
    pub production_targets: BTreeSet<String>,
}

pub fn require_scope_authorization(
    previous: &ScopeSnapshot,
    current: &ScopeSnapshot,
    event: &LabelTransition,
    material_change_at: DateTime<Utc>,
    permitted_in_scope_correction: bool,
) -> Result<()> {
    for snapshot in [previous, current] {
        if snapshot.child_scopes.is_empty()
            || snapshot.child_scopes.keys().collect::<BTreeSet<_>>()
                != snapshot.child_definitions.keys().collect::<BTreeSet<_>>()
        {
            return Err(Error::new(
                "incompletely_prepared",
                "each required child needs separate definition and write-scope digests",
            ));
        }
    }
    if previous == current {
        return Ok(());
    }
    let scope_changed = previous.child_scopes != current.child_scopes
        || previous.production_targets != current.production_targets;
    if event.occurred_at <= material_change_at && (scope_changed || !permitted_in_scope_correction)
    {
        return Err(Error::new(
            "renewed_authorization_required",
            "changed child scope or parent definition lacks later authorization",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentClass {
    Informational,
    Corrective,
    Ambiguous,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommentRevision {
    pub id: Uuid,
    pub author: Option<Uuid>,
    pub body_digest: String,
    pub edited_at: Option<DateTime<Utc>>,
    pub observed_revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Corrections {
    revisions: BTreeMap<Uuid, CommentRevision>,
    classes: BTreeMap<Uuid, CommentClass>,
    pending: BTreeSet<Uuid>,
    corrective_history: Vec<String>,
    #[serde(default)]
    known_corrective_ids: BTreeSet<Uuid>,
    pub watermark: u64,
}
/// Protected invalidation references contain no comment text or clearance.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectionInvalidations {
    pub corrective_revisions: BTreeSet<String>,
    pub corrective_comments: BTreeSet<Uuid>,
    pub pending: BTreeMap<Uuid, CommentRevision>,
}
impl CommentRevision {
    pub fn identity_digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &serde_json::json!({"id":self.id,"author":self.author,
            "body_digest":self.body_digest,"edited_at":self.edited_at}),
        )
    }
}
impl Corrections {
    pub fn invalidations(&self) -> CorrectionInvalidations {
        CorrectionInvalidations {
            corrective_revisions: self.corrective_history.iter().cloned().collect(),
            corrective_comments: self.known_corrective_ids.clone(),
            pending: self
                .pending
                .iter()
                .filter_map(|id| {
                    self.revisions.get(id).cloned().map(|mut revision| {
                        revision.observed_revision = 0;
                        (*id, revision)
                    })
                })
                .collect(),
        }
    }
    /// Restoration can retain invalidation and pending work, never completion.
    pub fn from_invalidations(references: CorrectionInvalidations) -> Result<Self> {
        if references.corrective_revisions.len() > 16_384
            || references.corrective_comments.len() > 16_384
            || references.pending.len() > 16_384
            || references.corrective_comments.iter().any(Uuid::is_nil)
        {
            return Err(Error::new(
                "bounded_input",
                "correction invalidation references exceed finite identity bounds",
            ));
        }
        for digest in &references.corrective_revisions {
            crate::evidence::validate_digest(digest)?;
        }
        if references
            .pending
            .iter()
            .any(|(id, r)| id.is_nil() || *id != r.id || r.observed_revision != 0)
        {
            return Err(Error::new(
                "record_corrupt",
                "pending correction identity references are malformed",
            ));
        }
        Ok(Self {
            watermark: references.corrective_revisions.len() as u64,
            corrective_history: references.corrective_revisions.into_iter().collect(),
            known_corrective_ids: references.corrective_comments,
            pending: references.pending.keys().copied().collect(),
            revisions: references.pending,
            classes: BTreeMap::new(),
        })
    }
    /// Stable phase identity from authoritative pending native revisions; local
    /// polling counters never create a new classification budget.
    pub fn pending_revision_digest(&self) -> Result<String> {
        let revisions = self
            .pending
            .iter()
            .map(|id| {
                self.revisions
                    .get(id)
                    .ok_or_else(|| {
                        Error::new("record_corrupt", "pending comment revision is absent")
                    })
                    .and_then(CommentRevision::identity_digest)
            })
            .collect::<Result<Vec<_>>>()?;
        canonical_digest(1, &revisions)
    }
    pub fn revision(&self, id: Uuid) -> Option<&CommentRevision> {
        self.revisions.get(&id)
    }
    pub fn observe(
        &mut self,
        revision: CommentRevision,
        correctors: &BTreeSet<Uuid>,
        host_evidence_comments: &BTreeSet<Uuid>,
        verified_cli_corrector: Option<Uuid>,
    ) -> Result<bool> {
        if host_evidence_comments.contains(&revision.id) && verified_cli_corrector.is_none() {
            return Ok(false);
        }
        let actor = verified_cli_corrector.or(revision.author);
        if actor.is_none_or(|a| !correctors.contains(&a)) {
            return Ok(false);
        }
        if self.revisions.get(&revision.id).is_some_and(|old| {
            old.body_digest == revision.body_digest && old.edited_at == revision.edited_at
        }) {
            return Ok(false);
        }
        if self
            .revisions
            .get(&revision.id)
            .is_some_and(|old| old.observed_revision >= revision.observed_revision)
        {
            return Err(Error::new(
                "stale_result",
                "comment observation is older than retained revision",
            ));
        }
        let id = revision.id;
        self.pending.insert(id);
        self.revisions.insert(id, revision);
        // An edited known correction is ineligible immediately, even if classification
        // or steering is waiting for RUNNING admission.
        if self.known_corrective_ids.contains(&id) || verified_cli_corrector.is_some() {
            self.record_corrective_revision(id)?;
        }
        if verified_cli_corrector.is_some() {
            self.classes.insert(id, CommentClass::Corrective);
        }
        Ok(true)
    }
    fn record_corrective_revision(&mut self, id: Uuid) -> Result<()> {
        let digest = self
            .revisions
            .get(&id)
            .ok_or_else(|| Error::invalid("unknown correction"))?
            .identity_digest()?;
        self.known_corrective_ids.insert(id);
        if !self.corrective_history.contains(&digest) {
            if self.corrective_history.len() >= 16_384 {
                return Err(Error::new(
                    "bounded_input",
                    "correction history exceeds finite revision bound",
                ));
            }
            self.corrective_history.push(digest);
            self.corrective_history.sort();
            self.watermark = self.corrective_history.len() as u64;
        }
        Ok(())
    }
    pub fn classify(&mut self, id: Uuid, expected_digest: &str, class: CommentClass) -> Result<()> {
        if self
            .revisions
            .get(&id)
            .is_none_or(|r| r.body_digest != expected_digest)
        {
            return Err(Error::new(
                "stale_result",
                "classification does not cover the current comment revision",
            ));
        }
        self.classes.insert(id, class);
        if class == CommentClass::Ambiguous {
            self.pending.insert(id);
        } else {
            self.pending.remove(&id);
        }
        if class == CommentClass::Corrective {
            self.record_corrective_revision(id)?;
        }
        Ok(())
    }
    pub fn generation(&self) -> Result<String> {
        canonical_digest(1, &self.corrective_history.iter().collect::<BTreeSet<_>>())
    }
    pub fn gates_current(&self, worker_watermark: u64) -> bool {
        self.pending.is_empty() && worker_watermark == self.watermark
    }
    pub fn pending_ids(&self) -> Vec<Uuid> {
        self.pending.iter().copied().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteeringResult {
    Delivered { turn_id: String, watermark: u64 },
    NoActiveTurn,
    StaleTurn,
    Failed,
}
#[derive(Debug, Clone, Default)]
pub struct CorrectionDelivery {
    pub required: u64,
    pub consumed: u64,
    pub current_turn: Option<String>,
    pub queued: bool,
}
impl CorrectionDelivery {
    pub fn observed(&mut self, watermark: u64) {
        self.required = self.required.max(watermark);
        self.queued = self.required > self.consumed;
    }
    pub fn steered(&mut self, result: SteeringResult) {
        if let SteeringResult::Delivered { turn_id, watermark } = result {
            if self.current_turn.as_ref() == Some(&turn_id) && watermark == self.required {
                // Delivery is not processing. Only a verified outcome consumes watermark.
                self.queued = false;
            }
        } else {
            self.queued = self.required > self.consumed;
        }
    }
    pub fn completed(&mut self, turn_id: &str, watermark: u64) -> Result<()> {
        if self.current_turn.as_deref() != Some(turn_id) {
            return Err(Error::new(
                "stale_result",
                "completion belongs to a different or already ended turn",
            ));
        }
        self.current_turn = None;
        if watermark != self.required {
            self.queued = true;
            return Err(Error::new(
                "stale_result",
                "completed session did not process current correction",
            ));
        }
        self.consumed = watermark;
        self.queued = false;
        Ok(())
    }
}
