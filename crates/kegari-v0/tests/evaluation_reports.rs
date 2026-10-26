use kegari_v0::{digest::bytes_digest, evaluation::*, evaluation_reports::*};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{MetadataExt, symlink},
    path::PathBuf,
    process::Command,
};

fn suite() -> SuiteManifest {
    SuiteManifest::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("evaluations/manifest.json"),
    )
    .unwrap()
}
fn archive(suite: &SuiteManifest) -> (tempfile::TempDir, ReportStore) {
    let temp = tempfile::tempdir().unwrap();
    let store =
        ReportStore::create(&temp.path().canonicalize().unwrap().join("reports"), suite).unwrap();
    (temp, store)
}
fn report(suite: &SuiteManifest, outcome: TrialOutcome) -> TrialReport {
    let key = suite.plan().unwrap()[0].clone();
    let variant = suite
        .variants
        .iter()
        .find(|v| v.id == key.variant && v.case_id == key.case_id)
        .unwrap();
    let mut r = TrialReport::unrun(
        suite,
        variant,
        key,
        TrialOutcome::Blocked,
        "deterministic report-reader fixture",
    )
    .unwrap();
    r.outcome = outcome;
    r.evidence_class = EvidenceClass::DeterministicReplay;
    if r.outcome == TrialOutcome::DeterministicPassHumanPending {
        r.grade = Some(Grade {
            assertions: variant
                .expected_outcomes
                .iter()
                .map(String::as_str)
                .chain([
                    "runtime_scope",
                    "prohibited_effects",
                    "actual_candidate_provenance",
                ])
                .map(|id| AssertionResult {
                    id: id.into(),
                    passed: true,
                    detail: "reader fixture, not agent evidence".into(),
                })
                .collect(),
            deterministic_pass: true,
            human_review: "pending".into(),
        });
    }
    r
}

#[test]
fn declared_success_cannot_omit_duplicate_or_contradict_grading_predicates() {
    let suite = suite();
    for invalid in ["omit", "duplicate", "contradict"] {
        let (_temp, store) = archive(&suite);
        let mut r = report(&suite, TrialOutcome::DeterministicPassHumanPending);
        let grade = r.grade.as_mut().unwrap();
        match invalid {
            "omit" => {
                grade.assertions.pop();
            }
            "duplicate" => grade.assertions.push(grade.assertions[0].clone()),
            _ => grade.deterministic_pass = false,
        }
        store.finish(&r).unwrap();
        assert_eq!(
            ReportArchive::open(store.root()).err().unwrap().code,
            "evaluation_record_conflict",
            "{invalid}"
        );
    }
}

#[test]
fn inspection_preserves_unknown_attempts_and_missing_results_without_writes() {
    let suite = suite();
    let (_temp, store) = archive(&suite);
    let keys = suite.plan().unwrap();
    store.started(&keys[0]).unwrap();
    let before: Vec<_> = fs::read_dir(store.root())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let loaded = ReportArchive::open(store.root()).unwrap();
    assert_eq!(loaded.reports().len(), 56);
    assert_eq!(loaded.reports()[0].outcome, TrialOutcome::Interrupted);
    assert!(loaded.reports()[0].unresolved_remote_effects);
    assert!(
        loaded.reports()[1..]
            .iter()
            .all(|r| r.outcome == TrialOutcome::Skipped)
    );
    assert_eq!(
        loaded.inspection().unwrap()["assessment"]["actual_agent_trials"],
        0
    );
    assert!(loaded.inspection().unwrap()["trials"][0]["elapsed_ms"].is_null());
    assert!(loaded.inspection().unwrap()["trials"][0]["started_at"].is_string());
    assert!(loaded.inspection().unwrap()["trials"][0]["ended_at"].is_null());
    assert!(loaded.inspection().unwrap()["trials"][1]["started_at"].is_null());
    assert!(
        loaded.reports()[0]
            .runtime_build
            .starts_with("unavailable:")
    );
    assert_eq!(store.interrupted(&suite).unwrap().len(), 1);
    assert_eq!(fs::read_dir(store.root()).unwrap().count(), before.len());
    assert_eq!(
        fs::metadata(store.root().join("suite.json"))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn comparison_keeps_failed_history_and_marks_changed_grading_incomparable() {
    let suite = suite();
    let (_left, left) = archive(&suite);
    let (_right, right) = archive(&suite);
    let failed = report(&suite, TrialOutcome::Failed);
    left.started(&failed.trial).unwrap();
    left.finish(&failed).unwrap();
    let passed = report(&suite, TrialOutcome::DeterministicPassHumanPending);
    right.started(&passed.trial).unwrap();
    right.finish(&passed).unwrap();
    let old = fs::read(
        left.root()
            .join(format!("{}.result.json", failed.trial.name().unwrap())),
    )
    .unwrap();
    let comparison = compare(
        &ReportArchive::open(left.root()).unwrap(),
        &ReportArchive::open(right.root()).unwrap(),
    )
    .unwrap();
    let row = comparison["trials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["trial"]["variant"] == failed.trial.variant && r["trial"]["repetition"] == 1)
        .unwrap();
    assert_eq!(row["prior_failure_requires_resolution"], true);
    assert_eq!(row["baseline_outcome"], "failed");
    assert_eq!(row["candidate_outcome"], "deterministic_pass_human_pending");
    assert_eq!(comparison["qualified"], false);
    assert_eq!(
        old,
        fs::read(
            left.root()
                .join(format!("{}.result.json", failed.trial.name().unwrap()))
        )
        .unwrap()
    );
    let mut changed = suite.clone();
    changed.variants[0].scorer_digest = bytes_digest(b"changed grading fixture");
    let (_changed, store) = archive(&changed);
    let r = report(&changed, TrialOutcome::Blocked);
    store.finish(&r).unwrap();
    let comparison = compare(
        &ReportArchive::open(left.root()).unwrap(),
        &ReportArchive::open(store.root()).unwrap(),
    )
    .unwrap();
    assert!(comparison["trials"].as_array().unwrap().iter().any(|r| {
        r["comparable"] == false
            && r["differences"]
                .as_array()
                .unwrap()
                .contains(&json!("scorer"))
    }));
}

#[test]
fn reports_reject_replaced_roots_links_corruption_and_undeclared_results() {
    let suite = suite();
    let (temp, store) = archive(&suite);
    let root = store.root().to_owned();
    let moved = temp.path().join("preserved");
    fs::rename(&root, &moved).unwrap();
    fs::create_dir(&root).unwrap();
    assert!(store.started(&suite.plan().unwrap()[0]).is_err());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    fs::remove_dir(&root).unwrap();
    fs::rename(&moved, &root).unwrap();
    let file = root.join(format!(
        "{}.result.json",
        suite.plan().unwrap()[0].name().unwrap()
    ));
    symlink(root.join("suite.json"), &file).unwrap();
    assert!(ReportArchive::open(&root).is_err());
    fs::remove_file(&file).unwrap();
    fs::hard_link(root.join("suite.json"), &file).unwrap();
    assert!(ReportArchive::open(&root).is_err());
    fs::remove_file(&file).unwrap();
    store.finish(&report(&suite, TrialOutcome::Failed)).unwrap();
    fs::write(&file, b"{").unwrap();
    assert!(ReportArchive::open(&root).is_err());
    fs::remove_file(file).unwrap();
    fs::write(root.join("undeclared.result.json"), b"{}").unwrap();
    assert!(ReportArchive::open(&root).is_err());
}

#[test]
fn historical_versions_remain_inspectable_but_cannot_execute() {
    let suite = suite();
    let (_temp, store) = archive(&suite);
    let mut historical = suite.clone();
    historical.codex_version = "historical-provider-version".into();
    historical.grader_version = "historical-grader-version".into();
    fs::write(
        store.root().join("suite.json"),
        serde_json::to_vec(&historical).unwrap(),
    )
    .unwrap();
    assert!(historical.validate().is_err());
    assert!(historical.plan().is_err());
    let loaded = ReportArchive::open(store.root()).unwrap();
    assert_eq!(loaded.suite().codex_version, "historical-provider-version");
    assert!(
        loaded
            .reports()
            .iter()
            .all(|r| r.grader_version == historical.grader_version)
    );
    assert_eq!(
        loaded.inspection().unwrap()["assessment"]["qualified"],
        false
    );
}

#[test]
fn comparison_distinguishes_inputs_permissions_and_evidence_classes() {
    let suite = suite();
    for difference in ["input", "permission_configuration", "evidence_class"] {
        let (_left, left) = archive(&suite);
        let (_right, right) = archive(&suite);
        let a = report(&suite, TrialOutcome::Failed);
        let mut b = a.clone();
        match difference {
            "input" => b.input_digest = bytes_digest(b"different actual task input"),
            "permission_configuration" => {
                b.permission_configuration = Some(json!({"network":"allowed"}))
            }
            _ => b.evidence_class = EvidenceClass::NotRun,
        }
        left.finish(&a).unwrap();
        right.finish(&b).unwrap();
        let comparison = compare(
            &ReportArchive::open(left.root()).unwrap(),
            &ReportArchive::open(right.root()).unwrap(),
        )
        .unwrap();
        let row = comparison["trials"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["trial"] == json!(a.trial))
            .unwrap();
        assert_eq!(row["comparable"], false);
        assert!(
            row["differences"]
                .as_array()
                .unwrap()
                .contains(&json!(difference))
        );
        assert!(comparison["trials"].as_array().unwrap().iter().any(|r| {
            r["differences"]
                .as_array()
                .unwrap()
                .contains(&json!("original_result_unavailable"))
        }));
    }
}

/// Structurally complete report-reader fixture only. It does not represent a
/// provider run, and this test never supplies live authority or launches one.
fn actual_report_fixture(suite: &SuiteManifest) -> TrialReport {
    let mut claim = report(suite, TrialOutcome::DeterministicPassHumanPending);
    claim.evidence_class = EvidenceClass::ActualAgent;
    claim.profile = suite.profiles[0].binding.clone();
    let variant = &suite.variants[0];
    claim.instruction_manifest = Some(json!({
        "schema_version":1,"runtime_version":claim.runtime_version,"runtime_build":claim.runtime_build,
        "asset_version":"report-fixture","assets":{},"workflow_path":"/fixture/WORKFLOW.md","workflow_digest":bytes_digest(b"fixture"),
        "role":variant.role,"phase":variant.phase,"workspace_id":uuid::Uuid::nil(),"operation_id":uuid::Uuid::nil(),
        "work_generation":"report-fixture","correction_revision":"report-fixture","context_digest":bytes_digest(b"fixture"),
        "prompt_digest":bytes_digest(b"fixture"),"repository_revision":null,"repository_root":null,"cwd":"/fixture",
        "codex_home":"/fixture-home","codex_protocol":"report-fixture","codex_version":claim.codex_version,
        "profile":claim.profile.as_ref().unwrap().profile,"supplied_files":[],"provider_discovery":{"state":"unavailable","reason":"report fixture"},
        "tools":[],"permissions":{"read_roots":[],"source_write_paths":[],"resource_write_paths":[],"scratch_roots":[],
        "repository_code_execution":false,"enforcement_profile":"report-fixture","network":"denied"}
    }));
    claim.permission_configuration = Some(json!({"network":"denied"}));
    claim.tool_definitions = Some(vec![]);
    claim.tool_contract_digest =
        Some(kegari_v0::digest::canonical_digest(1, &Vec::<Value>::new()).unwrap());
    claim.authority_reference = Some("report-fixture-no-live-authority".into());
    claim.input_digest = bytes_digest(b"fixture input");
    claim.turns_started = 1;
    claim.trace = vec![TraceEntry {
        sequence: 0,
        method: "turn/completed".into(),
        input_digest: None,
        output_digest: Some(bytes_digest(b"fixture completion")),
        status: "TurnCompleted".into(),
    }];
    claim.evidence_references = vec!["report-fixture-only".into()];
    claim
}

#[test]
fn successful_actual_shape_requires_a_turn_trace_and_ordered_durable_times() {
    let mut suite = suite();
    suite.profiles[0].binding = Some(ProfileBinding {
        profile: kegari_v0::config::Profile {
            tool: "codex".into(),
            model: "report-fixture-model".into(),
            reasoning: "high".into(),
            speed: kegari_v0::config::Speed::Standard,
        },
        account_id: "report-fixture".into(),
        operator_reference: "not-live-authority".into(),
    });
    suite.profiles[0].unresolved = None;
    for invalid in [
        "valid",
        "zero_turns",
        "empty_trace",
        "no_evidence",
        "invalid_time",
        "reversed",
        "before_admission",
    ] {
        let (_temp, store) = archive(&suite);
        store.started(&suite.plan().unwrap()[0]).unwrap();
        let mut claim = actual_report_fixture(&suite);
        match invalid {
            "zero_turns" => claim.turns_started = 0,
            "empty_trace" => claim.trace.clear(),
            "no_evidence" => claim.evidence_references.clear(),
            "invalid_time" => claim.ended_at = "invalid".into(),
            "reversed" => claim.ended_at = "2000-01-01T00:00:00Z".into(),
            "before_admission" => claim.started_at = "2000-01-01T00:00:00Z".into(),
            _ => {}
        }
        store.finish(&claim).unwrap();
        let result = ReportArchive::open(store.root());
        if invalid == "valid" {
            assert!(result.is_ok());
        } else {
            assert_eq!(
                result.err().unwrap().code,
                "evaluation_record_conflict",
                "{invalid}"
            );
        }
    }
}

#[test]
fn actual_agent_label_cannot_replace_missing_runtime_provenance() {
    let mut suite = suite();
    suite.profiles[0].binding = Some(ProfileBinding {
        profile: kegari_v0::config::Profile {
            tool: "codex".into(),
            model: "fixture-model".into(),
            reasoning: "high".into(),
            speed: kegari_v0::config::Speed::Standard,
        },
        account_id: "fixture-account".into(),
        operator_reference: "fixture-only-no-live-authority".into(),
    });
    suite.profiles[0].unresolved = None;
    let (_temp, store) = archive(&suite);
    let mut claim = report(&suite, TrialOutcome::Failed);
    claim.profile = suite.profiles[0].binding.clone();
    claim.evidence_class = EvidenceClass::ActualAgent;
    store.started(&claim.trial).unwrap();
    store.finish(&claim).unwrap();
    assert_eq!(
        ReportArchive::open(store.root()).err().unwrap().code,
        "evaluation_record_conflict"
    );
}

#[test]
fn executable_plan_and_inspection_are_json_and_start_no_trials() {
    let binary = env!("CARGO_BIN_EXE_kegari-v0-evaluation");
    let output = Command::new(binary)
        .args([
            "plan",
            concat!(env!("CARGO_MANIFEST_DIR"), "/evaluations/manifest.json"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data"]["max_trials"], 56);
    assert_eq!(value["data"]["dispatch_authorized"], false);
    let suite = suite();
    let (_temp, store) = archive(&suite);
    let output = Command::new(binary)
        .arg("inspect")
        .arg(store.root())
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data"]["assessment"]["actual_agent_trials"], 0);
    assert_eq!(fs::read_dir(store.root()).unwrap().count(), 1);
}

#[test]
fn review_prose_is_retained_verbatim_for_human_review_without_qualifying_replay() {
    let suite = suite();
    let variant = suite
        .variants
        .iter()
        .find(|variant| variant.id == "clean_review")
        .unwrap();
    let key = suite
        .plan()
        .unwrap()
        .into_iter()
        .find(|key| key.variant == variant.id)
        .unwrap();
    let (_temp, store) = archive(&suite);
    let mut report = TrialReport::unrun(
        &suite,
        variant,
        key,
        TrialOutcome::Blocked,
        "synthetic review-report reader fixture",
    )
    .unwrap();
    report.evidence_class = EvidenceClass::DeterministicReplay;
    report.outcome = TrialOutcome::Failed;
    let prose =
        "src/limit.py: conflicting revision deadbee; preserve this disputed claim for human review";
    report.review_claim = Some(ObservedReviewClaim {
        schema_version: 1,
        operation: uuid::Uuid::from_u128(1),
        proposal: uuid::Uuid::from_u128(2),
        thread: "synthetic-thread".into(),
        completed_turn: "synthetic-turn".into(),
        source_revision: "1".repeat(40),
        repository: None,
        instruction_digest: bytes_digest(b"synthetic instructions"),
        tool_contract_digest: bytes_digest(b"synthetic tools"),
        outcome_reference: "synthetic:outcome".into(),
        claim: serde_json::from_value(json!({"schema_version":1,"verdict":"findings","unresolved_findings":[prose],"supporting_links":[]})).unwrap(),
    });
    store.finish(&report).unwrap();
    let archive = ReportArchive::open(store.root()).unwrap();
    let inspection = archive.inspection().unwrap();
    let row = inspection["trials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["trial"]["variant"] == "clean_review" && row["review_claim"].is_object())
        .unwrap();
    assert_eq!(
        row["review_claim"]["claim"]["unresolved_findings"][0],
        prose
    );
    assert_eq!(row["outcome"], "failed");
    assert_eq!(inspection["assessment"]["actual_agent_trials"], 0);
    assert!(
        inspection["qualification"]
            .as_str()
            .unwrap()
            .starts_with("not_granted")
    );
    let mut legacy = serde_json::to_value(&report).unwrap();
    legacy.as_object_mut().unwrap().remove("review_claim");
    assert!(
        serde_json::from_value::<TrialReport>(legacy)
            .unwrap()
            .review_claim
            .is_none()
    );
}

#[test]
fn grading_error_cannot_be_relabelled_passed_or_hidden_by_later_success() {
    // Report data only: no production worker, account or authority is created.
    let suite = suite();
    let failure = GradingError {
        stage: GradingStage::ScorerLoad,
        error: kegari_v0::error::Error::new("grader_changed", "scorer was modified")
            .detail(json!({"component":"exact retained error details"})),
    };
    let mut blocked = report(&suite, TrialOutcome::Blocked);
    blocked.grading_error = Some(failure.clone());
    blocked.observed_effects = Some(vec!["data-only:observed-effect".into()]);
    blocked.turns_started = 2;
    blocked.usage_tokens = Some(37);
    let (_left_temp, left) = archive(&suite);
    left.started(&blocked.trial).unwrap();
    left.finish(&blocked).unwrap();
    let left = ReportArchive::open(left.root()).unwrap();
    let loaded = &left.reports()[0];
    assert_eq!(loaded.grading_error, Some(failure.clone()));
    assert_eq!(loaded.observed_effects, blocked.observed_effects);
    assert_eq!(loaded.turns_started, 2);
    assert_eq!(loaded.usage_tokens, Some(37));
    assert_eq!(
        left.inspection().unwrap()["trials"][0]["grading_error"],
        serde_json::to_value(&failure).unwrap()
    );
    let (_right_temp, right) = archive(&suite);
    let passed = report(&suite, TrialOutcome::DeterministicPassHumanPending);
    right.finish(&passed).unwrap();
    let right = ReportArchive::open(right.root()).unwrap();
    let compared = compare(&left, &right).unwrap();
    let trial = compared["trials"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["trial"] == serde_json::to_value(&blocked.trial).unwrap())
        .unwrap();
    assert_eq!(trial["prior_failure_requires_resolution"], true);
    assert_eq!(
        trial["baseline_grading_error"],
        serde_json::to_value(&failure).unwrap()
    );
    assert_eq!(compared["qualified"], false);
    for outcome in [
        TrialOutcome::DeterministicPassHumanPending,
        TrialOutcome::Failed,
    ] {
        let (_temp, store) = archive(&suite);
        let mut forged = passed.clone();
        forged.outcome = outcome;
        forged.grading_error = Some(failure.clone());
        store.finish(&forged).unwrap();
        assert_eq!(
            ReportArchive::open(store.root()).err().unwrap().code,
            "evaluation_record_conflict"
        );
        assert!(
            assess(&suite, &[forged])
                .unwrap()
                .blocking_trials
                .contains(&passed.trial.name().unwrap())
        );
    }
}

#[test]
fn absent_grading_fields_remain_legacy_unknown_not_empty_observations() {
    let suite = suite();
    let original = report(&suite, TrialOutcome::Blocked);
    let bytes = serde_json::to_vec(&original).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(value.get("grading_error").is_none());
    assert!(value.get("observed_effects").is_none());
    let restored: TrialReport = serde_json::from_slice(&bytes).unwrap();
    assert!(restored.grading_error.is_none());
    assert!(restored.observed_effects.is_none());
    assert_eq!(serde_json::to_vec(&restored).unwrap(), bytes);
    let (_temp, store) = archive(&suite);
    store.finish(&restored).unwrap();
    let archive = ReportArchive::open(store.root()).unwrap();
    assert!(archive.inspection().unwrap()["trials"][0]["observed_effects"].is_null());
    let mut observed_empty = restored;
    observed_empty.observed_effects = Some(vec![]);
    let encoded = serde_json::to_value(&observed_empty).unwrap();
    assert_eq!(encoded["observed_effects"], json!([]));
}

#[test]
fn grading_error_is_an_independent_assessment_blocker() {
    // Reuses the existing complete report-reader data fixture, not a worker
    // capability or a claim that native model execution occurred.
    let mut suite = suite();
    suite.profiles[0].binding = Some(ProfileBinding {
        profile: kegari_v0::config::Profile {
            tool: "codex".into(),
            model: "report-fixture-model".into(),
            reasoning: "high".into(),
            speed: kegari_v0::config::Speed::Standard,
        },
        account_id: "report-fixture".into(),
        operator_reference: "not-live-authority".into(),
    });
    suite.profiles[0].unresolved = None;
    let mut shape = actual_report_fixture(&suite);
    let name = shape.trial.name().unwrap();
    assert!(
        !assess(&suite, &[shape.clone()])
            .unwrap()
            .blocking_trials
            .contains(&name)
    );
    shape.grading_error = Some(GradingError {
        stage: GradingStage::Grade,
        error: kegari_v0::error::Error::new(
            "evaluation_fidelity",
            "actual grader rejected changed fixture",
        ),
    });
    assert!(
        assess(&suite, &[shape])
            .unwrap()
            .blocking_trials
            .contains(&name)
    );
}

#[test]
fn independent_boundary_diagnostics_survive_archive_and_legacy_absence_stays_unknown() {
    // Explicit report data; no actual runtime/worker/authority is constructed.
    let suite = suite();
    let mut data = report(&suite, TrialOutcome::Blocked);
    let boundary = IndependentBoundaryObservation {
        schema_version: 1,
        source_revision: "1".repeat(40),
        candidate_digest: bytes_digest(b"data-only source snapshot"),
        check: IndependentBoundaryCheck::Unavailable {
            error: kegari_v0::error::Error::new("access_denied", "data-only fenced observation"),
        },
    };
    boundary.validate().unwrap();
    data.independent_boundary = Some(boundary.clone());
    data.grading_error = Some(GradingError {
        stage: GradingStage::ScorerLoad,
        error: kegari_v0::error::Error::new("record_missing", "data-only missing scorer"),
    });
    let (_temp, store) = archive(&suite);
    store.started(&data.trial).unwrap();
    store.finish(&data).unwrap();
    let loaded = ReportArchive::open(store.root()).unwrap();
    assert_eq!(
        serde_json::to_value(&loaded.reports()[0].independent_boundary).unwrap(),
        serde_json::to_value(Some(boundary)).unwrap()
    );
    assert!(loaded.reports()[0].grading_error.is_some());
    assert_eq!(
        loaded.inspection().unwrap()["trials"][0]["independent_boundary"]["check"]["state"],
        "unavailable"
    );
    let mut legacy = serde_json::to_value(&data).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("independent_boundary");
    let decoded: TrialReport = serde_json::from_value(legacy).unwrap();
    assert!(decoded.independent_boundary.is_none());
    assert!(
        serde_json::to_value(decoded)
            .unwrap()
            .get("independent_boundary")
            .is_none()
    );
    let mut invalid = serde_json::to_value(data.independent_boundary.unwrap()).unwrap();
    invalid["check"]["invented_pass"] = json!(true);
    assert!(serde_json::from_value::<IndependentBoundaryObservation>(invalid).is_err());
}

#[test]
fn declared_protocol_incomplete_inventory_survives_archive_and_legacy_absence() {
    // Report-reader data only; these values cannot create a worker, native
    // capability, permission, session or successful disclosure assertion.
    use kegari_v0::codex::{
        DeclaredProtocolObservation, ProtocolCaptureIssue, ProtocolCaptureState,
    };
    let suite = suite();
    let mut data = report(&suite, TrialOutcome::Blocked);
    let observation = DeclaredProtocolObservation {
        schema_version: 1,
        operation: uuid::Uuid::from_u128(21),
        runtime: uuid::Uuid::from_u128(22),
        workspace: uuid::Uuid::from_u128(23),
        trial_digest: Some(kegari_v0::digest::canonical_digest(1, &data.trial).unwrap()),
        suite_digest: data.suite_digest.clone(),
        fixture_digest: data.fixture_digest.clone(),
        contract_scope_digest: bytes_digest(b"report-only scope"),
        instruction_digest: bytes_digest(b"report-only instructions"),
        session_binding_digest: Some(bytes_digest(b"report-only session")),
        thread_id: Some("report-only-thread".into()),
        state: ProtocolCaptureState::Incomplete,
        issues: [
            ProtocolCaptureIssue::MissingRequestInventory,
            ProtocolCaptureIssue::UnobservedTransportTail,
        ]
        .into_iter()
        .collect(),
        tool_request_attempts: 2,
        observed_server_requests: 2,
        processed_server_requests: Some(3),
        terminal_turns: 1,
        records: vec![],
    };
    observation.validate().unwrap();
    data.declared_protocol = Some(observation.clone());
    let (_temp, store) = archive(&suite);
    store.started(&data.trial).unwrap();
    store.finish(&data).unwrap();
    let loaded = ReportArchive::open(store.root()).unwrap();
    assert_eq!(
        serde_json::to_value(&loaded.reports()[0].declared_protocol).unwrap(),
        serde_json::to_value(Some(&observation)).unwrap()
    );
    let inspected = loaded.inspection().unwrap();
    let actual = &inspected["trials"][0]["declared_protocol"];
    assert_eq!(actual["state"], "incomplete");
    assert_eq!(actual["observed_server_requests"], 2);
    assert_eq!(actual["processed_server_requests"], 3);
    assert!(
        actual["issues"]
            .as_array()
            .unwrap()
            .contains(&json!("unobserved_transport_tail"))
    );
    let mut legacy = serde_json::to_value(&data).unwrap();
    legacy.as_object_mut().unwrap().remove("declared_protocol");
    let restored: TrialReport = serde_json::from_value(legacy).unwrap();
    assert!(restored.declared_protocol.is_none());
    assert!(
        serde_json::to_value(restored)
            .unwrap()
            .get("declared_protocol")
            .is_none()
    );
}
