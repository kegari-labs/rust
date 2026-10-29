//! Synthetic provider facts with real publication/reconciliation/context code.
//! This does not invoke Codex or establish actual-source/native qualification.
use super::*;
use crate::{configuration::ConfigurationInputs, corrections::wire_fixture as wire};
use kegari_config::{Config, EvaluationBinding, EvaluationChild};
use kegari_control_api::identity::Actor;
use kegari_storage::PrivateDir;
use kegari_types::control::Lease;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

struct Inputs(Arc<crate::evaluation_provider::EvaluationProviders>);
impl ConfigurationInputs for Inputs {
    fn environment(&self, _: &str) -> Option<String> {
        None
    }
    fn transport(&self, name: &str) -> Option<Arc<dyn kegari_providers::HttpTransport>> {
        name.starts_with("evaluation-")
            .then(|| self.0.fixture_transport(name))
    }
    fn configure_evaluations(&self, config: &Config) -> Result<()> {
        self.0.configure(config)
    }
    fn evaluation_providers(&self) -> Option<Arc<crate::evaluation_provider::EvaluationProviders>> {
        Some(self.0.clone())
    }
    fn codex<'a>(
        &'a self,
        _: &'a kegari_config::Connection,
        _: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<kegari_codex::CodexConnectionMetadata>> + Send + 'a>>
    {
        Box::pin(async { panic!("context fixture must not launch Codex") })
    }
}

#[tokio::test]
async fn fresh_child_namespace_uses_real_publication_gate_and_complete_declared_context() {
    for (variant, child) in [
        ("implementation", 101),
        ("testing", 102),
        ("documentation", 103),
        ("no_change", 101),
        ("unfamiliar_tool", 101),
        ("stale_guidance", 101),
        ("missing_tests", 102),
        ("unavailable_capability", 101),
    ] {
        check_fresh_child(variant, child).await;
    }
}

async fn check_fresh_child(variant_id: &str, child_id: u128) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let suite_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations");
    let manifest: SuiteManifest = read_json(&suite_root.join("manifest.json")).unwrap();
    let variant = manifest
        .variants
        .iter()
        .find(|v| v.id == variant_id)
        .unwrap();
    let execution = root.join("execution");
    std::fs::create_dir(&execution).unwrap();
    let grader = root.join("grader");
    std::fs::create_dir(&grader).unwrap();
    let operation = Uuid::from_u128(700);
    let mut fixture = prepare_worker_fixture(
        &suite_root,
        variant,
        &root.join("seed"),
        &grader,
        &execution,
        operation,
    )
    .unwrap();
    let mut config = wire::config();
    let workspace_id = Uuid::from_u128(40);
    let mut workspace = config.workspaces.remove("example").unwrap();
    workspace.id = workspace_id;
    workspace.linear_workspace_id = Uuid::from_u128(2);
    workspace.workflow = Some(fixture.workflow_path().unwrap().to_str().unwrap().into());
    workspace.linear.authorization_label_id = Uuid::from_u128(7);
    let states = workspace
        .linear
        .states
        .remove(&Uuid::from_u128(10).to_string())
        .unwrap();
    let mut state_value = serde_json::to_value(states).unwrap();
    for (name, n) in [
        ("prepared", 6),
        ("running", 8),
        ("blocked", 9),
        ("child_done", 10),
        ("parent_accepted", 11),
        ("cancelled", 12),
    ] {
        state_value[name] = json!(Uuid::from_u128(n));
    }
    workspace.linear.states.insert(
        Uuid::from_u128(3).to_string(),
        serde_json::from_value(state_value).unwrap(),
    );
    workspace.independent_review.enabled = false;
    let mut repository = workspace.repositories.remove("42").unwrap();
    repository.hostname = format!("github.{workspace_id}.evaluation.invalid");
    repository.local_checks =
        kegari_context::workflow::Workflow::load(&fixture.workflow_path().unwrap())
            .unwrap()
            .front_matter
            .checks
            .local
            .keys()
            .cloned()
            .collect();
    repository.integrated_checks = vec![];
    repository.source = fixture.repository.to_str().unwrap().into();
    workspace.repositories.insert("101".into(), repository);
    workspace.github.repositories = vec!["101".into()];
    workspace.github.pr_authors = vec!["202".into()];
    let binding = EvaluationBinding {
        schema_version: 2,
        suite_manifest: suite_root.join("manifest.json").to_str().unwrap().into(),
        suite_digest: manifest.digest().unwrap(),
        report_root: root.join("reports").to_str().unwrap().into(),
        fixture_root: root.join("seeds").to_str().unwrap().into(),
        case: variant.case_id.clone(),
        variant: variant.id.clone(),
        profile: "operator_evaluated".into(),
        repository_id: 101,
        pull_request: 7,
        author_id: 202,
        base_commit: Some("a".repeat(40)),
        head_commit: Some("a".repeat(40)),
        child: Some(EvaluationChild {
            parent_id: Uuid::from_u128(100),
            child_id: Uuid::from_u128(child_id),
        }),
    };
    workspace.evaluation = Some(binding.clone());
    for provider in ["linear", "github"] {
        let mut connection = config.connections.remove(provider).unwrap();
        let name = EvaluationBinding::connection(workspace_id, provider);
        connection.endpoint = Some(EvaluationBinding::endpoint(workspace_id, provider));
        if provider == "github" {
            connection.account_id = "303".into();
            workspace.github_connection = name.clone();
        } else {
            workspace.linear_connection = name.clone();
        }
        config.connections.insert(name, connection);
    }
    config
        .workspaces
        .insert("Kegari evaluation".into(), workspace);
    config.runtime.execution_root = execution.to_str().unwrap().into();
    config.validate().unwrap();
    let providers = Arc::new(crate::evaluation_provider::EvaluationProviders::default());
    providers.configure(&config).unwrap();
    providers
        .child_wire_fixture(workspace_id, fixture.child_issues(&binding).unwrap())
        .unwrap();
    let config = Arc::new(wire::CacheConfiguration(config));
    let inputs = Arc::new(Inputs(providers));
    let secrets = Arc::new(wire::CacheSecrets);
    let key = Arc::new(kegari_secrets::AttestationKey::new("fixture".into(), vec![7; 32]).unwrap());
    let control = Arc::new(
        crate::control::ControlHost::setup(
            PrivateDir::create(&root.join("control")).unwrap(),
            Uuid::from_u128(1),
            Uuid::from_u128(800),
        )
        .unwrap(),
    );
    let observations = Arc::new(
        crate::observations::ObservationCache::with_inputs(
            config.clone(),
            secrets.clone(),
            key.clone(),
            PrivateDir::create(&root.join("references")).unwrap(),
            inputs.clone(),
            control.clone(),
        )
        .await
        .unwrap(),
    );
    let before = observations.refresh().await.unwrap();
    let parent = before.parent(workspace_id, Uuid::from_u128(100)).unwrap();
    assert_eq!(
        parent
            .fresh_gate(
                Some(Uuid::from_u128(child_id)),
                crate::reconcile::GatePurpose::NewOperation,
                &crate::reconcile::DependencyOutcomes::default(),
                Utc::now(),
                chrono::Duration::seconds(30)
            )
            .unwrap_err()
            .code,
        "authorization_unpublished"
    );
    let authorization = parent
        .authorization_gate(Utc::now(), chrono::Duration::seconds(30))
        .unwrap();
    control
        .lifecycle(
            crate::control::MutationRequest {
                request_id: Uuid::from_u128(9000),
                runtime_id: Uuid::from_u128(1),
                actor: Actor {
                    identity: "fixture-operator".into(),
                    principal: None,
                    roles: ["lifecycle".into()].into(),
                },
                kind: "lifecycle".into(),
                canonical_targets: vec![Uuid::from_u128(1).to_string()],
                expected: BTreeMap::new(),
                payload: json!({}),
            },
            crate::control::Lifecycle::Start,
            0,
        )
        .unwrap();
    let lease = Lease {
        runtime_id: Uuid::from_u128(1),
        owner_generation: 1,
        owner_id: Uuid::from_u128(9001),
    };
    control.attach_reconciled(lease.clone()).unwrap();
    let expected_owner = lease.clone();
    let publisher = Arc::new(crate::preparation::AuthorizationPublisher::with_inputs(
        config,
        control.clone(),
        observations.clone(),
        secrets,
        key.clone(),
        inputs,
        Arc::new(move |actual| {
            if actual == &expected_owner {
                Ok(())
            } else {
                Err(Error::new("ownership_conflict", "fixture owner changed"))
            }
        }),
    ));
    let published = publisher.publish(&lease, authorization).await.unwrap();
    let after = observations.refresh().await.unwrap();
    let parent = after.parent(workspace_id, Uuid::from_u128(100)).unwrap();
    let gate = parent
        .fresh_gate(
            Some(Uuid::from_u128(child_id)),
            crate::reconcile::GatePurpose::NewOperation,
            &crate::reconcile::DependencyOutcomes::default(),
            Utc::now(),
            chrono::Duration::seconds(30),
        )
        .unwrap();
    assert_eq!(
        gate.work().generation.authorization,
        kegari_digest::canonical_digest(1, &published.snapshot().event).unwrap()
    );
    fixture
        .attach_child_context(HostChildContext::from_parent(parent, &gate, &key).unwrap())
        .unwrap();
    let actual = crate::runtime_workers::authorized_context(
        parent,
        &gate,
        "implementation",
        operation,
        &["101".into()],
        json!({"children":[],"parent":null,"acceptance":null,"history":[],"release":null}),
    )
    .unwrap();
    let work = gate.work().digest().unwrap();
    let host = HostContext {
        child: fixture.child_context.as_ref(),
        canary_operand: None,
        workspace: workspace_id,
        operation,
        profile: gate.profile(),
        phase: "implementation",
        work_generation: &work,
        correction_revision: &gate.work().generation.correction,
        repository_revision: binding.head_commit.as_deref(),
        independent_pull_request: None,
    };
    let expected = fixture
        .context_binding
        .as_ref()
        .unwrap()
        .materialize(&fixture.input, &host)
        .unwrap();
    assert_eq!(
        actual, expected,
        "{variant_id}: production context must match the literal declared template"
    );
    let retained_evidence = canonical_digest(1, &expected["evidence"]).unwrap();
    crate::runtime::RuntimeCoordinator::check_retained_child_evaluation_evidence(
        &actual,
        &retained_evidence,
    )
    .unwrap();
    let mut injected = actual.clone();
    injected["evidence"]["forbidden_canary_path"] = json!("/unexpected-canary");
    assert!(
        crate::runtime::RuntimeCoordinator::check_retained_child_evaluation_evidence(
            &injected,
            &retained_evidence
        )
        .is_err()
    );
    let mut changed_native = actual.clone();
    changed_native["evidence"]["native_context"] = json!({});
    assert!(
        crate::runtime::RuntimeCoordinator::check_retained_child_evaluation_evidence(
            &changed_native,
            &retained_evidence
        )
        .is_err()
    );
    verify_fixture_context(&actual, &fixture, &host).unwrap();
    let mut tampered = actual.clone();
    tampered["issue"]["description"] = json!("an easier replacement task");
    assert_eq!(
        verify_fixture_context(&tampered, &fixture, &host)
            .unwrap_err()
            .code,
        "evaluation_fidelity"
    );
    assert!(
        control.snapshot().unwrap().active_operations.is_empty(),
        "actual publication admission retires before model admission"
    );
}
