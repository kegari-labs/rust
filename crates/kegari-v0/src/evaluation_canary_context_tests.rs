//! Real private-owner/creation-anchor IO plus explicit context data. No runtime
//! request, native authority, permit, WorkerContract or joined run is fabricated.
use super::*;

fn declared() -> (NativeContextBinding, Value, crate::providers::PullRequest) {
    let recipe: Value = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations/native/pr_attack-v1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let binding = serde_json::from_value(recipe["binding"].clone()).unwrap();
    let mut input = recipe["context_without_base"].clone();
    // Inspected generated E07 identities are data only in this component test.
    input["issue"]["pull_request"]["base_sha"] = json!("21bc1e88cef10799c0eed6fd8ecf841aba3eb91d");
    input["scope"]["base"] = input["issue"]["pull_request"]["base_sha"].clone();
    let mut pr = input["issue"]["pull_request"].clone();
    pr["id"] = json!(1001);
    pr["number"] = json!(17);
    pr["url"] = json!("https://github.invalid/evaluation/fixture/pull/17");
    pr["head_sha"] = json!("4a9fd6935fd53fcf16156aed23bb274d78b66e96");
    (binding, input, serde_json::from_value(pr).unwrap())
}
fn binding(case: &Case, pr: &crate::providers::PullRequest) -> CanaryOperandBinding {
    let component = bytes_digest(b"explicit context component, no native authority");
    CanaryOperandBinding {
        identity: case.owner.identity.clone(),
        command_digest: component.clone(),
        request_digest: component.clone(),
        lease_digest: component.clone(),
        source_digest: component.clone(),
        source_selection_digest: component.clone(),
        authority_digest: component.clone(),
        phase_digest: component,
        profile: case.suite.profiles[0]
            .binding
            .as_ref()
            .unwrap()
            .profile
            .clone(),
        head_revision: pr.head_sha.clone(),
        pull_request_digest: canonical_digest(1, pr).unwrap(),
        max_turns: case.variant.bounds.max_turns,
        max_seconds: case.variant.bounds.max_seconds,
    }
}
fn host<'a>(
    case: &'a Case,
    pr: &'a crate::providers::PullRequest,
    operand: Option<&'a OwnedCanaryPathOperand>,
) -> HostContext<'a> {
    HostContext {
        workspace: case.owner.identity.workspace,
        operation: case.owner.identity.operation,
        profile: &case.suite.profiles[0].binding.as_ref().unwrap().profile,
        phase: "independent_review",
        work_generation: "",
        correction_revision: "",
        repository_revision: Some(&pr.head_sha),
        independent_pull_request: Some(pr),
        canary_operand: operand,
    }
}
#[test]
fn schema_three_requires_exact_closed_inventory_and_original_owned_operand() {
    let case = Case::new();
    let (declaration, input, pr) = declared();
    declaration.validate(&input).unwrap();
    assert_eq!(
        declaration
            .materialize(&input, &host(&case, &pr, None))
            .unwrap_err()
            .code,
        "canary_binding"
    );
    let original = serde_json::to_value(&declaration).unwrap();
    for version in [1, 2, 4] {
        let mut changed = original.clone();
        changed["schema_version"] = json!(version);
        let changed: NativeContextBinding = serde_json::from_value(changed).unwrap();
        assert!(changed.validate(&input).is_err());
    }
    for field in ["pointer", "source"] {
        let mut changed = original.clone();
        changed["slots"][0][field] = if field == "pointer" {
            json!("/scope/forbidden_canary_path")
        } else {
            json!("operation_id")
        };
        let changed: NativeContextBinding = serde_json::from_value(changed).unwrap();
        assert!(changed.validate(&input).is_err());
    }
    let mut changed = original.clone();
    changed["source_write_paths"] = json!(["src/limit.py"]);
    assert!(
        serde_json::from_value::<NativeContextBinding>(changed)
            .unwrap()
            .validate(&input)
            .is_err()
    );
    let mut extra = input.clone();
    extra["evidence"]["answer"] = json!("not declared");
    assert!(declaration.validate(&extra).is_err());
    let mut raw = input.clone();
    raw["evidence"]["forbidden_canary_path"] = json!("/tmp/raw-path-is-not-a-capability");
    assert!(declaration.validate(&raw).is_err());
    assert_eq!(serde_json::to_value(&declaration).unwrap(), original);
}
#[test]
fn schema_three_uses_original_protected_path_and_only_augments_empty_evidence() {
    let case = Case::new();
    case.stage();
    let (declaration, input, pr) = declared();
    let expected_binding = binding(&case, &pr);
    let operand = case
        .owner
        .context_operand(expected_binding.clone())
        .unwrap();
    operand
        .verify_binding(&case.owner, &expected_binding)
        .unwrap();
    let expected_path = case.grader.path().join("canary-diagnostic/marker.bin");
    let materialized = declaration
        .materialize(&input, &host(&case, &pr, Some(&operand)))
        .unwrap();
    assert_eq!(
        materialized["evidence"],
        json!({"forbidden_canary_path":expected_path})
    );
    let marker = std::fs::read(&expected_path).unwrap();
    assert!(
        !serde_json::to_vec(&materialized)
            .unwrap()
            .windows(marker.len())
            .any(|w| w == marker)
    );
    let mut actual = materialized.clone();
    actual["evidence"] = json!({});
    let before = actual.clone();
    operand.augment_context(&mut actual).unwrap();
    assert_eq!(actual, materialized);
    let mut restored = actual.clone();
    restored["evidence"] = json!({});
    assert_eq!(restored, before);
    assert_eq!(
        operand.augment_context(&mut actual).unwrap_err().code,
        "canary_binding"
    );
    assert_eq!(actual, materialized);
    let mut wrong = before;
    wrong["operation"]["id"] = json!(Uuid::new_v4());
    let unchanged = wrong.clone();
    assert_eq!(
        operand.augment_context(&mut wrong).unwrap_err().code,
        "canary_binding"
    );
    assert_eq!(wrong, unchanged);
    assert!(case.owner.diagnostic().unwrap().binding.is_none());
    assert!(!case.owner.diagnostic().unwrap().final_readback);
}
#[test]
fn canary_operand_rejects_foreign_owner_request_source_profile_and_host() {
    let case = Case::new();
    case.stage();
    let (_, _, pr) = declared();
    let original = binding(&case, &pr);
    let operand = case.owner.context_operand(original.clone()).unwrap();
    let foreign = Case::new();
    foreign.stage();
    assert_eq!(
        operand
            .verify_binding(&foreign.owner, &original)
            .unwrap_err()
            .code,
        "canary_binding"
    );
    for which in 0..5 {
        let mut changed = original.clone();
        match which {
            0 => changed.request_digest = bytes_digest(b"changed request"),
            1 => changed.source_digest = bytes_digest(b"changed source"),
            2 => changed.profile.model = "different".into(),
            3 => changed.identity.command = Uuid::new_v4(),
            _ => changed.lease_digest = bytes_digest(b"changed owner"),
        }
        assert_eq!(
            operand
                .verify_binding(&case.owner, &changed)
                .unwrap_err()
                .code,
            "canary_binding"
        );
    }
    let mut different = pr.clone();
    different.head_sha = "1111111111111111111111111111111111111111".into();
    assert_eq!(
        operand
            .path_for_host(&host(&case, &different, Some(&operand)))
            .unwrap_err()
            .code,
        "canary_binding"
    );
    let mut foreign_host = host(&case, &pr, Some(&operand));
    foreign_host.operation = Uuid::new_v4();
    assert_eq!(
        operand.path_for_host(&foreign_host).unwrap_err().code,
        "canary_binding"
    );
}
#[test]
fn missing_or_identically_replaced_operand_marker_refuses_without_remint() {
    for replacement in [false, true] {
        let case = Case::new();
        case.stage();
        let (_, _, pr) = declared();
        let operand = case.owner.context_operand(binding(&case, &pr)).unwrap();
        let directory = case.owner.lock().unwrap().directory.clone().unwrap();
        let bytes = directory.read_bytes("marker.bin", 1024).unwrap();
        std::fs::remove_file(directory.path().join("marker.bin")).unwrap();
        if replacement {
            directory
                .create_bytes_exclusive("marker.bin", &bytes)
                .unwrap();
        }
        let error = operand.revalidate().unwrap_err();
        assert_eq!(
            error.code,
            if replacement {
                "canary_conflict"
            } else {
                "record_missing"
            }
        );
        assert!(case.owner.context_operand(binding(&case, &pr)).is_err());
        assert_eq!(directory.path().join("marker.bin").exists(), replacement);
        assert!(case.owner.diagnostic().unwrap().error.is_some());
        assert!(!case.owner.diagnostic().unwrap().final_readback);
    }
}
#[test]
fn canary_operand_export_requires_staged_owner_and_cannot_rebind() {
    let case = Case::new();
    let (_, _, pr) = declared();
    assert_eq!(
        case.owner
            .context_operand(binding(&case, &pr))
            .err()
            .unwrap()
            .code,
        "canary_unavailable"
    );
    let staged = Case::new();
    staged.stage();
    let original = binding(&staged, &pr);
    let _operand = staged.owner.context_operand(original.clone()).unwrap();
    let directory = staged.owner.lock().unwrap().directory.clone().unwrap();
    let before = directory.read_bytes("initial.json", 4096).unwrap();
    assert_eq!(
        staged.owner.context_operand(original).err().unwrap().code,
        "canary_binding"
    );
    assert_eq!(directory.read_bytes("initial.json", 4096).unwrap(), before);
    assert!(!directory.path().join("binding.json").exists());
}

#[test]
fn prepared_schema_three_fixture_retains_original_operand_and_exact_context() {
    let mut case = Case::new();
    let (declaration, input, pr) = declared();
    let source = case.suite_root.join(&case.variant.fixture);
    std::fs::create_dir_all(source.join("repository")).unwrap();
    std::fs::write(
        source.join("repository/WORKFLOW.md"),
        "---\nschema_version: 1\nchecks:\n  local: {}\n---\n{{ evidence | json }}\n",
    )
    .unwrap();
    std::fs::write(
        source.join("context.json"),
        serde_json::to_vec(&input).unwrap(),
    )
    .unwrap();
    std::fs::write(
        source.join("context-binding.json"),
        serde_json::to_vec(&declaration).unwrap(),
    )
    .unwrap();
    case.variant.fixture_digest = tree_digest(&source).unwrap();
    *case
        .suite
        .variants
        .iter_mut()
        .find(|v| v.id == case.variant.id)
        .unwrap() = case.variant.clone();
    case.trial = case
        .suite
        .plan()
        .unwrap()
        .into_iter()
        .find(|t| t.variant == case.variant.id)
        .unwrap();
    // Start a fresh actual private owner for this captured component fixture.
    // The initial Case owner was never staged and confers no runtime authority.
    let identity = case.owner.identity.clone();
    case.owner = CanaryMaterialization::pending(CanaryStageInput {
        runtime: identity.runtime,
        workspace: identity.workspace,
        command: identity.command,
        operation: identity.operation,
        suite: &case.suite,
        variant: &case.variant,
        trial: &case.trial,
        deadline: tokio::time::Instant::now() + Duration::from_secs(60),
    })
    .unwrap();
    case.stage();
    let execution = case.root.join("workers");
    std::fs::create_dir(&execution).unwrap();
    let mut fixture = prepare_worker_fixture(
        &case.suite_root,
        &case.variant,
        &case.root.join("fixture"),
        case.grader.path(),
        &execution,
        identity.operation,
    )
    .unwrap();
    assert!(fixture.requires_canary_context().unwrap());
    fixture.attach_canary(case.owner.clone()).unwrap();
    let original = fixture.preparation_digest.clone();
    // No operand preserves the existing exact canonical preparation shape.
    assert_eq!(original, canonical_digest(1, &json!({
        "root":fixture.root,"repository":fixture.repository,"scratch":fixture.scratch,
        "codex_home":fixture.codex_home,"grader_root":fixture.grader_root,
        "before":fixture.before,"input":fixture.input,"prior":fixture.prior,
        "fixture_digest":fixture.fixture_digest,"context_binding":fixture.context_binding,
        "worker_placement":fixture.worker_placement,"worker_manifest":fixture.worker_manifest
    })).unwrap());
    let operand = case.owner.context_operand(binding(&case, &pr)).unwrap();
    fixture.attach_canary_operand(operand.clone()).unwrap();
    assert!(Arc::ptr_eq(
        fixture.canary_operand.as_ref().unwrap(),
        &operand
    ));
    assert_ne!(fixture.preparation_digest, original);
    fixture.verify_preparation().unwrap();
    let host = host(&case, &pr, fixture.canary_operand.as_deref());
    let actual = declaration.materialize(&input, &host).unwrap();
    verify_fixture_context(&actual, &fixture, &host).unwrap();
    for changed in [
        json!({}),
        json!({"forbidden_canary_path":"/tmp/substituted"}),
    ] {
        let mut altered = actual.clone();
        altered["evidence"] = changed;
        assert_eq!(
            verify_fixture_context(&altered, &fixture, &host)
                .unwrap_err()
                .code,
            "evaluation_fidelity"
        );
    }
    let saved = fixture.preparation_digest.clone();
    assert_eq!(
        fixture.attach_canary_operand(operand).unwrap_err().code,
        "canary_binding"
    );
    assert_eq!(fixture.preparation_digest, saved);
    fixture.canary_operand = None;
    assert_eq!(
        fixture.verify_preparation().unwrap_err().code,
        "evaluation_fidelity"
    );
    assert!(case.owner.diagnostic().unwrap().binding.is_none());
    assert!(!case.owner.diagnostic().unwrap().final_readback);
}
