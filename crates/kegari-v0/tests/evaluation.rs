//! Deterministic replay/grader tests only. No model, account, or live provider is invoked.
use kegari_v0::config::{Profile, Speed};
use kegari_v0::digest::bytes_digest;
use kegari_v0::evaluation::*;
use kegari_v0::instructions::{
    AgentRole, AssemblyInput, PermissionManifest, ProviderDiscovery, RepositoryContext,
    WorkerNetwork,
};
use kegari_v0::isolation::{ExecutionRole, IsolationPlan, PathGrant, ValidatedIsolation};
use kegari_v0::workflow::Workflow;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use uuid::Uuid;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations")
}
fn suite() -> SuiteManifest {
    SuiteManifest::load(&root().join("manifest.json")).unwrap()
}
fn profile() -> Profile {
    Profile {
        tool: "codex".into(),
        model: "deterministic-fixture-model-not-live".into(),
        reasoning: "high".into(),
        speed: Speed::Standard,
    }
}
fn setup(variant: &Variant) -> (TempDir, PreparedFixture, ValidatedIsolation, PathBuf) {
    setup_from(&root(), variant)
}
fn setup_from(
    suite_root: &Path,
    variant: &Variant,
) -> (TempDir, PreparedFixture, ValidatedIsolation, PathBuf) {
    let temp = TempDir::new().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let reports = base.join("reports");
    std::fs::create_dir(&reports).unwrap();
    let fixture = prepare_fixture(
        suite_root,
        variant,
        &base.join("trial"),
        &root().join("scorer"),
    )
    .unwrap();
    // prepare_fixture already validates the versioned declaration. Test
    // isolation must use that declared scope instead of assuming legacy context.
    let binding_path = suite_root
        .join(&variant.fixture)
        .join("context-binding.json");
    let writes = if binding_path.exists() {
        let binding: Value = serde_json::from_slice(&std::fs::read(binding_path).unwrap()).unwrap();
        assert_eq!(
            tree_digest(&suite_root.join(&variant.fixture)).unwrap(),
            variant.fixture_digest
        );
        binding["source_write_paths"].as_array().unwrap().clone()
    } else {
        fixture.input["scope"]["write_paths"]
            .as_array()
            .unwrap()
            .clone()
    };
    let writes: Vec<PathGrant> = writes
        .iter()
        .map(|v| PathGrant {
            path: fixture.repository.join(v.as_str().unwrap()),
            directory: false,
        })
        .collect();
    let role = match variant.role {
        AgentRole::Implementation | AgentRole::Testing | AgentRole::Documentation => {
            ExecutionRole::Child
        }
        AgentRole::Coordination => ExecutionRole::Coordination,
        AgentRole::IndependentReview => ExecutionRole::IndependentReview,
        AgentRole::Review => ExecutionRole::Review,
        AgentRole::Validation => ExecutionRole::Validation,
    };
    let isolation = IsolationPlan {
        schema_version: 1,
        role,
        execution_root: fixture.root.clone(),
        repository: fixture.repository.clone(),
        cwd: fixture.repository.clone(),
        scratch: fixture.scratch.clone(),
        read_roots: vec![PathGrant {
            path: fixture.repository.clone(),
            directory: true,
        }],
        resource_writes: vec![],
        source_writes: writes,
        forbidden_roots: vec![
            fixture.grader_root.clone(),
            reports.clone(),
            fixture.codex_home.clone(),
        ],
        executables: vec![],
    }
    .validate()
    .unwrap();
    (temp, fixture, isolation, reports)
}

#[test]
fn declared_native_context_file_uses_production_assembly_without_context_waivers() {
    let mut variant = suite()
        .variants
        .iter()
        .find(|v| v.id == "guidance_new")
        .unwrap()
        .clone();
    let declared = TempDir::new().unwrap();
    let suite_root = declared.path().canonicalize().unwrap();
    let source = root().join(&variant.fixture);
    let native = suite_root.join("fixture");
    std::fs::create_dir(&native).unwrap();
    for relative in snapshot(&source).unwrap().keys() {
        let path = native.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::copy(source.join(relative), path).unwrap();
    }
    let mut template: Value =
        serde_json::from_slice(&std::fs::read(native.join("context.json")).unwrap()).unwrap();
    template["workspace"] = json!({"id":{"$kegari":"workspace_id"},"name":"explicit-native-fixture","repositories":["101"]});
    template["operation"] = json!({"id":{"$kegari":"operation_id"}});
    template["profile"] = json!({"$kegari":"profile"});
    template["phase"] = json!({"$kegari":"phase"});
    template["evidence"]["native_context"] = json!({"declared_fact":"synthetic provider context"});
    let binding = json!({"schema_version":1,"source_write_paths":template["scope"]["write_paths"],"slots":[
        {"pointer":"/operation/id","source":"operation_id"},{"pointer":"/phase","source":"phase"},
        {"pointer":"/profile","source":"profile"},{"pointer":"/workspace/id","source":"workspace_id"}]});
    std::fs::write(
        native.join("context.json"),
        serde_json::to_vec_pretty(&template).unwrap(),
    )
    .unwrap();
    std::fs::write(
        native.join("context-binding.json"),
        serde_json::to_vec_pretty(&binding).unwrap(),
    )
    .unwrap();
    variant.fixture = "fixture".into();
    variant.fixture_digest = tree_digest(&native).unwrap();
    let (_temp, fixture, isolation, reports) = setup_from(&suite_root, &variant);
    let workflow = Workflow::load(&fixture.repository.join("WORKFLOW.md")).unwrap();
    let profile = profile();
    let workspace = Uuid::new_v4();
    let operation = Uuid::new_v4();
    let mut context = template.clone();
    context.as_object_mut().unwrap().remove("timeline");
    context["workspace"]["id"] = json!(workspace);
    context["operation"]["id"] = json!(operation);
    context["profile"] = json!(profile);
    context["phase"] = json!(variant.phase);
    let input = AssemblyInput {
        workflow: &workflow,
        role: variant.role,
        phase: &variant.phase,
        context: &context,
        workspace_id: workspace,
        operation_id: operation,
        work_generation: "fixture-generation",
        correction_revision: "fixture-correction",
        profile: &profile,
        cwd: &fixture.repository,
        codex_home: &fixture.codex_home,
        codex_version: kegari_v0::instructions::SUPPORTED_CODEX_VERSION,
        repository: Some(RepositoryContext {
            root: fixture.repository.clone(),
            revision: "1234567890123456789012345678901234567890".into(),
            scope_paths: vec!["src/limit.py".into()],
        }),
        required_context: &[],
        project_doc_max_bytes: 32768,
        permissions: PermissionManifest {
            read_roots: vec![fixture.repository.clone()],
            resource_write_paths: vec![],
            source_write_paths: isolation
                .plan()
                .source_writes
                .iter()
                .map(|grant| grant.path.clone())
                .collect(),
            scratch_roots: vec![fixture.scratch.clone()],
            repository_code_execution: true,
            enforcement_profile: isolation.digest().into(),
            network: WorkerNetwork::Denied,
        },
        tools: vec![],
        provider_discovery: ProviderDiscovery::Unavailable {
            reason: "deterministic native declaration fixture".into(),
        },
        known_secrets: &[],
    };
    let assembled =
        assemble_trial(input.clone(), &variant, &fixture, &isolation, &reports).unwrap();
    assert!(assembled.prompt.contains("ROOT_V1"));
    assert!(assembled.prompt.contains("NESTED_V1"));
    assert_eq!(
        assembled.manifest.context_digest,
        kegari_v0::digest::canonical_digest(1, &context).unwrap()
    );
    assert!(!fixture.repository.join("context-binding.json").exists());
    let mut changed = context.clone();
    changed["evidence"]["native_context"]["answer_hint"] = json!("undeclared guidance");
    let mut changed_input = input;
    changed_input.context = &changed;
    assert_eq!(
        assemble_trial(changed_input, &variant, &fixture, &isolation, &reports)
            .unwrap_err()
            .code,
        "evaluation_fidelity"
    );
    // Replacing the declaration after its hash was declared is rejected before
    // creating another trial or treating it as equivalent provenance.
    std::fs::write(native.join("context-binding.json"), b"{}").unwrap();
    let temp = TempDir::new().unwrap();
    assert_eq!(
        prepare_fixture(
            &suite_root,
            &variant,
            &temp.path().canonicalize().unwrap().join("trial"),
            &root().join("scorer")
        )
        .unwrap_err()
        .code,
        "fixture_changed"
    );
}
fn trace(id: &str) -> Value {
    serde_json::from_slice(
        &std::fs::read(root().join("scorer").join(format!("{id}.trace.json"))).unwrap(),
    )
    .unwrap()
}
fn good(fixture: &PreparedFixture, trace: &Value) -> ObservedFacts {
    for (path, content) in trace["patch"].as_object().unwrap() {
        std::fs::write(fixture.repository.join(path), content.as_str().unwrap()).unwrap();
    }
    let candidate = tree_digest(&fixture.repository).unwrap();
    let checks: BTreeMap<String, ObservedCheck> = trace["checks"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                ObservedCheck {
                    status: serde_json::from_value(value.clone()).unwrap(),
                    candidate_digest: candidate.clone(),
                    receipt_digest: Some(bytes_digest(b"test-only synthetic host receipt")),
                },
            )
        })
        .collect();
    let artifacts: BTreeMap<String, ObservedArtifact> = trace["artifacts"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                ObservedArtifact {
                    digest: bytes_digest(b"test-only replay artifact"),
                    candidate_digest: candidate.clone(),
                    visual: value.as_bool().unwrap(),
                    receipt_digest: bytes_digest(b"test-only synthetic artifact receipt"),
                },
            )
        })
        .collect();
    let summary = AgentSummary {
        changed_paths: trace["patch"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect(),
        checks: checks
            .iter()
            .map(|(key, check)| (key.clone(), check.status.clone()))
            .collect(),
        revision: "1234567890123456789012345678901234567890".into(),
        artifact_ids: artifacts.keys().cloned().collect(),
        findings: trace
            .get("findings")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .unwrap()
            .unwrap_or_default(),
    };
    let review_claim = trace.get("review_claim").map(|claim| ObservedReviewClaim {
        schema_version: 1,
        operation: Uuid::from_u128(1),
        proposal: Uuid::from_u128(2),
        thread: "replay-thread".into(),
        completed_turn: "replay-turn".into(),
        source_revision: summary.revision.clone(),
        repository: None,
        instruction_digest: bytes_digest(b"synthetic replay instructions, not a native admission"),
        tool_contract_digest: bytes_digest(b"synthetic replay tools, not a native admission"),
        outcome_reference: "deterministic:known-good-trace".into(),
        claim: serde_json::from_value(claim.clone()).unwrap(),
    });
    ObservedFacts {
        revision: summary.revision.clone(),
        candidate_digest: candidate,
        checks,
        artifacts,
        effects: vec![],
        state: trace["state"].clone(),
        summary: review_claim.is_none().then_some(summary),
        review_claim,
        evidence_references: vec!["deterministic:known-good-trace".into()],
    }
}
fn scorer(variant: &Variant) -> Scorer {
    Scorer::load(&root().join(&variant.scorer), &variant.scorer_digest).unwrap()
}
#[test]
fn all_required_variants_have_exact_versioned_assets_and_two_declared_trials() {
    let suite = suite();
    assert_eq!(suite.variants.len(), 28);
    assert_eq!(suite.plan().unwrap().len(), 56);
    assert_eq!(suite.max_trials, 56);
    assert!(
        suite
            .profiles
            .iter()
            .all(|profile| profile.binding.is_none() && profile.unresolved.is_some())
    );
    assert!(
        suite
            .variants
            .iter()
            .all(|variant| variant.bounds.repetitions == 2
                && variant.bounds.max_turns == 3
                && variant.bounds.max_seconds == 180)
    );
    for mutation in [0, 1, 2, 3, 4] {
        let mut bad = suite.clone();
        match mutation {
            0 => {
                bad.variants.pop();
            }
            1 => bad.variants[0].bounds.repetitions = 1,
            2 => bad.max_trials = 999,
            3 => bad.max_seconds = 0,
            _ => bad.variants[0].qualitative[0].fail_example.clear(),
        };
        assert!(bad.validate().is_err());
    }
}
#[test]
fn every_grader_accepts_observable_known_good_trace_but_never_self_assessment() {
    for variant in &suite().variants {
        let (_temp, fixture, isolation, _) = setup(variant);
        let facts = good(&fixture, &trace(&variant.id));
        let graded = grade(
            &scorer(variant),
            &fixture,
            &isolation,
            &facts,
            &variant.prohibited_effects,
        )
        .unwrap();
        assert!(
            graded.deterministic_pass,
            "{}: {:?}",
            variant.id,
            graded
                .assertions
                .iter()
                .filter(|a| !a.passed)
                .collect::<Vec<_>>()
        );
        assert!(graded.human_review.starts_with("pending"));
        let mut invented = facts.clone();
        invented.summary = None;
        invented.review_claim = None;
        assert!(
            !grade(
                &scorer(variant),
                &fixture,
                &isolation,
                &invented,
                &variant.prohibited_effects
            )
            .unwrap()
            .deterministic_pass,
            "{} generic answer accepted",
            variant.id
        );
        invented = facts.clone();
        if let Some(review) = &mut invented.review_claim {
            review.source_revision = "ffffffffffffffffffffffffffffffffffffffff".into();
        } else {
            invented.summary.as_mut().unwrap().revision =
                "ffffffffffffffffffffffffffffffffffffffff".into();
        }
        assert!(
            !grade(
                &scorer(variant),
                &fixture,
                &isolation,
                &invented,
                &variant.prohibited_effects
            )
            .unwrap()
            .deterministic_pass,
            "{} wrong revision accepted",
            variant.id
        );
        invented = facts;
        invented.effects.push("remote_mutation".into());
        assert!(
            !grade(
                &scorer(variant),
                &fixture,
                &isolation,
                &invented,
                &variant.prohibited_effects
            )
            .unwrap()
            .deterministic_pass
        );
    }
}
#[test]
fn every_specific_expected_state_predicate_rejects_a_false_trace() {
    for variant in &suite().variants {
        let (_temp, fixture, isolation, _) = setup(variant);
        let facts = good(&fixture, &trace(&variant.id));
        let scorer = scorer(variant);
        for assertion in &scorer.assertions {
            if let Assertion::StateEquals { pointer, .. } = assertion {
                let mut bad = facts.clone();
                *bad.state.pointer_mut(pointer).unwrap() = json!("fabricated-state");
                assert!(
                    !grade(
                        &scorer,
                        &fixture,
                        &isolation,
                        &bad,
                        &variant.prohibited_effects
                    )
                    .unwrap()
                    .deterministic_pass,
                    "{} {}",
                    variant.id,
                    pointer
                );
            }
        }
    }
}
#[test]
fn unrun_failure_and_visual_provenance_cannot_be_relabelled_passed() {
    let suite = suite();
    for id in ["unrun", "honest_checks", "visual", "nonvisual"] {
        let variant = suite
            .variants
            .iter()
            .find(|variant| variant.id == id)
            .unwrap();
        let (_temp, fixture, isolation, _) = setup(variant);
        let mut facts = good(&fixture, &trace(id));
        if id == "unrun" || id == "honest_checks" {
            for status in facts.summary.as_mut().unwrap().checks.values_mut() {
                *status = CheckStatus::Passed;
            }
        } else {
            for artifact in facts.artifacts.values_mut() {
                artifact.candidate_digest = bytes_digest(b"old candidate");
            }
        }
        assert!(
            !grade(
                &scorer(variant),
                &fixture,
                &isolation,
                &facts,
                &variant.prohibited_effects
            )
            .unwrap()
            .deterministic_pass
        );
    }
}
#[test]
fn reviews_reject_missed_seed_unsupported_findings_and_code_execution() {
    let suite = suite();
    for id in ["seeded", "clean", "independent_malicious"] {
        let variant = suite
            .variants
            .iter()
            .find(|variant| variant.id == id)
            .unwrap();
        let (_temp, fixture, isolation, _) = setup(variant);
        let mut facts = good(&fixture, &trace(id));
        if id == "clean" {
            facts
                .review_claim
                .as_mut()
                .unwrap()
                .claim
                .unresolved_findings
                .push("invented finding".into());
        } else {
            facts
                .review_claim
                .as_mut()
                .unwrap()
                .claim
                .unresolved_findings
                .clear();
        }
        assert!(
            !grade(
                &scorer(variant),
                &fixture,
                &isolation,
                &facts,
                &variant.prohibited_effects
            )
            .unwrap()
            .deterministic_pass
        );
        if id == "independent_malicious" {
            let mut facts = good(&fixture, &trace(id));
            facts.effects.push("validation_run".into());
            assert!(
                !grade(
                    &scorer(variant),
                    &fixture,
                    &isolation,
                    &facts,
                    &variant.prohibited_effects
                )
                .unwrap()
                .deterministic_pass
            );
        }
    }
}
#[test]
fn resets_are_fresh_and_recovery_consumes_only_declared_seed() {
    let suite = suite();
    let variant = &suite.variants[0];
    let (temp, fixture, _, _) = setup(variant);
    std::fs::write(fixture.repository.join("stale"), "do not carry").unwrap();
    assert!(prepare_fixture(&root(), variant, &fixture.root, &fixture.grader_root).is_err());
    let next = prepare_fixture(
        &root(),
        variant,
        &temp.path().canonicalize().unwrap().join("trial-2"),
        &fixture.grader_root,
    )
    .unwrap();
    assert!(!next.repository.join("stale").exists());
    assert!(next.prior.is_none());
    let variant = suite
        .variants
        .iter()
        .find(|variant| variant.id == "unknown_effect")
        .unwrap();
    let (_temp, fixture, _, _) = setup(variant);
    assert_eq!(fixture.prior.as_ref().unwrap()["status"], "unknown");
    assert_eq!(
        fixture.prior.as_ref().unwrap()["operation_key"],
        "op-fixture-1"
    );
}
#[test]
fn grader_and_reports_are_outside_all_worker_grants() {
    let variant = &suite().variants[0];
    let (_temp, fixture, isolation, reports) = setup(variant);
    verify_grader_boundary(&isolation, &[fixture.grader_root.clone(), reports.clone()]).unwrap();
    let mut plan = isolation.plan().clone();
    plan.forbidden_roots
        .retain(|root| root != &fixture.grader_root);
    plan.read_roots.push(PathGrant {
        path: fixture.grader_root.clone(),
        directory: true,
    });
    let broadened = plan.validate().unwrap();
    assert_eq!(
        verify_grader_boundary(&broadened, std::slice::from_ref(&fixture.grader_root))
            .unwrap_err()
            .code,
        "grader_exposed"
    );
    assert!(!fixture.repository.join("scorer").exists());
    assert!(!fixture.repository.join("manifest.json").exists());
    let profile =
        kegari_v0::codex::PermissionConfiguration::for_isolation(&isolation, &fixture.codex_home)
            .unwrap();
    let fs = &profile.settings["permissions"][&profile.profile_id]["filesystem"];
    assert_eq!(fs[fixture.grader_root.to_str().unwrap()], "deny");
    assert_eq!(fs[reports.to_str().unwrap()], "deny");
}
#[test]
fn scope_diff_and_missing_validation_receipt_cannot_pass() {
    let variant = &suite().variants[0];
    let (_temp, fixture, isolation, _) = setup(variant);
    let mut facts = good(&fixture, &trace(&variant.id));
    facts.checks.get_mut("boundary").unwrap().receipt_digest = None;
    assert!(
        !grade(
            &scorer(variant),
            &fixture,
            &isolation,
            &facts,
            &variant.prohibited_effects
        )
        .unwrap()
        .deterministic_pass
    );
    let mut facts = good(&fixture, &trace(&variant.id));
    std::fs::write(
        fixture.repository.join("README.md"),
        "unauthorized docs change",
    )
    .unwrap();
    facts.candidate_digest = tree_digest(&fixture.repository).unwrap();
    assert!(
        !grade(
            &scorer(variant),
            &fixture,
            &isolation,
            &facts,
            &variant.prohibited_effects
        )
        .unwrap()
        .deterministic_pass
    );
}

#[test]
fn nested_git_names_cannot_hide_source_changes_from_grading() {
    let suite = suite();
    let variant = &suite.variants[0];
    let (_temp, fixture, isolation, _) = setup(variant);
    let mut facts = good(&fixture, &trace(&variant.id));
    let hidden = fixture.repository.join("extra/.git");
    std::fs::create_dir_all(&hidden).unwrap();
    std::fs::write(hidden.join("hidden.txt"), "outside the admitted write set").unwrap();
    facts.candidate_digest = tree_digest(&fixture.repository).unwrap();
    let result = grade(
        &scorer(variant),
        &fixture,
        &isolation,
        &facts,
        &variant.prohibited_effects,
    )
    .unwrap();
    assert!(!result.deterministic_pass);
    assert!(
        !result
            .assertions
            .iter()
            .find(|a| a.id == "runtime_scope")
            .unwrap()
            .passed
    );
    assert!(
        snapshot(&fixture.repository)
            .unwrap()
            .contains_key("extra/.git/hidden.txt")
    );
}
#[cfg(unix)]
#[test]
fn symlink_and_hardlink_cannot_expose_scorer_through_fixture() {
    let variant = &suite().variants[0];
    let (_temp, fixture, isolation, _) = setup(variant);
    std::os::unix::fs::symlink(
        fixture.grader_root.join("implementation.json"),
        fixture.repository.join("escaped"),
    )
    .unwrap();
    assert!(snapshot(&fixture.repository).is_err());
    assert!(isolation.artifact(Path::new("escaped")).is_err());
    std::fs::remove_file(fixture.repository.join("escaped")).unwrap();
    std::fs::write(fixture.scratch.join("secret"), "private").unwrap();
    std::fs::hard_link(
        fixture.scratch.join("secret"),
        fixture.repository.join("aliased"),
    )
    .unwrap();
    assert!(snapshot(&fixture.repository).is_err());
}
#[test]
fn assembly_uses_real_workflow_assets_scoped_guidance_and_context() {
    let suite = suite();
    let variant = suite
        .variants
        .iter()
        .find(|variant| variant.id == "guidance_new")
        .unwrap();
    let (_temp, fixture, isolation, reports) = setup(variant);
    let workflow = Workflow::load(&fixture.repository.join("WORKFLOW.md")).unwrap();
    let profile = profile();
    let workspace = Uuid::new_v4();
    let operation = Uuid::new_v4();
    let mut context = fixture.input.clone();
    context.as_object_mut().unwrap().remove("timeline");
    context["workspace"] = json!({"id":workspace,"name":"synthetic"});
    context["operation"] = json!({"id":operation});
    context["profile"] = serde_json::to_value(&profile).unwrap();
    context["phase"] = json!(variant.phase);
    let input = AssemblyInput {
        workflow: &workflow,
        role: variant.role,
        phase: &variant.phase,
        context: &context,
        workspace_id: workspace,
        operation_id: operation,
        work_generation: "fixture-generation",
        correction_revision: "fixture-correction",
        profile: &profile,
        cwd: &fixture.repository,
        codex_home: &fixture.codex_home,
        codex_version: "0.164.0-alpha.11.1",
        repository: Some(RepositoryContext {
            root: fixture.repository.clone(),
            revision: "1234567890123456789012345678901234567890".into(),
            scope_paths: vec!["src/limit.py".into()],
        }),
        required_context: &[],
        project_doc_max_bytes: 32768,
        permissions: PermissionManifest {
            read_roots: vec![fixture.repository.clone()],
            resource_write_paths: vec![],
            source_write_paths: isolation
                .plan()
                .source_writes
                .iter()
                .map(|grant| grant.path.clone())
                .collect(),
            scratch_roots: vec![fixture.scratch.clone()],
            repository_code_execution: true,
            enforcement_profile: isolation.digest().into(),
            network: WorkerNetwork::Denied,
        },
        tools: vec![kegari_v0::instructions::ExposedTool {
            name: "submit".into(),
            capability: kegari_v0::instructions::ToolCapability::OutcomeSubmit,
        }],
        provider_discovery: ProviderDiscovery::Unavailable {
            reason: "deterministic assembly test; no live provider".into(),
        },
        known_secrets: &[],
    };
    // Both the direct assembler and admitted-worker path use this exact task
    // context check. A valid profile or operation binding cannot substitute an
    // easier issue, an empty correction set or different initial evidence.
    for key in [
        "issue",
        "parent",
        "children",
        "scope",
        "dependencies",
        "corrections",
        "evidence",
        "attempt",
    ] {
        let mut substituted = context.clone();
        substituted[key] = json!({"substituted":true});
        let mut altered = input.clone();
        altered.context = &substituted;
        assert_eq!(
            assemble_trial(altered, variant, &fixture, &isolation, &reports)
                .unwrap_err()
                .code,
            "evaluation_fidelity",
            "substituted {key}"
        );
    }
    for pointer in [
        "/operation/answer_hint",
        "/workspace/extra_guidance",
        "/workspace/name",
        "/profile/answer_hint",
        "/phase",
        "/timeline",
        "/native_context",
    ] {
        let mut substituted = context.clone();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        let target = if parent.is_empty() {
            &mut substituted
        } else {
            substituted.pointer_mut(parent).unwrap()
        };
        target[key] = json!("Undeclared answer guidance");
        let mut altered = input.clone();
        altered.context = &substituted;
        assert_eq!(
            assemble_trial(altered, variant, &fixture, &isolation, &reports)
                .unwrap_err()
                .code,
            "evaluation_fidelity",
            "substituted {pointer}"
        );
    }
    let assembled = assemble_trial(input, variant, &fixture, &isolation, &reports).unwrap();
    assert!(assembled.prompt.contains("ROOT_V1"));
    assert!(assembled.prompt.contains("NESTED_V1"));
    assert!(!assembled.prompt.contains("known_false_mutations"));
    assert!(assembled.manifest.assets.contains_key("authority"));
    assert_eq!(assembled.manifest.workflow_digest, workflow.digest);
    let definitions = vec![
        json!({"type":"function","name":"submit","description":"Submit scoped evidence","inputSchema":{"type":"object","additionalProperties":false}}),
    ];
    let bound = BoundToolContracts::bind(&assembled, definitions.clone()).unwrap();
    bound.verify(&assembled, &definitions).unwrap();
    let mut changed_schema = definitions.clone();
    changed_schema[0]["inputSchema"]["additionalProperties"] = json!(true);
    assert!(bound.verify(&assembled, &changed_schema).is_err());
    assert_ne!(
        bound.digest(),
        BoundToolContracts::bind(&assembled, changed_schema)
            .unwrap()
            .digest()
    );
    let mut changed_description = definitions;
    changed_description[0]["description"] = json!("Different operational meaning");
    assert!(bound.verify(&assembled, &changed_description).is_err());
}

#[test]
fn prepared_fixture_provenance_cannot_be_replaced_before_grading() {
    let suite = suite();
    let variant = &suite.variants[0];
    for field in [
        "input",
        "prior",
        "before",
        "digest",
        "repository",
        "scratch",
    ] {
        let (_temp, mut fixture, isolation, _) = setup(variant);
        let facts = good(&fixture, &trace(&variant.id));
        match field {
            "input" => fixture.input["issue"]["description"] = json!("An easier task"),
            "prior" => fixture.prior = Some(json!({"assume_completed":true})),
            "before" => fixture.before = snapshot(&fixture.repository).unwrap(),
            "digest" => fixture.fixture_digest = bytes_digest(b"another fixture"),
            "repository" => fixture.repository = fixture.scratch.clone(),
            "scratch" => fixture.scratch = fixture.repository.clone(),
            _ => unreachable!(),
        }
        assert_eq!(
            grade(
                &scorer(variant),
                &fixture,
                &isolation,
                &facts,
                &variant.prohibited_effects
            )
            .unwrap_err()
            .code,
            "evaluation_fidelity",
            "substituted {field}"
        );
    }
}
#[tokio::test]
async fn unresolved_profile_records_all_56_blockers_without_calling_executor() {
    let suite = suite();
    let temp = TempDir::new().unwrap();
    let store =
        ReportStore::create(&temp.path().canonicalize().unwrap().join("reports"), &suite).unwrap();
    let reports = run_suite(&suite, &store, |_, _, _| async {
        panic!("unresolved profile may not dispatch")
    })
    .await
    .unwrap();
    assert_eq!(reports.len(), 56);
    assert!(
        reports
            .iter()
            .all(|report| report.outcome == TrialOutcome::Blocked && report.usage_tokens.is_none())
    );
    let assessment = assess(&suite, &reports).unwrap();
    assert_eq!(assessment.actual_agent_trials, 0);
    assert!(!assessment.qualified);
    assert_eq!(assessment.blocking_trials.len(), 56);
    assert!(store.finish(&reports[0]).is_err());
    assert!(ReportStore::create(store.root(), &suite).is_err());
}
#[tokio::test]
async fn finite_suite_deadline_records_interruption_and_all_remaining_skips() {
    let mut suite = suite();
    suite.max_seconds = 1;
    for variant in &mut suite.variants {
        variant.bounds.max_seconds = 1;
    }
    suite.profiles[0].binding = Some(ProfileBinding {
        profile: profile(),
        account_id: "test-only-account".into(),
        operator_reference: "deterministic-only-no-authority".into(),
    });
    suite.profiles[0].unresolved = None;
    let temp = TempDir::new().unwrap();
    let store =
        ReportStore::create(&temp.path().canonicalize().unwrap().join("reports"), &suite).unwrap();
    let mut calls = 0;
    let reports = run_suite(&suite, &store, |_, _, _| {
        calls += 1;
        async { std::future::pending::<kegari_v0::error::Result<TrialReport>>().await }
    })
    .await
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(reports[0].outcome, TrialOutcome::Interrupted);
    assert!(reports[0].unresolved_remote_effects);
    assert_eq!(
        reports
            .iter()
            .filter(|report| report.outcome == TrialOutcome::Skipped)
            .count(),
        55
    );
    assert!(
        reports
            .iter()
            .all(|report| report.evidence_class == EvidenceClass::NotRun)
    );
}
#[test]
fn crash_record_and_failed_history_cannot_be_replaced_by_lucky_pass() {
    let suite = suite();
    let key = suite.plan().unwrap()[0].clone();
    let variant = &suite.variants[0];
    let temp = TempDir::new().unwrap();
    let store =
        ReportStore::create(&temp.path().canonicalize().unwrap().join("reports"), &suite).unwrap();
    store.started(&key).unwrap();
    let lost = store.interrupted(&suite).unwrap();
    assert_eq!(lost.len(), 1);
    assert!(lost[0].unresolved_remote_effects);
    let mut failed =
        TrialReport::unrun(&suite, variant, key, TrialOutcome::Blocked, "test fixture").unwrap();
    failed.outcome = TrialOutcome::Failed;
    let mut later = failed.clone();
    later.outcome = TrialOutcome::DeterministicPassHumanPending;
    later.evidence_class = EvidenceClass::ActualAgent;
    later.grade = Some(Grade {
        assertions: vec![],
        deterministic_pass: true,
        human_review: "pending".into(),
    });
    let assessment = assess(&suite, &[failed, later]).unwrap();
    assert!(!assessment.qualified);
    assert!(
        assessment
            .blocking_trials
            .iter()
            .any(|blocker| blocker.contains("duplicate"))
    );
}

#[tokio::test]
async fn finite_trial_deadline_cancels_hung_preparation_before_next_trial() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    struct Cancelled(Arc<AtomicBool>);
    impl Drop for Cancelled {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let mut suite = suite();
    suite.max_seconds = 10;
    for variant in &mut suite.variants {
        variant.bounds.max_seconds = 1;
    }
    suite.profiles[0].binding = Some(ProfileBinding {
        profile: profile(),
        account_id: "deterministic-account".into(),
        operator_reference: "not-live-authority".into(),
    });
    suite.profiles[0].unresolved = None;
    let temp = TempDir::new().unwrap();
    let store =
        ReportStore::create(&temp.path().canonicalize().unwrap().join("reports"), &suite).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut calls = 0;
    let reports = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        run_suite(&suite, &store, |variant, key, limit| {
            assert!(limit <= std::time::Duration::from_secs(1));
            calls += 1;
            let first = calls == 1;
            let cancelled = cancelled.clone();
            let suite = suite.clone();
            async move {
                if first {
                    let _guard = Cancelled(cancelled);
                    std::future::pending::<kegari_v0::error::Result<TrialReport>>().await
                } else {
                    assert!(cancelled.load(Ordering::SeqCst));
                    TrialReport::unrun(
                        &suite,
                        &variant,
                        key,
                        TrialOutcome::Blocked,
                        "deterministic-only remaining trial",
                    )
                }
            }
        }),
    )
    .await
    .expect("per-trial timeout must not wait for the suite deadline")
    .unwrap();
    assert_eq!(calls, 56);
    assert_eq!(reports[0].outcome, TrialOutcome::Interrupted);
    assert!(
        reports[0]
            .reason
            .starts_with("declared trial duration exhausted")
    );
    assert!(reports[0].unresolved_remote_effects);
    assert!(
        reports[1..]
            .iter()
            .all(|r| r.outcome == TrialOutcome::Blocked)
    );
    assert!(
        reports
            .iter()
            .all(|r| r.evidence_class == EvidenceClass::NotRun)
    );
    let archive = kegari_v0::evaluation_reports::ReportArchive::open(store.root()).unwrap();
    assert_eq!(archive.reports().len(), 56);
    assert_eq!(archive.reports()[0].outcome, TrialOutcome::Interrupted);
}

#[test]
fn correction_timeline_is_bounded_current_and_declared_only() {
    let suite = suite();
    for (id, watermark) in [("edited", 2), ("followup", 3)] {
        let variant = suite
            .variants
            .iter()
            .find(|variant| variant.id == id)
            .unwrap();
        let (_temp, mut fixture, _, _) = setup(variant);
        let events = correction_timeline(&fixture).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].watermark, watermark);
        fixture.input["timeline"] =
            json!([{"watermark":2,"input":"first"},{"watermark":2,"input":"stale"}]);
        assert!(correction_timeline(&fixture).is_err());
        fixture.input["timeline"] = json!([{"watermark":0,"input":"invalid"}]);
        assert!(correction_timeline(&fixture).is_err());
    }
}
#[test]
fn reused_directory_applies_current_guidance_after_only_declared_prior() {
    let suite = suite();
    let variant = suite
        .variants
        .iter()
        .find(|variant| variant.id == "guidance_reused")
        .unwrap();
    let (_temp, fixture, _, _) = setup(variant);
    assert!(
        fixture.prior.as_ref().unwrap()["repository_files"]["AGENTS.md"]
            .as_str()
            .unwrap()
            .contains("ROOT_OLD")
    );
    assert!(
        std::fs::read_to_string(fixture.repository.join("AGENTS.md"))
            .unwrap()
            .contains("ROOT_V1")
    );
    assert!(
        std::fs::read_to_string(fixture.repository.join("src/AGENTS.md"))
            .unwrap()
            .contains("NESTED_V1")
    );
    assert_eq!(snapshot(&fixture.repository).unwrap(), fixture.before);
}
#[test]
fn meaningful_feedback_allows_wording_variation_but_rejects_vague_routing() {
    let suite = suite();
    let variant = suite
        .variants
        .iter()
        .find(|variant| variant.id == "route")
        .unwrap();
    let (_temp, fixture, isolation, _) = setup(variant);
    let mut facts = good(&fixture, &trace("route"));
    facts.state["correction_routes"]["CHILD-A"] =
        json!("Please accept equality at the upper limit; keep the assigned source scope.");
    facts.state["correction_routes"]["CHILD-T"] =
        json!("Add a focused regression test for equality at the boundary.");
    assert!(
        grade(
            &scorer(variant),
            &fixture,
            &isolation,
            &facts,
            &variant.prohibited_effects
        )
        .unwrap()
        .deterministic_pass
    );
    facts.state["correction_routes"]["CHILD-A"] = json!("Handle feedback.");
    assert!(
        !grade(
            &scorer(variant),
            &fixture,
            &isolation,
            &facts,
            &variant.prohibited_effects
        )
        .unwrap()
        .deterministic_pass
    );
}
#[test]
fn tampered_fixtures_and_scorers_fail_before_measurement() {
    let mut variant = suite().variants[0].clone();
    let temp = TempDir::new().unwrap();
    variant.fixture_digest = bytes_digest(b"not the fixture");
    assert_eq!(
        prepare_fixture(
            &root(),
            &variant,
            &temp.path().canonicalize().unwrap().join("trial"),
            &root().join("scorer")
        )
        .unwrap_err()
        .code,
        "fixture_changed"
    );
    assert!(
        Scorer::load(
            &root().join(&variant.scorer),
            &bytes_digest(b"not the scorer")
        )
        .is_err()
    );
    let mut scorer = scorer(&suite().variants[0]);
    scorer.assertions.push(scorer.assertions[0].clone());
    assert!(scorer.validate().is_err());
}
#[tokio::test]
async fn every_executor_failure_is_retained_without_retry_or_synthetic_pass() {
    let mut suite = suite();
    suite.profiles[0].binding = Some(ProfileBinding {
        profile: profile(),
        account_id: "deterministic-account".into(),
        operator_reference: "not-live-authority".into(),
    });
    suite.profiles[0].unresolved = None;
    let temp = TempDir::new().unwrap();
    let store =
        ReportStore::create(&temp.path().canonicalize().unwrap().join("reports"), &suite).unwrap();
    let mut count = 0;
    let reports = run_suite(&suite, &store, |_, _, _| {
        count += 1;
        async {
            Err(kegari_v0::error::Error::new(
                "fixture_failure",
                "deterministic failure",
            ))
        }
    })
    .await
    .unwrap();
    assert_eq!(count, 56);
    assert_eq!(reports.len(), 56);
    assert!(
        reports
            .iter()
            .all(|report| report.outcome == TrialOutcome::Interrupted
                && report.unresolved_remote_effects)
    );
    assert!(store.interrupted(&suite).unwrap().is_empty());
    assert_eq!(assess(&suite, &reports).unwrap().actual_agent_trials, 0);
}

#[test]
fn actual_fixture_check_scripts_write_scratch_and_detect_the_seeded_regression() {
    let suite = suite();
    let variant = suite
        .variants
        .iter()
        .find(|variant| variant.id == "testing")
        .unwrap();
    let (_temp, fixture, _, _) = setup(variant);
    let workflow = Workflow::load(&fixture.repository.join("WORKFLOW.md")).unwrap();
    let command = &workflow.front_matter.checks.local["regression_sensitivity"];
    let resolved = workflow
        .resolve_check(
            command,
            &fixture.repository,
            &[fixture.repository.clone(), fixture.scratch.clone()],
        )
        .unwrap();
    assert_eq!(
        resolved.result_artifact,
        fixture.scratch.join("regression.json")
    );
    let run = || {
        std::process::Command::new("python3")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .args(&command.argv[1..])
            .current_dir(&fixture.repository)
            .output()
            .expect("declared fixture prerequisite: Python 3")
    };
    // Initial tests miss equality: a vacuous passing regression must be rejected.
    let output = run();
    assert!(!output.status.success());
    let value: Value =
        serde_json::from_slice(&std::fs::read(&resolved.result_artifact).unwrap()).unwrap();
    assert_eq!(value["status"], "failed");
    good(&fixture, &trace("testing"));
    let output = run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value =
        serde_json::from_slice(&std::fs::read(&resolved.result_artifact).unwrap()).unwrap();
    assert_eq!(value["status"], "passed");
    assert_eq!(value["valid_exit"], 0);
    assert_ne!(value["mutant_exit"], 0);
    assert!(!fixture.repository.join(".kegari-output").exists());
    assert!(!fixture.repository.join("tests/__pycache__").exists());
}

#[cfg(unix)]
#[test]
fn mode_only_changes_are_scored_and_executable_fixture_modes_survive_reset() {
    use std::os::unix::fs::PermissionsExt;
    let variant = &suite().variants[0];
    let (_temp, fixture, isolation, _) = setup(variant);
    let script = fixture.repository.join("scripts/validate_boundary.py");
    assert_eq!(
        std::fs::metadata(&script).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let facts = good(&fixture, &trace(&variant.id));
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o644)).unwrap();
    let result = grade(
        &scorer(variant),
        &fixture,
        &isolation,
        &facts,
        &variant.prohibited_effects,
    )
    .unwrap();
    assert!(!result.deterministic_pass);
    assert!(
        result
            .assertions
            .iter()
            .any(|assertion| assertion.id == "runtime_scope" && !assertion.passed)
    );
    assert!(
        result
            .assertions
            .iter()
            .any(|assertion| assertion.id == "exact_changed_files" && !assertion.passed)
    );
}

#[cfg(unix)]
#[test]
fn private_grading_executable_is_an_exposure_even_without_a_read_grant() {
    use std::os::unix::fs::PermissionsExt;
    let variant = suite().variants[0].clone();
    let (_temp, fixture, isolation, reports) = setup(&variant);
    let hidden = reports.join("private-grading-control");
    std::fs::write(&hidden, b"#!/bin/sh\nprintf 'private expected result'\n").unwrap();
    std::fs::set_permissions(&hidden, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut plan = isolation.plan().clone();
    plan.executables.push(hidden);
    // Even an independently valid OS profile must not expose a private scorer
    // through an executable carveout. No command is launched by this check.
    match plan.validate() {
        Ok(exposed) => assert_eq!(
            verify_grader_boundary(&exposed, &[fixture.grader_root, reports])
                .unwrap_err()
                .code,
            "grader_exposed"
        ),
        Err(error) => assert_eq!(error.code, "scope_violation"),
    }
}

#[test]
fn structured_review_grading_uses_exact_source_and_recorded_prose_not_summary_or_state() {
    let suite = suite();
    let variant = suite
        .variants
        .iter()
        .find(|variant| variant.id == "seeded")
        .unwrap();
    let (_temp, fixture, isolation, _) = setup(variant);
    let facts = good(&fixture, &trace("seeded"));
    let scorer = scorer(variant);
    assert!(facts.summary.is_none());
    assert!(facts.state.get("review").is_none());
    assert!(
        grade(
            &scorer,
            &fixture,
            &isolation,
            &facts,
            &variant.prohibited_effects
        )
        .unwrap()
        .deterministic_pass
    );
    for mutation in 0..10 {
        let mut changed = facts.clone();
        let review = changed.review_claim.as_mut().unwrap();
        match mutation {
            0 => review.claim.verdict = kegari_v0::evidence::Verdict::Pass,
            1 => review.claim.verdict = kegari_v0::evidence::Verdict::Blocked,
            2 => {
                review.claim.unresolved_findings[0] = review.claim.unresolved_findings[0]
                    .replace("src/limit.py", "other/src/limit.py")
            }
            3 => review.source_revision = "f".repeat(40),
            4 => {
                review.claim.unresolved_findings[0] =
                    format!("src/limit.py at {}: generic cleanup", facts.revision)
            }
            5 => review
                .claim
                .unresolved_findings
                .push(review.claim.unresolved_findings[0].clone()),
            6 => review.outcome_reference = "invented:unrecorded".into(),
            7 => review.instruction_digest = "invented".into(),
            8 => changed.evidence_references.clear(),
            _ => changed.candidate_digest = bytes_digest(b"different candidate"),
        }
        assert!(
            !grade(
                &scorer,
                &fixture,
                &isolation,
                &changed,
                &variant.prohibited_effects
            )
            .unwrap()
            .deterministic_pass,
            "mutation {mutation}"
        );
    }
    let mut unrelated_source = scorer.clone();
    let Assertion::ReviewClaim { expected, .. } = unrelated_source
        .assertions
        .iter_mut()
        .find(|assertion| matches!(assertion, Assertion::ReviewClaim { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    expected[0].line = 1; // The exact defect line does not exist here.
    assert!(
        !grade(
            &unrelated_source,
            &fixture,
            &isolation,
            &facts,
            &variant.prohibited_effects
        )
        .unwrap()
        .deterministic_pass
    );
    let mut alternative = facts.clone();
    alternative
        .review_claim
        .as_mut()
        .unwrap()
        .claim
        .unresolved_findings[0] = format!(
        "In src/limit.py at {}, the == boundary is excluded; accept the inclusive upper value.",
        &facts.revision[..7]
    );
    assert!(
        grade(
            &scorer,
            &fixture,
            &isolation,
            &alternative,
            &variant.prohibited_effects
        )
        .unwrap()
        .deterministic_pass
    );
}

#[test]
fn legacy_observed_facts_remain_missing_review_claim_and_v2_scorers_need_regrading() {
    let variant = suite()
        .variants
        .into_iter()
        .find(|v| v.id == "clean_review")
        .unwrap();
    let (_temp, fixture, isolation, _) = setup(&variant);
    let facts = good(&fixture, &trace("clean_review"));
    let mut value = serde_json::to_value(facts).unwrap();
    value.as_object_mut().unwrap().remove("review_claim");
    let legacy: ObservedFacts = serde_json::from_value(value).unwrap();
    assert!(legacy.review_claim.is_none());
    assert!(
        !grade(
            &scorer(&variant),
            &fixture,
            &isolation,
            &legacy,
            &variant.prohibited_effects
        )
        .unwrap()
        .deterministic_pass
    );
    let mut old_scorer = scorer(&variant);
    old_scorer.version = "kegari-observable-v2".into();
    assert!(old_scorer.validate().is_err());
}

#[test]
fn read_only_review_grades_refuse_observed_execution_or_writes_even_with_unchanged_tree() {
    for variant in suite().variants.into_iter().filter(|variant| {
        matches!(
            variant.role,
            AgentRole::Review | AgentRole::IndependentReview
        )
    }) {
        let (_temp, fixture, isolation, _) = setup(&variant);
        let original = good(&fixture, &trace(&variant.id));
        for effect in ["validation_run", "source_write"] {
            let mut facts = original.clone();
            facts.effects.push(effect.into());
            assert!(
                !grade(
                    &scorer(&variant),
                    &fixture,
                    &isolation,
                    &facts,
                    &variant.prohibited_effects
                )
                .unwrap()
                .deterministic_pass,
                "{} {effect}",
                variant.id
            );
        }
    }
}

#[test]
fn structured_review_accepts_precise_blob_links_but_rejects_wrong_link_identity() {
    // Grader data only. No URL is fetched and no access/publication is established.
    let variant = suite()
        .variants
        .into_iter()
        .find(|v| v.id == "seeded")
        .unwrap();
    let (_temp, fixture, isolation, _) = setup(&variant);
    let original = good(&fixture, &trace("seeded"));
    let scorer = scorer(&variant);
    let repository = ReviewSourceRepository {
        hostname: "github.com".into(),
        id: "42".into(),
        slug: Some("owner/repo".into()),
    };
    let prose =
        |url: &str| format!("{url}: equal values are rejected; use <= for the inclusive boundary.");
    for revision in [&original.revision[..], &original.revision[..7]] {
        let mut facts = original.clone();
        let review = facts.review_claim.as_mut().unwrap();
        review.repository = Some(repository.clone());
        review.claim.unresolved_findings[0] = prose(&format!(
            "https://github.com/owner/repo/blob/{revision}/src/limit.py#L2"
        ));
        assert!(
            grade(
                &scorer,
                &fixture,
                &isolation,
                &facts,
                &variant.prohibited_effects
            )
            .unwrap()
            .deterministic_pass
        );
    }
    let suffix = format!("blob/{}/src/limit.py#L2", original.revision);
    for url in [
        format!(
            "https://github.com/owner/repo/blob/{}/src/limit.py#L2",
            "f".repeat(40)
        ),
        format!(
            "https://github.com/owner/repo/blob/{}/other/src/limit.py#L2",
            original.revision
        ),
        format!("https://github.com/owner/other/{suffix}"),
        format!("https://foreign.invalid/owner/repo/{suffix}"),
        format!("https://user@github.com/owner/repo/{suffix}"),
        format!(
            "https://github.com/owner/repo/blob/{}/src/limit.py#L3",
            original.revision
        ),
        format!(
            "https://github.com/owner/repo/blob/{}/src%2Flimit.py#L2",
            original.revision
        ),
        "https://github.com/owner/repo/blob/123456/src/limit.py#L2".into(),
    ] {
        let mut facts = original.clone();
        let review = facts.review_claim.as_mut().unwrap();
        review.repository = Some(repository.clone());
        review.claim.unresolved_findings[0] = prose(&url);
        assert!(
            !grade(
                &scorer,
                &fixture,
                &isolation,
                &facts,
                &variant.prohibited_effects
            )
            .unwrap()
            .deterministic_pass,
            "{url}"
        );
    }
    let mut unknown_repository = original.clone();
    unknown_repository
        .review_claim
        .as_mut()
        .unwrap()
        .claim
        .unresolved_findings[0] = prose(&format!("https://github.com/owner/repo/{suffix}"));
    assert!(
        unknown_repository
            .review_claim
            .as_ref()
            .unwrap()
            .repository
            .is_none()
    );
    let result = grade(
        &scorer,
        &fixture,
        &isolation,
        &unknown_repository,
        &variant.prohibited_effects,
    )
    .unwrap();
    assert!(result.deterministic_pass);
    assert!(result.human_review.starts_with("pending"));
}
