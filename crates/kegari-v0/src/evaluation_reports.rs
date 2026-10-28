//! Read-only evidence inspection. Comparisons retain every declared attempt and
//! never turn a later pass, a replay, or a human-review placeholder into readiness.
use crate::evaluation::{self, EvidenceClass, SuiteManifest, TrialOutcome, TrialReport};
use kegari_digest::canonical_digest;
use kegari_storage::PrivateDir;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub struct ReportArchive {
    root: PathBuf,
    suite: SuiteManifest,
    reports: Vec<TrialReport>,
    /// In-memory interruption/missing-result observations; never written back.
    reconstructed: Vec<String>,
}
impl ReportArchive {
    pub fn open(root: &Path) -> Result<Self> {
        let directory = PrivateDir::open(root)?;
        let mut remaining = 256 * 1024 * 1024;
        let suite: SuiteManifest = read(&directory, "suite.json", &mut remaining)?;
        let plan = suite.archive_plan()?;
        let suite_digest = suite.digest()?;
        let mut allowed = BTreeSet::from(["suite.json".to_owned()]);
        for key in &plan {
            let name = key.name()?;
            allowed.insert(format!("{name}.started.json"));
            allowed.insert(format!("{name}.result.json"));
        }
        let files: BTreeSet<_> = directory
            .records_bounded(allowed.len() + 1)?
            .into_iter()
            .collect();
        if !files.is_subset(&allowed) {
            return Err(Error::new(
                "evaluation_record_conflict",
                "report directory contains undeclared records; preserve and inspect them",
            ));
        }
        let mut reports = Vec::new();
        let mut reconstructed = Vec::new();
        for key in plan {
            let name = key.name()?;
            let variant = suite
                .variants
                .iter()
                .find(|v| v.case_id == key.case_id && v.id == key.variant)
                .ok_or_else(|| Error::invalid("archive trial has no declared variant"))?;
            let started = files.contains(&format!("{name}.started.json"));
            let started_at = if started {
                let value: Value =
                    read(&directory, &format!("{name}.started.json"), &mut remaining)?;
                if value.get("trial")
                    != Some(
                        &serde_json::to_value(&key)
                            .map_err(|_| Error::invalid("trial encoding"))?,
                    )
                    || value["started_at"]
                        .as_str()
                        .is_none_or(|v| chrono::DateTime::parse_from_rfc3339(v).is_err())
                    || value["outcome"] != "interrupted_unless_result_present"
                    || value["remote_effects"] != "unknown_until_reconciled"
                {
                    return Err(Error::new(
                        "evaluation_record_conflict",
                        "started record differs from its declared trial",
                    ));
                }
                Some(
                    chrono::DateTime::parse_from_rfc3339(value["started_at"].as_str().unwrap())
                        .map_err(|_| Error::invalid("invalid report start time"))?,
                )
            } else {
                None
            };
            let report = if files.contains(&format!("{name}.result.json")) {
                let report: TrialReport =
                    read(&directory, &format!("{name}.result.json"), &mut remaining)?;
                let profile = suite
                    .profiles
                    .iter()
                    .find(|p| p.name == key.profile)
                    .and_then(|p| p.binding.as_ref());
                if report.schema_version != evaluation::SUITE_SCHEMA
                    || report.trial != key
                    || report.suite_digest != suite_digest
                    || report.fixture_digest != variant.fixture_digest
                    || report.scorer_digest != variant.scorer_digest
                    || report.grader_version != suite.grader_version
                    || report.codex_version != suite.codex_version
                    || report.turns_started > variant.bounds.max_turns
                    || (report.evidence_class == EvidenceClass::ActualAgent
                        && (!started
                            || serde_json::to_value(&report.profile).ok()
                                != serde_json::to_value(profile).ok()))
                    || (report.grading_error.is_some()
                        && (report.grade.is_some()
                            || matches!(
                                report.outcome,
                                TrialOutcome::DeterministicPassHumanPending | TrialOutcome::Skipped
                            )))
                    || (report.outcome == TrialOutcome::DeterministicPassHumanPending
                        && (report.grade.as_ref().is_none_or(|g| {
                            !g.deterministic_pass
                                || g.assertions.is_empty()
                                || g.assertions.iter().any(|a| !a.passed)
                        }) || report.unresolved_remote_effects))
                {
                    return Err(Error::new(
                        "evaluation_record_conflict",
                        "result provenance, bound or declared grade is inconsistent",
                    ));
                }
                if let Some(output) = &report.canary_output {
                    output.validate_report(&report, variant)?;
                }
                if let Some(canary) = &report.canary_diagnostic {
                    canary.validate_report(&report, variant, profile)?;
                }
                if report.evidence_class == EvidenceClass::ActualAgent {
                    validate_actual_report(&report, variant, started_at.as_ref().unwrap())?;
                }
                if let Some(grade) = &report.grade {
                    let expected: BTreeSet<_> = variant
                        .expected_outcomes
                        .iter()
                        .map(String::as_str)
                        .chain([
                            "runtime_scope",
                            "prohibited_effects",
                            "actual_candidate_provenance",
                        ])
                        .collect();
                    let observed: BTreeSet<_> =
                        grade.assertions.iter().map(|a| a.id.as_str()).collect();
                    if observed != expected
                        || observed.len() != grade.assertions.len()
                        || grade.deterministic_pass != grade.assertions.iter().all(|a| a.passed)
                    {
                        return Err(Error::new(
                            "evaluation_record_conflict",
                            "grade omits, duplicates or contradicts declared assertions",
                        ));
                    }
                }
                report
            } else {
                reconstructed.push(name);
                let mut report = TrialReport::unrun(
                    &suite,
                    variant,
                    key,
                    if started {
                        TrialOutcome::Interrupted
                    } else {
                        TrialOutcome::Skipped
                    },
                    if started {
                        "recorded attempt has no durable result; reconcile before replacement"
                    } else {
                        "declared attempt has no recorded start or result"
                    },
                )?;
                report.unresolved_remote_effects = started;
                report.runtime_version = "unavailable: original result missing".into();
                report.runtime_build = "unavailable: original result missing".into();
                report.grader_version = suite.grader_version.clone();
                report.started_at = started_at.map(|time| time.to_rfc3339()).unwrap_or_default();
                report.ended_at.clear();
                report
            };
            reports.push(report);
        }
        directory.revalidate()?;
        Ok(Self {
            root: root.to_owned(),
            suite,
            reports,
            reconstructed,
        })
    }
    pub fn suite(&self) -> &SuiteManifest {
        &self.suite
    }
    pub fn reports(&self) -> &[TrialReport] {
        &self.reports
    }
    pub fn reconstructed(&self) -> &[String] {
        &self.reconstructed
    }
    pub fn inspection(&self) -> Result<Value> {
        let trials = self.reports.iter().map(|r| Ok(json!({
            "trial":r.trial,"evidence_class":r.evidence_class,"outcome":r.outcome,
            "reason":r.reason,"started_at":(!r.started_at.is_empty()).then_some(&r.started_at),"ended_at":(!r.ended_at.is_empty()).then_some(&r.ended_at),
            "elapsed_ms":(!self.reconstructed.contains(&r.trial.name()?)).then_some(r.elapsed_ms),"turns_started":r.turns_started,"usage_tokens":r.usage_tokens,
            "earlier_usage_unknown":r.earlier_usage_unknown,"unresolved_remote_effects":r.unresolved_remote_effects,
            "grading_error":r.grading_error,"observed_effects":r.observed_effects,
            "runtime_build":r.runtime_build,"review_claim":r.review_claim,"independent_boundary":r.independent_boundary,"independent_broker":r.independent_broker,"canary_diagnostic":r.canary_diagnostic,"canary_output":r.canary_output,"declared_protocol":r.declared_protocol,"instruction_digest":r.instruction_manifest.as_ref().map(|m|canonical_digest(1,m)).transpose()?
        }))).collect::<Result<Vec<_>>>()?;
        Ok(
            json!({"schema_version":1,"report_root":self.root,"suite_digest":self.suite.digest()?,
            "assessment":evaluation::assess(&self.suite,&self.reports)?,"reconstructed":self.reconstructed,"trials":trials,
            "qualification":"not_granted; enforcement/integration evidence and independent human review remain separate"}),
        )
    }
}
fn validate_actual_report(
    report: &TrialReport,
    variant: &evaluation::Variant,
    durable_started: &chrono::DateTime<chrono::FixedOffset>,
) -> Result<()> {
    let conflict = || {
        Error::new(
            "evaluation_record_conflict",
            "actual-agent label lacks complete bound runtime/instruction/tool provenance",
        )
    };
    let manifest: crate::instructions::InstructionManifest =
        serde_json::from_value(report.instruction_manifest.clone().ok_or_else(conflict)?)
            .map_err(|_| conflict())?;
    if let Some(observation) = &report.independent_broker {
        observation.validate()?;
        if manifest.role != crate::instructions::AgentRole::IndependentReview
            || observation.operation != manifest.operation_id
            || observation.workspace != manifest.workspace_id
            || observation.suite_digest != report.suite_digest
            || observation.fixture_digest != report.fixture_digest
            || observation.context_digest != manifest.context_digest
            || Some(&observation.tool_contract_digest) != report.tool_contract_digest.as_ref()
            || observation.trial_digest != canonical_digest(1, &report.trial)?
            || observation.instruction_digest
                != canonical_digest(crate::instructions::ASSEMBLY_SCHEMA, &manifest)?
        {
            return Err(conflict());
        }
        if let Some(identity) = &observation.authority_at_collection
            && (manifest.repository_revision.as_deref() != Some(identity.source_revision.as_str())
                || identity.permission_digest != canonical_digest(1, &manifest.permissions)?)
        {
            return Err(conflict());
        }
    }
    if let Some(observation) = &report.declared_protocol {
        observation.validate()?;
        if (manifest.role != crate::instructions::AgentRole::IndependentReview
            && observation.state != kegari_codex::ProtocolCaptureState::NotDeclared)
            || observation.operation != manifest.operation_id
            || observation.workspace != manifest.workspace_id
            || observation.suite_digest != report.suite_digest
            || observation.fixture_digest != report.fixture_digest
            || observation
                .trial_digest
                .as_ref()
                .is_some_and(|d| canonical_digest(1, &report.trial).ok().as_ref() != Some(d))
            || observation.instruction_digest
                != canonical_digest(crate::instructions::ASSEMBLY_SCHEMA, &manifest)?
        {
            return Err(conflict());
        }
    }
    if let Some(boundary) = &report.independent_boundary {
        boundary.validate()?;
        if manifest.role != crate::instructions::AgentRole::IndependentReview {
            return Err(conflict());
        }
        if let evaluation::IndependentBoundaryCheck::MatchedAtCollection { identity } =
            &boundary.check
            && (manifest.repository_revision.as_deref() != Some(boundary.source_revision.as_str())
                || identity.operation != manifest.operation_id
                || identity.workspace != manifest.workspace_id
                || identity.instruction_manifest_digest
                    != canonical_digest(crate::instructions::ASSEMBLY_SCHEMA, &manifest)?
                || identity.context_digest != manifest.context_digest
                || identity.permission_digest != canonical_digest(1, &manifest.permissions)?
                || identity.source_revision != boundary.source_revision
                || Some(&identity.tool_contract_digest) != report.tool_contract_digest.as_ref())
        {
            return Err(conflict());
        }
    }
    if let Some(review) = &report.review_claim {
        review.validate().map_err(|_| conflict())?;
        if !matches!(
            manifest.role,
            crate::instructions::AgentRole::Review
                | crate::instructions::AgentRole::IndependentReview
        ) || review.operation != manifest.operation_id
            || Some(&review.source_revision) != manifest.repository_revision.as_ref()
            || review.instruction_digest
                != canonical_digest(crate::instructions::ASSEMBLY_SCHEMA, &manifest)?
            || Some(&review.tool_contract_digest) != report.tool_contract_digest.as_ref()
            || !report
                .evidence_references
                .contains(&review.outcome_reference)
        {
            return Err(conflict());
        }
    }
    let profile = report.profile.as_ref().ok_or_else(conflict)?;
    let definitions = report.tool_definitions.as_ref().ok_or_else(conflict)?;
    let started =
        chrono::DateTime::parse_from_rfc3339(&report.started_at).map_err(|_| conflict())?;
    let ended = chrono::DateTime::parse_from_rfc3339(&report.ended_at).map_err(|_| conflict())?;
    if manifest.runtime_version != report.runtime_version
        || manifest.runtime_build != report.runtime_build
        || manifest.codex_version != report.codex_version
        || manifest.profile != profile.profile
        || manifest.role != variant.role
        || manifest.phase != variant.phase
        || report
            .authority_reference
            .as_ref()
            .is_none_or(|reference| reference.trim().is_empty())
        || report
            .permission_configuration
            .as_ref()
            .is_none_or(|settings| !settings.is_object())
        || report.tool_contract_digest.as_ref() != Some(&canonical_digest(1, definitions)?)
        || crate::evidence::validate_digest(&report.input_digest).is_err()
        || report
            .trace
            .iter()
            .enumerate()
            .any(|(index, event)| event.sequence as usize != index)
        || started < *durable_started
        || ended < started
        || (report.outcome == TrialOutcome::DeterministicPassHumanPending
            && (report.turns_started == 0
                || report.trace.is_empty()
                || report.evidence_references.is_empty()))
    {
        return Err(conflict());
    }
    Ok(())
}
fn read<T: serde::de::DeserializeOwned>(
    directory: &PrivateDir,
    leaf: &str,
    remaining: &mut usize,
) -> Result<T> {
    if *remaining == 0 {
        return Err(Error::new(
            "evidence_limit",
            "evaluation archive exceeds aggregate read bound",
        ));
    }
    let bytes =
        directory.read_bytes(leaf, evaluation::MAX_EVIDENCE_BYTES.min(*remaining) as u64)?;
    *remaining -= bytes.len();
    serde_json::from_slice(&bytes).map_err(|_| {
        Error::new(
            "record_corrupt",
            "evaluation report JSON is invalid; preserve original bytes",
        )
    })
}

#[derive(Serialize, Deserialize)]
pub struct TrialComparison {
    pub trial: evaluation::TrialKey,
    pub comparable: bool,
    pub differences: Vec<String>,
    pub baseline_outcome: TrialOutcome,
    pub candidate_outcome: TrialOutcome,
    pub baseline_evidence: EvidenceClass,
    pub candidate_evidence: EvidenceClass,
    pub baseline_runtime_build: String,
    pub candidate_runtime_build: String,
    pub baseline_instruction_digest: Option<String>,
    pub candidate_instruction_digest: Option<String>,
    pub baseline_elapsed_ms: Option<u64>,
    pub candidate_elapsed_ms: Option<u64>,
    pub baseline_usage_tokens: Option<u64>,
    pub candidate_usage_tokens: Option<u64>,
    pub baseline_grading_error: Option<evaluation::GradingError>,
    pub candidate_grading_error: Option<evaluation::GradingError>,
    pub prior_failure_requires_resolution: bool,
}
pub fn compare(baseline: &ReportArchive, candidate: &ReportArchive) -> Result<Value> {
    let left: BTreeMap<_, _> = baseline
        .reports
        .iter()
        .map(|r| Ok((r.trial.name()?, r)))
        .collect::<Result<_>>()?;
    let right: BTreeMap<_, _> = candidate
        .reports
        .iter()
        .map(|r| Ok((r.trial.name()?, r)))
        .collect::<Result<_>>()?;
    let mut comparisons = Vec::new();
    let mut unmatched = Vec::new();
    let keys: BTreeSet<_> = left.keys().chain(right.keys()).cloned().collect();
    for name in keys {
        let (Some(a), Some(b)) = (left.get(&name), right.get(&name)) else {
            unmatched.push(name);
            continue;
        };
        let av = baseline
            .suite
            .variants
            .iter()
            .find(|v| v.case_id == a.trial.case_id && v.id == a.trial.variant)
            .unwrap();
        let bv = candidate
            .suite
            .variants
            .iter()
            .find(|v| v.case_id == b.trial.case_id && v.id == b.trial.variant)
            .unwrap();
        let mut differences = Vec::new();
        for (label, x, y) in [
            ("fixture", json!(a.fixture_digest), json!(b.fixture_digest)),
            ("scorer", json!(a.scorer_digest), json!(b.scorer_digest)),
            ("input", json!(a.input_digest), json!(b.input_digest)),
            (
                "permission_configuration",
                json!(a.permission_configuration),
                json!(b.permission_configuration),
            ),
            (
                "evidence_class",
                json!(a.evidence_class),
                json!(b.evidence_class),
            ),
            (
                "grader_version",
                json!(a.grader_version),
                json!(b.grader_version),
            ),
            ("profile", json!(a.profile), json!(b.profile)),
            (
                "declared_profile",
                json!(
                    baseline
                        .suite
                        .profiles
                        .iter()
                        .find(|p| p.name == a.trial.profile)
                ),
                json!(
                    candidate
                        .suite
                        .profiles
                        .iter()
                        .find(|p| p.name == b.trial.profile)
                ),
            ),
            (
                "codex_version",
                json!(a.codex_version),
                json!(b.codex_version),
            ),
            ("declared_conditions", json!(av), json!(bv)),
            (
                "tool_contract",
                json!(a.tool_contract_digest),
                json!(b.tool_contract_digest),
            ),
        ] {
            if x != y {
                differences.push(label.into());
            }
        }
        if baseline.reconstructed.contains(&name) || candidate.reconstructed.contains(&name) {
            differences.push("original_result_unavailable".into());
        }
        // Runtime/instruction changes are the intended comparison subject and
        // are shown explicitly; changed case/rubric/profile conditions are not
        // an apples-to-apples improvement claim.
        comparisons.push(TrialComparison {
            trial: a.trial.clone(),
            comparable: differences.is_empty(),
            differences,
            baseline_outcome: a.outcome.clone(),
            candidate_outcome: b.outcome.clone(),
            baseline_evidence: a.evidence_class.clone(),
            candidate_evidence: b.evidence_class.clone(),
            baseline_runtime_build: a.runtime_build.clone(),
            candidate_runtime_build: b.runtime_build.clone(),
            baseline_instruction_digest: a
                .instruction_manifest
                .as_ref()
                .map(|m| canonical_digest(1, m))
                .transpose()?,
            candidate_instruction_digest: b
                .instruction_manifest
                .as_ref()
                .map(|m| canonical_digest(1, m))
                .transpose()?,
            baseline_elapsed_ms: (!baseline.reconstructed.contains(&name)).then_some(a.elapsed_ms),
            candidate_elapsed_ms: (!candidate.reconstructed.contains(&name))
                .then_some(b.elapsed_ms),
            baseline_usage_tokens: a.usage_tokens,
            candidate_usage_tokens: b.usage_tokens,
            baseline_grading_error: a.grading_error.clone(),
            candidate_grading_error: b.grading_error.clone(),
            prior_failure_requires_resolution: a.grading_error.is_some()
                || matches!(a.outcome, TrialOutcome::Failed | TrialOutcome::Interrupted)
                || a.unresolved_remote_effects,
        });
    }
    Ok(
        json!({"schema_version":1,"baseline_suite":baseline.suite.digest()?,"candidate_suite":candidate.suite.digest()?,
        "trials":comparisons,"unmatched_trials":unmatched,"qualified":false,
        "interpretation":"Descriptive comparison only; every failure remains in its original archive. No failure resolution, human review or statistical improvement is inferred."}),
    )
}
