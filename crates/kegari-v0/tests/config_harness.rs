//! Deterministic production-path coverage for V0-T01, T33 and T34.
//! These do not substitute for provider discovery/sandbox or actual-agent trials.
use kegari_config::*;
use kegari_v0::config::{ConfigReload, config_revision, inspect_reload, load_workflows};
use kegari_v0::digest::{bytes_digest, canonical_digest};
use kegari_v0::instructions::*;
use kegari_v0::workflow::*;
use serde_json::{Value, json};
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

fn id(number: u128) -> Uuid {
    Uuid::from_u128(number)
}
fn profile() -> Profile {
    Profile {
        tool: "codex".into(),
        model: "fixture-model".into(),
        reasoning: "high".into(),
        speed: Speed::Standard,
    }
}
fn configured() -> Config {
    let proof = json!({"workflow_ref":"fixture/repo/.github/workflows/caller.yml@refs/heads/trusted","workflow_sha":"0123456789012345678901234567890123456789","job_workflow_ref":"fixture/trusted/.github/workflows/proof.yml@0123456789012345678901234567890123456789","job_workflow_sha":"0123456789012345678901234567890123456789","event_name":"workflow_dispatch"});
    let exact = json!({"workflow_id":1,"workflow_revision":"0123456789012345678901234567890123456789","source_input":"source_sha","operation_input":"operation_id","observed_source_output":"tested_sha","proof":proof});
    serde_json::from_value(json!({
        "schema_version":1,
        "runtime":{"id":id(1),"host_id":"verified-test-host","max_agents":4},
        "connections":{
            "linear":{"kind":"linear","account_id":"linear-user","endpoint":"https://api.linear.app/graphql","credential_ref":"keychain:linear"},
            "github":{"kind":"github","account_id":"github-user","endpoint":"https://api.github.com/","credential_ref":"keychain:github"},
            "codex":{"kind":"codex","account_id":"codex-account","auth_home":"/private/tmp/kegari-test-auth","max_agents":2}
        },
        "workspaces":{"example":{
            "id":id(2),"linear_workspace_id":id(3),"linear_connection":"linear","github_connection":"github","codex_connection":"codex","workflow":"/private/tmp/WORKFLOW.md",
            "linear":{"teams":"all","projects":"all","users":"all","authorization_label_id":id(4),"states":{"team":{"prepared":id(10),"running":id(11),"blocked":id(12),"child_done":id(13),"parent_accepted":id(14),"cancelled":id(15)}}},
            "github":{"organizations":[],"repositories":["101"]},
            "actors":{"authorizers":[id(5)],"correctors":[id(5)],"acceptors":[id(5)]},
            "profiles":{"coordination":profile()},
            "routing_labels":{"tool":{id(20).to_string():"codex"},"model":{id(21).to_string():"fixture-model"},"reasoning":{id(22).to_string():"high"},"speed":{id(23).to_string():"standard"}},
            "max_agents":2,"release":{"mode":"manual","spacing_seconds":0},
            "repositories":{"101":{"source":"/private/tmp/source","local_checks":["local"],"integrated_checks":["e2e"],"ci":[{"workflow_id":1,"required_checks":["suite"],"proof":proof}],"production_ci":[{"workflow_id":2,"required_checks":["suite"],"proof":proof}],"no_change_validation":exact,"deployment":"none"}}
        }},
        "control":{"local_users":{"501":{"id":"operator","linear_user_id":id(5),"roles":["inspect","configure","lifecycle","correct","accept","release","recover"]}}}
    })).unwrap()
}

fn evaluation_config_for_validation() -> Config {
    let mut config = configured();
    let workspace = config.workspaces.get_mut("example").unwrap();
    let workspace_id = workspace.id;
    workspace.independent_review.enabled = true;
    workspace.profiles.independent_review = Some(profile());
    workspace.github.pr_authors = vec!["77".into()];
    workspace.github_connection = EvaluationBinding::connection(workspace_id, "github");
    workspace.linear_connection = EvaluationBinding::connection(workspace_id, "linear");
    workspace.repositories.get_mut("101").unwrap().hostname =
        format!("github.{workspace_id}.evaluation.invalid");
    workspace.evaluation = Some(EvaluationBinding {
        schema_version: 1,
        suite_manifest: "/private/tmp/evaluation-suite.json".into(),
        suite_digest: bytes_digest(b"declared test data only"),
        report_root: "/private/tmp/evaluation-reports".into(),
        fixture_root: "/private/tmp/evaluation-fixtures".into(),
        case: "case".into(),
        variant: "variant".into(),
        profile: "profile".into(),
        repository_id: 101,
        pull_request: 1,
        author_id: 77,
        base_commit: "1".repeat(40),
        head_commit: "2".repeat(40),
    });
    for provider in ["github", "linear"] {
        let mut connection = config.connections.remove(provider).unwrap();
        connection.endpoint = Some(EvaluationBinding::endpoint(workspace_id, provider));
        config.connections.insert(
            EvaluationBinding::connection(workspace_id, provider),
            connection,
        );
    }
    config.validate().unwrap();
    config
}

#[test]
fn evaluation_digest_validation_matches_backend_reference_grammar_and_errors() {
    let config = evaluation_config_for_validation();
    let workspace = &config.workspaces["example"];
    for (digest, accepted) in [
        (format!("sha256:{}", "a".repeat(64)), true),
        (format!("sha256:v1:{}", "0123456789abcdef".repeat(4)), true),
        (format!("sha256:{}", "0".repeat(64)), true),
        (String::new(), false),
        ("a".repeat(64), false),
        (format!("SHA256:{}", "a".repeat(64)), false),
        (format!("sha256:v0:{}", "a".repeat(64)), false),
        (format!("sha256:v2:{}", "a".repeat(64)), false),
        (format!("sha256:v5:{}", "a".repeat(64)), false),
        (format!("sha256:v01:{}", "a".repeat(64)), false),
        (format!("sha256:{}", "a".repeat(63)), false),
        (format!("sha256:v1:{}", "a".repeat(65)), false),
        (format!("sha256:{}", "A".repeat(64)), false),
        (format!("sha256:v1:{}", "g".repeat(64)), false),
        (format!(" sha256:{}", "a".repeat(64)), false),
        (format!("sha256:{}\n", "a".repeat(64)), false),
        (format!("sha256:{}", "é".repeat(32)), false),
    ] {
        let expected = kegari_v0::evidence::validate_digest(&digest);
        assert_eq!(
            expected.is_ok(),
            accepted,
            "unexpected grammar for {digest:?}"
        );
        let mut declaration = workspace.evaluation.clone().unwrap();
        declaration.suite_digest = digest.clone();
        assert_eq!(
            declaration.validate(&config, workspace),
            expected,
            "configuration digest validation differs for {digest:?}"
        );
    }
}

#[test]
fn evaluation_oid_validation_matches_backend_revision_grammar_and_errors() {
    let config = evaluation_config_for_validation();
    let workspace = &config.workspaces["example"];
    for (oid, accepted) in [
        ("a".repeat(40), true),
        ("0123456789abcdef".repeat(4), true),
        ("0".repeat(40), true),
        ("0".repeat(64), true),
        (String::new(), false),
        ("main".into(), false),
        ("a".repeat(39), false),
        ("a".repeat(41), false),
        ("a".repeat(63), false),
        ("a".repeat(65), false),
        ("A".repeat(40), false),
        ("g".repeat(64), false),
        (format!(" {}", "a".repeat(40)), false),
        (format!("{}\n", "a".repeat(40)), false),
        ("é".repeat(20), false),
    ] {
        let expected = kegari_v0::git::validate_oid(&oid);
        assert_eq!(expected.is_ok(), accepted, "unexpected grammar for {oid:?}");
        for field in ["base_commit", "head_commit"] {
            let mut declaration = workspace.evaluation.clone().unwrap();
            if field == "base_commit" {
                declaration.base_commit = oid.clone();
            } else {
                declaration.head_commit = oid.clone();
            }
            assert_eq!(
                declaration.validate(&config, workspace),
                expected,
                "configuration {field} validation differs for {oid:?}"
            );
        }
    }
}

#[test]
fn typed_config_defaults_and_reproducible_revision() {
    let config = configured();
    config.validate().unwrap();
    let text = toml::to_string(&config).unwrap();
    let parsed = Config::parse(&text).unwrap();
    assert_eq!(parsed.runtime.poll_ms, 30_000);
    assert_eq!(parsed.runtime.max_attempts_per_phase, 3);
    assert_eq!(parsed.runtime.max_turns, 20);
    assert_eq!(parsed.runtime.min_free_bytes, 1_073_741_824);
    assert!(!parsed.workspaces["example"].independent_review.enabled);
    assert_eq!(
        config_revision(&config).unwrap(),
        config_revision(&parsed).unwrap()
    );
}

#[test]
fn supplied_delivery_policy_preserves_legacy_bytes_and_config_revision() {
    // Exact former Repository serialization shape, independent of the new
    // Option serializer. No new version/null/discriminator may enter old data.
    #[derive(serde::Serialize)]
    struct LegacyRepository<'a> {
        source: &'a String,
        hostname: &'a String,
        main: &'a String,
        staging: &'a String,
        local_checks: &'a Vec<String>,
        integrated_checks: &'a Vec<String>,
        ci: &'a Vec<CiSuite>,
        production_ci: &'a Vec<CiSuite>,
        review_context: &'a String,
        no_change_validation: &'a ExactRevisionWorkflow,
        deployment: &'a Deployment,
    }
    let config = configured();
    config.validate().unwrap();
    let repository = &config.workspaces["example"].repositories["101"];
    let legacy = LegacyRepository {
        source: &repository.source,
        hostname: &repository.hostname,
        main: &repository.main,
        staging: &repository.staging,
        local_checks: &repository.local_checks,
        integrated_checks: &repository.integrated_checks,
        ci: repository.ci.as_ref().unwrap(),
        production_ci: repository.production_ci.as_ref().unwrap(),
        review_context: &repository.review_context,
        no_change_validation: repository.no_change_validation.as_ref().unwrap(),
        deployment: repository.deployment.as_ref().unwrap(),
    };
    let old_bytes = serde_json::to_vec(&legacy).unwrap();
    assert_eq!(serde_json::to_vec(repository).unwrap(), old_bytes);
    assert_eq!(
        toml::to_string(repository).unwrap(),
        toml::to_string(&legacy).unwrap()
    );
    let decoded: Repository = serde_json::from_slice(&old_bytes).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), old_bytes);
    assert_eq!(
        canonical_digest(1, &decoded).unwrap(),
        canonical_digest(1, &legacy).unwrap()
    );
    let mut old_config = serde_json::to_value(&config).unwrap();
    old_config["workspaces"]["example"]["repositories"]["101"] =
        serde_json::to_value(&legacy).unwrap();
    assert_eq!(
        config_revision(&config).unwrap(),
        canonical_digest(1, &old_config).unwrap()
    );
    let old_text = toml::to_string(&config).unwrap();
    assert_eq!(
        config_revision(&Config::parse(&old_text).unwrap()).unwrap(),
        config_revision(&config).unwrap()
    );
}

#[test]
fn omitted_delivery_policies_remain_unresolved_and_roundtrip_without_defaults() {
    let full = serde_json::to_value(configured()).unwrap();
    for omitted in ["ci", "production_ci", "no_change_validation", "deployment"] {
        let mut value = full.clone();
        value["workspaces"]["example"]["repositories"]["101"]
            .as_object_mut()
            .unwrap()
            .remove(omitted);
        let config: Config = serde_json::from_value(value).unwrap();
        config.validate().unwrap();
        let encoded = serde_json::to_value(&config).unwrap();
        assert!(
            encoded["workspaces"]["example"]["repositories"]["101"]
                .get(omitted)
                .is_none()
        );
        assert_eq!(
            config_revision(&Config::parse(&toml::to_string(&config).unwrap()).unwrap()).unwrap(),
            config_revision(&config).unwrap()
        );
    }
    let mut value = full;
    let repository = value["workspaces"]["example"]["repositories"]["101"]
        .as_object_mut()
        .unwrap();
    for field in ["ci", "production_ci", "no_change_validation", "deployment"] {
        repository.remove(field);
    }
    let config: Config = serde_json::from_value(value).unwrap();
    config.validate().unwrap();
    let repository = &config.workspaces["example"].repositories["101"];
    repository.validate_supplied_policy().unwrap();
    for (field, result) in [
        ("ci", repository.staging_ci().map(|_| ())),
        (
            "production_ci",
            repository.production_ci_policy().map(|_| ()),
        ),
        (
            "no_change_validation",
            repository.no_change_workflow().map(|_| ()),
        ),
        ("deployment", repository.deployment_policy().map(|_| ())),
    ] {
        let error = result.unwrap_err();
        assert_eq!(error.code, "delivery_policy_unresolved");
        assert!(error.message.contains(field));
    }
    assert_eq!(repository.local_checks, ["local"]);
    assert_eq!(repository.integrated_checks, ["e2e"]);
}

// The removed catalog tests described obsolete target-tool policy. The public
// configuration now rejects that entire surface rather than retaining an opt-in
// adapter. Generic account/repository/resource authority remains independently tested.
#[test]
fn configuration_without_tool_catalog_preserves_bytes_and_revision() {
    let config = configured();
    let value = serde_json::to_value(&config).unwrap();
    assert!(
        value["workspaces"]["example"]
            .get("managed_execution")
            .is_none()
    );
    let decoded: Config = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), value);
    assert_eq!(
        config_revision(&decoded).unwrap(),
        config_revision(&config).unwrap()
    );
    let text = toml::to_string(&config).unwrap();
    assert!(!text.contains("managed_execution"));
    assert_eq!(
        config_revision(&Config::parse(&text).unwrap()).unwrap(),
        config_revision(&config).unwrap()
    );
}

#[test]
fn generic_test_resources_keep_scope_resolution_and_disposable_write_requirement() {
    let home = TempDir::new().unwrap();
    let mut config = configured();
    config
        .workspaces
        .get_mut("example")
        .unwrap()
        .test_resources
        .insert(
            "fixtures".into(),
            TestResource {
                path: "~/fixtures".into(),
                writable: false,
                disposable: false,
            },
        );
    config.validate().unwrap();
    config.resolve_paths(home.path(), |_| None).unwrap();
    let resource = &config.workspaces["example"].test_resources["fixtures"];
    assert_eq!(
        resource.path,
        home.path().join("fixtures").to_str().unwrap()
    );
    assert!(!resource.writable && !resource.disposable);
    let mut changed = config.clone();
    changed
        .workspaces
        .get_mut("example")
        .unwrap()
        .test_resources
        .get_mut("fixtures")
        .unwrap()
        .writable = true;
    assert!(changed.validate().is_err());
    changed
        .workspaces
        .get_mut("example")
        .unwrap()
        .test_resources
        .get_mut("fixtures")
        .unwrap()
        .disposable = true;
    changed.validate().unwrap();
    assert_ne!(
        config_revision(&changed).unwrap(),
        config_revision(&config).unwrap()
    );
    let mut unknown = serde_json::to_value(&config).unwrap();
    unknown["workspaces"]["example"]["test_resources"]["fixtures"]["executables"] = json!([]);
    assert!(serde_json::from_value::<Config>(unknown).is_err());
}

#[test]
fn invalid_reload_fences_mutations_and_does_not_change_valid_snapshot() {
    let config = configured();
    let before = config_revision(&config).unwrap();
    let update = toml::to_string(&config)
        .unwrap()
        .replace("max_agents = 4", "max_agents = 0");
    assert!(matches!(
        inspect_reload(&before, &update),
        ConfigReload::Invalid {
            fence_all_mutations: true,
            ..
        }
    ));
    assert_eq!(before, config_revision(&config).unwrap());
    assert!(matches!(
        inspect_reload(&before, &toml::to_string(&config).unwrap()),
        ConfigReload::Unchanged
    ));
}
#[test]
fn workflow_frontmatter_is_closed_and_rejects_duplicate_nested_keys() {
    for source in [
        "---\n[]\n---\ntext",
        "---\nnull\n---\ntext",
        "---\nmodel: cheap\n---\ntext",
        "---\nschema_version: 1\nschema_version: 1\n---\ntext",
        "---\nhooks:\n  timeout_ms: 1\n  timeout_ms: 2\n---\ntext",
        "---\nchecks:\n  local:\n    suite: {argv: [true], result_artifact: x}\n---\ntext",
        "---\nchecks:\n  local:\n    suite: {argv: [test], result_artifact: x}\n    suite: {argv: [other], result_artifact: y}\n---\ntext",
        "---\nhooks: {timeout_ms: 0}\n---\ntext",
        "   ",
        "---\nschema_version: 1\nmissing closing delimiter",
    ] {
        assert!(
            Workflow::parse(source, "/configured/WORKFLOW.md").is_err(),
            "{source}"
        );
    }
}
#[test]
fn strict_template_outputs_and_control_flow_preserve_structured_values() {
    let source = r#"{% assign heading = issue.title | strip | upcase %}{{ heading }}
{% if issue.ready == true %}{% for child in children %}{{ forloop.index }}:{{ child.id }};{% endfor %}{% elsif issue.ready == false %}blocked{% else %}unknown{% endif %}
{{ children | json }} {{ issue.optional | default: "missing" }} {{ names | join: "," }} {{ names | size }} {{ html | escape }}"#;
    let workflow = Workflow::parse(source, "/configured/WORKFLOW.md").unwrap();
    let output = workflow.render(&json!({"issue":{"title":" title ","ready":true,"optional":null},"children":[{"id":"a"},{"id":"b"}],"names":["x","y"],"html":"<secret>&"})).unwrap();
    assert!(output.starts_with("TITLE\n1:a;2:b;"));
    assert!(output.contains(r#"[{"id":"a"},{"id":"b"}] missing x,y 2 &lt;secret&gt;&amp;"#));
    assert_eq!(
        Workflow::parse(
            "{% for x in xs %}bad{% else %}empty{% endfor %}",
            "/workflow"
        )
        .unwrap()
        .render(&json!({"xs":[]}))
        .unwrap(),
        "empty"
    );
}
#[test]
fn strict_template_errors_do_not_silently_weaken_a_request() {
    for source in [
        "{{ issue.missing }}",
        "{{ missing | default: 'invented' }}",
        "{{ issue | upcase }}",
        "{{ issue | join: ',' }}",
        "{{ issue }}",
        "{% if missing %}x{% endif %}",
        "{{ issue[99] }}",
    ] {
        let result = Workflow::parse(source, "/workflow")
            .and_then(|workflow| workflow.render(&json!({"issue":{"title":"x"}})));
        assert!(result.is_err(), "{source}");
    }
    for source in [
        "{{ issue.title | unsupported }}",
        "{% include 'other' %}",
        "{% if true %}unfinished",
        "{{ title",
        "{% assign = true %}",
    ] {
        assert!(Workflow::parse(source, "/workflow").is_err(), "{source}");
    }
}
#[test]
fn workflow_render_has_finite_bounds_and_retains_snapshot() {
    let workflow =
        Workflow::parse("{% for item in items %}{{ item }}{% endfor %}", "/workflow").unwrap();
    assert!(workflow.render(&json!({"items":vec!["x";10001]})).is_err());
    let fixture = TempDir::new().unwrap();
    let path = fixture.path().join("WORKFLOW.md");
    std::fs::write(&path, "old {{ issue.title }}").unwrap();
    let old = Workflow::load(&path).unwrap();
    std::fs::write(&path, "new {{ issue.title }}").unwrap();
    let new = Workflow::load(&path).unwrap();
    assert_ne!(old.digest, new.digest);
    assert_eq!(
        old.render(&json!({"issue":{"title":"result"}})).unwrap(),
        "old result"
    );
}
#[test]
fn candidate_cwd_wins_over_conflicting_workflow_relative_scripts_and_artifacts() {
    let fixture = TempDir::new().unwrap();
    let candidate = fixture.path().join("candidate");
    let workflow_dir = fixture.path().join("workflow");
    std::fs::create_dir_all(&candidate).unwrap();
    std::fs::create_dir_all(&workflow_dir).unwrap();
    std::fs::write(
        candidate.join("check.sh"),
        "printf candidate > result.txt\n",
    )
    .unwrap();
    std::fs::write(workflow_dir.join("check.sh"), "printf wrong > result.txt\n").unwrap();
    std::fs::write(workflow_dir.join("result.txt"), "stale").unwrap();
    let path = workflow_dir.join("WORKFLOW.md");
    std::fs::write(&path,"---\nchecks:\n  local:\n    suite:\n      argv: [sh, check.sh]\n      result_artifact: result.txt\n---\nRun required checks\n").unwrap();
    let workflow = Workflow::load(&path).unwrap();
    let resolved = workflow
        .resolve_check(
            &workflow.front_matter.checks.local["suite"],
            &candidate,
            std::slice::from_ref(&candidate),
        )
        .unwrap();
    let status = std::process::Command::new(&resolved.argv[0])
        .args(&resolved.argv[1..])
        .current_dir(&resolved.cwd)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        std::fs::read_to_string(&resolved.result_artifact).unwrap(),
        "candidate"
    );
    assert_eq!(
        std::fs::read_to_string(workflow_dir.join("result.txt")).unwrap(),
        "stale"
    );
    std::fs::remove_file(candidate.join("check.sh")).unwrap();
    let result = std::process::Command::new(&resolved.argv[0])
        .args(&resolved.argv[1..])
        .current_dir(&resolved.cwd)
        .output()
        .unwrap();
    assert!(!result.status.success());
    let outside = CheckCommand {
        argv: vec!["true".into()],
        services: vec![],
        result_artifact: workflow_dir
            .join("result.txt")
            .to_string_lossy()
            .into_owned(),
    };
    assert!(
        workflow
            .resolve_check(&outside, &candidate, std::slice::from_ref(&candidate))
            .is_err()
    );
    let traversal = CheckCommand {
        argv: vec!["true".into()],
        result_artifact: "../workflow/result.txt".into(),
        services: vec![],
    };
    assert!(
        workflow
            .resolve_check(&traversal, &candidate, std::slice::from_ref(&candidate))
            .is_err()
    );
    std::fs::write(workflow_dir.join("context.md"), "linked resource").unwrap();
    assert_eq!(
        workflow
            .resolve_resource("context.md", std::slice::from_ref(&workflow_dir))
            .unwrap(),
        workflow_dir.canonicalize().unwrap().join("context.md")
    );
    assert!(
        workflow
            .resolve_resource("context.md", std::slice::from_ref(&candidate))
            .is_err()
    );
}
#[cfg(unix)]
#[test]
fn artifact_symlink_cannot_escape_declared_roots() {
    let fixture = TempDir::new().unwrap();
    let candidate = fixture.path().join("candidate");
    let outside = fixture.path().join("outside");
    std::fs::create_dir(&candidate).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, candidate.join("output")).unwrap();
    let workflow = Workflow::parse("task", "/workflow").unwrap();
    let command = CheckCommand {
        argv: vec!["true".into()],
        result_artifact: "output/result.json".into(),
        services: vec![],
    };
    assert!(
        workflow
            .resolve_check(&command, &candidate, std::slice::from_ref(&candidate))
            .is_err()
    );
}
struct AssemblyFixture {
    _dir: TempDir,
    repository: PathBuf,
    scratch: PathBuf,
    auth: PathBuf,
    workflow: Workflow,
    context: Value,
    profile: Profile,
}
impl AssemblyFixture {
    fn new() -> Self {
        let fixture = TempDir::new().unwrap();
        let repository = fixture.path().join("repo");
        let scratch = fixture.path().join("scratch");
        let auth = fixture.path().join("auth");
        std::fs::create_dir_all(repository.join("src")).unwrap();
        std::fs::create_dir(&scratch).unwrap();
        std::fs::create_dir(&auth).unwrap();
        std::fs::write(
            repository.join("AGENTS.md"),
            "Root project convention: preserve stable interfaces.",
        )
        .unwrap();
        std::fs::write(
            repository.join("src/AGENTS.md"),
            "Nested project convention: test numerical boundaries.",
        )
        .unwrap();
        std::fs::write(
            auth.join("AGENTS.md"),
            "Global scoped convention: report evidence accurately.",
        )
        .unwrap();
        let profile = profile();
        let context = json!({"issue":{"id":id(4),"description":"Implement scoped boundary behavior"},"parent":{"id":id(5),"description":"End-to-end outcome"},"children":[],"workspace":{"id":id(2),"name":"Fixture"},"scope":{"write_repository":"101","write_paths":["src/"]},"dependencies":[],"corrections":[],"evidence":[],"attempt":1,"phase":"implementation","profile":profile,"operation":{"id":id(6)}});
        let workflow = Workflow::parse(STARTER, fixture.path().join("WORKFLOW.md")).unwrap();
        Self {
            _dir: fixture,
            repository,
            scratch,
            auth,
            workflow,
            context,
            profile,
        }
    }
    fn input(&self) -> AssemblyInput<'_> {
        AssemblyInput {
            workflow: &self.workflow,
            role: AgentRole::Implementation,
            phase: "implementation",
            context: &self.context,
            workspace_id: id(2),
            operation_id: id(6),
            work_generation: "work-1",
            correction_revision: "correction-1",
            profile: &self.profile,
            cwd: &self.repository,
            codex_home: &self.auth,
            codex_version: SUPPORTED_CODEX_VERSION,
            repository: Some(RepositoryContext {
                root: self.repository.clone(),
                revision: "0123456789012345678901234567890123456789".into(),
                scope_paths: vec!["src/".into()],
            }),
            required_context: &[],
            project_doc_max_bytes: 32768,
            permissions: PermissionManifest {
                read_roots: vec![self.repository.clone()],
                resource_write_paths: vec![],
                source_write_paths: vec![self.repository.join("src")],
                scratch_roots: vec![self.scratch.clone()],
                repository_code_execution: true,
                enforcement_profile: "fixture-policy-v1".into(),
                network: WorkerNetwork::BrokerOnly,
            },
            tools: vec![ExposedTool {
                name: "submit_outcome".into(),
                capability: ToolCapability::OutcomeSubmit,
            }],
            provider_discovery: ProviderDiscovery::Unavailable {
                reason: "no live provider trial in deterministic test".into(),
            },
            known_secrets: &[],
        }
    }
}
#[test]
fn delivered_starter_and_embedded_assets_assemble_without_development_checkout() {
    verify_assets().unwrap();
    let fixture = AssemblyFixture::new();
    let assembled = assemble(fixture.input()).unwrap();
    assert!(
        assembled
            .prompt
            .starts_with("# Kegari runtime authority contract")
    );
    assert!(assembled.prompt.contains("# Scoped child implementation"));
    assert!(
        assembled
            .prompt
            .contains("# Correction, continuation and recovery")
    );
    assert!(assembled.prompt.contains("Root project convention"));
    assert!(assembled.prompt.contains("Nested project convention"));
    assert!(assembled.prompt.contains("Global scoped convention"));
    assert!(assembled.prompt.contains("# Generic repository work"));
    assert_eq!(assembled.manifest.supplied_files.len(), 3);
    assert!(matches!(
        assembled.manifest.provider_discovery,
        ProviderDiscovery::Unavailable { .. }
    ));
    assert_eq!(assembled.manifest.assets.len(), 3);
    assert_eq!(assembled.manifest.codex_version, SUPPORTED_CODEX_VERSION);
    let exported = fixture._dir.path().join("new-workflow.md");
    export_starter(&exported).unwrap();
    assert_eq!(
        Workflow::load(&exported)
            .unwrap()
            .render(&fixture.context)
            .unwrap(),
        fixture.workflow.render(&fixture.context).unwrap()
    );
    assert!(export_starter(&exported).is_err());
    assert_eq!(std::fs::read_to_string(exported).unwrap(), STARTER);
}
#[test]
fn literal_workflow_cannot_omit_admitted_task_corrections_or_prior_results() {
    let mut fixture = AssemblyFixture::new();
    let path = fixture._dir.path().join("literal-workflow.md");
    std::fs::write(&path, "Use the project conventions.\n").unwrap();
    fixture.workflow = Workflow::load(&path).unwrap();
    fixture.context["issue"]["description"] = json!("unique-child-task-872");
    fixture.context["parent"]["description"] = json!("unique-parent-outcome-913");
    fixture.context["dependencies"] = json!([{"result":"unique-dependency-644"}]);
    fixture.context["corrections"] = json!([{"body":"unique-correction-251"}]);
    fixture.context["evidence"] = json!([{"result":"unique-prior-result-469"}]);
    let assembled = assemble(fixture.input()).unwrap();
    let structured = assembled
        .prompt
        .split("# Admitted work context (structured task data)\n\n")
        .nth(1)
        .unwrap()
        .split("\n\n# Applicable repository-owned guidance")
        .next()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(structured).unwrap(),
        fixture.context
    );
    for expected in [
        "unique-child-task-872",
        "unique-parent-outcome-913",
        "unique-dependency-644",
        "unique-correction-251",
        "unique-prior-result-469",
    ] {
        assert!(assembled.prompt.contains(expected));
    }
    assert!(assembled.prompt.ends_with("Use the project conventions.\n"));
    assert_eq!(
        assembled.manifest.context_digest,
        canonical_digest(1, &fixture.context).unwrap()
    );
    assert_eq!(
        assembled.manifest.prompt_digest,
        bytes_digest(assembled.prompt.as_bytes())
    );
}
#[test]
fn complete_read_only_review_history_must_fit_with_the_instruction_envelope() {
    // This exercises production assembly, not archive authentication or an
    // actual model review. A compact publication must not become a truncated
    // reviewer prompt when the resolved history is too large.
    let mut fixture = AssemblyFixture::new();
    fixture.workflow =
        Workflow::parse("Review the complete supplied evidence.", "/workflow").unwrap();
    fixture.context["phase"] = json!("parent_integrated_review");
    fixture.context["evidence"] = json!({
        "attempts": [
            {"result":"failed", "details":"earlier-failure-must-remain-visible"},
            {"result":"passed", "details":"later-pass-does-not-erase-failure"}
        ],
        "history": "h".repeat(64 * 1024)
    });
    fn review_input(fixture: &AssemblyFixture) -> AssemblyInput<'_> {
        let mut input = fixture.input();
        input.role = AgentRole::Review;
        input.phase = "parent_integrated_review";
        input.permissions.source_write_paths.clear();
        input.permissions.repository_code_execution = false;
        input
    }
    let assembled = assemble(review_input(&fixture)).unwrap();
    let structured = assembled
        .prompt
        .split("# Admitted work context (structured task data)\n\n")
        .nth(1)
        .unwrap()
        .split("\n\n# Applicable repository-owned guidance")
        .next()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(structured).unwrap(),
        fixture.context
    );
    assert!(!assembled.manifest.permissions.repository_code_execution);
    assert!(assembled.manifest.permissions.source_write_paths.is_empty());

    fixture.context["evidence"]["history"] = json!("");
    let envelope = serde_json::to_vec(&fixture.context).unwrap().len();
    fixture.context["evidence"]["history"] = json!("h".repeat(MAX_CONTEXT_BYTES - envelope - 1));
    assert!(serde_json::to_vec(&fixture.context).unwrap().len() < MAX_CONTEXT_BYTES);
    assert_eq!(
        assemble(review_input(&fixture)).unwrap_err().code,
        "context_too_large",
        "the entire prompt must fit; a digest or truncated history is no fallback"
    );
}
#[test]
fn scratch_cannot_overlap_repository_in_either_direction() {
    let fixture = AssemblyFixture::new();
    for scratch in [
        &fixture.repository,
        &fixture.repository.join("src"),
        fixture.repository.parent().unwrap(),
    ] {
        let mut input = fixture.input();
        input.permissions.scratch_roots = vec![scratch.to_path_buf()];
        assert_eq!(assemble(input).unwrap_err().code, "access_denied");
    }
}
#[test]
fn reused_directories_refresh_nested_guidance_and_prevent_incompatible_resume() {
    let fixture = AssemblyFixture::new();
    let old = assemble(fixture.input()).unwrap();
    std::fs::write(
        fixture.repository.join("src/AGENTS.override.md"),
        "Override: use the replacement scoped rule.",
    )
    .unwrap();
    let new = assemble(fixture.input()).unwrap();
    assert!(new.prompt.contains("replacement scoped rule"));
    assert!(!new.prompt.contains("test numerical boundaries"));
    assert!(!old.manifest.resume_compatible(&new.manifest));
    assert!(old.manifest.resume_compatible(&old.manifest));
    std::fs::remove_file(fixture.repository.join("src/AGENTS.override.md")).unwrap();
    assert!(
        old.manifest
            .resume_compatible(&assemble(fixture.input()).unwrap().manifest)
    );
}
#[test]
fn role_tools_and_write_capabilities_cannot_expand_in_assembly() {
    let fixture = AssemblyFixture::new();
    for role in [
        AgentRole::Coordination,
        AgentRole::Review,
        AgentRole::IndependentReview,
        AgentRole::Validation,
    ] {
        let mut input = fixture.input();
        input.role = role;
        assert!(assemble(input).is_err());
    }
    let mut input = fixture.input();
    input.role = AgentRole::Coordination;
    input.permissions.source_write_paths.clear();
    input.permissions.repository_code_execution = false;
    input.tools = vec![ExposedTool {
        name: "route_correction".into(),
        capability: ToolCapability::CorrectionProposal,
    }];
    let assembled = assemble(input).unwrap();
    assert!(assembled.prompt.contains("# Read-only parent coordination"));
    let mut input = fixture.input();
    input.role = AgentRole::IndependentReview;
    input.permissions.source_write_paths.clear();
    input.permissions.repository_code_execution = false;
    input.tools = vec![ExposedTool {
        name: "feedback".into(),
        capability: ToolCapability::ReviewFeedback,
    }];
    let assembled = assemble(input).unwrap();
    assert!(assembled.prompt.contains("strictly read/feedback-only"));
    let mut input = fixture.input();
    input.tools.push(ExposedTool {
        name: "route".into(),
        capability: ToolCapability::CorrectionProposal,
    });
    assert!(assemble(input).is_err());
}
#[test]
fn secrets_other_workspaces_missing_context_and_oversize_context_block_assembly() {
    let mut fixture = AssemblyFixture::new();
    fixture.context["workspace"]["id"] = json!(id(999));
    assert!(assemble(fixture.input()).is_err());
    fixture.context["workspace"]["id"] = json!(id(2));
    fixture.context["issue"]["credentials"] = json!("protected");
    assert!(assemble(fixture.input()).is_err());
    fixture.context["issue"]
        .as_object_mut()
        .unwrap()
        .remove("credentials");
    fixture.context["issue"]["description"] = json!("secret-material");
    let secrets = vec!["secret-material".to_owned()];
    let mut input = fixture.input();
    input.known_secrets = &secrets;
    let error = assemble(input).unwrap_err();
    assert!(!error.to_string().contains("secret-material"));
    fixture.context["issue"]["description"] = json!("safe");
    let missing = [fixture.repository.join("missing.md")];
    let mut input = fixture.input();
    input.required_context = &missing;
    assert!(assemble(input).is_err());
    let mut input = fixture.input();
    input.project_doc_max_bytes = 1;
    assert!(assemble(input).is_err());
    let mut input = fixture.input();
    input
        .permissions
        .read_roots
        .push(fixture._dir.path().to_owned());
    assert!(assemble(input).is_err());
}
#[cfg(unix)]
#[test]
fn symlinked_repository_guidance_cannot_read_sibling_private_context() {
    let fixture = AssemblyFixture::new();
    let outside = fixture._dir.path().join("unrelated.md");
    std::fs::write(&outside, "unrelated private workspace").unwrap();
    std::os::unix::fs::symlink(&outside, fixture.repository.join("AGENTS.override.md")).unwrap();
    assert!(assemble(fixture.input()).is_err());
}
#[test]
fn incremental_context_records_actual_turn_revision_without_full_prompt_copy() {
    let input = IncrementalInput::new(
        id(6),
        "thread".into(),
        "turn".into(),
        IncrementalKind::Correction,
        "comment-rev-2".into(),
        "Apply the current correction",
        &[],
    )
    .unwrap();
    let serialized = serde_json::to_string(&input).unwrap();
    assert!(!serialized.contains("Apply the current correction"));
    assert!(serialized.contains("comment-rev-2"));
    assert!(
        IncrementalInput::new(
            id(6),
            "thread".into(),
            String::new(),
            IncrementalKind::Recovery,
            "rev".into(),
            "context",
            &[]
        )
        .is_err()
    );
}

#[test]
fn malformed_template_paths_never_alias_valid_context() {
    for source in [
        "{{ issue..title }}",
        "{{ issue. }}",
        "{{ issue['title']suffix }}",
        "{{ .issue }}",
    ] {
        assert!(
            Workflow::parse(source, "/workflow")
                .and_then(|w| w.render(&json!({"issue":{"title":"valid","suffix":"also-valid"}})))
                .is_err(),
            "{source}"
        );
    }
    assert_eq!(
        Workflow::parse("{{ issue['title'] }} {{ values[0] }}", "/workflow")
            .unwrap()
            .render(&json!({"issue":{"title":"valid"},"values":["first"]}))
            .unwrap(),
        "valid first"
    );
}
#[test]
fn escaped_secret_strings_do_not_evade_context_exclusion() {
    let mut fixture = AssemblyFixture::new();
    fixture.context["issue"]["description"] = json!("private\"token\nsecret");
    let secrets = vec!["private\"token\nsecret".to_owned()];
    let mut input = fixture.input();
    input.known_secrets = &secrets;
    assert!(assemble(input).is_err());
}
fn review_artifact(bytes: &[u8], encoding: &str) -> Value {
    use base64::Engine;
    let proof = kegari_v0::execution::ArtifactProof {
        path: PathBuf::from("/owned/scratch/original-artifact.bin"),
        digest: bytes_digest(bytes),
        bytes: bytes.len() as u64,
        validation_operation: id(7),
        candidate_vector_digest: None,
    };
    json!({"proof_digest":canonical_digest(1,&proof).unwrap(),"proof":proof,"encoding":encoding,
        "data":if encoding == "utf8" { std::str::from_utf8(bytes).unwrap().to_owned() }
            else { base64::engine::general_purpose::STANDARD.encode(bytes) }})
}
fn install_review_artifacts(fixture: &mut AssemblyFixture, artifacts: Value) {
    fixture.workflow = Workflow::parse("Review the exact supplied history.", "/workflow").unwrap();
    fixture.context["evidence"] = json!({"delivery_assignment":{"integrated_validation_review":{
        "validation_artifacts":artifacts}}});
}
fn readonly_input(fixture: &AssemblyFixture) -> AssemblyInput<'_> {
    let mut input = fixture.input();
    input.role = AgentRole::Review;
    input.permissions.source_write_paths.clear();
    input.permissions.repository_code_execution = false;
    input
}
#[test]
fn review_artifact_decoding_preserves_binary_nul_and_refuses_known_secret_bytes() {
    let secret = "only-original-decoded-secret".to_owned();
    let secrets = vec![secret.clone()];
    for (bytes, encoding) in [
        (secret.as_bytes().to_vec(), "utf8"),
        (
            [vec![0xff, 0], secret.as_bytes().to_vec()].concat(),
            "base64",
        ),
    ] {
        let mut fixture = AssemblyFixture::new();
        install_review_artifacts(&mut fixture, json!([review_artifact(&bytes, encoding)]));
        let mut input = readonly_input(&fixture);
        input.known_secrets = &secrets;
        assert_eq!(assemble(input).unwrap_err().code, "access_denied");
    }
    let mut fixture = AssemblyFixture::new();
    install_review_artifacts(
        &mut fixture,
        json!([review_artifact(&[0xff, 0, 1, 2], "base64")]),
    );
    let result = assemble(readonly_input(&fixture)).unwrap();
    assert!(!result.prompt.contains('\0'));
    assert!(!result.manifest.permissions.repository_code_execution);
    let input = result
        .prompt
        .split("# Admitted work context (structured task data)\n\n")
        .nth(1)
        .unwrap()
        .split("\n\n# Applicable repository-owned guidance")
        .next()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(input).unwrap(),
        fixture.context
    );
}
#[test]
fn review_artifact_identity_shape_encoding_and_aggregate_are_checked_before_assembly() {
    let original = review_artifact(b"original", "base64");
    for kind in 0..6 {
        let mut fixture = AssemblyFixture::new();
        let mut changed = original.clone();
        match kind {
            0 => changed["proof_digest"] = json!(canonical_digest(1, &"different").unwrap()),
            1 => changed["data"] = json!("!!!!"),
            2 => changed["encoding"] = json!("hex"),
            3 => changed["qualified"] = json!(true),
            4 => {
                changed["proof"]["bytes"] = json!(original["proof"]["bytes"].as_u64().unwrap() + 1);
                changed["proof_digest"] = json!(canonical_digest(1, &changed["proof"]).unwrap());
            }
            _ => changed["data"] = json!("ZGlmZmVyZW50"),
        }
        assert_ne!(
            changed, original,
            "negative case {kind} must alter the fixture"
        );
        install_review_artifacts(&mut fixture, json!([changed]));
        assert_eq!(
            assemble(readonly_input(&fixture)).unwrap_err().code,
            "invalid_context"
        );
    }
    let mut fixture = AssemblyFixture::new();
    install_review_artifacts(
        &mut fixture,
        json!([
            review_artifact(&vec![b'a'; 600_000], "utf8"),
            review_artifact(&vec![b'b'; 600_000], "utf8")
        ]),
    );
    assert_eq!(
        assemble(readonly_input(&fixture)).unwrap_err().code,
        "context_too_large"
    );
    install_review_artifacts(&mut fixture, json!([original.clone(), original]));
    assert_eq!(
        assemble(readonly_input(&fixture)).unwrap_err().code,
        "invalid_context"
    );
}
#[cfg(unix)]
#[test]
fn global_instruction_symlink_cannot_disguise_authentication_material() {
    let fixture = AssemblyFixture::new();
    std::fs::write(
        fixture.auth.join("auth.json"),
        "private provider authentication",
    )
    .unwrap();
    std::os::unix::fs::symlink(
        fixture.auth.join("auth.json"),
        fixture.auth.join("AGENTS.override.md"),
    )
    .unwrap();
    assert!(assemble(fixture.input()).is_err());
}
#[test]
fn host_path_resolution_is_transactional_and_workflow_check_references_are_verified() {
    let fixture = TempDir::new().unwrap();
    let mut config = configured();
    let before = config_revision(&config).unwrap();
    config.workspaces.get_mut("example").unwrap().workflow = Some("$WORKFLOW".into());
    let symbolic = config_revision(&config).unwrap();
    assert!(config.resolve_paths(fixture.path(), |_| None).is_err());
    assert_eq!(symbolic, config_revision(&config).unwrap());
    assert_ne!(before, symbolic);
    let workflow = fixture.path().join("WORKFLOW.md");
    std::fs::write(
        &workflow,
        "---\nchecks:\n  local:\n    local: {argv: [true], result_artifact: local.json}\n---\nTask",
    )
    .unwrap();
    config
        .resolve_paths(fixture.path(), |name| {
            (name == "WORKFLOW").then(|| workflow.to_string_lossy().into_owned())
        })
        .unwrap();
    assert!(load_workflows(&config).is_err());
    std::fs::write(&workflow,"---\nchecks:\n  local:\n    local: {argv: ['true'], result_artifact: local.json}\n  integrated:\n    e2e: {argv: ['true'], result_artifact: e2e.json}\n---\nTask").unwrap();
    assert_eq!(load_workflows(&config).unwrap().len(), 1);
}

#[test]
fn omitted_workflow_and_check_catalog_use_installed_guidance_without_file_lookup() {
    let mut value = serde_json::to_value(configured()).unwrap();
    value["workspaces"]["example"]
        .as_object_mut()
        .unwrap()
        .remove("workflow");
    let repository = value["workspaces"]["example"]["repositories"]["101"]
        .as_object_mut()
        .unwrap();
    repository.remove("local_checks");
    repository.remove("integrated_checks");
    let config: Config = serde_json::from_value(value).unwrap();
    config.validate().unwrap();
    let loaded = load_workflows(&config).unwrap();
    let workflow = &loaded[&id(2)];
    assert!(workflow.path.is_none());
    assert!(workflow.embedded.is_some());
    assert!(workflow.front_matter.checks.local.is_empty());
    assert!(workflow.front_matter.checks.integrated.is_empty());
    assert!(
        serde_json::to_value(&config).unwrap()["workspaces"]["example"]
            .get("workflow")
            .is_none()
    );
    // Absence is descriptive and grants neither a passed check nor execution.
    let mut selected = config.clone();
    let dir = TempDir::new().unwrap();
    selected.workspaces.get_mut("example").unwrap().workflow =
        Some(dir.path().join("missing.md").to_str().unwrap().into());
    assert!(load_workflows(&selected).is_err());
    let invalid = dir.path().join("invalid.md");
    std::fs::write(&invalid, "---\nunknown: true\n---\nTask").unwrap();
    selected.workspaces.get_mut("example").unwrap().workflow =
        Some(invalid.to_str().unwrap().into());
    assert!(load_workflows(&selected).is_err());
}
#[test]
fn installed_workflow_assembly_records_an_asset_origin_and_preserves_required_context() {
    let mut fixture = AssemblyFixture::new();
    fixture.workflow = Workflow::installed_default().unwrap();
    let assembled = assemble(fixture.input()).unwrap();
    assert!(assembled.manifest.workflow_path.is_none());
    let embedded = assembled.manifest.workflow_embedded.as_ref().unwrap();
    assert_eq!(embedded.digest, assembled.manifest.workflow_digest);
    assert_eq!(
        assembled.manifest.assets["default_workflow"],
        embedded.digest
    );
    assert!(
        assembled
            .prompt
            .contains("Implement scoped boundary behavior")
    );
    assert!(assembled.prompt.contains("Root project convention"));
    assert!(assembled.prompt.contains("empty or unborn repository"));
    assert_eq!(
        assembled
            .prompt
            .matches("# Generic repository work")
            .count(),
        1
    );
    let encoded = serde_json::to_value(&assembled.manifest).unwrap();
    assert!(encoded.get("workflow_path").is_none());
    fixture.workflow.path = Some(fixture.repository.join("pretended.md"));
    assert!(assemble(fixture.input()).is_err());
}
