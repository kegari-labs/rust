//! Revision-bound gates and authenticated delivery evidence.
//!
//! Provider observations in this module are host inputs from complete adapter
//! reads. They must never be accepted as worker tool arguments. A worker's prose
//! or submitted result reference cannot manufacture a `Verified*` value.

use crate::digest::{bytes_digest, canonical_bytes, canonical_digest};
use crate::error::{Error, Result};
use crate::git::{CandidateManifest, DeltaManifest, RepositoryId, Revision, validate_oid};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const REVIEW_CONTEXT: &str = "kegari/codex-review";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Generation {
    pub authorization: String,
    pub definition: String,
    pub correction: String,
    pub scope: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkIdentity {
    pub linear_endpoint: String,
    pub linear_workspace: Uuid,
    pub parent: Uuid,
    pub child: Option<Uuid>,
    pub generation: Generation,
}

impl WorkIdentity {
    pub fn validate(&self) -> Result<()> {
        let endpoint = url::Url::parse(&self.linear_endpoint)
            .map_err(|_| Error::invalid("work identity has invalid Linear endpoint"))?;
        if endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || endpoint.as_str() != self.linear_endpoint
            || self.linear_workspace.is_nil()
            || self.parent.is_nil()
            || self.child.is_some_and(|id| id.is_nil())
        {
            return Err(Error::invalid(
                "work identity must use canonical provider IDs and endpoint",
            ));
        }
        for digest in [
            &self.generation.authorization,
            &self.generation.definition,
            &self.generation.correction,
            &self.generation.scope,
        ] {
            validate_digest(digest)?;
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String> {
        self.validate()?;
        canonical_digest(1, self)
    }
}

pub fn validate_digest(value: &str) -> Result<()> {
    let suffix = value
        .strip_prefix("sha256:v1:")
        .or_else(|| value.strip_prefix("sha256:"));
    if !suffix.is_some_and(|s| {
        s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err(Error::invalid(
            "evidence reference requires an explicit SHA-256 digest/schema",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionBinding {
    pub repository: RepositoryId,
    pub head: String,
    pub base: String,
    pub tested_commit: String,
    pub tested_tree: String,
}

impl RevisionBinding {
    pub fn validate(&self) -> Result<()> {
        self.repository.validate()?;
        for oid in [
            &self.head,
            &self.base,
            &self.tested_commit,
            &self.tested_tree,
        ] {
            validate_oid(oid)?;
        }
        Ok(())
    }
}

/// Key material is borrowed from the host's Keychain-backed secret service; it
/// has no Debug/Serialize implementation and is never written by this module.
pub struct AttestationKey {
    id: String,
    bytes: Vec<u8>,
}

impl AttestationKey {
    pub fn new(id: String, bytes: Vec<u8>) -> Result<Self> {
        if id.trim().is_empty() || bytes.len() < 32 {
            return Err(Error::invalid(
                "attestation key needs a stable ID and at least 256 bits",
            ));
        }
        Ok(Self { id, bytes })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn sign<T: Serialize + Clone>(&self, purpose: &str, payload: &T) -> Result<Signed<T>> {
        if purpose.is_empty() {
            return Err(Error::invalid("signed evidence requires a domain purpose"));
        }
        let mut record = Signed {
            schema_version: 1,
            key_id: self.id.clone(),
            purpose: purpose.into(),
            payload: payload.clone(),
            signature: String::new(),
        };
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.bytes)
            .map_err(|_| Error::invalid("invalid attestation key"))?;
        mac.update(&record.signing_bytes()?);
        record.signature = hex::encode(mac.finalize().into_bytes());
        Ok(record)
    }

    pub fn verify<T: Serialize>(&self, purpose: &str, signed: &Signed<T>) -> Result<()> {
        if signed.schema_version != 1 || signed.key_id != self.id || signed.purpose != purpose {
            return Err(Error::new(
                "invalid_attestation",
                "attestation schema/key/purpose does not match",
            ));
        }
        let signature = hex::decode(&signed.signature)
            .map_err(|_| Error::new("invalid_attestation", "invalid authentication tag"))?;
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.bytes)
            .map_err(|_| Error::invalid("invalid attestation key"))?;
        mac.update(&signed.signing_bytes()?);
        mac.verify_slice(&signature).map_err(|_| {
            Error::new(
                "invalid_attestation",
                "authentication tag verification failed",
            )
        })
    }
}

impl Drop for AttestationKey {
    fn drop(&mut self) {
        // Avoid retaining an unnecessary copy after the host operation. Keychain
        // owns persistence; no security claim depends on optimizer behavior here.
        self.bytes.fill(0);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signed<T> {
    pub schema_version: u32,
    pub key_id: String,
    pub purpose: String,
    pub payload: T,
    pub signature: String,
}

impl<T: Serialize> Signed<T> {
    fn signing_bytes(&self) -> Result<Vec<u8>> {
        canonical_bytes(&serde_json::json!({
            "schema_version": self.schema_version,
            "key_id": self.key_id,
            "purpose": self.purpose,
            "payload": self.payload,
        }))
    }

    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publication {
    pub provider: String,
    pub record_id: String,
    pub writer_account_id: String,
    pub payload_digest: String,
    pub observed_at: DateTime<Utc>,
    pub url: String,
}

impl Publication {
    pub fn verify<T: Serialize>(
        &self,
        payload: &T,
        provider: &str,
        expected_writer: &str,
    ) -> Result<()> {
        if self.provider != provider
            || self.record_id.trim().is_empty()
            || self.writer_account_id != expected_writer
            || expected_writer.is_empty()
            || self.payload_digest != canonical_digest(1, payload)?
        {
            return Err(Error::new(
                "evidence_unpublished",
                "required authoritative publication is missing or does not match the observed writer/payload",
            ));
        }
        let url = url::Url::parse(&self.url)
            .map_err(|_| Error::new("evidence_unpublished", "invalid evidence publication link"))?;
        if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
            return Err(Error::new(
                "evidence_unpublished",
                "evidence publication requires a safe HTTPS link",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Findings,
    Blocked,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewKind {
    Agent {
        session_id: String,
        instruction_manifest: String,
    },
    Derived {
        source_attestations: Vec<String>,
        selected_patches: Vec<String>,
        assembly_proof: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewAttestation {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub owner_generation: u64,
    pub operation_id: Uuid,
    pub work: WorkIdentity,
    pub revision: RevisionBinding,
    pub pull_request: Option<u64>,
    pub delta_digest: String,
    pub verdict: Verdict,
    pub unresolved_findings: Vec<String>,
    pub supporting_links: Vec<String>,
    pub review: ReviewKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewStatus {
    pub repository: RepositoryId,
    pub head: String,
    pub context: String,
    pub state: String,
    pub writer_account_id: String,
    pub attestation_record_id: String,
}

#[derive(Clone, Debug)]
pub struct VerifiedReview {
    attestation: ReviewAttestation,
    digest: String,
}

impl VerifiedReview {
    pub fn attestation(&self) -> &ReviewAttestation {
        &self.attestation
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// An exact-head commit status alone cannot pass this gate. Its corresponding
/// signed PR (or no-change Linear) record must be observed under the bound user.
pub struct ReviewRequirements<'a> {
    pub work: &'a WorkIdentity,
    pub revision: &'a RevisionBinding,
    pub delta_digest: &'a str,
    pub writer: &'a str,
}

pub fn verify_review(
    signed: &Signed<ReviewAttestation>,
    publication: &Publication,
    status: Option<&ReviewStatus>,
    key: &AttestationKey,
    requirements: ReviewRequirements<'_>,
) -> Result<VerifiedReview> {
    let ReviewRequirements {
        work: expected_work,
        revision: expected_revision,
        delta_digest: expected_delta,
        writer: expected_writer,
    } = requirements;
    key.verify("codex_review", signed)?;
    let a = &signed.payload;
    a.work.validate()?;
    a.revision.validate()?;
    validate_digest(&a.delta_digest)?;
    if a.schema_version != 1
        || a.runtime_id.is_nil()
        || a.operation_id.is_nil()
        || a.owner_generation == 0
        || &a.work != expected_work
        || &a.revision != expected_revision
        || a.delta_digest != expected_delta
        || a.verdict != Verdict::Pass
        || !a.unresolved_findings.is_empty()
        || a.supporting_links.is_empty()
    {
        return Err(Error::new(
            "review_gate",
            "Codex review is stale, incomplete or has unresolved findings",
        ));
    }
    match &a.review {
        ReviewKind::Agent {
            session_id,
            instruction_manifest,
        } => {
            if session_id.is_empty() {
                return Err(Error::new(
                    "review_gate",
                    "review session identity is missing",
                ));
            }
            validate_digest(instruction_manifest)?;
        }
        ReviewKind::Derived {
            source_attestations,
            selected_patches,
            assembly_proof,
        } => {
            if source_attestations.is_empty()
                || selected_patches.is_empty()
                || a.work.child.is_some()
            {
                return Err(Error::new(
                    "review_gate",
                    "derived review requires complete child sources and a parent candidate",
                ));
            }
            for digest in source_attestations
                .iter()
                .chain(selected_patches.iter())
                .chain(std::iter::once(assembly_proof))
            {
                validate_digest(digest)?;
            }
        }
    }
    let provider = if a.pull_request.is_some() {
        "github"
    } else {
        "linear"
    };
    publication.verify(signed, provider, expected_writer)?;
    if a.pull_request.is_some() {
        let s = status
            .ok_or_else(|| Error::new("review_gate", "required review commit status is missing"))?;
        if s.repository != a.revision.repository
            || s.head != a.revision.head
            || s.context != REVIEW_CONTEXT
            || s.state != "success"
            || s.writer_account_id != expected_writer
            || s.attestation_record_id != publication.record_id
        {
            return Err(Error::new(
                "review_gate",
                "status does not match the authenticated exact-head review",
            ));
        }
    } else if status.is_some() || a.work.child.is_none() {
        return Err(Error::new(
            "review_gate",
            "no-change review must be published to its child without a fictional PR status",
        ));
    }
    Ok(VerifiedReview {
        attestation: a.clone(),
        digest: signed.digest()?,
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Local,
    HostedCi,
    Integrated,
    ProductionCi,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Conclusion {
    Success,
    Failure,
    Skipped,
    Neutral,
    Cancelled,
    Pending,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runner {
    LocalSandbox,
    GitHubHosted,
    SelfHosted,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub reference: String,
    pub digest: String,
}

impl ArtifactReference {
    fn validate(&self) -> Result<()> {
        if self.reference.trim().is_empty() {
            return Err(Error::new(
                "evidence_missing",
                "artifact reference is missing",
            ));
        }
        validate_digest(&self.digest)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckRequirement {
    pub id: String,
    pub kind: CheckKind,
    pub workflow_id: Option<u64>,
    pub command: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckRun {
    pub id: String,
    pub kind: CheckKind,
    pub operation_id: Uuid,
    pub work_digest: String,
    pub revision: RevisionBinding,
    pub workflow_id: Option<u64>,
    pub run_id: Option<u64>,
    pub attempt: u32,
    pub runner: Runner,
    pub command: Vec<String>,
    pub environment_identity: String,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub conclusion: Conclusion,
    pub artifacts: Vec<ArtifactReference>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckSnapshot {
    pub pagination_complete: bool,
    pub observed_at: DateTime<Utc>,
    pub runs: Vec<CheckRun>,
}

/// Checks every configured suite member; failed/skipped/missing jobs and
/// self-hosted jobs cannot be concealed by one successful status or an agent's
/// choice of a smaller suite. All attempts remain in the snapshot as evidence.
pub fn verify_checks(
    requirements: &[CheckRequirement],
    work: &WorkIdentity,
    revision: &RevisionBinding,
    snapshot: &CheckSnapshot,
) -> Result<Vec<CheckRun>> {
    if !snapshot.pagination_complete {
        return Err(Error::new(
            "provider_pagination",
            "check snapshot is incomplete",
        ));
    }
    if requirements.is_empty() {
        return Err(Error::invalid(
            "a complete explicit validation suite is required",
        ));
    }
    revision.validate()?;
    let work_digest = work.digest()?;
    let mut seen = BTreeSet::new();
    let mut verified = Vec::new();
    for requirement in requirements {
        if requirement.id.is_empty() || !seen.insert((&requirement.id, &requirement.kind)) {
            return Err(Error::invalid(
                "validation suite has missing/duplicate check identities",
            ));
        }
        let mut matching: Vec<&CheckRun> = snapshot
            .runs
            .iter()
            .filter(|r| {
                r.id == requirement.id && r.kind == requirement.kind && r.revision == *revision
            })
            .collect();
        matching.sort_by_key(|r| (r.started_at, r.run_id, r.attempt));
        let run = matching.last().copied().ok_or_else(|| {
            Error::new(
                "validation_gate",
                format!(
                    "required check {} did not run against the current revision",
                    requirement.id
                ),
            )
        })?;
        if matching.len() > 1 {
            let previous = matching[matching.len() - 2];
            if (previous.started_at, previous.run_id, previous.attempt)
                == (run.started_at, run.run_id, run.attempt)
                && previous != run
            {
                return Err(Error::new(
                    "provider_response",
                    "conflicting observations for the same check attempt",
                ));
            }
        }
        if run.operation_id.is_nil()
            || run.work_digest != work_digest
            || run.attempt == 0
            || run.conclusion != Conclusion::Success
            || run.environment_identity.is_empty()
            || run.started_at.is_none()
            || run.completed_at.is_none()
            || run.completed_at < run.started_at
            || run
                .completed_at
                .is_some_and(|end| end > snapshot.observed_at)
            || run.artifacts.is_empty()
        {
            return Err(Error::new(
                "validation_gate",
                format!(
                    "required check {} has no completed successful run with current provenance",
                    requirement.id
                ),
            ));
        }
        match requirement.kind {
            CheckKind::Local | CheckKind::Integrated => {
                if requirement.command.is_empty()
                    || run.command != requirement.command
                    || run.runner != Runner::LocalSandbox
                    || run.workflow_id.is_some()
                    || run.run_id.is_some()
                {
                    return Err(Error::new(
                        "validation_gate",
                        "local/integrated command or execution environment does not match its configured contract",
                    ));
                }
            }
            CheckKind::HostedCi | CheckKind::ProductionCi => {
                if requirement.workflow_id.is_none()
                    || requirement.workflow_id == Some(0)
                    || run.workflow_id != requirement.workflow_id
                    || run.run_id.is_none()
                    || run.run_id == Some(0)
                    || run.runner != Runner::GitHubHosted
                {
                    return Err(Error::new(
                        "validation_gate",
                        "required CI did not run in the configured GitHub-hosted workflow",
                    ));
                }
            }
        }
        for artifact in &run.artifacts {
            artifact.validate()?;
        }
        verified.push(run.clone());
    }
    Ok(verified)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryProtection {
    pub repository: RepositoryId,
    pub branch: String,
    pub observed_base: String,
    pub required_contexts: BTreeSet<String>,
    pub strict_current_base: bool,
    pub squash_allowed: bool,
    pub extra_approvals_satisfied: bool,
    pub code_owner_reviews_satisfied: bool,
    pub admin_bypass: bool,
    pub observed_at: DateTime<Utc>,
}

impl RepositoryProtection {
    pub fn verify(
        &self,
        revision: &RevisionBinding,
        branch: &str,
        required_contexts: &BTreeSet<String>,
    ) -> Result<()> {
        if self.repository != revision.repository
            || self.branch != branch
            || self.observed_base != revision.base
            || !self.strict_current_base
            || !self.squash_allowed
            || self.admin_bypass
            || !self.extra_approvals_satisfied
            || !self.code_owner_reviews_satisfied
            || !self.required_contexts.contains(REVIEW_CONTEXT)
            || !required_contexts.is_subset(&self.required_contexts)
        {
            return Err(Error::new(
                "repository_policy",
                "current-base checks, squash policy or required approvals do not support this merge",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChildOutcome {
    Changed {
        pull_request: u64,
        delta: Box<DeltaManifest>,
    },
    NoChange {
        reason: String,
        change_gates_inapplicable: BTreeSet<String>,
        exact_sha_validation: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildEvidence {
    pub schema_version: u32,
    pub work: WorkIdentity,
    pub revision: RevisionBinding,
    pub fulfilled_criteria: BTreeSet<String>,
    pub outcome: ChildOutcome,
    pub checks: CheckSnapshot,
    pub review_digest: String,
    pub results_for_dependents: Vec<ArtifactReference>,
}

pub struct ChildRequirements<'a> {
    pub work: &'a WorkIdentity,
    pub criteria: &'a BTreeSet<String>,
    pub revision: &'a RevisionBinding,
    pub checks: &'a [CheckRequirement],
    pub required_contexts: &'a BTreeSet<String>,
}

pub fn no_change_digest(
    work: &WorkIdentity,
    revision: &RevisionBinding,
    criteria: &BTreeSet<String>,
    reason: &str,
) -> Result<String> {
    canonical_digest(
        1,
        &serde_json::json!({ "kind": "no_change", "work": work, "revision": revision, "fulfilled_criteria": criteria, "reason": reason }),
    )
}

#[derive(Clone, Debug)]
pub struct VerifiedChild {
    evidence: ChildEvidence,
    digest: String,
}

impl VerifiedChild {
    pub fn evidence(&self) -> &ChildEvidence {
        &self.evidence
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

pub fn verify_child(
    evidence: &ChildEvidence,
    requirements: ChildRequirements<'_>,
    review: &VerifiedReview,
    protection: Option<&RepositoryProtection>,
    publication: &Publication,
    linear_writer: &str,
) -> Result<VerifiedChild> {
    evidence.work.validate()?;
    evidence.revision.validate()?;
    if evidence.schema_version != 1
        || evidence.work.child.is_none()
        || &evidence.work != requirements.work
        || &evidence.revision != requirements.revision
        || requirements.criteria.is_empty()
        || evidence.fulfilled_criteria != *requirements.criteria
        || evidence.review_digest != review.digest
        || review.attestation.work != evidence.work
        || review.attestation.revision != evidence.revision
        || evidence.results_for_dependents.is_empty()
    {
        return Err(Error::new(
            "child_incomplete",
            "child scope, acceptance criteria, revision, review or dependent results are incomplete",
        ));
    }
    for reference in &evidence.results_for_dependents {
        reference.validate()?;
    }
    if !requirements
        .checks
        .iter()
        .any(|c| c.kind == CheckKind::Local)
        || !requirements
            .checks
            .iter()
            .any(|c| c.kind == CheckKind::HostedCi)
    {
        return Err(Error::invalid(
            "child completion requires separate local and GitHub-hosted validation suites",
        ));
    }
    verify_checks(
        requirements.checks,
        &evidence.work,
        &evidence.revision,
        &evidence.checks,
    )?;
    match &evidence.outcome {
        ChildOutcome::Changed {
            pull_request,
            delta,
        } => {
            if *pull_request == 0
                || review.attestation.pull_request != Some(*pull_request)
                || delta.repository != evidence.revision.repository
                || delta.parent_id != evidence.work.parent
                || Some(delta.child_id) != evidence.work.child
                || delta.work_generation != evidence.work.generation.definition
                || delta.correction_generation != evidence.work.generation.correction
                || delta.validated_base.commit != evidence.revision.base
                || delta.validated_head.commit != evidence.revision.head
                || delta.validated_head.tree != evidence.revision.tested_tree
                || delta.staging_parent != delta.validated_base
                || delta.staging_commit.tree != delta.validated_head.tree
                || delta.digest()? != review.attestation.delta_digest
                || delta.paths.is_empty()
            {
                return Err(Error::new(
                    "staging_gate",
                    "observed squash/delta does not match this child's reviewed candidate",
                ));
            }
            protection
                .ok_or_else(|| {
                    Error::new(
                        "repository_policy",
                        "repository protection observation is missing",
                    )
                })?
                .verify(
                    &evidence.revision,
                    "staging",
                    requirements.required_contexts,
                )?;
        }
        ChildOutcome::NoChange {
            reason,
            change_gates_inapplicable,
            exact_sha_validation,
        } => {
            let required = BTreeSet::from([
                "branch".into(),
                "pull_request".into(),
                "staging_merge".into(),
            ]);
            if reason.trim().is_empty()
                || exact_sha_validation.trim().is_empty()
                || *change_gates_inapplicable != required
                || review.attestation.pull_request.is_some()
                || review.attestation.delta_digest
                    != no_change_digest(
                        &evidence.work,
                        &evidence.revision,
                        &evidence.fulfilled_criteria,
                        reason,
                    )?
                || !matches!(review.attestation.review, ReviewKind::Agent { .. })
            {
                return Err(Error::new(
                    "no_change_gate",
                    "no-change completion needs its exact-SHA validation, review and explicit inapplicable gates",
                ));
            }
        }
    }
    publication.verify(evidence, "linear", linear_writer)?;
    Ok(VerifiedChild {
        evidence: evidence.clone(),
        digest: canonical_digest(1, evidence)?,
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegratedRun {
    pub id: String,
    pub operation_id: Uuid,
    pub e2e_child: Uuid,
    pub candidate_vector_digest: String,
    pub command: Vec<String>,
    pub test_resources: BTreeSet<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub conclusion: Conclusion,
    pub after_staging_integration: bool,
    pub artifacts: Vec<ArtifactReference>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoEvidence {
    pub asset_digest: String,
    pub media_type: String,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    pub candidate_vector_digest: String,
    pub validation_operation: Uuid,
    pub linear_asset_id: String,
    pub parent_association_id: String,
    pub associated_parent: Uuid,
    pub playback_verified: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionBaseline {
    pub repository: RepositoryId,
    pub production: Revision,
    pub deployed_revision: Option<String>,
    pub deployed_artifact: Option<String>,
    pub deployment_free: bool,
    /// A known partially delivered predecessor, never an unknown action. This
    /// reference permits declared recovery divergence, not a success claim.
    pub successor_recovery_operation: Option<Uuid>,
}

impl ProductionBaseline {
    pub fn validate(&self) -> Result<()> {
        self.repository.validate()?;
        self.production.validate()?;
        if self.deployment_free {
            if self.deployed_revision.is_some() || self.deployed_artifact.is_some() {
                return Err(Error::invalid(
                    "deployment-free baseline must not invent deployment activity",
                ));
            }
        } else {
            let deployed = self.deployed_revision.as_deref().ok_or_else(|| {
                Error::new("production_baseline", "deployed source revision is unknown")
            })?;
            validate_oid(deployed)?;
            let artifact = self
                .deployed_artifact
                .as_deref()
                .ok_or_else(|| Error::new("production_baseline", "deployed artifact is unknown"))?;
            validate_digest(artifact)?;
            if deployed != self.production.commit && self.successor_recovery_operation.is_none() {
                return Err(Error::new(
                    "production_baseline",
                    "main/deployment divergence needs explicit reconciled successor recovery",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentEvidence {
    pub schema_version: u32,
    pub work: WorkIdentity,
    pub required_children: BTreeSet<Uuid>,
    pub e2e_child: Uuid,
    pub child_results: BTreeMap<Uuid, String>,
    pub executive_summary: String,
    pub touched_components: Vec<String>,
    pub fulfilled_criteria: BTreeSet<String>,
    pub caveats: Vec<String>,
    pub go: bool,
    pub baselines: Vec<ProductionBaseline>,
    pub candidates: Vec<CandidateManifest>,
    pub dependency_snapshot_digest: String,
    pub production_targets: BTreeSet<String>,
    pub integrated_results: Vec<IntegratedRun>,
    pub visual: bool,
    pub video: Option<VideoEvidence>,
    pub publication_operation: Uuid,
    pub created_at: DateTime<Utc>,
    pub supersedes: Option<String>,
}

impl ParentEvidence {
    pub fn token(&self) -> Result<String> {
        canonical_digest(1, self)
    }
    pub fn candidate_vector_digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &serde_json::json!({ "baselines": self.baselines, "candidates": self.candidates }),
        )
    }
}

pub struct ParentRequirements<'a> {
    pub work: &'a WorkIdentity,
    pub required_children: &'a BTreeSet<Uuid>,
    pub e2e_child: Uuid,
    pub criteria: &'a BTreeSet<String>,
    pub integrated_commands: &'a BTreeMap<String, Vec<String>>,
    pub test_resources: &'a BTreeSet<String>,
    pub production_targets: &'a BTreeSet<String>,
    pub visual: bool,
}

#[derive(Clone, Debug)]
pub struct VerifiedParent {
    manifest: ParentEvidence,
    token: String,
    child_satisfaction_digest: String,
}

impl VerifiedParent {
    pub fn manifest(&self) -> &ParentEvidence {
        &self.manifest
    }
    pub fn token(&self) -> &str {
        &self.token
    }
}

/// This predicate is deliberately later than candidate construction. The E2E
/// child and every testing/documentation/no-change responsibility must be here.
pub fn verify_parent(
    evidence: &ParentEvidence,
    requirements: ParentRequirements<'_>,
    children: &[VerifiedChild],
    publication: &Publication,
    linear_writer: &str,
) -> Result<VerifiedParent> {
    evidence.work.validate()?;
    if evidence.schema_version != 1
        || evidence.work.child.is_some()
        || &evidence.work != requirements.work
        || requirements.required_children.is_empty()
        || &evidence.required_children != requirements.required_children
        || !requirements
            .required_children
            .contains(&requirements.e2e_child)
        || evidence.e2e_child != requirements.e2e_child
        || evidence.fulfilled_criteria != *requirements.criteria
        || requirements.criteria.is_empty()
        || evidence.production_targets != *requirements.production_targets
        || evidence.visual != requirements.visual
        || evidence.executive_summary.trim().is_empty()
        || evidence.touched_components.is_empty()
        || !evidence.go
        || evidence.publication_operation.is_nil()
    {
        return Err(Error::new(
            "parent_incomplete",
            "parent scope, required responsibilities or current result summary is incomplete",
        ));
    }
    let mut actual = BTreeMap::new();
    for child in children {
        let id = child.evidence.work.child.ok_or_else(|| {
            Error::new(
                "parent_incomplete",
                "parent cannot substitute for a required child",
            )
        })?;
        if child.evidence.work.parent != evidence.work.parent
            || child.evidence.work.linear_workspace != evidence.work.linear_workspace
            || child.evidence.work.linear_endpoint != evidence.work.linear_endpoint
            || actual.insert(id, child.digest.clone()).is_some()
        {
            return Err(Error::new(
                "parent_incomplete",
                "duplicate or cross-parent child evidence",
            ));
        }
    }
    if actual.keys().copied().collect::<BTreeSet<_>>() != *requirements.required_children
        || actual != evidence.child_results
    {
        return Err(Error::new(
            "parent_incomplete",
            "every required child must have its current verified result, including no-change and E2E work",
        ));
    }
    if evidence.baselines.is_empty() {
        return Err(Error::new(
            "production_baseline",
            "parent has no exact production baseline vector",
        ));
    }
    let mut repositories = BTreeSet::new();
    for baseline in &evidence.baselines {
        baseline.validate()?;
        if !repositories.insert(baseline.repository.clone()) {
            return Err(Error::new(
                "production_baseline",
                "duplicate repository baseline",
            ));
        }
    }
    let mut candidate_repositories = BTreeSet::new();
    let mut selected_delta_digests = BTreeSet::new();
    for candidate in &evidence.candidates {
        if candidate.schema_version != 1
            || candidate.parent_id != evidence.work.parent
            || candidate.work_generation != evidence.work.generation.definition
            || candidate.correction_generation != evidence.work.generation.correction
            || !candidate_repositories.insert(candidate.repository.clone())
            || candidate.ordered_delta_digests.len() != candidate.ordered_patch_digests.len()
            || candidate.ordered_delta_digests.is_empty()
            || candidate.result.is_none()
            || candidate.combined_patch_digest.is_none()
            || !evidence.baselines.iter().any(|b| {
                b.repository == candidate.repository && b.production == candidate.production
            })
        {
            return Err(Error::new(
                "candidate_provenance",
                "isolated candidate vector is incomplete or stale",
            ));
        }
        for digest in &candidate.ordered_delta_digests {
            validate_digest(digest)?;
            if !selected_delta_digests.insert(digest) {
                return Err(Error::new(
                    "candidate_provenance",
                    "selected delta is duplicated",
                ));
            }
        }
        for digest in &candidate.ordered_patch_digests {
            validate_digest(digest)?;
        }
    }
    let required_delta_digests: BTreeSet<String> = children
        .iter()
        .filter_map(|child| match &child.evidence.outcome {
            ChildOutcome::Changed { delta, .. } => Some(delta.digest()),
            ChildOutcome::NoChange { .. } => None,
        })
        .collect::<Result<_>>()?;
    if selected_delta_digests
        .into_iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        != required_delta_digests
    {
        return Err(Error::new(
            "candidate_provenance",
            "selected deltas must account for exactly the required changed child contributions; extra or missing patches block readiness",
        ));
    }
    validate_digest(&evidence.dependency_snapshot_digest)?;
    let vector = evidence.candidate_vector_digest()?;
    if requirements.integrated_commands.is_empty() {
        return Err(Error::invalid(
            "parent needs configured integrated validation",
        ));
    }
    let mut integrated_ids = BTreeSet::new();
    for run in &evidence.integrated_results {
        if !integrated_ids.insert(&run.id) {
            return Err(Error::new(
                "integrated_gate",
                "ambiguous integrated check result",
            ));
        }
    }
    for (id, command) in requirements.integrated_commands {
        let run = evidence
            .integrated_results
            .iter()
            .find(|r| &r.id == id)
            .ok_or_else(|| {
                Error::new(
                    "integrated_gate",
                    format!("integrated check {id} did not run"),
                )
            })?;
        if &run.command != command
            || command.is_empty()
            || run.operation_id.is_nil()
            || run.e2e_child != evidence.e2e_child
            || run.candidate_vector_digest != vector
            || run.test_resources != *requirements.test_resources
            || run.conclusion != Conclusion::Success
            || run.completed_at < run.started_at
            || run.completed_at > evidence.created_at
            || !run.after_staging_integration
            || run.artifacts.is_empty()
        {
            return Err(Error::new(
                "integrated_gate",
                "integrated validation is failed, unperformed or belongs to another candidate/resource set",
            ));
        }
        for artifact in &run.artifacts {
            artifact.validate()?;
        }
    }
    if evidence.visual {
        let video = evidence.video.as_ref().ok_or_else(|| {
            Error::new(
                "video_gate",
                "visual parent requires an associated candidate recording",
            )
        })?;
        validate_digest(&video.asset_digest)?;
        if video.media_type != "video/mp4"
            || video.video_codec != "h264"
            || video
                .audio_codec
                .as_ref()
                .is_some_and(|codec| codec != "aac")
            || video.candidate_vector_digest != vector
            || video.linear_asset_id.is_empty()
            || video.parent_association_id.is_empty()
            || video.associated_parent != evidence.work.parent
            || !video.playback_verified
            || !evidence
                .integrated_results
                .iter()
                .any(|r| r.operation_id == video.validation_operation)
        {
            return Err(Error::new(
                "video_gate",
                "recording format, actual candidate, validation or verified Linear association/playback does not match",
            ));
        }
    } else if evidence.video.is_some() {
        return Err(Error::new(
            "video_gate",
            "nonvisual evidence must not manufacture a required recording",
        ));
    }
    publication.verify(evidence, "linear", linear_writer)?;
    let satisfaction: Vec<_> = children.iter().map(|child| {
        let delta = match &child.evidence.outcome {
            ChildOutcome::Changed { delta, .. } => Some(delta.digest()),
            ChildOutcome::NoChange { .. } => None,
        }.transpose()?;
        Ok(serde_json::json!({ "work": child.evidence.work, "criteria": child.evidence.fulfilled_criteria, "changed_delta": delta }))
    }).collect::<Result<_>>()?;
    Ok(VerifiedParent {
        manifest: evidence.clone(),
        token: evidence.token()?,
        child_satisfaction_digest: canonical_digest(1, &satisfaction)?,
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acceptance {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub operation_id: Uuid,
    pub work: WorkIdentity,
    pub reviewed_token: String,
    pub control_actor: String,
    pub linear_actor: String,
    pub provider_writer: String,
    pub accepted_at: DateTime<Utc>,
}

pub struct AcceptanceActor<'a> {
    pub control_actor: &'a str,
    pub linear_actor: &'a str,
    pub provider_writer: &'a str,
    pub has_accept_role: bool,
    pub permitted_acceptors: &'a BTreeSet<String>,
}

/// Only creates the record to publish. Publication and final generation refresh
/// are separately required by `verify_acceptance`; closure is a separate write.
pub struct AcceptanceRequest<'a> {
    pub supplied_token: &'a str,
    pub current_work: &'a WorkIdentity,
    pub pending_correction: bool,
    pub actor: AcceptanceActor<'a>,
    pub runtime_id: Uuid,
    pub operation_id: Uuid,
    pub now: DateTime<Utc>,
}

pub fn prepare_acceptance(
    parent: &VerifiedParent,
    request: AcceptanceRequest<'_>,
    key: &AttestationKey,
) -> Result<Signed<Acceptance>> {
    let AcceptanceRequest {
        supplied_token,
        current_work,
        pending_correction,
        actor,
        runtime_id,
        operation_id,
        now,
    } = request;
    if supplied_token != parent.token || &parent.manifest.work != current_work || pending_correction
    {
        return Err(Error::new(
            "stale_result",
            "review token or generation is no longer current",
        ));
    }
    if !actor.has_accept_role
        || !actor.permitted_acceptors.contains(actor.linear_actor)
        || actor.control_actor.is_empty()
        || actor.provider_writer.is_empty()
    {
        return Err(Error::new(
            "access_denied",
            "authenticated actor lacks control/Linear acceptance authority",
        ));
    }
    if runtime_id.is_nil() || operation_id.is_nil() || now < parent.manifest.created_at {
        return Err(Error::invalid("acceptance identity/time is invalid"));
    }
    key.sign(
        "human_acceptance",
        &Acceptance {
            schema_version: 1,
            runtime_id,
            operation_id,
            work: current_work.clone(),
            reviewed_token: supplied_token.into(),
            control_actor: actor.control_actor.into(),
            linear_actor: actor.linear_actor.into(),
            provider_writer: actor.provider_writer.into(),
            accepted_at: now,
        },
    )
}

#[derive(Clone, Debug)]
pub struct VerifiedAcceptance {
    acceptance: Acceptance,
    digest: String,
}

impl VerifiedAcceptance {
    pub fn acceptance(&self) -> &Acceptance {
        &self.acceptance
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

pub fn verify_acceptance(
    signed: &Signed<Acceptance>,
    publication: &Publication,
    parent: &VerifiedParent,
    current_work: &WorkIdentity,
    pending_correction: bool,
    permitted_acceptors: &BTreeSet<String>,
    key: &AttestationKey,
) -> Result<VerifiedAcceptance> {
    key.verify("human_acceptance", signed)?;
    let a = &signed.payload;
    if a.schema_version != 1
        || a.work != *current_work
        || a.work != parent.manifest.work
        || a.reviewed_token != parent.token
        || pending_correction
        || !permitted_acceptors.contains(&a.linear_actor)
        || a.control_actor.is_empty()
        || a.runtime_id.is_nil()
        || a.operation_id.is_nil()
        || a.accepted_at < parent.manifest.created_at
    {
        return Err(Error::new(
            "stale_result",
            "published acceptance is not a permitted decision for the current result",
        ));
    }
    publication.verify(signed, "linear", &a.provider_writer)?;
    if publication.observed_at < a.accepted_at {
        return Err(Error::new(
            "evidence_unpublished",
            "acceptance has no post-write publication observation",
        ));
    }
    Ok(VerifiedAcceptance {
        acceptance: a.clone(),
        digest: signed.digest()?,
    })
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CarriedAcceptance {
    pub schema_version: u32,
    pub original_reviewed_token: String,
    pub original_acceptance_digest: String,
    pub current_evidence_token: String,
    pub unchanged_patch_proof: String,
    pub production_validation_digest: String,
    pub created_at: DateTime<Utc>,
}

/// Deliberately not deserializable. Recovery recomputes its proof from retained
/// accepted/current manifests and newly verified production checks.
#[derive(Clone, Debug)]
pub struct VerifiedCarry {
    record: CarriedAcceptance,
}

impl VerifiedCarry {
    pub fn record(&self) -> &CarriedAcceptance {
        &self.record
    }
}

#[derive(Clone, Debug)]
pub struct VerifiedProduction {
    revision: RevisionBinding,
    work: WorkIdentity,
    checks: Vec<CheckRun>,
    review_digest: String,
}

impl VerifiedProduction {
    pub fn revision(&self) -> &RevisionBinding {
        &self.revision
    }
    pub fn work(&self) -> &WorkIdentity {
        &self.work
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &serde_json::json!({ "revision": self.revision, "work": self.work, "checks": self.checks, "review": self.review_digest }),
        )
    }
}

pub struct ProductionRequirements<'a> {
    pub parent: &'a VerifiedParent,
    pub revision: &'a RevisionBinding,
    pub checks: &'a [CheckRequirement],
    pub required_contexts: &'a BTreeSet<String>,
}

pub fn verify_production(
    requirements: ProductionRequirements<'_>,
    snapshot: &CheckSnapshot,
    review: &VerifiedReview,
    protection: &RepositoryProtection,
) -> Result<VerifiedProduction> {
    if requirements.checks.is_empty()
        || requirements
            .checks
            .iter()
            .any(|c| c.kind != CheckKind::ProductionCi)
        || review.attestation.work != requirements.parent.manifest.work
        || review.attestation.revision != *requirements.revision
        || !matches!(review.attestation.review, ReviewKind::Derived { .. })
        || !requirements.parent.manifest.candidates.iter().any(|c| {
            c.repository == requirements.revision.repository
                && c.production.commit == requirements.revision.base
                && c.result.as_ref().is_some_and(|r| {
                    r.commit == requirements.revision.head
                        && r.tree == requirements.revision.tested_tree
                })
        })
    {
        return Err(Error::new(
            "production_validation",
            "production tests/review must cover the exact isolated parent candidate",
        ));
    }
    let checks = verify_checks(
        requirements.checks,
        &requirements.parent.manifest.work,
        requirements.revision,
        snapshot,
    )?;
    protection.verify(
        requirements.revision,
        "main",
        requirements.required_contexts,
    )?;
    Ok(VerifiedProduction {
        revision: requirements.revision.clone(),
        work: requirements.parent.manifest.work.clone(),
        checks,
        review_digest: review.digest.clone(),
    })
}

/// Carries only exact unchanged ordered artifacts and unchanged intended work.
/// New evidence/video is machine-validated, never represented as human reviewed.
pub fn carry_acceptance(
    original: &VerifiedParent,
    newer: &VerifiedParent,
    acceptance: &VerifiedAcceptance,
    known_released_baseline_proof: &ArtifactReference,
    production_validation: &[VerifiedProduction],
    now: DateTime<Utc>,
) -> Result<VerifiedCarry> {
    if acceptance.acceptance.reviewed_token != original.token
        || acceptance.acceptance.work != original.manifest.work
        || original.manifest.work != newer.manifest.work
        || original.manifest.required_children != newer.manifest.required_children
        || original.child_satisfaction_digest != newer.child_satisfaction_digest
        || original.manifest.fulfilled_criteria != newer.manifest.fulfilled_criteria
        || original.manifest.production_targets != newer.manifest.production_targets
        || original.manifest.dependency_snapshot_digest != newer.manifest.dependency_snapshot_digest
        || original.manifest.visual != newer.manifest.visual
        || now < newer.manifest.created_at
    {
        return Err(Error::new(
            "acceptance_required",
            "changed intent/scope/children/dependencies need fresh human acceptance",
        ));
    }
    known_released_baseline_proof.validate()?;
    let old: Vec<_> = original
        .manifest
        .candidates
        .iter()
        .map(|c| {
            (
                &c.repository,
                &c.ordered_delta_digests,
                &c.ordered_patch_digests,
            )
        })
        .collect();
    let new: Vec<_> = newer
        .manifest
        .candidates
        .iter()
        .map(|c| {
            (
                &c.repository,
                &c.ordered_delta_digests,
                &c.ordered_patch_digests,
            )
        })
        .collect();
    if old != new || old.is_empty() {
        return Err(Error::new(
            "acceptance_required",
            "ordered exact selected patches changed or are absent",
        ));
    }
    for candidate in &newer.manifest.candidates {
        let result = candidate.result.as_ref().ok_or_else(|| {
            Error::new(
                "candidate_provenance",
                "new candidate has no verified result",
            )
        })?;
        if !production_validation.iter().any(|r| {
            r.work == newer.manifest.work
                && r.revision.repository == candidate.repository
                && r.revision.head == result.commit
                && r.revision.base == candidate.production.commit
                && r.revision.tested_tree == result.tree
                && r.checks
                    .iter()
                    .all(|run| run.completed_at.is_some_and(|t| t <= now))
        }) {
            return Err(Error::new(
                "production_validation",
                "fresh production validation is missing for a carried candidate",
            ));
        }
    }
    let validation_digests: Vec<String> = production_validation
        .iter()
        .map(VerifiedProduction::digest)
        .collect::<Result<_>>()?;
    Ok(VerifiedCarry {
        record: CarriedAcceptance {
            schema_version: 1,
            original_reviewed_token: original.token.clone(),
            original_acceptance_digest: acceptance.digest.clone(),
            current_evidence_token: newer.token.clone(),
            unchanged_patch_proof: canonical_digest(
                1,
                &serde_json::json!({ "patches": new, "known_released_baseline": known_released_baseline_proof }),
            )?,
            production_validation_digest: canonical_digest(1, &validation_digests)?,
            created_at: now,
        },
    })
}

/// Builds a derived status payload, not an extra agent review. All selected
/// patches must be accounted for by verified original/replacement reviews.
pub struct HostOperation {
    pub runtime_id: Uuid,
    pub owner_generation: u64,
    pub operation_id: Uuid,
}

pub fn derive_review(
    parent: &VerifiedParent,
    candidate: &CandidateManifest,
    revision: RevisionBinding,
    pull_request: u64,
    sources: &[VerifiedReview],
    operation: HostOperation,
) -> Result<ReviewAttestation> {
    let HostOperation {
        runtime_id,
        owner_generation,
        operation_id,
    } = operation;
    if parent.manifest.work.child.is_some()
        || !parent.manifest.candidates.contains(candidate)
        || candidate
            .result
            .as_ref()
            .is_none_or(|r| r.commit != revision.head || r.tree != revision.tested_tree)
        || candidate.production.commit != revision.base
        || candidate.repository != revision.repository
        || pull_request == 0
    {
        return Err(Error::new(
            "derived_review_gate",
            "production review has no exact candidate assembly identity",
        ));
    }
    let mut source_digests = Vec::new();
    for delta_digest in &candidate.ordered_delta_digests {
        let matching: Vec<&VerifiedReview> = sources
            .iter()
            .filter(|r| {
                r.attestation.delta_digest == *delta_digest
                    && r.attestation.work.parent == parent.manifest.work.parent
                    && r.attestation.revision.repository == candidate.repository
                    && r.attestation.verdict == Verdict::Pass
                    && r.attestation.unresolved_findings.is_empty()
                    && matches!(r.attestation.review, ReviewKind::Agent { .. })
            })
            .collect();
        if matching.len() != 1 {
            return Err(Error::new(
                "derived_review_gate",
                "selected delta lacks exactly one verified source review",
            ));
        }
        source_digests.push(matching[0].digest.clone());
    }
    let proof = canonical_digest(1, candidate)?;
    Ok(ReviewAttestation {
        schema_version: 1,
        runtime_id,
        owner_generation,
        operation_id,
        work: parent.manifest.work.clone(),
        revision,
        pull_request: Some(pull_request),
        delta_digest: candidate
            .combined_patch_digest
            .clone()
            .ok_or_else(|| Error::new("derived_review_gate", "assembly patch digest missing"))?,
        verdict: Verdict::Pass,
        unresolved_findings: vec![],
        supporting_links: source_digests.clone(),
        review: ReviewKind::Derived {
            source_attestations: source_digests,
            selected_patches: candidate.ordered_patch_digests.clone(),
            assembly_proof: proof,
        },
    })
}

/// Useful for deterministic evidence capture; the caller records the original
/// path separately and must enforce allowed roots and current-run provenance.
pub fn artifact_bytes(reference: impl Into<String>, bytes: &[u8]) -> ArtifactReference {
    ArtifactReference {
        reference: reference.into(),
        digest: bytes_digest(bytes),
    }
}
