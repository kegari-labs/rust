//! Real assembly fixtures; no manifest JSON is promoted to resume authority.
use kegari_config::{Profile, Speed};
use kegari_context::{instructions::*, workflow::Workflow};
use serde_json::{Value, json};
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

struct Fixture {
    _root: TempDir,
    repository: PathBuf,
    auth: PathBuf,
    scratch: PathBuf,
    workflow: Workflow,
    profile: Profile,
}
impl Fixture {
    fn new(template: &str) -> Self {
        let root = TempDir::new().unwrap();
        let repository = root.path().join("repository");
        let auth = root.path().join("auth");
        let scratch = root.path().join("scratch");
        for path in [&repository, &auth, &scratch] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(repository.join("AGENTS.md"), "Preserve exact boundaries.").unwrap();
        let workflow = Workflow::parse(template, root.path().join("WORKFLOW.md")).unwrap();
        Self {
            _root: root,
            repository,
            auth,
            scratch,
            workflow,
            profile: Profile {
                tool: "codex".into(),
                model: "fixture".into(),
                reasoning: "high".into(),
                speed: Speed::Standard,
            },
        }
    }
    fn context(&self, operation: Uuid, attempt: u64) -> Value {
        json!({"issue":{"description":"Resolve the same task."},"parent":null,"children":[],
            "workspace":{"id":Uuid::from_u128(1)},"scope":{"write_paths":["src/"]},
            "dependencies":[],"corrections":[],"evidence":{},"attempt":attempt,
            "phase":"implementation","profile":self.profile,"operation":{"id":operation}})
    }
    fn assemble(&self, operation: Uuid, context: &Value) -> AssembledInstructions {
        self.assemble_with_build(operation, context, "sha256:context-fixture-build")
    }
    fn assemble_with_build(
        &self,
        operation: Uuid,
        context: &Value,
        runtime_build: &str,
    ) -> AssembledInstructions {
        assemble(AssemblyInput {
            runtime_build,
            workflow: &self.workflow,
            role: AgentRole::Implementation,
            phase: "implementation",
            context,
            workspace_id: Uuid::from_u128(1),
            operation_id: operation,
            work_generation: "exact-work",
            correction_revision: "exact-correction",
            profile: &self.profile,
            cwd: &self.repository,
            codex_home: &self.auth,
            codex_version: SUPPORTED_CODEX_VERSION,
            repository: Some(RepositoryContext {
                root: self.repository.clone(),
                revision: "a".repeat(40),
                scope_paths: vec!["src/".into()],
            }),
            required_context: &[],
            project_doc_max_bytes: 32768,
            permissions: PermissionManifest {
                read_roots: vec![self.repository.clone()],
                source_write_paths: vec![self.repository.join("src")],
                resource_write_paths: vec![],
                scratch_roots: vec![self.scratch.clone()],
                repository_code_execution: true,
                enforcement_profile: "fixture-boundary".into(),
                network: WorkerNetwork::BrokerOnly,
            },
            tools: vec![ExposedTool {
                name: "outcome_submit".into(),
                capability: ToolCapability::OutcomeSubmit,
            }],
            provider_discovery: ProviderDiscovery::Unavailable {
                reason: "offline fixture".into(),
            },
            known_secrets: &[],
        })
        .unwrap()
    }
}

#[test]
fn genuine_assembly_normalizes_only_documented_admission_metadata() {
    let f = Fixture::new("Use the same verified task.\n");
    let old = Uuid::from_u128(2);
    let new = Uuid::from_u128(3);
    let original = f.assemble(old, &f.context(old, 1));
    let current = f.assemble(new, &f.context(new, 2));
    let proof = original.verify_resume(&current).unwrap();
    assert_eq!(proof.previous_manifest(), original.manifest_digest);
    assert_eq!(proof.current_manifest(), current.manifest_digest);
    assert_ne!(original.prompt, current.prompt);
    assert_ne!(original.manifest_digest, current.manifest_digest);
    assert!(proof.digest().starts_with("sha256:v1:"));
    for (field, value) in [
        ("issue", json!({"description":"A different task."})),
        ("evidence", json!({"nested":{"operation":{"id":new}}})),
        ("corrections", json!(["new requirement"])),
        ("dependencies", json!(["different result"])),
    ] {
        let mut context = f.context(new, 2);
        context[field] = value;
        assert_eq!(
            original
                .verify_resume(&f.assemble(new, &context))
                .unwrap_err()
                .code,
            "resume_incompatible"
        );
    }
}

#[test]
fn rendered_workflow_and_actual_guidance_remain_exact() {
    for template in ["Task {{ operation.id }}", "Attempt {{ attempt }}"] {
        let f = Fixture::new(template);
        let old = Uuid::from_u128(2);
        let new = Uuid::from_u128(3);
        let original = f.assemble(old, &f.context(old, 1));
        let current = f.assemble(new, &f.context(new, 2));
        assert_eq!(
            original.verify_resume(&current).unwrap_err().code,
            "resume_incompatible"
        );
    }
    let f = Fixture::new("Same task.");
    let operation = Uuid::from_u128(2);
    let original = f.assemble(operation, &f.context(operation, 1));
    std::fs::write(
        f.repository.join("AGENTS.md"),
        "Different project instruction.",
    )
    .unwrap();
    assert!(
        !original
            .manifest
            .resume_compatible(&f.assemble(operation, &f.context(operation, 1)).manifest)
    );
}

#[test]
fn rewritten_visible_manifests_and_legacy_records_do_not_mint_resume_proof() {
    let f = Fixture::new("Same task.");
    let operation = Uuid::from_u128(2);
    let original = f.assemble(operation, &f.context(operation, 1));
    let mut forged = original.clone();
    forged.prompt.push_str(" Changed instructions.");
    forged.manifest.prompt_digest = kegari_digest::bytes_digest(forged.prompt.as_bytes());
    forged.manifest_digest =
        kegari_digest::canonical_digest(ASSEMBLY_SCHEMA, &forged.manifest).unwrap();
    assert_eq!(
        original.verify_resume(&forged).unwrap_err().code,
        "resume_incompatible"
    );
    let mut legacy = serde_json::to_value(&original.manifest).unwrap();
    legacy.as_object_mut().unwrap().remove("resume_semantics");
    let decoded: InstructionManifest = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), legacy);
    assert!(!decoded.resume_compatible(&decoded));
    for change in [
        "phase",
        "correction_revision",
        "codex_protocol",
        "context_digest",
    ] {
        let mut manifest = serde_json::to_value(&original.manifest).unwrap();
        manifest[change] = json!("changed");
        let changed: InstructionManifest = serde_json::from_value(manifest).unwrap();
        // A display-only compatibility comparison cannot authenticate changed
        // component hashes. The actual private assembly seal always must pass.
        let mut changed_assembly = original.clone();
        changed_assembly.manifest = changed;
        changed_assembly.manifest_digest =
            kegari_digest::canonical_digest(ASSEMBLY_SCHEMA, &changed_assembly.manifest).unwrap();
        assert!(original.verify_resume(&changed_assembly).is_err());
    }
}

#[test]
fn supplied_runtime_identity_is_exact_and_part_of_sealed_resume_compatibility() {
    let fixture = Fixture::new("Same task.");
    let operation = Uuid::from_u128(2);
    let context = fixture.context(operation, 1);
    let supplied_build = "caller-provided-build-identity";
    let original = fixture.assemble_with_build(operation, &context, supplied_build);
    let repeated = fixture.assemble_with_build(operation, &context, supplied_build);
    assert_eq!(original.manifest.runtime_build, supplied_build);
    assert_eq!(original.manifest.runtime_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(original.prompt, repeated.prompt);
    assert_eq!(
        serde_json::to_vec(&original.manifest).unwrap(),
        serde_json::to_vec(&repeated.manifest).unwrap()
    );
    assert_eq!(original.manifest_digest, repeated.manifest_digest);
    let proof = original.verify_resume(&repeated).unwrap();
    assert_eq!(proof.previous_manifest(), original.manifest_digest);
    assert_eq!(proof.current_manifest(), repeated.manifest_digest);

    let other = fixture.assemble_with_build(operation, &context, "different-build-identity");
    assert_eq!(other.manifest.runtime_build, "different-build-identity");
    assert_eq!(original.prompt, other.prompt);
    assert_ne!(original.manifest_digest, other.manifest_digest);
    assert!(!original.manifest.resume_compatible(&other.manifest));
    assert_eq!(
        original.verify_resume(&other).unwrap_err().code,
        "resume_incompatible"
    );
}
