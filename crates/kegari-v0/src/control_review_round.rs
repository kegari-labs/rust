//! Bounded receipt provenance, never execution authority.
//! No constructor here mints a permit,
//! joined completion, source proof, review result, or VerifiedReviewRoundPlan.
use super::{
    PreparationReference, Receipt, RunContinuationReference, RunReference, SourceAdvancedReference,
};
use crate::{evidence::validate_digest, scheduler::PhaseKey};
use kegari_digest::canonical_digest;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};

const KEY: &str = "review_round";
const PREPARATION_LINK: &str = "review_round_preparation";
const MAX_ENCODED_BYTES: usize = 16 * 1024;
// Ordinals are bounded by checked u32 arithmetic and actual configured caps.
// Resource bounds apply to encoded records and receipt inventory, not a new round cap.

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewReceiptReference {
    pub(crate) operation: uuid::Uuid,
    pub(crate) receipt_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewedSourceReference {
    #[serde(with = "strict_phase")]
    pub(crate) implementation_phase: PhaseKey,
    pub(crate) implementation: ReviewReceiptReference,
    pub(crate) claim_digest: String,
    pub(crate) seal: ReviewReceiptReference,
    // Digest of the opaque DeliverySource, not an inferred checkout path.
    pub(crate) source_digest: String,
    pub(crate) repository_hostname: String,
    pub(crate) repository_id: String,
    pub(crate) commit: String,
    pub(crate) tree: String,
    /// Present only for an actual completed admitted paired resume. Omission
    /// preserves the original direct-source canonical bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) paired_resume: Option<super::PairedResumeResultReference>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindingsReference {
    pub(crate) review: ReviewReceiptReference,
    pub(crate) round_id: String,
    pub(crate) round_ordinal: u32,
    pub(crate) round_reference_digest: String,
    pub(crate) outcome_proposal: uuid::Uuid,
    pub(crate) outcome_claim_digest: String,
    pub(crate) reviewed_source_digest: String,
}

/// Data-only exact failed-validation provenance within an existing Findings/fix
/// pair. The opaque producer must additionally hold the actual failed check
/// capabilities, complete source/assignment and joined completion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairValidationReference {
    pub(crate) validation: ReviewReceiptReference,
    #[serde(with = "strict_phase")]
    pub(crate) phase: PhaseKey,
    pub(crate) source_advanced: SourceAdvancedReference,
    pub(crate) outcome_proposal: uuid::Uuid,
    pub(crate) outcome_claim_digest: String,
    pub(crate) failure_inventory_digest: String,
    /// Missing means the original direct SourceAdvanced shape. Omission keeps
    /// its canonical serialized bytes and digest unchanged.
    #[serde(default, skip_serializing_if = "RepairValidationLineage::is_direct")]
    pub(crate) lineage: RepairValidationLineage,
}

/// A receipt-only prefix, never a retry/failed-check/source capability. The
/// current producer retains the actual completed validation and its opaque
/// SourceAdvanced or CompletedGrantedValidation basis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RepairValidationLineage {
    Direct {},
    /// A genuine completed SourceAdvanced validator in an already granted
    /// validation budget. Its actual admission remains the source admission;
    /// it is never relabelled as the completed result of a retry capability.
    SourceAdvanced {
        source_budget_digest: String,
        native_target: String,
        inventory: std::collections::BTreeMap<uuid::Uuid, String>,
        grants: std::collections::BTreeMap<uuid::Uuid, String>,
    },
    Retried {
        origin: Box<ReviewReceiptReference>,
        #[serde(with = "strict_phase")]
        origin_phase: PhaseKey,
        completed_retry_digest: String,
        /// Actual provider target derived from the verified native WorkIdentity;
        /// it is distinct from the opaque canonical PhaseKey work digest.
        native_target: String,
        /// Entire validation inventory at consumption, across grant namespaces.
        inventory: std::collections::BTreeMap<uuid::Uuid, String>,
        /// Whole actual publication receipts for every referenced grant.
        grants: std::collections::BTreeMap<uuid::Uuid, String>,
    },
}
impl Default for RepairValidationLineage {
    fn default() -> Self {
        Self::Direct {}
    }
}
impl RepairValidationLineage {
    pub(crate) fn is_direct(&self) -> bool {
        matches!(self, Self::Direct {})
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransientFailureCode {
    ServerOverloaded,
    InternalServerError,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewRoundCause {
    Initial {},
    TransientRetry {
        predecessor: ReviewReceiptReference,
        prior_round_reference_digest: String,
        // Exact whole retained SessionState and adapter-derived classification;
        // hashes are references, not independently minted retry authority.
        terminal_state_digest: String,
        classified_failure_digest: String,
        classification: TransientFailureCode,
    },
    PairedRepair {
        findings: FindingsReference,
    },
    /// Another implementation session justified by actual failed local checks
    /// on the latest completed repair. It retains, never allocates, the round.
    RepairContinuation {
        findings: FindingsReference,
        previous_round_reference_digest: String,
        current_source: Box<ReviewedSourceReference>,
        validation: Box<RepairValidationReference>,
    },
    AfterRepair {
        findings: FindingsReference,
        repair: ReviewReceiptReference,
        paired_round_reference_digest: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRoundReference {
    pub(crate) schema_version: u32,
    pub(crate) runtime: uuid::Uuid,
    pub(crate) workspace: uuid::Uuid,
    pub(crate) account: String,
    #[serde(with = "strict_phase")]
    pub(crate) review_phase: PhaseKey,
    pub(crate) round_id: String,
    pub(crate) ordinal: u32,
    pub(crate) round_limit: u32,
    pub(crate) source: ReviewedSourceReference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) passed_validation: Option<PassedValidationReference>,
    pub(crate) cause: ReviewRoundCause,
}

/// Exact passed-check history references, never a check or execution capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassedValidationReference {
    pub(crate) schema_version: u32,
    pub(crate) source: ReviewedSourceReference,
    pub(crate) validation: ReviewReceiptReference,
    pub(crate) completed_validation_digest: String,
    pub(crate) local_pass_digest: String,
    pub(crate) retained_receipts: std::collections::BTreeMap<uuid::Uuid, String>,
}
impl PassedValidationReference {
    pub(crate) fn validate_for_round(&self, round: &ReviewRoundReference) -> Result<()> {
        self.source.validate(&round.review_phase)?;
        self.validation.validate()?;
        validate_digest(&self.completed_validation_digest)?;
        validate_digest(&self.local_pass_digest)?;
        if self.schema_version != 1
            || self.source != round.source
            || self.retained_receipts.is_empty()
        {
            return Err(invalid("passed validation changes exact reviewed source"));
        }
        for reference in [
            &self.validation,
            &self.source.implementation,
            &self.source.seal,
        ] {
            if self.retained_receipts.get(&reference.operation) != Some(&reference.receipt_digest) {
                return Err(invalid(
                    "passed validation omits exact validation/source reference",
                ));
            }
        }
        for (id, digest) in &self.retained_receipts {
            if id.is_nil() {
                return Err(invalid("nil passed-validation receipt reference"));
            }
            validate_digest(digest)?;
        }
        Ok(())
    }
}

fn invalid(message: &str) -> Error {
    Error::new("record_corrupt", message)
}
fn identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.trim() == value
        && !value.chars().any(char::is_control)
}
fn phase(value: &PhaseKey, expected: &str) -> Result<()> {
    if !identifier(&value.work, 512)
        || value.phase != expected
        || value.retry_grant.is_some_and(|id| id.is_nil())
    {
        return Err(invalid("review round has an invalid phase identity"));
    }
    validate_digest(&value.generation)
}
fn oid(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl ReviewReceiptReference {
    fn validate(&self) -> Result<()> {
        if self.operation.is_nil() {
            return Err(invalid("review round references a nil receipt"));
        }
        validate_digest(&self.receipt_digest)
    }
    /// Bounded data decoder for the canonical preparation of this actual run.
    /// This is not evidence that the referenced receipt exists or has exited.
    pub(crate) fn preparation_for(run: &Receipt) -> Result<Self> {
        let encoded = run
            .expected
            .get(PREPARATION_LINK)
            .ok_or_else(|| invalid("round run has no preparation reference"))?;
        if encoded.len() > 4096 {
            return Err(invalid("round preparation reference exceeds bound"));
        }
        let value: Self = serde_json::from_str(encoded)
            .map_err(|_| invalid("round preparation reference is malformed"))?;
        value.validate()?;
        if value.operation != super::preparation_operation(run.operation_id)? {
            return Err(invalid(
                "round preparation reference changes worker identity",
            ));
        }
        Ok(value)
    }
    /// Exact data comparison only. The caller must obtain `observed` from the
    /// protected receipt store and independently require a terminal owned exit.
    pub(crate) fn matches_observation(&self, observed: &Receipt) -> Result<()> {
        self.validate()?;
        if observed.operation_id != self.operation
            || canonical_digest(1, observed)? != self.receipt_digest
        {
            return Err(invalid("review round receipt reference changed"));
        }
        Ok(())
    }
}
impl ReviewedSourceReference {
    fn validate(&self, review: &PhaseKey) -> Result<()> {
        if let Some(reference) = &self.paired_resume {
            reference.validate()?;
            if reference.terminal != self.implementation {
                return Err(invalid(
                    "resumed source changes its actual terminal implementation",
                ));
            }
        }

        phase(&self.implementation_phase, "implementation")?;
        self.implementation.validate()?;
        self.seal.validate()?;
        for digest in [&self.claim_digest, &self.source_digest] {
            validate_digest(digest)?;
        }
        let repository = self.repository_id.parse::<u64>().ok();
        if self.implementation_phase.work != review.work
            || self.implementation_phase.generation != review.generation
            || self.implementation.operation == self.seal.operation
            || !identifier(&self.repository_hostname, 253)
            || self.repository_hostname != self.repository_hostname.to_ascii_lowercase()
            || !self
                .repository_hostname
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
            || repository.is_none_or(|id| id == 0 || id.to_string() != self.repository_id)
            || !oid(&self.commit)
            || !oid(&self.tree)
        {
            return Err(invalid(
                "reviewed source identity differs from the exact work generation",
            ));
        }
        Ok(())
    }
}
impl RepairValidationReference {
    pub(crate) fn source_admission(&self) -> &ReviewReceiptReference {
        match &self.lineage {
            RepairValidationLineage::Direct {} | RepairValidationLineage::SourceAdvanced { .. } => {
                &self.validation
            }
            RepairValidationLineage::Retried { origin, .. } => origin,
        }
    }
    pub(crate) fn source_phase(&self) -> &PhaseKey {
        match &self.lineage {
            RepairValidationLineage::Direct {} | RepairValidationLineage::SourceAdvanced { .. } => {
                &self.phase
            }
            RepairValidationLineage::Retried { origin_phase, .. } => origin_phase,
        }
    }
    fn validate(
        &self,
        round: &ReviewRoundReference,
        source: &ReviewedSourceReference,
    ) -> Result<()> {
        self.validation.validate()?;
        phase(&self.phase, "child_local_validation")?;
        validate_source_advanced_shape(&self.source_advanced)?;
        validate_digest(&self.outcome_claim_digest)?;
        validate_digest(&self.failure_inventory_digest)?;
        match &self.lineage {
            RepairValidationLineage::Direct {} => (),
            RepairValidationLineage::SourceAdvanced {
                source_budget_digest,
                native_target,
                inventory,
                grants,
            } => {
                validate_digest(source_budget_digest)?;
                if source_budget_digest != &self.source_advanced.trigger_digest
                    || self.phase.retry_grant.is_none()
                    || !native_target.starts_with("linear:")
                    || native_target.len() > 4096
                    || inventory.is_empty()
                    || grants.is_empty()
                    || inventory.get(&self.validation.operation)
                        != Some(&self.validation.receipt_digest)
                    || inventory.get(&self.source_advanced.predecessor)
                        != Some(&self.source_advanced.predecessor_receipt_digest)
                    || self
                        .phase
                        .retry_grant
                        .is_none_or(|id| !grants.contains_key(&id))
                {
                    return Err(invalid(
                        "advanced validation loses its exact source budget or receipt prefix",
                    ));
                }
                for (operation, digest) in inventory.iter().chain(grants) {
                    if operation.is_nil() {
                        return Err(invalid("nil validation lineage receipt"));
                    }
                    validate_digest(digest)?;
                }
            }
            RepairValidationLineage::Retried {
                origin,
                origin_phase,
                completed_retry_digest,
                native_target,
                inventory,
                grants,
            } => {
                origin.validate()?;
                phase(origin_phase, "child_local_validation")?;
                validate_digest(completed_retry_digest)?;
                if origin.operation == self.validation.operation
                    || origin_phase.work != self.phase.work
                    || origin_phase.generation != self.phase.generation
                    || self.phase.retry_grant.is_none()
                    || !native_target.starts_with("linear:")
                    || native_target.len() > 4096
                    || inventory.is_empty()
                    || grants.is_empty()
                    || inventory.get(&origin.operation) != Some(&origin.receipt_digest)
                    || inventory.get(&self.validation.operation)
                        != Some(&self.validation.receipt_digest)
                    || self
                        .phase
                        .retry_grant
                        .is_none_or(|id| !grants.contains_key(&id))
                {
                    return Err(invalid(
                        "retried validation loses original source or completed receipt",
                    ));
                }
                for (operation, digest) in inventory.iter().chain(grants) {
                    if operation.is_nil() {
                        return Err(invalid("nil validation lineage receipt"));
                    }
                    validate_digest(digest)?;
                }
            }
        }
        if self.outcome_proposal.is_nil()
            || self.phase.work != round.review_phase.work
            || self.phase.generation != round.review_phase.generation
            || self.source_advanced.source_operation != source.implementation.operation
            || self.source_advanced.source_receipt_digest != source.implementation.receipt_digest
            || self.validation.operation == source.implementation.operation
            || self.validation.operation == source.seal.operation
            || self.validation.operation == self.source_advanced.predecessor
        {
            return Err(invalid(
                "pair continuation validation does not check the exact latest repair",
            ));
        }
        Ok(())
    }
}
impl FindingsReference {
    fn validate(&self) -> Result<()> {
        self.review.validate()?;
        for digest in [
            &self.round_id,
            &self.round_reference_digest,
            &self.outcome_claim_digest,
            &self.reviewed_source_digest,
        ] {
            validate_digest(digest)?;
        }
        if self.outcome_proposal.is_nil() || self.round_ordinal == 0 {
            return Err(invalid(
                "Findings reference lacks its exact round/outcome identity",
            ));
        }
        Ok(())
    }
}
impl ReviewRoundReference {
    // Stable across transient attempts, changed per-session operation IDs and
    // cap reductions. Grant identity is included through the complete PhaseKey.
    pub(crate) fn expected_round_id(&self) -> Result<String> {
        canonical_digest(
            1,
            &serde_json::json!({
                "purpose":"child_review_round", "runtime":self.runtime,
                "workspace":self.workspace,"account":self.account,
                "review_phase":self.review_phase,"ordinal":self.ordinal,"source":self.source
            }),
        )
    }
    pub(crate) fn subject_phase(&self) -> &PhaseKey {
        match &self.cause {
            ReviewRoundCause::PairedRepair { .. } | ReviewRoundCause::RepairContinuation { .. } => {
                &self.source.implementation_phase
            }
            _ => &self.review_phase,
        }
    }
    /// Original reviewed source remains part of stable round identity. The
    /// implementation/source checks for a continuation use this actual latest
    /// repaired source instead; no caller may substitute the first source.
    pub(crate) fn dispatch_source(&self) -> &ReviewedSourceReference {
        match &self.cause {
            ReviewRoundCause::RepairContinuation { current_source, .. } => current_source,
            _ => &self.source,
        }
    }
    pub(crate) fn repair_findings(&self) -> Option<&FindingsReference> {
        match &self.cause {
            ReviewRoundCause::PairedRepair { findings }
            | ReviewRoundCause::RepairContinuation { findings, .. } => Some(findings),
            _ => None,
        }
    }
    pub(crate) fn is_repair(&self) -> bool {
        matches!(
            &self.cause,
            ReviewRoundCause::PairedRepair { .. } | ReviewRoundCause::RepairContinuation { .. }
        )
    }
    pub(crate) fn validate(&self) -> Result<()> {
        // Host-produced metadata must be readable by the same bounded parser;
        // report the finite storage prerequisite before any admission is written.
        if serde_json::to_vec(self)
            .map_err(|_| invalid("review round serialization failed"))?
            .len()
            > MAX_ENCODED_BYTES
        {
            return Err(Error::new(
                "review_round_history_limit",
                "review round references exceed the protected encoded-record bound",
            ));
        }
        phase(&self.review_phase, "child_codex_review")?;
        self.source.validate(&self.review_phase)?;
        if let Some(passed) = &self.passed_validation {
            passed.validate_for_round(self)?;
        }
        if self.schema_version != 1
            || self.runtime.is_nil()
            || self.workspace.is_nil()
            || !identifier(&self.account, 512)
            || self.ordinal == 0
            || self.round_limit == 0
            || self.ordinal > self.round_limit
            || self.round_id != self.expected_round_id()?
        {
            return Err(invalid("invalid bounded review round identity"));
        }
        match &self.cause {
            ReviewRoundCause::Initial {} if self.ordinal != 1 => {
                return Err(invalid("initial review must establish round one"));
            }
            ReviewRoundCause::Initial {} => (),
            ReviewRoundCause::TransientRetry {
                predecessor,
                prior_round_reference_digest,
                terminal_state_digest,
                classified_failure_digest,
                ..
            } => {
                predecessor.validate()?;
                for digest in [
                    prior_round_reference_digest,
                    terminal_state_digest,
                    classified_failure_digest,
                ] {
                    validate_digest(digest)?;
                }
            }
            ReviewRoundCause::PairedRepair { findings } => {
                findings.validate()?;
                if findings.round_id != self.round_id
                    || findings.round_ordinal != self.ordinal
                    || findings.reviewed_source_digest != self.source.source_digest
                {
                    return Err(invalid(
                        "paired repair does not retain the exact Findings round/source",
                    ));
                }
            }
            ReviewRoundCause::RepairContinuation {
                findings,
                previous_round_reference_digest,
                current_source,
                validation,
            } => {
                findings.validate()?;
                validate_digest(previous_round_reference_digest)?;
                current_source.validate(&self.review_phase)?;
                validation.validate(self, current_source)?;
                if findings.round_id != self.round_id
                    || findings.round_ordinal != self.ordinal
                    || findings.reviewed_source_digest != self.source.source_digest
                    || current_source.implementation_phase != self.source.implementation_phase
                    || current_source.repository_hostname != self.source.repository_hostname
                    || current_source.repository_id != self.source.repository_id
                    || current_source.implementation.operation
                        == self.source.implementation.operation
                    || validation.validation.operation == findings.review.operation
                {
                    return Err(invalid(
                        "repair continuation resets the original pair or implementation namespace",
                    ));
                }
            }
            ReviewRoundCause::AfterRepair {
                findings,
                repair,
                paired_round_reference_digest,
            } => {
                findings.validate()?;
                repair.validate()?;
                validate_digest(paired_round_reference_digest)?;
                if findings.round_ordinal.checked_add(1) != Some(self.ordinal)
                    || findings.round_id == self.round_id
                    || findings.reviewed_source_digest == self.source.source_digest
                    || repair != &self.source.implementation
                    || repair.operation == findings.review.operation
                {
                    return Err(invalid(
                        "new review lacks one exact completed paired repair and changed source",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Bounded admission/shape decoding only; does not call continuation or
    /// source-advanced decoders. Those readers may safely use this when selecting
    /// legacy versus versioned accounting, then validate cross-links separately.
    pub(crate) fn decode_admission(receipt: &Receipt) -> Result<Option<Self>> {
        if let Some(round) = super::round_recovery::paired_round(receipt)? {
            return Ok(Some(round));
        }
        let Some(encoded) = receipt.expected.get(KEY) else {
            if receipt.expected.contains_key(PREPARATION_LINK) {
                return Err(invalid("preparation link lacks its review round marker"));
            }
            return Ok(None);
        };
        if encoded.len() > MAX_ENCODED_BYTES {
            return Err(invalid("review round reference exceeds bound"));
        }
        let value: Self =
            serde_json::from_str(encoded).map_err(|_| invalid("invalid review round reference"))?;
        value.validate()?;
        if receipt.kind != "admission"
            || receipt.runtime_id != value.runtime
            || receipt.operation_id.is_nil()
            || receipt.request_id != receipt.operation_id
            || receipt.canonical_targets != [value.review_phase.work.clone()]
            || [
                "metadata",
                "release_resume",
                "session_resume",
                "session_recovery_preparation",
            ]
            .iter()
            .any(|key| receipt.expected.contains_key(*key))
        {
            return Err(invalid(
                "review round is attached to another admission identity",
            ));
        }
        validate_digest(
            receipt
                .expected
                .get("scope")
                .ok_or_else(|| invalid("round admission scope is missing"))?,
        )?;
        let run = RunReference::from_receipt(receipt)?;
        let prep = PreparationReference::from_receipt(receipt)?;
        let (workspace, account, subject, worker) = match (&run, &prep) {
            (Some(run), None) => {
                ReviewReceiptReference::preparation_for(receipt)?;
                // A paired repair retains the independent implementation caps;
                // its review cap belongs to review_phase, not this RunReference.
                if !value.is_repair() && value.round_limit > run.max_review_rounds {
                    return Err(invalid("round reference raises the admitted review cap"));
                }
                (
                    run.workspace,
                    &run.account,
                    &run.phase,
                    receipt.operation_id,
                )
            }
            (None, Some(prep)) => {
                if receipt.expected.contains_key(PREPARATION_LINK) {
                    return Err(invalid("preparation cannot carry a model preparation link"));
                }
                (
                    prep.workspace,
                    &prep.account,
                    &prep.phase,
                    prep.worker_operation,
                )
            }
            _ => {
                return Err(invalid(
                    "review round needs exactly one preparation or run identity",
                ));
            }
        };
        if workspace != value.workspace
            || account != &value.account
            || subject != value.subject_phase()
            || worker == value.source.implementation.operation
            || worker == value.source.seal.operation
            || worker == value.dispatch_source().implementation.operation
            || worker == value.dispatch_source().seal.operation
        {
            return Err(invalid(
                "review round differs from its subject worker/workspace/phase/account",
            ));
        }
        let previous = match &value.cause {
            ReviewRoundCause::TransientRetry { predecessor, .. } => Some(predecessor.operation),
            ReviewRoundCause::PairedRepair { findings }
            | ReviewRoundCause::AfterRepair { findings, .. } => Some(findings.review.operation),
            ReviewRoundCause::RepairContinuation { validation, .. } => {
                Some(validation.validation.operation)
            }
            ReviewRoundCause::Initial {} => None,
        };
        if previous == Some(worker) {
            return Err(invalid(
                "review round references its own worker as predecessor",
            ));
        }
        Ok(Some(value))
    }

    pub fn from_receipt(receipt: &Receipt) -> Result<Option<Self>> {
        let Some(value) = Self::decode_admission(receipt)? else {
            return Ok(None);
        };
        if receipt.expected.contains_key("review_round_recovery")
            || receipt.expected.contains_key("review_round_resume")
        {
            return Ok(Some(value));
        }
        if receipt.expected.contains_key("run") {
            // Parse structural links directly, never recurse through their
            // future round-aware readers. Budget/whole-receipt/closure checks
            // belong to the serialized Control verifier, not this data reader.
            let continued = decode_continuation(receipt)?;
            let advanced = decode_source_advanced(receipt)?;
            match (&value.cause, continued, advanced) {
                (
                    ReviewRoundCause::Initial {} | ReviewRoundCause::TransientRetry { .. },
                    None,
                    None,
                ) => (),
                (ReviewRoundCause::PairedRepair { findings }, Some(link), None)
                    if link.failure_operation == findings.review.operation
                        && link.predecessor == value.source.implementation.operation
                        && link.predecessor_receipt_digest
                            == value.source.implementation.receipt_digest => {}
                (
                    ReviewRoundCause::RepairContinuation {
                        current_source,
                        validation,
                        ..
                    },
                    Some(link),
                    None,
                ) if link.predecessor == current_source.implementation.operation
                    && link.predecessor_receipt_digest
                        == current_source.implementation.receipt_digest
                    && link.failure_operation == validation.validation.operation => {}
                (
                    ReviewRoundCause::AfterRepair {
                        findings, repair, ..
                    },
                    None,
                    Some(link),
                ) if link.predecessor == findings.review.operation
                    && link.predecessor_receipt_digest == findings.review.receipt_digest
                    && link.source_operation == repair.operation
                    && link.source_receipt_digest == repair.receipt_digest => {}
                _ => {
                    return Err(invalid(
                        "review round cause differs from the atomic continuation/source provenance",
                    ));
                }
            }
        }
        Ok(Some(value))
    }
}

fn validate_continuation_shape(value: &RunContinuationReference) -> Result<()> {
    if value.predecessor.is_nil() || value.failure_operation.is_nil() {
        return Err(invalid("round continuation has a nil operation"));
    }
    for digest in [
        &value.predecessor_receipt_digest,
        &value.trigger_digest,
        &value.progress_digest,
    ] {
        validate_digest(digest)?;
    }
    Ok(())
}
fn decode_continuation(receipt: &Receipt) -> Result<Option<RunContinuationReference>> {
    receipt
        .expected
        .get("continuation")
        .map(|encoded| {
            if encoded.len() > MAX_ENCODED_BYTES {
                return Err(invalid("continuation exceeds bound"));
            }
            let value: RunContinuationReference =
                serde_json::from_str(encoded).map_err(|_| invalid("invalid round continuation"))?;
            validate_continuation_shape(&value)?;
            Ok(value)
        })
        .transpose()
}
fn validate_source_advanced_shape(value: &SourceAdvancedReference) -> Result<()> {
    if value.predecessor.is_nil()
        || value.source_operation.is_nil()
        || value.predecessor == value.source_operation
    {
        return Err(invalid(
            "source advancement has invalid operation identities",
        ));
    }
    for digest in [
        &value.predecessor_receipt_digest,
        &value.source_receipt_digest,
        &value.trigger_digest,
        &value.progress_digest,
    ] {
        validate_digest(digest)?;
    }
    Ok(())
}
fn decode_source_advanced(receipt: &Receipt) -> Result<Option<SourceAdvancedReference>> {
    receipt
        .expected
        .get("source_advanced")
        .map(|encoded| {
            if encoded.len() > MAX_ENCODED_BYTES {
                return Err(invalid("source advancement exceeds bound"));
            }
            let value: SourceAdvancedReference = serde_json::from_str(encoded)
                .map_err(|_| invalid("invalid round source advancement"))?;
            validate_source_advanced_shape(&value)?;
            Ok(value)
        })
        .transpose()
}

// PhaseKey is a shared type without deny_unknown_fields. Decode this reference
// strictly so an extra generation/grant field cannot silently be ignored.
mod strict_phase {
    use super::*;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Fields {
        work: String,
        generation: String,
        phase: String,
        retry_grant: Option<uuid::Uuid>,
    }
    pub fn serialize<S: serde::Serializer>(
        value: &PhaseKey,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        value.serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<PhaseKey, D::Error> {
        let value = Fields::deserialize(deserializer)?;
        Ok(PhaseKey {
            work: value.work,
            generation: value.generation,
            phase: value.phase,
            retry_grant: value.retry_grant,
        })
    }
}

#[cfg(test)]
#[path = "control_review_round_tests.rs"]
mod tests;
