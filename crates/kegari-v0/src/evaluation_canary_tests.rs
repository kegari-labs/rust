use super::*;
use serde_json::json;

fn policy_bytes() -> Vec<u8> {
    serde_json::to_vec(
        &json!({"schema_version":1,"purpose":"independent_canary_diagnostic",
        "recipe":"os_random_256_ascii_v1","placement":"private_grader_child_v1",
        "host_input_set":"declared_host_inputs_v1","forms":FORMS}),
    )
    .unwrap()
}
fn shipped() -> SuiteManifest {
    serde_json::from_slice(
        &std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations/manifest.json"))
            .unwrap(),
    )
    .unwrap()
}
struct Case {
    _temp: tempfile::TempDir,
    root: PathBuf,
    suite_root: PathBuf,
    grader: PrivateDir,
    suite: SuiteManifest,
    variant: Variant,
    trial: TrialKey,
    owner: Arc<CanaryMaterialization>,
}
impl Case {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let suite_root = root.join("suite");
        std::fs::create_dir(&suite_root).unwrap();
        std::fs::create_dir(suite_root.join("diagnostics")).unwrap();
        let bytes = policy_bytes();
        std::fs::write(suite_root.join("diagnostics/canary-v1.json"), &bytes).unwrap();
        let mut suite = shipped();
        let index = suite
            .variants
            .iter()
            .position(|v| v.id == "independent_malicious")
            .unwrap();
        suite.variants[index].canary_diagnostic = Some(CanaryPolicyReference {
            schema_version: 1,
            path: "diagnostics/canary-v1.json".into(),
            digest: bytes_digest(&bytes),
        });
        // Explicit component metadata only; it authorizes no actual account/run.
        suite.profiles[0].binding = Some(ProfileBinding {
            profile: crate::config::Profile {
                tool: "codex".into(),
                model: "component-only".into(),
                reasoning: "high".into(),
                speed: crate::config::Speed::Standard,
            },
            account_id: "component-only".into(),
            operator_reference: "no-live-authority".into(),
        });
        suite.profiles[0].unresolved = None;
        let variant = suite.variants[index].clone();
        let trial = suite
            .plan()
            .unwrap()
            .into_iter()
            .find(|t| t.variant == variant.id)
            .unwrap();
        let operation = Uuid::new_v4();
        let grader = PrivateDir::create(&root.join("report"))
            .unwrap()
            .child("grader")
            .unwrap()
            .child(&operation.to_string())
            .unwrap();
        let owner = CanaryMaterialization::pending(CanaryStageInput {
            runtime: Uuid::new_v4(),
            workspace: Uuid::new_v4(),
            command: Uuid::new_v4(),
            operation,
            suite: &suite,
            variant: &variant,
            trial: &trial,
            deadline: tokio::time::Instant::now() + Duration::from_secs(60),
        })
        .unwrap();
        Self {
            _temp: temp,
            root,
            suite_root,
            grader,
            suite,
            variant,
            trial,
            owner,
        }
    }
    fn stage(&self) {
        self.owner.stage(&self.suite_root, &self.grader).unwrap();
    }
}
#[test]
fn absent_canary_preserves_actual_shipped_manifest_canonical_bytes_and_plan() {
    let bytes =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations/manifest.json"))
            .unwrap();
    let original: Value = serde_json::from_slice(&bytes).unwrap();
    let suite: SuiteManifest = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_value(&suite).unwrap(), original);
    assert_eq!(
        suite.digest().unwrap(),
        canonical_digest(SUITE_SCHEMA, &original).unwrap()
    );
    assert_eq!(suite.plan().unwrap().len(), 56);
    assert!(suite.variants.iter().all(|v| v.canary_diagnostic.is_none()));
    let case = Case::new();
    assert_ne!(case.suite.digest().unwrap(), suite.digest().unwrap());
    // Isolate the new field from the independently resolved component profile.
    let mut without_policy = case.suite.clone();
    without_policy
        .variants
        .iter_mut()
        .find(|v| v.id == case.variant.id)
        .unwrap()
        .canary_diagnostic = None;
    assert_ne!(
        case.suite.digest().unwrap(),
        without_policy.digest().unwrap()
    );
    let mut without_value = serde_json::to_value(&without_policy).unwrap();
    let with_value = serde_json::to_value(&case.suite).unwrap();
    let index = case
        .suite
        .variants
        .iter()
        .position(|v| v.id == case.variant.id)
        .unwrap();
    without_value["variants"][index]["canary_diagnostic"] =
        with_value["variants"][index]["canary_diagnostic"].clone();
    assert_eq!(without_value, with_value);
    assert_eq!(case.suite.variants.len(), 28);
    assert_eq!(case.suite.plan().unwrap().len(), 56);
}
#[test]
fn policy_loader_uses_actual_bounded_declared_bytes_and_refuses_unknown_fields() {
    let case = Case::new();
    let reference = case.variant.canary_diagnostic.as_ref().unwrap();
    let mut charged = 0;
    let loaded = load_policy(&case.suite_root, reference, &mut |n| {
        charged += n;
        Ok(())
    })
    .unwrap();
    assert_eq!(loaded, policy_bytes());
    assert!(charged >= loaded.len());
    let mut altered: Value = serde_json::from_slice(&loaded).unwrap();
    altered["extra_instruction"] = json!("not allowed");
    let altered = serde_json::to_vec(&altered).unwrap();
    std::fs::write(case.suite_root.join(&reference.path), &altered).unwrap();
    let mut exact = reference.clone();
    exact.digest = bytes_digest(&altered);
    assert_eq!(
        load_policy(&case.suite_root, &exact, &mut |_| Ok(()))
            .unwrap_err()
            .code,
        "invalid_config"
    );
    exact.path = "diagnostics/../canary-v1.json".into();
    assert!(exact.validate().is_err());
}
#[cfg(unix)]
#[test]
fn policy_loader_refuses_fixed_parent_symlink_leaf_link_and_hard_link() {
    use std::os::unix::fs::symlink;
    let case = Case::new();
    let reference = case.variant.canary_diagnostic.as_ref().unwrap();
    let parent = case.suite_root.join("diagnostics");
    let other = case.root.join("elsewhere");
    std::fs::rename(&parent, &other).unwrap();
    symlink(&other, &parent).unwrap();
    assert_eq!(
        load_policy(&case.suite_root, reference, &mut |_| Ok(()))
            .unwrap_err()
            .code,
        "scope_violation"
    );
    std::fs::remove_file(&parent).unwrap();
    std::fs::rename(&other, &parent).unwrap();
    let original = parent.join("canary-v1.json");
    let held = case.root.join("held.json");
    std::fs::rename(&original, &held).unwrap();
    symlink(&held, &original).unwrap();
    assert!(load_policy(&case.suite_root, reference, &mut |_| Ok(())).is_err());
    std::fs::remove_file(&original).unwrap();
    std::fs::hard_link(&held, &original).unwrap();
    assert_eq!(
        load_policy(&case.suite_root, reference, &mut |_| Ok(()))
            .unwrap_err()
            .code,
        "scope_violation"
    );
}
#[test]
fn protected_marker_is_single_generation_and_deletion_or_substitution_never_remints() {
    let case = Case::new();
    case.stage();
    let initial = case.owner.diagnostic().unwrap();
    assert!(initial.materialized);
    assert!(!initial.final_readback);
    let state = case.owner.lock().unwrap();
    let directory = state.directory.as_ref().unwrap().clone();
    let marker = state.marker.as_ref().unwrap().clone();
    drop(state);
    assert!(marker.starts_with(b"kegari-private-canary-v1-"));
    assert_eq!(marker.len(), b"kegari-private-canary-v1-".len() + 64);
    let public = serde_json::to_string(&initial).unwrap();
    for (_, form) in marker_forms(&marker) {
        assert!(!public.contains(std::str::from_utf8(&form).unwrap()));
    }
    let initial_bytes = directory
        .read_bytes("initial.json", CHUNK_BYTES as u64)
        .unwrap();
    std::fs::remove_file(directory.path().join("marker.bin")).unwrap();
    assert_eq!(
        case.owner.verify_originals().unwrap_err().code,
        "record_missing"
    );
    assert_eq!(
        case.owner
            .stage(&case.suite_root, &case.grader)
            .unwrap_err()
            .code,
        "canary_conflict"
    );
    assert!(!directory.path().join("marker.bin").exists());
    let failed = case.owner.finalize();
    assert!(failed.materialized);
    assert!(!failed.final_readback);
    assert!(matches!(
        failed.host_inputs,
        CanaryHostInputs::Unavailable { .. }
    ));
    assert!(failed.error.is_some());
    assert_eq!(failed.records, initial.records);
    directory
        .create_bytes_exclusive("marker.bin", b"substituted")
        .unwrap();
    assert_eq!(
        case.owner.verify_originals().unwrap_err().code,
        "canary_conflict"
    );
    assert_eq!(
        directory
            .read_bytes("initial.json", CHUNK_BYTES as u64)
            .unwrap(),
        initial_bytes
    );
}
#[test]
fn shared_budget_refuses_guarded_capture_before_actual_read_and_preserves_files() {
    let case = Case::new();
    case.stage();
    let shared = case.owner.budget.clone();
    while shared.reserve(128 * 1024).is_ok() {}
    while shared.reserve(1).is_ok() {}
    let source = case.root.join("input.txt");
    std::fs::write(&source, b"unchanged").unwrap();
    let before = case.owner.diagnostic().unwrap().records;
    let mut reads = 0;
    let error = fixture_capture::read_file_guarded(&source, 128, &mut |n| {
        case.owner.reserve(n)?;
        reads += 1;
        Ok(())
    })
    .err()
    .unwrap();
    assert_eq!(error.code, "observation_limit");
    assert_eq!(reads, 0);
    assert_eq!(case.owner.diagnostic().unwrap().records, before);
    assert_eq!(std::fs::read(&source).unwrap(), b"unchanged");
}
#[test]
fn retained_source_bytes_and_collision_records_use_same_capture_without_native_authority() {
    let case = Case::new();
    case.stage();
    let source = case.root.join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("file.txt"), b"original captured input").unwrap();
    let tree =
        fixture_capture::CapturedTree::capture_observed(&source, &mut |n| case.owner.reserve(n))
            .unwrap();
    std::fs::write(source.join("file.txt"), b"later mutable input").unwrap();
    let bytes = tree.file("file.txt").unwrap().bytes();
    assert_eq!(bytes, b"original captured input");
    case.owner
        .capture_input("component-source", bytes, &|| Ok(()))
        .unwrap();
    let state = case.owner.lock().unwrap();
    let directory = state.directory.as_ref().unwrap().clone();
    let marker = state.marker.as_ref().unwrap().clone();
    drop(state);
    assert_eq!(
        directory
            .read_bytes("input-0000-0000.bin", CHUNK_BYTES as u64)
            .unwrap(),
        bytes
    );
    assert_eq!(
        case.owner
            .capture_input("component-collision", &marker, &|| Ok(()))
            .unwrap_err()
            .code,
        "canary_input_collision"
    );
    let diagnostic = case.owner.diagnostic().unwrap();
    assert!(matches!(
        diagnostic.host_inputs,
        CanaryHostInputs::Collision { .. }
    ));
    assert!(!diagnostic.final_readback);
    assert!(diagnostic.binding.is_none()); // No fake contract/permit was constructed.
}
#[test]
fn existing_leaf_refusal_retains_expected_anchor_and_failure_report_readback() {
    let case = Case::new();
    case.stage();
    let directory = case
        .owner
        .lock()
        .unwrap()
        .directory
        .as_ref()
        .unwrap()
        .clone();
    directory
        .create_bytes_exclusive("partial.bin", b"unadopted existing bytes")
        .unwrap();
    let error = case
        .owner
        .write_original("partial.bin", b"original intended bytes")
        .unwrap_err();
    case.owner.note_failure(&error);
    let diagnostic = case.owner.diagnostic().unwrap();
    assert!(
        diagnostic.records.iter().any(
            |r| r.leaf == "partial.bin" && r.digest == bytes_digest(b"original intended bytes")
        )
    );
    assert!(diagnostic.materialized);
    assert!(!diagnostic.final_readback);
    assert!(matches!(
        diagnostic.host_inputs,
        CanaryHostInputs::Unavailable { .. }
    ));
    let report_root = case.root.join("archive");
    let store = ReportStore::create(&report_root, &case.suite).unwrap();
    store.started(&case.trial).unwrap();
    ReportStore::finish_owned_failure_with_canary(
        &report_root,
        &case.suite,
        &case.variant,
        &case.trial,
        &error,
        false,
        CanaryFailureObservations {
            inputs: Some(diagnostic.clone()),
            outputs: None,
        },
    )
    .unwrap();
    let archive = crate::evaluation_reports::ReportArchive::open(&report_root).unwrap();
    let report = archive
        .reports()
        .iter()
        .find(|r| r.trial == case.trial)
        .unwrap();
    assert_eq!(report.outcome, TrialOutcome::Blocked);
    assert_eq!(
        serde_json::to_value(&report.canary_diagnostic).unwrap(),
        serde_json::to_value(Some(diagnostic)).unwrap()
    );
    assert!(report.grade.is_none());
}
#[test]
fn expired_original_canary_deadline_cannot_create_or_reset_materialization() {
    let mut case = Case::new();
    Arc::get_mut(&mut case.owner).unwrap().deadline = tokio::time::Instant::now();
    assert_eq!(
        case.owner
            .stage(&case.suite_root, &case.grader)
            .unwrap_err()
            .code,
        "evaluation_deadline"
    );
    assert!(!case.grader.path().join("canary-diagnostic").exists());
    assert_eq!(
        case.owner
            .stage(&case.suite_root, &case.grader)
            .unwrap_err()
            .code,
        "canary_conflict"
    );
    assert!(!case.owner.diagnostic().unwrap().materialized);
}

#[test]
fn opted_in_suite_loader_reads_exact_policy_and_keeps_other_variants_unchanged() {
    fn copy_tree(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            assert!(!kind.is_symlink());
            if kind.is_dir() {
                copy_tree(&entry.path(), &to.join(entry.file_name()));
            } else {
                std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
            }
        }
    }
    let case = Case::new();
    copy_tree(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations"),
        &case.suite_root,
    );
    let manifest = case.suite_root.join("manifest.json");
    std::fs::write(&manifest, serde_json::to_vec(&case.suite).unwrap()).unwrap();
    let loaded = SuiteManifest::load(&manifest).unwrap();
    assert_eq!(loaded.digest().unwrap(), case.suite.digest().unwrap());
    assert_eq!(loaded.plan().unwrap().len(), 56);
    assert_eq!(
        loaded
            .variants
            .iter()
            .filter(|v| v.canary_diagnostic.is_some())
            .count(),
        1
    );
    std::fs::remove_file(case.suite_root.join("diagnostics/canary-v1.json")).unwrap();
    assert!(SuiteManifest::load(&manifest).is_err());
    assert!(!case.suite_root.join("diagnostics/canary-v1.json").exists());
}

#[test]
fn canary_json_capture_charges_before_retention_and_preserves_original_budget() {
    let case = Case::new();
    case.stage();
    let value = json!({"z":["actual value",23], "a":{"b":true}});
    assert_eq!(
        case.owner.captured_json(&value).unwrap(),
        crate::digest::canonical_bytes(&value).unwrap()
    );
    while case.owner.budget.reserve(128 * 1024).is_ok() {}
    while case.owner.budget.reserve(1).is_ok() {}
    assert_eq!(
        case.owner.captured_json(&value).unwrap_err().code,
        "observation_limit"
    );
    assert_eq!(
        case.owner.budget.reserve(1).unwrap_err().code,
        "observation_limit"
    );
    assert!(!case.owner.diagnostic().unwrap().final_readback);
}

#[test]
fn actual_scorer_load_failure_keeps_materialization_diagnostic_without_native_claim() {
    let case = Case::new();
    case.stage();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations");
    // Real fixture/storage/grader path; no worker, source cap or model ran.
    let prepared = prepare_fixture(
        &root,
        &case.variant,
        &case.root.join("fixture"),
        case.grader.path(),
    )
    .unwrap();
    let isolation = crate::isolation::IsolationPlan {
        managed_execution: None,
        schema_version: 1,
        role: ExecutionRole::IndependentReview,
        execution_root: prepared.root.clone(),
        repository: prepared.repository.clone(),
        cwd: prepared.repository.clone(),
        scratch: prepared.scratch.clone(),
        read_roots: vec![PathGrant {
            path: prepared.repository.clone(),
            directory: true,
        }],
        source_writes: vec![],
        resource_writes: vec![],
        forbidden_roots: vec![case.grader.path().to_owned(), prepared.codex_home.clone()],
    }
    .validate()
    .unwrap();
    let diagnostic = case.owner.diagnostic().unwrap();
    let facts = ObservedFacts {
        revision: "1".repeat(40),
        candidate_digest: canonical_digest(1, &snapshot(&prepared.repository).unwrap()).unwrap(),
        checks: BTreeMap::new(),
        artifacts: BTreeMap::new(),
        effects: vec![],
        state: json!({"data_only":true}),
        summary: None,
        review_claim: None,
        independent_boundary: None,
        independent_broker: None,
        canary_diagnostic: Some(diagnostic.clone()),
        canary_output: None,
        declared_protocol: None,
        evidence_references: vec![],
    };
    let mut report = TrialReport::unrun(
        &case.suite,
        &case.variant,
        case.trial.clone(),
        TrialOutcome::Blocked,
        "component only",
    )
    .unwrap();
    grading::collect(&mut report, &prepared, &isolation, &case.variant, &facts);
    assert_eq!(
        report.grading_error.as_ref().unwrap().stage,
        GradingStage::ScorerLoad
    );
    assert!(report.grade.is_none());
    assert_eq!(report.outcome, TrialOutcome::Blocked);
    assert_eq!(
        serde_json::to_value(&report.canary_diagnostic).unwrap(),
        serde_json::to_value(Some(diagnostic)).unwrap()
    );
    assert!(!report.canary_diagnostic.as_ref().unwrap().final_readback);
    assert_ne!(report.evidence_class, EvidenceClass::ActualAgent);
    let archive_root = case.root.join("graded-report");
    let store = ReportStore::create(&archive_root, &case.suite).unwrap();
    store.started(&case.trial).unwrap();
    store.finish(&report).unwrap();
    let archive = crate::evaluation_reports::ReportArchive::open(&archive_root).unwrap();
    let saved = archive
        .reports()
        .iter()
        .find(|r| r.trial == case.trial)
        .unwrap();
    assert_eq!(
        serde_json::to_value(saved).unwrap(),
        serde_json::to_value(report).unwrap()
    );
    assert!(!assess(&case.suite, archive.reports()).unwrap().qualified);
}

#[test]
fn descriptive_canary_report_cannot_contradict_policy_or_claim_final_without_enrollment() {
    let case = Case::new();
    case.stage();
    let report = TrialReport::unrun(
        &case.suite,
        &case.variant,
        case.trial.clone(),
        TrialOutcome::Blocked,
        "data only",
    )
    .unwrap();
    let diagnostic = case.owner.diagnostic().unwrap();
    diagnostic
        .validate_report(
            &report,
            &case.variant,
            case.suite.profiles[0].binding.as_ref(),
        )
        .unwrap();
    let mut changed = diagnostic.clone();
    changed.identity.policy_digest = bytes_digest(b"another policy");
    assert!(
        changed
            .validate_report(
                &report,
                &case.variant,
                case.suite.profiles[0].binding.as_ref()
            )
            .is_err()
    );
    let mut changed = diagnostic.clone();
    changed.final_readback = true;
    changed.host_inputs = CanaryHostInputs::AbsentInDeclaredSet { input_count: 1 };
    assert!(changed.validate().is_err());
    let mut absent = case.variant.clone();
    absent.canary_diagnostic = None;
    assert!(
        diagnostic
            .validate_report(&report, &absent, case.suite.profiles[0].binding.as_ref())
            .is_err()
    );
}

#[test]
fn policy_closed_versions_forms_and_size_never_enter_materialization() {
    let case = Case::new();
    let reference = case.variant.canary_diagnostic.as_ref().unwrap();
    for (field, value) in [
        ("schema_version", json!(2)),
        ("recipe", json!("public_seed")),
        ("placement", json!("source_root")),
        ("forms", json!(["literal_utf8", "literal_utf8"])),
        ("host_input_set", json!("all_inputs")),
    ] {
        let mut policy: Value = serde_json::from_slice(&policy_bytes()).unwrap();
        policy[field] = value;
        let bytes = serde_json::to_vec(&policy).unwrap();
        std::fs::write(case.suite_root.join(&reference.path), &bytes).unwrap();
        let mut declared = reference.clone();
        declared.digest = bytes_digest(&bytes);
        assert_eq!(
            load_policy(&case.suite_root, &declared, &mut |_| Ok(()))
                .unwrap_err()
                .code,
            "invalid_config"
        );
    }
    let bytes = vec![b' '; POLICY_BYTES + 1];
    std::fs::write(case.suite_root.join(&reference.path), &bytes).unwrap();
    let mut declared = reference.clone();
    declared.digest = bytes_digest(&bytes);
    let mut reads = 0;
    assert_eq!(
        load_policy(&case.suite_root, &declared, &mut |_| {
            reads += 1;
            Ok(())
        })
        .unwrap_err()
        .code,
        "evidence_limit"
    );
    assert_eq!(reads, 0);
    assert!(!case.grader.path().join("canary-diagnostic").exists());
}

#[test]
fn actual_native_fixture_empty_corrections_is_eligible_but_other_shapes_refuse() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("evaluations/fixtures/independent_malicious/context.json");
    let original: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(original["corrections"], json!([]));
    assert!(empty_corrections(&original));
    for invalid in [
        Value::Null,
        json!(false),
        json!(""),
        json!({}),
        json!([null]),
        json!([{"authorized":true}]),
    ] {
        let mut changed = original.clone();
        changed["corrections"] = invalid;
        assert!(!empty_corrections(&changed));
    }
    let mut absent = original;
    absent.as_object_mut().unwrap().remove("corrections");
    assert!(empty_corrections(&absent));
}

#[test]
fn protected_canary_initial_policy_marker_and_input_refuse_identical_replacement() {
    use std::os::unix::fs::PermissionsExt;
    for leaf in [
        "initial.json",
        "policy.json",
        "marker.bin",
        "input-0000-0000.bin",
    ] {
        let case = Case::new();
        case.stage();
        case.owner
            .capture_input(
                "actual-component-input",
                b"original retained input",
                &|| Ok(()),
            )
            .unwrap();
        let before = case.owner.diagnostic().unwrap();
        let directory = case
            .owner
            .lock()
            .unwrap()
            .directory
            .as_ref()
            .unwrap()
            .clone();
        let original = directory.read_bytes(leaf, CHUNK_BYTES as u64).unwrap();
        let replacement = directory.path().join("replacement.tmp");
        std::fs::write(&replacement, &original).unwrap();
        std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::rename(&replacement, directory.path().join(leaf)).unwrap();
        assert_eq!(
            directory.read_bytes(leaf, CHUNK_BYTES as u64).unwrap(),
            original
        );
        assert_eq!(
            case.owner.verify_originals().unwrap_err().code,
            "canary_conflict"
        );
        let final_data = case.owner.finalize();
        assert!(final_data.materialized);
        assert!(!final_data.final_readback);
        assert!(matches!(
            final_data.host_inputs,
            CanaryHostInputs::Unavailable { .. }
        ));
        assert_eq!(final_data.records, before.records);
        assert_eq!(
            case.owner
                .stage(&case.suite_root, &case.grader)
                .unwrap_err()
                .code,
            "canary_conflict"
        );
        assert_eq!(
            directory.read_bytes(leaf, CHUNK_BYTES as u64).unwrap(),
            original
        );
    }
}

#[test]
fn each_real_marker_form_matches_across_chunk_boundary_with_original_precedence() {
    for form_index in 0..FORMS.len() {
        let case = Case::new();
        case.stage();
        let marker = case.owner.lock().unwrap().marker.as_ref().unwrap().clone();
        let forms = marker_forms(&marker);
        let (name, needle) = &forms[form_index];
        let mut input = vec![b'.'; CHUNK_BYTES + needle.len()];
        let start = CHUNK_BYTES - needle.len() / 2;
        input[start..start + needle.len()].copy_from_slice(needle);
        assert_eq!(
            case.owner
                .capture_input("cross-chunk-component-input", &input, &|| Ok(()))
                .unwrap_err()
                .code,
            "canary_input_collision"
        );
        match case.owner.diagnostic().unwrap().host_inputs {
            CanaryHostInputs::Collision { input_digest, form } => {
                assert_eq!(input_digest, bytes_digest(&input));
                assert_eq!(form, *name);
            }
            _ => panic!("actual cross-chunk collision was lost"),
        }
    }
}

#[test]
fn actual_scan_checks_current_between_chunks_and_never_records_absence_on_failure() {
    let case = Case::new();
    case.stage();
    let marker = case.owner.lock().unwrap().marker.as_ref().unwrap().clone();
    let forms = marker_forms(&marker);
    let input = vec![b'.'; CHUNK_BYTES * 2];
    let calls = std::cell::Cell::new(0);
    let before = case.owner.diagnostic().unwrap().records;
    let error = case
        .owner
        .scan_input(&input, &forms, &|| {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                Err(Error::new(
                    "evaluation_deadline",
                    "component current boundary expired",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error.code, "evaluation_deadline");
    assert_eq!(calls.get(), 2);
    assert_eq!(case.owner.diagnostic().unwrap().records, before);
    assert!(!case.owner.diagnostic().unwrap().final_readback);
    let mut case = case;
    Arc::get_mut(&mut case.owner).unwrap().deadline = tokio::time::Instant::now();
    assert_eq!(
        case.owner
            .scan_input(&input, &forms, &|| Ok(()))
            .unwrap_err()
            .code,
        "evaluation_deadline"
    );
}

#[path = "evaluation_canary_context_tests.rs"]
mod context_operand;
