//! Real filesystem/Workflow/assembly components; no runtime permit or native worker.
use super::*;
use kegari_config::Speed;
use kegari_context::instructions::PermissionManifest;
use kegari_context::instructions::ProviderDiscovery;
use kegari_context::instructions::WorkerNetwork;
use kegari_context::workflow::Workflow;
use kegari_execution::IsolationPlan;
use serde_json::json;
use uuid::Uuid;

struct Case {
    _temp: tempfile::TempDir,
    suite: PathBuf,
    variant: Variant,
    fixture: Option<PreparedFixture>,
    isolation: ValidatedIsolation,
    context: Value,
    profile: Profile,
    workflow: Workflow,
    operation: Uuid,
    workspace: Uuid,
    reports: PathBuf,
    auth: PathBuf,
}
impl Case {
    fn new(native: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations");
        let manifest = SuiteManifest::load(&source.join("manifest.json")).unwrap();
        let mut variant = manifest
            .variants
            .iter()
            .find(|v| v.id == "coordination")
            .unwrap()
            .clone();
        let suite = root.join("declared");
        let input = suite.join("fixture");
        std::fs::create_dir_all(input.join("repository")).unwrap();
        let mut context: Value =
            read_json(&source.join("fixtures/coordination/context.json")).unwrap();
        if native {
            context["workspace"] = json!({"id":{"$kegari":"workspace_id"},"name":"synthetic"});
            context["operation"] = json!({"id":{"$kegari":"operation_id"}});
            context["profile"] = json!({"$kegari":"profile"});
            context["phase"] = json!({"$kegari":"phase"});
            std::fs::write(input.join("context-binding.json"), serde_json::to_vec(&json!({"schema_version":1,"source_write_paths":[],"slots":[
                {"pointer":"/operation/id","source":"operation_id"}, {"pointer":"/phase","source":"phase"},
                {"pointer":"/profile","source":"profile"}, {"pointer":"/workspace/id","source":"workspace_id"}]})).unwrap()).unwrap();
        }
        std::fs::write(
            input.join("context.json"),
            serde_json::to_vec(&context).unwrap(),
        )
        .unwrap();
        std::fs::write(input.join("substrate.json"), br#"{"schema_version":1,"kind":"coordination_empty","workflow":"coordination-workflow.md"}"#).unwrap();
        std::fs::write(input.join("coordination-workflow.md"), "---\nschema_version: 1\n---\nCoordinate {{ children | json }}. Task: {{ issue | json }}.\n").unwrap();
        variant.fixture = "fixture".into();
        variant.fixture_digest = tree_digest(&input).unwrap();
        let reports = root.join("reports");
        let grader = root.join("grader");
        let workers = root.join("workers");
        let auth = root.join("auth");
        for p in [&reports, &grader, &workers, &auth] {
            std::fs::create_dir(p).unwrap();
        }
        let operation = Uuid::new_v4();
        let workspace = Uuid::new_v4();
        let fixture = if native {
            prepare_worker_fixture(
                &suite,
                &variant,
                &root.join("seed"),
                &grader,
                &workers,
                operation,
            )
        } else {
            prepare_fixture(&suite, &variant, &root.join("seed"), &grader)
        }
        .unwrap();
        let workflow = Workflow::load(&fixture.workflow_path().unwrap()).unwrap();
        let (execution_root, repository, scratch) = if native {
            let execution_root = workers.join(operation.to_string());
            let repository = execution_root.join("repository");
            let scratch = execution_root.join("scratch");
            std::fs::create_dir_all(&repository).unwrap();
            std::fs::create_dir(&scratch).unwrap();
            (execution_root, repository, scratch)
        } else {
            (
                fixture.root.clone(),
                fixture.repository.clone(),
                fixture.scratch.clone(),
            )
        };
        let isolation = IsolationPlan {
            schema_version: 1,
            role: ExecutionRole::Coordination,
            execution_root,
            cwd: repository.clone(),
            repository: repository.clone(),
            scratch,
            read_roots: vec![PathGrant {
                path: repository,
                directory: true,
            }],
            source_writes: vec![],
            resource_writes: vec![],
            forbidden_roots: vec![grader, reports.clone(), auth.clone()],
        }
        .validate(crate::BUILD_ID)
        .unwrap();
        let profile = Profile {
            tool: "codex".into(),
            model: "component-not-live".into(),
            reasoning: "high".into(),
            speed: Speed::Standard,
        };
        context["workspace"] = json!({"id":workspace,"name":"synthetic"});
        context["operation"] = json!({"id":operation});
        context["profile"] = json!(profile);
        context["phase"] = json!(variant.phase);
        Self {
            _temp: temp,
            suite,
            variant,
            fixture: Some(fixture),
            isolation,
            context,
            profile,
            workflow,
            operation,
            workspace,
            reports,
            auth,
        }
    }
    fn assembly(&self, public: bool) -> Result<AssembledInstructions> {
        let fixture = self.fixture.as_ref().unwrap();
        let plan = self.isolation.plan();
        let input = AssemblyInput {
            runtime_build: crate::BUILD_ID,
            workflow: &self.workflow,
            role: self.variant.role,
            phase: &self.variant.phase,
            context: &self.context,
            workspace_id: self.workspace,
            operation_id: self.operation,
            work_generation: "component-generation",
            correction_revision: "component-correction",
            profile: &self.profile,
            cwd: &plan.repository,
            codex_home: if public {
                &fixture.codex_home
            } else {
                &self.auth
            },
            codex_version: instructions::SUPPORTED_CODEX_VERSION,
            repository: None,
            required_context: &[],
            project_doc_max_bytes: 32768,
            permissions: PermissionManifest {
                read_roots: vec![plan.repository.clone()],
                source_write_paths: vec![],
                resource_write_paths: vec![],
                scratch_roots: vec![plan.scratch.clone()],
                repository_code_execution: false,
                enforcement_profile: self.isolation.digest().into(),
                network: WorkerNetwork::Denied,
            },
            tools: vec![],
            provider_discovery: ProviderDiscovery::Unavailable {
                reason: "component-only empty substrate".into(),
            },
            known_secrets: &[],
        };
        if public {
            assemble_trial(
                input,
                &self.variant,
                fixture,
                &self.isolation,
                &self.reports,
            )
        } else {
            instructions::assemble(input)
        }
    }
    fn bind(&mut self, assembled: &AssembledInstructions) -> Result<PreparedFixture> {
        self.fixture.take().unwrap().bind_assembly(
            &self.variant,
            assembled,
            FixtureContext {
                value: &self.context,
                independent_pull_request: None,
            },
            &self.isolation,
            &self.workflow.digest,
            &self.reports,
        )
    }
}
#[test]
fn coordination_reset_and_public_assembly_use_captured_external_workflow() {
    let case = Case::new(false);
    let fixture = case.fixture.as_ref().unwrap();
    assert!(fixture.before.is_empty());
    assert!(snapshot(&fixture.repository).unwrap().is_empty());
    assert!(
        !fixture
            .workflow_path()
            .unwrap()
            .starts_with(&fixture.repository)
    );
    let assembled = case.assembly(true).unwrap();
    assert_eq!(assembled.manifest.repository_revision, None);
    assert_eq!(assembled.manifest.repository_root, None);
    assert_eq!(
        assembled.manifest.permissions.read_roots,
        vec![fixture.repository.clone()]
    );
    assert!(!assembled.manifest.permissions.repository_code_execution);
    // Public mutable paths cannot retarget the retained workflow.
    std::fs::write(fixture.workflow_path().unwrap(), "changed workflow").unwrap();
    assert_eq!(case.assembly(true).unwrap_err().code, "fixture_changed");
}
#[test]
fn coordination_owned_binding_checks_actual_empty_repository_context_and_operation() {
    let mut case = Case::new(true);
    let assembled = case.assembly(false).unwrap();
    let bound = case.bind(&assembled).unwrap();
    assert_eq!(bound.repository, case.isolation.plan().repository);
    assert!(bound.before.is_empty());
    assert!(bound.verify_worker_manifest(Some(&assembled)).is_ok());
    for failure in [
        "directory",
        "git",
        "context",
        "operation",
        "profile",
        "workflow",
        "grader",
    ] {
        let mut case = Case::new(true);
        let mut assembled = case.assembly(false).unwrap();
        match failure {
            "directory" => {
                std::fs::create_dir(case.isolation.plan().repository.join("hidden")).unwrap()
            }
            "git" => std::fs::create_dir(case.isolation.plan().repository.join(".git")).unwrap(),
            "context" => case.context["issue"]["description"] = json!("different actual task"),
            "operation" => assembled.manifest.operation_id = Uuid::new_v4(),
            "profile" => assembled.manifest.profile.model = "different".into(),
            "workflow" => {
                std::fs::write(case.workflow.path.as_ref().unwrap(), "different").unwrap()
            }
            "grader" => case.reports = case.isolation.plan().repository.clone(),
            _ => unreachable!(),
        }
        assert!(case.bind(&assembled).is_err(), "{failure}");
    }
}
#[test]
fn coordination_declaration_refuses_extra_source_prior_execution_and_unknown_fields() {
    for failure in [
        "source", "git", "empty", "prior", "role", "phase", "write", "check", "hook", "unknown",
        "leaf",
    ] {
        let case = Case::new(false);
        let input = case.suite.join("fixture");
        let mut variant = case.variant.clone();
        match failure {
            "source" => std::fs::write(input.join("repository/extra"),"source").unwrap(),
            "git" => std::fs::create_dir(input.join("repository/.git")).unwrap(),
            "empty" => std::fs::create_dir(input.join("repository/empty")).unwrap(),
            "prior" => variant.seed = Seed::DeclaredPrior{artifact:"prior.json".into(),digest:bytes_digest(b"{}")} ,
            "role" => variant.role = AgentRole::Implementation,
            "phase" => variant.phase = "implementation".into(),
            "write" => { let mut value:Value=read_json(&input.join("context.json")).unwrap(); value["scope"]["write_paths"]=json!(["source"]); std::fs::write(input.join("context.json"),serde_json::to_vec(&value).unwrap()).unwrap(); },
            "check" => std::fs::write(input.join("coordination-workflow.md"),"---\nchecks:\n  local:\n    run:\n      argv: [\"false\"]\n      result_artifact: result.json\n---\nNever run").unwrap(),
            "hook" => std::fs::write(input.join("coordination-workflow.md"),"---\nhooks:\n  before_run: forbidden\n---\nNever run").unwrap(),
            "unknown" => std::fs::write(input.join("substrate.json"),br#"{"schema_version":1,"kind":"coordination_empty","workflow":"coordination-workflow.md","extra":true}"#).unwrap(),
            "leaf" => std::fs::write(input.join("substrate.json"),br#"{"schema_version":1,"kind":"coordination_empty","workflow":"../workflow"}"#).unwrap(),
            _ => unreachable!(),
        }
        variant.fixture_digest = tree_digest(&input).unwrap();
        assert!(
            prepare_fixture(
                &case.suite,
                &variant,
                &case.suite.parent().unwrap().join("refused"),
                &case.fixture.as_ref().unwrap().grader_root
            )
            .is_err(),
            "{failure}"
        );
    }
}
#[test]
fn coordination_missing_native_context_still_refuses_and_captured_bytes_do_not_reread_source() {
    let case = Case::new(false);
    let fixture = case.fixture.as_ref().unwrap();
    let root = case.suite.parent().unwrap();
    assert_eq!(
        prepare_worker_fixture(
            &case.suite,
            &case.variant,
            &root.join("native-refused"),
            &fixture.grader_root,
            &root.join("workers"),
            Uuid::new_v4()
        )
        .unwrap_err()
        .code,
        "evaluation_fidelity"
    );
    let captured = fixture_capture::CapturedTree::capture(&case.suite.join("fixture")).unwrap();
    std::fs::write(
        case.suite.join("fixture/coordination-workflow.md"),
        "changed after capture",
    )
    .unwrap();
    let reset = prepare_captured_fixture(
        &case.suite,
        &case.variant,
        &captured,
        &root.join("captured"),
        &fixture.grader_root,
    )
    .unwrap();
    assert_eq!(
        Workflow::load(&reset.workflow_path().unwrap())
            .unwrap()
            .digest,
        case.workflow.digest
    );
}
#[test]
fn undeclared_legacy_preparation_digest_has_no_substrate_extension() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations");
    let manifest = SuiteManifest::load(&suite.join("manifest.json")).unwrap();
    let variant = manifest
        .variants
        .iter()
        .find(|v| v.id == "coordination")
        .unwrap();
    let fixture =
        prepare_fixture(&suite, variant, &root.join("legacy"), &suite.join("scorer")).unwrap();
    assert!(fixture.substrate.is_none());
    assert_eq!(fixture.preparation_digest,canonical_digest(1,&json!({"root":fixture.root,"repository":fixture.repository,"scratch":fixture.scratch,"codex_home":fixture.codex_home,"grader_root":fixture.grader_root,"before":fixture.before,"input":fixture.input,"prior":fixture.prior,"fixture_digest":fixture.fixture_digest,"context_binding":fixture.context_binding,"worker_placement":fixture.worker_placement,"worker_manifest":fixture.worker_manifest})).unwrap());
}

#[test]
fn unborn_fixture_keeps_installed_workflow_outside_absent_baseline() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let suite = Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations");
    let manifest = SuiteManifest::load(&suite.join("manifest.json")).unwrap();
    let variant = manifest
        .variants
        .iter()
        .find(|v| v.id == "unborn_repository")
        .unwrap();
    let grader = root.join("grader");
    std::fs::create_dir(&grader).unwrap();
    let fixture = prepare_fixture(&suite, variant, &root.join("trial"), &grader).unwrap();
    assert!(fixture.before.is_empty());
    assert!(
        std::fs::read_dir(&fixture.repository)
            .unwrap()
            .next()
            .is_none()
    );
    assert!(fixture.substrate.as_ref().unwrap().is_unborn());
    assert!(
        !fixture
            .workflow_path()
            .unwrap()
            .starts_with(&fixture.repository)
    );
    assert_eq!(
        Workflow::load(&fixture.workflow_path().unwrap())
            .unwrap()
            .digest,
        Workflow::installed_default().unwrap().digest
    );
    assert!(fixture.child_issues.is_some());
    fixture.verify_preparation().unwrap();
}
