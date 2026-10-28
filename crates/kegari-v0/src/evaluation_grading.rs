//! Preserve collected execution evidence when host-side scoring cannot finish.
//! This module has no execution, retirement or permission authority.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GradingStage {
    ScorerLoad,
    Grade,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GradingError {
    pub stage: GradingStage,
    pub error: Error,
}

pub(super) fn collect(
    report: &mut TrialReport,
    fixture: &PreparedFixture,
    isolation: &ValidatedIsolation,
    variant: &Variant,
    facts: &ObservedFacts,
) {
    // These are actual host observations, retained before any fallible scorer
    // work. Missing observation never calls this helper or becomes Some(empty).
    report.artifacts = facts.artifacts.clone();
    report.review_claim = facts.review_claim.clone();
    report.independent_boundary = facts.independent_boundary.clone();
    report.independent_broker = facts.independent_broker.clone();
    report.canary_diagnostic = facts.canary_diagnostic.clone();
    report.canary_output = facts.canary_output.clone();
    report.declared_protocol = facts.declared_protocol.clone();
    report.evidence_references = facts.evidence_references.clone();
    report.observed_effects = Some(facts.effects.clone());
    let result = Scorer::load(
        &fixture.grader_root.join(format!("{}.json", variant.id)),
        &variant.scorer_digest,
    )
    .map_err(|error| GradingError {
        stage: GradingStage::ScorerLoad,
        error,
    })
    .and_then(|scorer| {
        grade(
            &scorer,
            fixture,
            isolation,
            facts,
            &variant.prohibited_effects,
        )
        .map_err(|error| GradingError {
            stage: GradingStage::Grade,
            error,
        })
    });
    match result {
        Ok(result) => {
            if report.reason == "agent finished; observable grading required" {
                report.outcome = if result.deterministic_pass {
                    TrialOutcome::DeterministicPassHumanPending
                } else {
                    TrialOutcome::Failed
                };
                report.reason = if result.deterministic_pass {
                    "deterministic predicates satisfied; mandatory human review pending"
                } else {
                    "observable required behavior failed"
                }
                .into();
            }
            report.grade = Some(result);
        }
        Err(failure) => {
            report.omissions.push(format!(
                "Collected outcome before grading failure: {:?}; reason: {}",
                report.outcome, report.reason
            ));
            // Completed execution without a grade is blocked, not a model
            // failure or an unknown native exit. Existing execution failures
            // and interruptions retain their own classification and reason.
            if report.reason == "agent finished; observable grading required"
                || matches!(
                    report.outcome,
                    TrialOutcome::DeterministicPassHumanPending | TrialOutcome::Skipped
                )
            {
                report.outcome = TrialOutcome::Blocked;
                report.reason =
                    "Host grading failed; collected execution evidence is retained".into();
            }
            report.omissions.push(format!(
                "Host grading failed at {:?}: {}: {}",
                failure.stage, failure.error.code, failure.error.message
            ));
            report.grade = None;
            report.grading_error = Some(failure);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolation::{ExecutionRole, IsolationPlan, PathGrant};
    use serde_json::json;

    struct Fixture {
        _temp: tempfile::TempDir,
        suite: SuiteManifest,
        variant: Variant,
        prepared: PreparedFixture,
        isolation: ValidatedIsolation,
        facts: ObservedFacts,
    }
    impl Fixture {
        fn new() -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations");
            let suite = SuiteManifest::load(&root.join("manifest.json")).unwrap();
            let variant = suite.variants[0].clone();
            let temp = tempfile::tempdir().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let grader = base.join("grader");
            let reports = base.join("reports");
            std::fs::create_dir(&grader).unwrap();
            std::fs::create_dir(&reports).unwrap();
            std::fs::copy(
                root.join(&variant.scorer),
                grader.join(format!("{}.json", variant.id)),
            )
            .unwrap();
            let prepared = prepare_fixture(&root, &variant, &base.join("trial"), &grader).unwrap();
            let isolation = IsolationPlan {
                managed_execution: None,
                schema_version: 1,
                role: ExecutionRole::Child,
                execution_root: prepared.root.clone(),
                repository: prepared.repository.clone(),
                cwd: prepared.repository.clone(),
                scratch: prepared.scratch.clone(),
                read_roots: vec![PathGrant {
                    path: prepared.repository.clone(),
                    directory: true,
                }],
                resource_writes: vec![],
                source_writes: vec![],
                forbidden_roots: vec![
                    prepared.grader_root.clone(),
                    reports,
                    prepared.codex_home.clone(),
                ],
            }
            .validate()
            .unwrap();
            let candidate = canonical_digest(1, &snapshot(&prepared.repository).unwrap()).unwrap();
            let facts = ObservedFacts {
                revision: "1".repeat(40),
                candidate_digest: candidate.clone(),
                checks: BTreeMap::new(),
                artifacts: BTreeMap::from([(
                    "data-only-artifact".into(),
                    ObservedArtifact {
                        digest: bytes_digest(b"observed artifact"),
                        candidate_digest: candidate,
                        visual: false,
                        receipt_digest: bytes_digest(b"data-only receipt"),
                    },
                )]),
                effects: vec!["data-only-observed-effect".into()],
                state: json!({"component":"grader error retention; no worker executed"}),
                summary: None,
                review_claim: None,
                // Descriptive data only, used to verify failure-path retention.
                independent_broker: Some(serde_json::from_value(json!({"schema_version":1,"operation":uuid::Uuid::from_u128(31),"runtime":uuid::Uuid::from_u128(32),"workspace":uuid::Uuid::from_u128(33),
        "trial_digest":bytes_digest(b"data-only trial"),"suite_digest":bytes_digest(b"data-only suite"),"fixture_digest":bytes_digest(b"data-only fixture"),"instruction_digest":bytes_digest(b"data-only instruction"),
        "contract_scope_digest":bytes_digest(b"data-only scope"),"context_digest":bytes_digest(b"data-only context"),"tool_contract_digest":bytes_digest(b"data-only tools"),"isolation_digest":bytes_digest(b"data-only isolation"),
        "state":"unavailable","members":[],"receipts":[],"authority_at_collection":null,"gaps":["component_only_no_native_registration"]})).unwrap()),
                canary_diagnostic: None,
                canary_output: None,
                independent_boundary: Some(IndependentBoundaryObservation {
                    schema_version: 1,
                    source_revision: "1".repeat(40),
                    candidate_digest: bytes_digest(b"explicit synthetic boundary snapshot"),
                    check: IndependentBoundaryCheck::Unavailable {
                        error: Error::new(
                            "boundary_observation_unavailable",
                            "component has no registered native worker",
                        ),
                    },
                }),
                declared_protocol: Some(kegari_codex::DeclaredProtocolObservation {
                    schema_version: 1,
                    operation: uuid::Uuid::from_u128(1),
                    runtime: uuid::Uuid::from_u128(2),
                    workspace: uuid::Uuid::from_u128(3),
                    trial_digest: None,
                    suite_digest: bytes_digest(b"data-only suite"),
                    fixture_digest: bytes_digest(b"data-only fixture"),
                    contract_scope_digest: bytes_digest(b"data-only scope"),
                    instruction_digest: bytes_digest(b"data-only instructions"),
                    session_binding_digest: None,
                    thread_id: None,
                    state: kegari_codex::ProtocolCaptureState::Unavailable,
                    issues: BTreeSet::from([kegari_codex::ProtocolCaptureIssue::MissingSession]),
                    tool_request_attempts: 0,
                    observed_server_requests: 0,
                    processed_server_requests: None,
                    terminal_turns: 0,
                    transport_end: None,
                    records: vec![],
                }),
                evidence_references: vec!["data-only:collected-host-observation".into()],
            };
            Self {
                _temp: temp,
                suite,
                variant,
                prepared,
                isolation,
                facts,
            }
        }
        fn report(&self, outcome: TrialOutcome, reason: &str) -> TrialReport {
            let trial = self
                .suite
                .plan()
                .unwrap()
                .into_iter()
                .find(|t| t.variant == self.variant.id)
                .unwrap();
            let mut report = TrialReport::unrun(
                &self.suite,
                &self.variant,
                trial,
                TrialOutcome::Blocked,
                "synthetic collected report data",
            )
            .unwrap();
            report.evidence_class = EvidenceClass::DeterministicReplay;
            report.outcome = outcome;
            report.reason = reason.into();
            report.turns_started = 2;
            report.usage_tokens = Some(37);
            report.earlier_usage_unknown = true;
            report.elapsed_ms = 123;
            report.authority_reference = Some("data-only:unqualified-authority-reference".into());
            report.instruction_manifest = Some(json!({"data_only":"recorded provenance"}));
            report.permission_configuration = Some(json!({"network":"denied"}));
            report.trace.push(TraceEntry {
                sequence: 1,
                method: "observed-data".into(),
                input_digest: Some(bytes_digest(b"input")),
                output_digest: Some(bytes_digest(b"output")),
                status: "data-only".into(),
            });
            report
        }
    }

    fn assert_retained(before: &Value, report: &TrialReport, facts: &ObservedFacts) {
        let after = serde_json::to_value(report).unwrap();
        for (key, value) in before.as_object().unwrap() {
            if ![
                "outcome",
                "reason",
                "omissions",
                "grade",
                "artifacts",
                "review_claim",
                "independent_boundary",
                "independent_broker",
                "declared_protocol",
                "evidence_references",
                "observed_effects",
                "grading_error",
            ]
            .contains(&key.as_str())
            {
                assert_eq!(after.get(key), Some(value), "retained {key}");
            }
        }
        assert_eq!(report.observed_effects.as_ref(), Some(&facts.effects));
        assert_eq!(
            serde_json::to_value(&report.independent_boundary).unwrap(),
            serde_json::to_value(&facts.independent_boundary).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&report.declared_protocol).unwrap(),
            serde_json::to_value(&facts.declared_protocol).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&report.independent_broker).unwrap(),
            serde_json::to_value(&facts.independent_broker).unwrap()
        );
        assert_eq!(report.evidence_references, facts.evidence_references);
        assert_eq!(
            serde_json::to_value(&report.artifacts).unwrap(),
            serde_json::to_value(&facts.artifacts).unwrap()
        );
        assert!(report.grade.is_none());
    }

    #[test]
    fn actual_scorer_load_error_keeps_collected_evidence_and_blocks_ungraded_success() {
        let f = Fixture::new();
        let path = f
            .prepared
            .grader_root
            .join(format!("{}.json", f.variant.id));
        std::fs::write(&path, b"changed scorer").unwrap();
        let expected = Scorer::load(&path, &f.variant.scorer_digest).unwrap_err();
        let mut report = f.report(
            TrialOutcome::Failed,
            "agent finished; observable grading required",
        );
        assert!(report.observed_effects.is_none());
        let before = serde_json::to_value(&report).unwrap();
        collect(&mut report, &f.prepared, &f.isolation, &f.variant, &f.facts);
        assert_eq!(report.outcome, TrialOutcome::Blocked);
        assert_eq!(
            report.grading_error,
            Some(GradingError {
                stage: GradingStage::ScorerLoad,
                error: expected
            })
        );
        assert!(!report.unresolved_remote_effects);
        assert_retained(&before, &report, &f.facts);
    }

    #[test]
    fn actual_grade_error_keeps_failed_and_interrupted_execution_distinct() {
        let mut f = Fixture::new();
        let scorer = Scorer::load(
            &f.prepared
                .grader_root
                .join(format!("{}.json", f.variant.id)),
            &f.variant.scorer_digest,
        )
        .unwrap();
        f.prepared.input["unexpected_change"] = json!(true);
        let expected = grade(
            &scorer,
            &f.prepared,
            &f.isolation,
            &f.facts,
            &f.variant.prohibited_effects,
        )
        .unwrap_err();
        assert_eq!(expected.code, "evaluation_fidelity");
        for outcome in [TrialOutcome::Failed, TrialOutcome::Interrupted] {
            let mut report = f.report(outcome.clone(), "actual execution result remains separate");
            report.unresolved_remote_effects = outcome == TrialOutcome::Interrupted;
            let before = serde_json::to_value(&report).unwrap();
            collect(&mut report, &f.prepared, &f.isolation, &f.variant, &f.facts);
            assert_eq!(report.outcome, outcome);
            assert_eq!(report.reason, "actual execution result remains separate");
            assert_eq!(
                report.grading_error,
                Some(GradingError {
                    stage: GradingStage::Grade,
                    error: expected.clone()
                })
            );
            assert_retained(&before, &report, &f.facts);
        }
    }

    #[tokio::test]
    async fn shared_suite_persists_actual_grader_error_report_without_unrun_fallback() {
        // The executor below is declared report-data replay, never a native
        // execution claim. It invokes the same actual final grading helper.
        let mut f = Fixture::new();
        f.suite.profiles[0].binding = Some(ProfileBinding {
            profile: kegari_config::Profile {
                tool: "codex".into(),
                model: "deterministic-component-only".into(),
                reasoning: "high".into(),
                speed: kegari_config::Speed::Standard,
            },
            account_id: "component-only-no-native-account".into(),
            operator_reference: "component-only-no-authority".into(),
        });
        f.suite.profiles[0].unresolved = None;
        let path = f
            .prepared
            .grader_root
            .join(format!("{}.json", f.variant.id));
        std::fs::write(&path, b"changed scorer").unwrap();
        let mut collected = f.report(
            TrialOutcome::Failed,
            "agent finished; observable grading required",
        );
        collect(
            &mut collected,
            &f.prepared,
            &f.isolation,
            &f.variant,
            &f.facts,
        );
        let expected = serde_json::to_value(&collected).unwrap();
        let root = f._temp.path().canonicalize().unwrap().join("suite-report");
        let store = ReportStore::create(&root, &f.suite).unwrap();
        let mut calls = 0;
        let reports = run_suite(&f.suite, &store, |variant, key, _| {
            calls += 1;
            let result = if key == collected.trial {
                Ok(collected.clone())
            } else {
                TrialReport::unrun(
                    &f.suite,
                    &variant,
                    key,
                    TrialOutcome::Blocked,
                    "other declared trial not executed by component",
                )
            };
            async move { result }
        })
        .await
        .unwrap();
        assert_eq!(calls, f.suite.plan().unwrap().len());
        let actual = reports.iter().find(|r| r.trial == collected.trial).unwrap();
        assert_eq!(serde_json::to_value(actual).unwrap(), expected);
        assert_eq!(actual.evidence_class, EvidenceClass::DeterministicReplay);
        assert!(!actual.unresolved_remote_effects);
        let archive = crate::evaluation_reports::ReportArchive::open(&root).unwrap();
        let saved = archive
            .reports()
            .iter()
            .find(|r| r.trial == collected.trial)
            .unwrap();
        assert_eq!(serde_json::to_value(saved).unwrap(), expected);
        assert!(
            store.finish(saved).is_err(),
            "immutable result cannot be replaced"
        );
        let variant = &f.variant;
        ReportStore::finish_owned_failure(
            &root,
            &f.suite,
            variant,
            &collected.trial,
            &Error::new("later_cleanup_error", "must not erase recorded report"),
            true,
        )
        .unwrap();
        let reread = crate::evaluation_reports::ReportArchive::open(&root).unwrap();
        assert_eq!(
            serde_json::to_value(
                reread
                    .reports()
                    .iter()
                    .find(|r| r.trial == collected.trial)
                    .unwrap()
            )
            .unwrap(),
            expected
        );
    }

    #[test]
    fn successful_grading_retains_original_grade_and_actual_empty_effect_observation() {
        let mut f = Fixture::new();
        f.facts.effects.clear();
        let scorer = Scorer::load(
            &f.prepared
                .grader_root
                .join(format!("{}.json", f.variant.id)),
            &f.variant.scorer_digest,
        )
        .unwrap();
        let expected = grade(
            &scorer,
            &f.prepared,
            &f.isolation,
            &f.facts,
            &f.variant.prohibited_effects,
        )
        .unwrap();
        let mut report = f.report(
            TrialOutcome::Failed,
            "agent finished; observable grading required",
        );
        collect(&mut report, &f.prepared, &f.isolation, &f.variant, &f.facts);
        assert!(report.grading_error.is_none());
        assert_eq!(report.observed_effects, Some(vec![]));
        assert_eq!(
            serde_json::to_value(report.grade.as_ref().unwrap()).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        assert_eq!(
            report.outcome,
            if expected.deterministic_pass {
                TrialOutcome::DeterministicPassHumanPending
            } else {
                TrialOutcome::Failed
            }
        );
    }
    #[test]
    fn scorer_load_failure_preserves_partial_canary_output_data_without_grading_it() {
        let mut f = Fixture::new();
        // Descriptive reducer input only, not an enrollment or authenticated
        // live match. The real fallible scorer path must retain it unchanged.
        let output:CanaryOutputObservation=serde_json::from_value(json!({
            "schema_version":1,"identity":{"runtime":uuid::Uuid::from_u128(1),"workspace":uuid::Uuid::from_u128(2),"command":uuid::Uuid::from_u128(3),"operation":uuid::Uuid::from_u128(4),"trial_digest":bytes_digest(b"trial"),"suite_digest":bytes_digest(b"suite"),"fixture_digest":bytes_digest(b"fixture"),"scorer_digest":bytes_digest(b"scorer"),"policy_digest":bytes_digest(b"policy"),"profile_digest":bytes_digest(b"profile")},
            "enrollment":null,"coverage":"partial","projection":"independent_declared_egress_v1","matching":"decoded_utf8_atoms_and_channel_concat_v1","pieces":[],"matches":[],"issues":["coverage_not_qualified","capture_gap"],"error_records":[],"final_readback":false
        })).unwrap();
        output.validate().unwrap();
        f.facts.canary_output = Some(output.clone());
        let path = f
            .prepared
            .grader_root
            .join(format!("{}.json", f.variant.id));
        std::fs::write(&path, b"changed scorer").unwrap();
        let mut report = f.report(
            TrialOutcome::Failed,
            "agent finished; observable grading required",
        );
        collect(&mut report, &f.prepared, &f.isolation, &f.variant, &f.facts);
        assert_eq!(report.outcome, TrialOutcome::Blocked);
        assert!(report.grade.is_none());
        assert!(report.grading_error.is_some());
        assert_eq!(
            serde_json::to_value(&report.canary_output).unwrap(),
            serde_json::to_value(Some(output)).unwrap()
        );
    }
}
