//! Revision-bound gates and authenticated delivery evidence.
//!
//! Provider observations in this module are host inputs from complete adapter
//! reads. They must never be accepted as worker tool arguments. A worker's prose
//! or submitted result reference cannot manufacture a `Verified*` value.

use crate::digest::{bytes_digest, canonical_bytes, canonical_digest};
use crate::error::{Error, Result};
use crate::git::{
    CandidateManifest, DeltaManifest, RepositoryId, Revision, VerifiedProductionIntegration,
    validate_oid,
};
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
        #[derive(Serialize)]
        struct Envelope<'a, T> {
            schema_version: u32,
            key_id: &'a str,
            purpose: &'a str,
            payload: &'a T,
        }
        canonical_bytes(&Envelope {
            schema_version: self.schema_version,
            key_id: &self.key_id,
            purpose: &self.purpose,
            payload: &self.payload,
        })
    }

    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, self)
    }
}

/// Canonical whole-comment framing shared by publication and recovery. Framing
/// is deliberately exact: quoted examples and incidental markers are not records.
pub fn encode_signed_publication<T: Serialize>(
    purpose: &str,
    signed: &Signed<T>,
) -> Result<String> {
    if purpose.is_empty()
        || purpose.len() > 80
        || !purpose.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        || signed.purpose != purpose
        || signed.schema_version != 1
    {
        return Err(Error::new(
            "invalid_attestation",
            "publication domain/schema is invalid",
        ));
    }
    let payload = serde_json::to_string(signed)
        .map_err(|_| Error::invalid("cannot encode canonical publication"))?;
    let body = format!("<!-- kegari:v0:{purpose} -->\n```json\n{payload}\n```");
    if body.len() > 1024 * 1024 {
        return Err(Error::new(
            "bounded_input",
            "signed publication exceeds 1 MiB",
        ));
    }
    Ok(body)
}

pub fn decode_signed_publication<T: serde::de::DeserializeOwned + Serialize>(
    body: &str,
    purpose: &str,
    key: &AttestationKey,
) -> Result<Signed<T>> {
    if body.len() > 1024 * 1024 {
        return Err(Error::new(
            "bounded_input",
            "signed publication exceeds 1 MiB",
        ));
    }
    let prefix = format!("<!-- kegari:v0:{purpose} -->\n```json\n");
    let payload = body
        .strip_prefix(&prefix)
        .and_then(|s| s.strip_suffix("\n```"))
        .ok_or_else(|| {
            Error::new(
                "invalid_attestation",
                "publication is not an exact canonical record",
            )
        })?;
    let signed: Signed<T> = serde_json::from_str(payload)
        .map_err(|_| Error::new("invalid_attestation", "publication schema is invalid"))?;
    if encode_signed_publication(purpose, &signed)? != body {
        return Err(Error::new(
            "invalid_attestation",
            "publication contains noncanonical or duplicate fields",
        ));
    }
    key.verify(purpose, &signed)?;
    Ok(signed)
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
    AgentValidation {
        session_id: String,
        instruction_manifest: String,
        validation: Box<ValidationEvidence>,
        assessment: ValidationAssessment,
    },
    Derived {
        source_attestations: Vec<String>,
        selected_patches: Vec<String>,
        assembly_proof: String,
    },
}

#[path = "evidence_validation.rs"]
mod validation;
pub use validation::{ValidationAssessment, ValidationEvidence, ValidationRequirementAssessment};

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
    if a.schema_version
        != if matches!(a.review, ReviewKind::AgentValidation { .. }) {
            2
        } else {
            1
        }
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
        ReviewKind::AgentValidation {
            session_id,
            instruction_manifest,
            validation,
            assessment,
        } => {
            if session_id.is_empty() || a.work.child.is_none() {
                return Err(Error::new(
                    "review_gate",
                    "discovered child review lacks its original agent session",
                ));
            }
            validate_digest(instruction_manifest)?;
            validation.validate(&a.work, &a.revision)?;
            validation.verify_assessment(assessment, true)?;
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
    Replacement {
        related_pull_request: u64,
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

pub struct ReplacementGates<'a> {
    pub staged_result: &'a VerifiedChild,
    pub related_pull_request: u64,
    pub repository: &'a RepositoryId,
    pub github_publication: &'a Publication,
    pub github_writer: &'a str,
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
    verify_child_inner(
        evidence,
        requirements,
        review,
        protection,
        publication,
        linear_writer,
        None,
    )
}

pub fn verify_replacement_child(
    evidence: &ChildEvidence,
    requirements: ChildRequirements<'_>,
    review: &VerifiedReview,
    gates: ReplacementGates<'_>,
    publication: &Publication,
    linear_writer: &str,
) -> Result<VerifiedChild> {
    if !matches!(evidence.outcome, ChildOutcome::Replacement { .. }) {
        return Err(Error::new(
            "replacement_gate",
            "replacement validation requires an explicit replacement outcome",
        ));
    }
    verify_child_inner(
        evidence,
        requirements,
        review,
        None,
        publication,
        linear_writer,
        Some(gates),
    )
}

/// Check a draft without inventing a publication or minting a completion proof.
pub fn validate_child_candidate(
    evidence: &ChildEvidence,
    requirements: ChildRequirements<'_>,
    review: &VerifiedReview,
    protection: Option<&RepositoryProtection>,
) -> Result<()> {
    validate_child_candidate_inner(evidence, requirements, review, protection, None)
}
fn validate_child_candidate_inner(
    evidence: &ChildEvidence,
    requirements: ChildRequirements<'_>,
    review: &VerifiedReview,
    protection: Option<&RepositoryProtection>,
    replacement_gates: Option<ReplacementGates<'_>>,
) -> Result<()> {
    evidence.work.validate()?;
    evidence.revision.validate()?;
    if evidence.schema_version
        != if matches!(
            review.attestation.review,
            ReviewKind::AgentValidation { .. }
        ) {
            2
        } else {
            1
        }
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
    if let ReviewKind::AgentValidation {
        validation,
        assessment,
        ..
    } = &review.attestation.review
    {
        validation.validate(&evidence.work, &evidence.revision)?;
        validation.verify_assessment(assessment, true)?;
        let actual: Vec<_> = evidence
            .checks
            .runs
            .iter()
            .filter(|r| r.kind == CheckKind::Local)
            .cloned()
            .collect();
        if canonical_digest(1, &actual)?
            != canonical_digest(1, &validation.runs(&evidence.revision)?)?
            || canonical_digest(
                1,
                &requirements
                    .checks
                    .iter()
                    .filter(|r| r.kind == CheckKind::Local)
                    .collect::<Vec<_>>(),
            )? != canonical_digest(1, &validation.requirements()?)?
        {
            return Err(Error::new(
                "validation_adequacy",
                "published local evidence differs from independently reviewed obligations",
            ));
        }
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
                || delta.replacement.is_some()
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
                || delta.review_digest()? != review.attestation.delta_digest
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
        ChildOutcome::Replacement {
            related_pull_request,
            delta,
        } => {
            let gates = replacement_gates.ok_or_else(|| {
                Error::new(
                    "replacement_gate",
                    "separate staged-result and related PR publication proofs are required",
                )
            })?;
            let mapping = delta.replacement.as_ref().ok_or_else(|| {
                Error::new(
                    "replacement_mapping",
                    "replacement has no original contribution mapping",
                )
            })?;
            let staged = gates.staged_result.evidence();
            let staged_revision = match &staged.outcome {
                ChildOutcome::Changed { delta, .. } => &delta.staging_commit,
                ChildOutcome::NoChange { .. } => &Revision {
                    commit: staged.revision.tested_commit.clone(),
                    tree: staged.revision.tested_tree.clone(),
                },
                ChildOutcome::Replacement { .. } => {
                    return Err(Error::new(
                        "replacement_gate",
                        "staged proof must be a normal integration or evidenced no-change result",
                    ));
                }
            };
            if *related_pull_request == 0
                || *related_pull_request != gates.related_pull_request
                || delta.repository != *gates.repository
                || delta.repository != evidence.revision.repository
                || delta.parent_id != evidence.work.parent
                || Some(delta.child_id) != evidence.work.child
                || delta.work_generation != evidence.work.generation.definition
                || delta.correction_generation != evidence.work.generation.correction
                || delta.validated_base.commit != evidence.revision.base
                || delta.validated_head.commit != evidence.revision.head
                || delta.validated_head.tree != evidence.revision.tested_tree
                || delta.review_digest()? != review.attestation.delta_digest
                || !matches!(
                    review.attestation.review,
                    ReviewKind::Agent { .. } | ReviewKind::AgentValidation { .. }
                )
                || staged.work != evidence.work
                || staged.fulfilled_criteria != evidence.fulfilled_criteria
                || mapping.staging_result_digest != gates.staged_result.digest()
                || delta.staging_commit != *staged_revision
                || delta.staging_parent != *staged_revision
                || mapping.reason.trim().is_empty()
                || delta.original_contributions()?.is_empty()
                || mapping.originals.iter().any(|m| {
                    m.child_id != delta.child_id
                        || m.parent_id != delta.parent_id
                        || m.repository != delta.repository
                        || m.scope != delta.scope
                })
            {
                return Err(Error::new(
                    "replacement_gate",
                    "replacement mapping, scoped review or independently validated staged result differs",
                ));
            }
            gates
                .github_publication
                .verify(evidence, "github", gates.github_writer)?;
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
                || !matches!(
                    review.attestation.review,
                    ReviewKind::Agent { .. } | ReviewKind::AgentValidation { .. }
                )
            {
                return Err(Error::new(
                    "no_change_gate",
                    "no-change completion needs its exact-SHA validation, review and explicit inapplicable gates",
                ));
            }
        }
    }
    Ok(())
}

fn verify_child_inner(
    evidence: &ChildEvidence,
    requirements: ChildRequirements<'_>,
    review: &VerifiedReview,
    protection: Option<&RepositoryProtection>,
    publication: &Publication,
    linear_writer: &str,
    replacement_gates: Option<ReplacementGates<'_>>,
) -> Result<VerifiedChild> {
    validate_child_candidate_inner(
        evidence,
        requirements,
        review,
        protection,
        replacement_gates,
    )?;
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
    /// Complete per-target source inventory; a common deployed_revision is only
    /// present when every target serves the same commit.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub target_revisions: BTreeMap<String, String>,
    pub deployed_artifact: Option<String>,
    /// Exact target artifact inventory. `deployed_artifact`, when present, is
    /// only a common digest shared by every target, never a synthetic aggregate.
    #[serde(default)]
    pub target_artifacts: BTreeMap<String, String>,
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
            if self.deployed_revision.is_some()
                || self.deployed_artifact.is_some()
                || !self.target_artifacts.is_empty()
                || !self.target_revisions.is_empty()
            {
                return Err(Error::invalid(
                    "deployment-free baseline must not invent deployment activity",
                ));
            }
        } else {
            if self.deployed_revision.is_none() && self.target_revisions.is_empty() {
                return Err(Error::new(
                    "production_baseline",
                    "deployed source revisions are unknown",
                ));
            }
            if let Some(deployed) = &self.deployed_revision {
                validate_oid(deployed)?;
            }
            if !self.target_revisions.is_empty()
                && self.target_revisions.keys().collect::<BTreeSet<_>>()
                    != self.target_artifacts.keys().collect::<BTreeSet<_>>()
            {
                return Err(Error::new(
                    "production_baseline",
                    "target source and artifact inventories differ",
                ));
            }
            for (target, revision) in &self.target_revisions {
                validate_oid(revision)?;
                if target.trim().is_empty()
                    || self
                        .deployed_revision
                        .as_ref()
                        .is_some_and(|common| common != revision)
                {
                    return Err(Error::new(
                        "production_baseline",
                        "target identity or common source revision is inconsistent",
                    ));
                }
            }
            if self.deployed_artifact.is_none() && self.target_artifacts.is_empty() {
                return Err(Error::new(
                    "production_baseline",
                    "deployed target artifacts are unknown",
                ));
            }
            if let Some(artifact) = &self.deployed_artifact {
                validate_digest(artifact)?;
            }
            for (target, artifact) in &self.target_artifacts {
                if target.trim().is_empty()
                    || self
                        .deployed_artifact
                        .as_ref()
                        .is_some_and(|common| common != artifact)
                {
                    return Err(Error::new(
                        "production_baseline",
                        "target identity or common artifact digest is inconsistent",
                    ));
                }
                validate_digest(artifact)?;
            }
            if self.successor_recovery_operation.is_none()
                && (self
                    .deployed_revision
                    .as_ref()
                    .is_some_and(|revision| revision != &self.production.commit)
                    || self
                        .target_revisions
                        .values()
                        .any(|revision| revision != &self.production.commit))
            {
                return Err(Error::new(
                    "production_baseline",
                    "main/deployment divergence needs explicit reconciled successor recovery",
                ));
            }
        }
        Ok(())
    }
    pub fn source_for_target(&self, target: &str) -> Option<&str> {
        if self.target_revisions.is_empty() {
            self.deployed_revision.as_deref()
        } else {
            self.target_revisions.get(target).map(String::as_str)
        }
    }
    pub fn artifact_for_target(&self, target: &str) -> Option<&str> {
        if self.target_artifacts.is_empty() {
            self.deployed_artifact.as_deref()
        } else {
            self.target_artifacts.get(target).map(String::as_str)
        }
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
    pub required_contributions: BTreeMap<Uuid, BTreeSet<String>>,
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
    /// Host-reconstructed complete contribution history for the current child
    /// responsibilities, including earlier known effects of partial releases.
    pub required_contributions: &'a BTreeMap<Uuid, BTreeSet<String>>,
    pub production_integrations: &'a [VerifiedProductionIntegration],
}

#[derive(Clone, Debug)]
pub struct VerifiedParent {
    manifest: ParentEvidence,
    token: String,
    child_satisfaction_digest: String,
    source_review_digests: BTreeMap<String, String>,
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
        || evidence.required_contributions != *requirements.required_contributions
        || evidence
            .required_contributions
            .keys()
            .any(|id| !requirements.required_children.contains(id))
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
    let mut accounted_contributions = BTreeSet::new();
    let required_delta_digests: BTreeSet<String> = evidence
        .required_contributions
        .values()
        .flatten()
        .cloned()
        .collect();
    if required_delta_digests.len()
        != evidence
            .required_contributions
            .values()
            .map(BTreeSet::len)
            .sum::<usize>()
    {
        return Err(Error::new(
            "candidate_provenance",
            "one original contribution is assigned to multiple child responsibilities",
        ));
    }
    let mut reviewed_mappings = BTreeMap::new();
    for child in children {
        if let ChildOutcome::Changed { delta, .. } | ChildOutcome::Replacement { delta, .. } =
            &child.evidence.outcome
        {
            let contributions = delta.original_contributions()?;
            if evidence
                .required_contributions
                .get(&delta.child_id)
                .is_none_or(|required| !contributions.is_subset(required))
            {
                return Err(Error::new(
                    "candidate_provenance",
                    "current child contribution is missing from the required complete inventory",
                ));
            }
            reviewed_mappings.insert(delta.digest()?, contributions);
        }
    }
    let mut used_integrations = BTreeSet::new();
    for candidate in &evidence.candidates {
        if candidate.schema_version != 1
            || candidate.parent_id != evidence.work.parent
            || candidate.work_generation != evidence.work.generation.definition
            || candidate.correction_generation != evidence.work.generation.correction
            || !candidate_repositories.insert(candidate.repository.clone())
            || candidate.ordered_delta_digests.len() != candidate.ordered_patch_digests.len()
            || candidate
                .ordered_delta_digests
                .iter()
                .collect::<BTreeSet<_>>()
                != candidate.selected_contributions.keys().collect()
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
            let contributions = &candidate.selected_contributions[digest];
            let expected = reviewed_mappings.get(digest).cloned().or_else(|| {
                required_delta_digests
                    .contains(digest)
                    .then(|| BTreeSet::from([digest.clone()]))
            });
            if expected.as_ref() != Some(contributions) {
                return Err(Error::new(
                    "candidate_provenance",
                    "selected artifact has no exact reviewed contribution or replacement mapping",
                ));
            }
            for contribution in contributions {
                if !accounted_contributions.insert(contribution.clone()) {
                    return Err(Error::new(
                        "candidate_provenance",
                        "required contribution is selected more than once",
                    ));
                }
            }
        }
        for digest in &candidate.ordered_patch_digests {
            validate_digest(digest)?;
        }
        if candidate.ordered_delta_digests.is_empty()
            && (candidate.result.as_ref() != Some(&candidate.production)
                || candidate.combined_patch_digest.as_deref() != Some(bytes_digest(&[]).as_str()))
        {
            return Err(Error::new(
                "candidate_provenance",
                "no-change candidate must exactly retain the production baseline without a manufactured commit",
            ));
        }
        for integration in &candidate.production_integrations {
            if integration.repository != candidate.repository
                || integration.parent_id != evidence.work.parent
                || integration.observed_main != candidate.production
                || !requirements
                    .production_integrations
                    .iter()
                    .any(|p| p.record() == integration)
                || !used_integrations.insert(integration.digest()?)
            {
                return Err(Error::new(
                    "production_provenance",
                    "already-integrated contribution requires its current verified Git-object proof",
                ));
            }
            for contribution in integration.selected_contributions.values().flatten() {
                if !accounted_contributions.insert(contribution.clone()) {
                    return Err(Error::new(
                        "candidate_provenance",
                        "already-integrated contribution is selected or counted again",
                    ));
                }
            }
        }
    }
    if accounted_contributions != required_delta_digests {
        return Err(Error::new(
            "candidate_provenance",
            "selected, replaced and already-integrated artifacts must account for exactly every required child contribution",
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
            ChildOutcome::Changed { delta, .. } | ChildOutcome::Replacement { delta, .. } => Some(delta.digest()),
            ChildOutcome::NoChange { .. } => None,
        }.transpose()?;
        Ok(serde_json::json!({ "work": child.evidence.work, "criteria": child.evidence.fulfilled_criteria, "changed_delta": delta }))
    }).collect::<Result<_>>()?;
    Ok(VerifiedParent {
        manifest: evidence.clone(),
        token: evidence.token()?,
        child_satisfaction_digest: canonical_digest(1, &satisfaction)?,
        source_review_digests: children
            .iter()
            .filter_map(|child| match &child.evidence.outcome {
                ChildOutcome::Changed { delta, .. } | ChildOutcome::Replacement { delta, .. } => {
                    Some(
                        delta
                            .digest()
                            .map(|digest| (digest, child.evidence.review_digest.clone())),
                    )
                }
                ChildOutcome::NoChange { .. } => None,
            })
            .collect::<Result<_>>()?,
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

/// Recovery authority is rooted in a verified human decision, not in mutable
/// candidate metadata. This capability cannot be deserialized from a receipt.
#[derive(Clone, Debug)]
pub struct VerifiedAcceptedCandidate {
    candidate_digest: String,
    evidence_token: String,
    acceptance_digest: String,
}

impl VerifiedAcceptedCandidate {
    pub fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }
    pub fn evidence_token(&self) -> &str {
        &self.evidence_token
    }
    pub fn acceptance_digest(&self) -> &str {
        &self.acceptance_digest
    }
}

/// Historical contribution authority, derived only from the exact actual
/// admitted plan and its authenticated original publications. It is not current
/// parent readiness and does not authorize a new merge or deployment.
pub(crate) fn recorded_candidate_authority(
    origin: &crate::delivery::VerifiedReleaseOrigin,
    candidate: &CandidateManifest,
) -> Result<VerifiedAcceptedCandidate> {
    if !origin.manifest().candidates.contains(candidate) {
        return Err(Error::new(
            "acceptance_required",
            "historical release did not accept this candidate",
        ));
    }
    Ok(VerifiedAcceptedCandidate {
        candidate_digest: canonical_digest(1, candidate)?,
        evidence_token: origin.plan().evidence_token.clone(),
        acceptance_digest: origin.plan().acceptance_digest.clone(),
    })
}

pub fn verify_accepted_candidate(
    parent: &VerifiedParent,
    acceptance: &VerifiedAcceptance,
    carry: Option<&VerifiedCarry>,
    candidate: &CandidateManifest,
) -> Result<VerifiedAcceptedCandidate> {
    let a = acceptance.acceptance();
    if !parent.manifest.candidates.contains(candidate)
        || a.work != parent.manifest.work
        || (a.reviewed_token != parent.token
            && carry.is_none_or(|c| {
                c.record.original_reviewed_token != a.reviewed_token
                    || c.record.original_acceptance_digest != acceptance.digest()
                    || c.record.current_evidence_token != parent.token
            }))
    {
        return Err(Error::new(
            "acceptance_required",
            "candidate recovery requires its authenticated accepted evidence or verified unchanged carry lineage",
        ));
    }
    Ok(VerifiedAcceptedCandidate {
        candidate_digest: canonical_digest(1, candidate)?,
        evidence_token: parent.token.clone(),
        acceptance_digest: acceptance.digest().into(),
    })
}

#[derive(Clone, Debug)]
pub struct VerifiedProduction {
    revision: RevisionBinding,
    work: WorkIdentity,
    checks: Vec<CheckRun>,
    review_digest: String,
    existing_production: bool,
}

impl VerifiedProduction {
    pub fn is_existing_production(&self) -> bool {
        self.existing_production
    }
    pub fn revision(&self) -> &RevisionBinding {
        &self.revision
    }
    pub fn work(&self) -> &WorkIdentity {
        &self.work
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &serde_json::json!({ "revision": self.revision, "work": self.work, "checks": self.checks, "review": self.review_digest, "existing_production":self.existing_production }),
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
            !c.ordered_delta_digests.is_empty()
                && c.repository == requirements.revision.repository
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
        existing_production: false,
    })
}

/// Exact validation of an unchanged accepted production candidate. Prior
/// contributions require their verified integration provenance; a no-change
/// candidate has no prior contribution to invent. Neither path creates a PR.
pub fn verify_existing_production(
    parent: &VerifiedParent,
    revision: &RevisionBinding,
    requirements: &[CheckRequirement],
    snapshot: &CheckSnapshot,
    integrations: &[crate::git::VerifiedProductionIntegration],
) -> Result<VerifiedProduction> {
    revision.validate()?;
    let candidate = parent.manifest.candidates.iter().find(|candidate|
        candidate.repository==revision.repository && candidate.ordered_delta_digests.is_empty()
            && candidate.result.as_ref()==Some(&candidate.production)
            && candidate.production.commit==revision.head && revision.base==revision.head
            && revision.tested_commit==revision.head && candidate.production.tree==revision.tested_tree
    ).ok_or_else(||Error::new("production_validation", "existing-source validation needs the unchanged exact accepted production candidate"))?;
    if requirements.is_empty()
        || requirements
            .iter()
            .any(|r| r.kind != CheckKind::ProductionCi)
        || candidate
            .production_integrations
            .iter()
            .any(|record| !integrations.iter().any(|proof| proof.record() == record))
    {
        return Err(Error::new(
            "production_validation",
            "existing source needs complete exact production checks and verified prior contribution provenance",
        ));
    }
    let checks = verify_checks(requirements, &parent.manifest.work, revision, snapshot)?;
    Ok(VerifiedProduction {
        revision: revision.clone(),
        work: parent.manifest.work.clone(),
        checks,
        review_digest: canonical_digest(1, &candidate.production_integrations)?,
        existing_production: true,
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
    known_released_baseline_proof.validate()?;
    // Preserve semantic diagnostics, but a public artifact URL/hash is not an
    // authenticated released-baseline chain and can never mint this capability.
    let _ = validate_carry_acceptance(
        original,
        newer,
        acceptance,
        &known_released_baseline_proof.digest,
        production_validation,
        now,
    )?;
    Err(Error::new(
        "acceptance_carry_baseline",
        "carry requires an actual authenticated released-baseline transition",
    ))
}

pub(crate) fn carry_verified_acceptance(
    original: &VerifiedParent,
    newer: &VerifiedParent,
    acceptance: &VerifiedAcceptance,
    baseline: &crate::runtime_delivery::VerifiedBaselineTransition,
    configuration: &crate::config::Config,
    production_validation: &[VerifiedProduction],
    now: DateTime<Utc>,
) -> Result<VerifiedCarry> {
    baseline.revalidate(configuration)?;
    if !baseline.binds_current_parent(original, newer) {
        return Err(Error::new(
            "acceptance_carry_baseline",
            "carry transition does not bind original and current complete evidence",
        ));
    }
    validate_carry_acceptance(
        original,
        newer,
        acceptance,
        baseline.digest(),
        production_validation,
        now,
    )
}

/// Recovery consumes an opaque authenticated producer-written historical subject,
/// an independently hydrated current parent and freshly reobserved full proofs.
/// It preserves the old acceptance/carry bytes and issues no dispatch authority.
pub(crate) fn recover_current_carry_semantics(
    subject: &crate::delivery::RetainedCarrySubject,
    parent: &VerifiedParent,
    production: &[VerifiedProduction],
    permitted: &BTreeSet<String>,
) -> Result<(VerifiedAcceptance, VerifiedCarry)> {
    subject.validate_retained()?;
    let signed = subject.original_acceptance();
    let acceptance = &signed.payload;
    let carry = subject.carry_record();
    if subject.current_evidence() != parent.manifest()
        || carry.current_evidence_token != parent.token()
        || carry.original_reviewed_token != acceptance.reviewed_token
        || carry.original_acceptance_digest != signed.digest()?
        || acceptance.work != parent.manifest().work
        || acceptance.schema_version != 1
        || acceptance.runtime_id.is_nil()
        || acceptance.operation_id.is_nil()
        || acceptance.control_actor.is_empty()
        || !permitted.contains(&acceptance.linear_actor)
        || production.len() != parent.manifest().candidates.len()
        || production
            .iter()
            .map(|p| &p.revision.repository)
            .collect::<BTreeSet<_>>()
            != parent
                .manifest()
                .candidates
                .iter()
                .map(|p| &p.repository)
                .collect()
        || canonical_digest(
            1,
            &production
                .iter()
                .map(VerifiedProduction::digest)
                .collect::<Result<Vec<_>>>()?,
        )? != carry.production_validation_digest
    {
        return Err(Error::new(
            "acceptance_carry",
            "recovered carry differs from exact original acceptance or current full validation",
        ));
    }
    for candidate in &parent.manifest().candidates {
        let result = candidate.result.as_ref().ok_or_else(|| {
            Error::new("candidate_provenance", "recovered candidate result missing")
        })?;
        if !production.iter().any(|proof| {
            proof.work == parent.manifest().work
                && proof.revision.repository == candidate.repository
                && proof.revision.head == result.commit
                && proof.revision.base == candidate.production.commit
                && proof.revision.tested_tree == result.tree
                && !proof.checks.is_empty()
                && proof.checks.iter().all(|run| {
                    run.completed_at
                        .is_some_and(|t| t >= parent.manifest().created_at && t <= carry.created_at)
                })
        }) {
            return Err(Error::new(
                "production_validation",
                "recovered carry lost its exact full production validation",
            ));
        }
    }
    Ok((
        VerifiedAcceptance {
            acceptance: acceptance.clone(),
            digest: signed.digest()?,
        },
        VerifiedCarry {
            record: carry.clone(),
        },
    ))
}

fn validate_carry_acceptance(
    original: &VerifiedParent,
    newer: &VerifiedParent,
    acceptance: &VerifiedAcceptance,
    known_released_baseline_proof: &str,
    production_validation: &[VerifiedProduction],
    now: DateTime<Utc>,
) -> Result<VerifiedCarry> {
    if acceptance.acceptance.reviewed_token != original.token
        || acceptance.acceptance.work != original.manifest.work
        || original.manifest.work != newer.manifest.work
        || original.manifest.required_children != newer.manifest.required_children
        || original.manifest.required_contributions != newer.manifest.required_contributions
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
    validate_digest(known_released_baseline_proof)?;
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
    if production_validation.len() != newer.manifest.candidates.len()
        || production_validation
            .iter()
            .map(|proof| &proof.revision.repository)
            .collect::<BTreeSet<_>>()
            != newer
                .manifest
                .candidates
                .iter()
                .map(|candidate| &candidate.repository)
                .collect::<BTreeSet<_>>()
    {
        return Err(Error::new(
            "production_validation",
            "carry requires exactly one full production proof per selected repository",
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
                && !r.checks.is_empty()
                && r.checks.iter().all(|run| {
                    run.completed_at
                        .is_some_and(|t| t >= newer.manifest.created_at && t <= now)
                })
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionTargetBaseline {
    pub provider: String,
    pub target: String,
    pub repository: RepositoryId,
    pub source: Revision,
    pub artifact_digest: String,
    pub provider_record: String,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoReleaseRequired {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub operation_id: Uuid,
    pub work: WorkIdentity,
    pub evidence_token: String,
    pub acceptance_digest: String,
    pub baselines: Vec<ProductionBaseline>,
    pub targets: Vec<ProductionTargetBaseline>,
    pub recorded_at: DateTime<Utc>,
}

pub struct NoReleaseRequirements<'a> {
    pub current_work: &'a WorkIdentity,
    pub pending_correction: bool,
    /// Complete configured target ID -> (provider, repository) inventory.
    pub targets: &'a BTreeMap<String, (String, RepositoryId)>,
}

#[derive(Clone, Debug)]
pub struct VerifiedNoReleaseRequired {
    record: NoReleaseRequired,
    digest: String,
}

impl VerifiedNoReleaseRequired {
    pub fn record(&self) -> &NoReleaseRequired {
        &self.record
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

/// Current semantic authority; the runtime additionally requires the actual
/// published carry capability and its current native/owned reference checks.
pub(crate) struct NoReleaseAcceptance<'a> {
    pub acceptance: &'a VerifiedAcceptance,
    pub carry: Option<&'a VerifiedCarry>,
}

fn no_release_acceptance_matches(
    current_token: &str,
    reviewed_token: &str,
    acceptance_digest: &str,
    carry: Option<&CarriedAcceptance>,
) -> bool {
    match carry {
        None => reviewed_token == current_token,
        Some(carry) => {
            carry.schema_version == 1
                && reviewed_token != current_token
                && carry.original_reviewed_token == reviewed_token
                && carry.original_acceptance_digest == acceptance_digest
                && carry.current_evidence_token == current_token
        }
    }
}

fn validate_no_release_required(
    record: &NoReleaseRequired,
    parent: &VerifiedParent,
    authority: NoReleaseAcceptance<'_>,
    requirements: NoReleaseRequirements<'_>,
) -> Result<()> {
    let p = parent.manifest();
    let acceptance = authority.acceptance;
    if record.schema_version != 1
        || record.runtime_id.is_nil()
        || record.operation_id.is_nil()
        || &record.work != requirements.current_work
        || record.work != p.work
        || requirements.pending_correction
        || record.evidence_token != parent.token()
        || record.acceptance_digest != acceptance.digest()
        || !no_release_acceptance_matches(
            parent.token(),
            &acceptance.acceptance().reviewed_token,
            acceptance.digest(),
            authority.carry.map(VerifiedCarry::record),
        )
        || acceptance.acceptance().work != p.work
        || record.baselines != p.baselines
        || record.recorded_at < p.created_at
        || authority
            .carry
            .is_some_and(|carry| record.recorded_at < carry.record().created_at)
        || p.candidates.iter().any(|c| {
            !c.ordered_delta_digests.is_empty() || c.result.as_ref() != Some(&c.production)
        })
        || requirements
            .targets
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>()
            != p.production_targets
    {
        return Err(Error::new(
            "no_release_gate",
            "no-release-required needs the current integrated outcome and its exact human acceptance or published carry with no selected change",
        ));
    }
    let mut seen = BTreeSet::new();
    for target in &record.targets {
        target.source.validate()?;
        validate_digest(&target.artifact_digest)?;
        if !seen.insert(target.target.clone())
            || target.provider_record.is_empty()
            || target.provider.is_empty()
            || target.observed_at > record.recorded_at
            || requirements.targets.get(&target.target)
                != Some(&(target.provider.clone(), target.repository.clone()))
            || !p.baselines.iter().any(|b| {
                b.repository == target.repository
                    && !b.deployment_free
                    && b.production == target.source
                    && b.source_for_target(&target.target) == Some(target.source.commit.as_str())
                    && b.artifact_for_target(&target.target)
                        == Some(target.artifact_digest.as_str())
            })
        {
            return Err(Error::new(
                "no_release_gate",
                "every production target must already serve the exact validated baseline artifact",
            ));
        }
    }
    if seen != p.production_targets
        || p.baselines.iter().any(|b| {
            let configured: BTreeSet<_> = requirements
                .targets
                .iter()
                .filter(|(_, (_, repository))| repository == &b.repository)
                .map(|(target, _)| target.clone())
                .collect();
            if b.deployment_free {
                !configured.is_empty()
            } else {
                configured.is_empty()
                    || (!b.target_artifacts.is_empty()
                        && b.target_artifacts.keys().cloned().collect::<BTreeSet<_>>()
                            != configured)
            }
        })
    {
        return Err(Error::new(
            "no_release_gate",
            "a production target is missing or a required deployment remains unfinished",
        ));
    }
    Ok(())
}

pub fn prepare_no_release_required(
    record: NoReleaseRequired,
    parent: &VerifiedParent,
    acceptance: &VerifiedAcceptance,
    requirements: NoReleaseRequirements<'_>,
    key: &AttestationKey,
) -> Result<Signed<NoReleaseRequired>> {
    prepare_no_release_required_with_acceptance(
        record,
        parent,
        NoReleaseAcceptance {
            acceptance,
            carry: None,
        },
        requirements,
        key,
    )
}
pub(crate) fn prepare_no_release_required_with_acceptance(
    record: NoReleaseRequired,
    parent: &VerifiedParent,
    authority: NoReleaseAcceptance<'_>,
    requirements: NoReleaseRequirements<'_>,
    key: &AttestationKey,
) -> Result<Signed<NoReleaseRequired>> {
    validate_no_release_required(&record, parent, authority, requirements)?;
    key.sign("no_release_required", &record)
}

pub fn verify_no_release_required(
    signed: &Signed<NoReleaseRequired>,
    parent: &VerifiedParent,
    acceptance: &VerifiedAcceptance,
    requirements: NoReleaseRequirements<'_>,
    publication: &Publication,
    writer: &str,
    key: &AttestationKey,
) -> Result<VerifiedNoReleaseRequired> {
    verify_no_release_required_with_acceptance(
        signed,
        parent,
        NoReleaseAcceptance {
            acceptance,
            carry: None,
        },
        requirements,
        publication,
        writer,
        key,
    )
}
pub(crate) fn verify_no_release_required_with_acceptance(
    signed: &Signed<NoReleaseRequired>,
    parent: &VerifiedParent,
    authority: NoReleaseAcceptance<'_>,
    requirements: NoReleaseRequirements<'_>,
    publication: &Publication,
    writer: &str,
    key: &AttestationKey,
) -> Result<VerifiedNoReleaseRequired> {
    key.verify("no_release_required", signed)?;
    validate_no_release_required(&signed.payload, parent, authority, requirements)?;
    publication.verify(signed, "linear", writer)?;
    Ok(VerifiedNoReleaseRequired {
        record: signed.payload.clone(),
        digest: signed.digest()?,
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
        || candidate.ordered_delta_digests.is_empty()
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
        let reviewed = parent
            .source_review_digests
            .get(delta_digest)
            .ok_or_else(|| {
                Error::new(
                    "derived_review_gate",
                    "selected artifact lacks its verified child review binding",
                )
            })?;
        let matching: Vec<&VerifiedReview> = sources
            .iter()
            .filter(|r| {
                &r.digest == reviewed
                    && r.attestation.work.parent == parent.manifest.work.parent
                    && r.attestation.revision.repository == candidate.repository
                    && r.attestation.verdict == Verdict::Pass
                    && r.attestation.unresolved_findings.is_empty()
                    && matches!(
                        r.attestation.review,
                        ReviewKind::Agent { .. } | ReviewKind::AgentValidation { .. }
                    )
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

#[cfg(test)]
mod baseline_revision_tests {
    use super::*;
    fn legacy() -> serde_json::Value {
        serde_json::json!({"repository":{"hostname":"github.com","id":"77"},
            "production":{"commit":"1111111111111111111111111111111111111111","tree":"2222222222222222222222222222222222222222"},
            "deployed_revision":"1111111111111111111111111111111111111111",
            "deployed_artifact":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "target_artifacts":{"production":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
            "deployment_free":false,"successor_recovery_operation":null})
    }
    #[test]
    fn absent_target_sources_preserve_original_signed_manifest_and_digest() {
        let value = legacy();
        let digest = canonical_digest(1, &value).unwrap();
        let baseline: ProductionBaseline = serde_json::from_value(value.clone()).unwrap();
        baseline.validate().unwrap();
        assert!(baseline.target_revisions.is_empty());
        assert_eq!(serde_json::to_value(&baseline).unwrap(), value);
        assert_eq!(canonical_digest(1, &baseline).unwrap(), digest);
        let key = AttestationKey::new("legacy-baseline".into(), vec![25; 32]).unwrap();
        let signed = key.sign("legacy_baseline", &value).unwrap();
        let recovered: Signed<ProductionBaseline> =
            serde_json::from_value(serde_json::to_value(signed).unwrap()).unwrap();
        key.verify("legacy_baseline", &recovered).unwrap();
    }
    #[test]
    fn mixed_target_sources_are_complete_and_never_an_ordinary_baseline() {
        let mut baseline: ProductionBaseline = serde_json::from_value(legacy()).unwrap();
        baseline.deployed_revision = None;
        baseline.target_revisions = BTreeMap::from([
            ("production".into(), "1".repeat(40)),
            ("secondary".into(), "3".repeat(40)),
        ]);
        baseline.target_artifacts.insert(
            "secondary".into(),
            baseline.deployed_artifact.clone().unwrap(),
        );
        assert!(baseline.validate().is_err());
        baseline.successor_recovery_operation = Some(Uuid::from_u128(20));
        baseline.validate().unwrap();
        assert_eq!(
            baseline.source_for_target("secondary"),
            Some("3333333333333333333333333333333333333333")
        );
        baseline.target_revisions.remove("secondary");
        assert!(baseline.validate().is_err());
    }
}

#[cfg(test)]
#[path = "evidence_no_release_carry_tests.rs"]
mod no_release_carry_tests;
