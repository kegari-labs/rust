use kegari_config::Profile;
use kegari_context::{instructions::*, workflow::Workflow};
use kegari_digest::canonical_digest;
use serde_json::{Value, json};
use uuid::Uuid;

#[test]
fn complete_authenticated_history_uses_existing_public_projection_and_readonly_assembly() {
    // Production protected archive I/O, original public context projection,
    // and instruction assembly. Descriptive data creates no worker/exec cap.
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let repository = base.join("repository");
    let scratch = base.join("scratch");
    let auth = base.join("auth");
    for path in [&repository, &scratch, &auth] {
        std::fs::create_dir(path).unwrap();
    }
    let records = kegari_storage::PrivateDir::create(&base.join("records")).unwrap();
    let key = kegari_secrets::AttestationKey::new("assembly-fixture".into(), vec![9; 32]).unwrap();
    let (namespace, evidence, artifacts) = crate::validation_archive::tests::descriptive_fixture();
    let archive =
        crate::validation_archive::retain(&records, &key, namespace.clone(), &evidence, |proof| {
            Ok(artifacts[&canonical_digest(1, proof)?].clone())
        })
        .unwrap();
    let bodies: Vec<_> = archive.evidence().attempts.iter().flat_map(|attempt| &attempt.artifacts)
        .map(|proof| json!({"proof_digest":canonical_digest(1,proof).unwrap(),"proof":proof,"encoding":"utf8",
            "data":String::from_utf8(archive.read_artifact(proof).unwrap()).unwrap()})).collect();
    let original =
        json!({"delivery_review":{"validation":archive.evidence(),"validation_artifacts":bodies}});
    let public = crate::delivery::public_model_context(&original).unwrap();
    assert!(original.pointer("/delivery_review/validation/history/0/original_binding/work/generation/authorization").is_some());
    assert!(public.pointer("/delivery_review/validation/history/0/original_binding/work/generation/authorization").is_none());
    assert_eq!(public.pointer("/delivery_review/validation/history/0/original_binding/work/generation/authorization_digest"),
        original.pointer("/delivery_review/validation/history/0/original_binding/work/generation/authorization"));
    let profile = Profile {
        tool: "codex".into(),
        model: "fixture-model".into(),
        reasoning: "high".into(),
        speed: kegari_config::Speed::Standard,
    };
    let operation = Uuid::new_v4();
    let context = json!({"issue":{"description":"Assess every actual attempt"},"parent":{},"children":[],
        "workspace":{"id":namespace.workspace_id},"scope":{},"dependencies":[],"corrections":[],
        "evidence":{"delivery_assignment":public},"attempt":1,"phase":"child_review","profile":profile,"operation":{"id":operation}});
    let workflow = Workflow::parse(
        "Review the complete supplied history.",
        base.join("WORKFLOW.md"),
    )
    .unwrap();
    let input = |context| AssemblyInput {
        runtime_build: crate::BUILD_ID,
        workflow: &workflow,
        role: AgentRole::Review,
        phase: "child_review",
        context,
        workspace_id: namespace.workspace_id,
        operation_id: operation,
        work_generation: "work",
        correction_revision: "correction",
        profile: &profile,
        cwd: &repository,
        codex_home: &auth,
        codex_version: SUPPORTED_CODEX_VERSION,
        repository: Some(RepositoryContext {
            root: repository.clone(),
            revision: Some("a".repeat(40)),
            scope_paths: vec![".".into()],
        }),
        required_context: &[],
        project_doc_max_bytes: 32768,
        permissions: PermissionManifest {
            read_roots: vec![repository.clone()],
            resource_write_paths: vec![],
            source_write_paths: vec![],
            scratch_roots: vec![scratch.clone()],
            repository_code_execution: false,
            enforcement_profile: "fixture-readonly".into(),
            network: WorkerNetwork::BrokerOnly,
        },
        tools: vec![ExposedTool {
            name: "outcome_submit".into(),
            capability: ToolCapability::OutcomeSubmit,
        }],
        provider_discovery: ProviderDiscovery::Unavailable {
            reason: "no native model observation in component test".into(),
        },
        known_secrets: &[],
    };
    let assembled = assemble(input(&context)).unwrap();
    let structured = assembled
        .prompt
        .split("# Admitted work context (structured task data)\n\n")
        .nth(1)
        .unwrap()
        .split("\n\n# Applicable repository-owned guidance")
        .next()
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(structured).unwrap(), context);
    assert_eq!(
        context
            .pointer("/evidence/delivery_assignment/delivery_review/validation/attempts")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(!assembled.manifest.permissions.repository_code_execution);
    assert!(assembled.manifest.permissions.source_write_paths.is_empty());
    let mut credential_context = context.clone();
    credential_context["issue"]["authorization"] = json!("Bearer credential-shaped-value");
    assert_eq!(
        assemble(input(&credential_context)).unwrap_err().code,
        "access_denied"
    );
}

#[test]
fn context_artifact_digest_cases_match_backend_validator() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../kegari-context/tests/fixtures/digest-syntax.json"
    ))
    .unwrap();
    for value in corpus["accepted"].as_array().unwrap() {
        crate::evidence::validate_digest(value.as_str().unwrap()).unwrap();
    }
    let expected: kegari_types::error::Error =
        serde_json::from_value(corpus["error"].clone()).unwrap();
    for value in corpus["rejected"].as_array().unwrap() {
        assert_eq!(
            crate::evidence::validate_digest(value.as_str().unwrap()).unwrap_err(),
            expected
        );
    }
}
