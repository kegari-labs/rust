//! Deterministic production-path coverage for V0-T01, T33 and T34.
//! These do not substitute for provider discovery/sandbox or actual-agent trials.
use kegari_config::*;
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_v0::config::{ConfigReload, config_revision, inspect_reload, load_workflows};
use serde_json::json;
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
        child: None,
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
        base_commit: Some("1".repeat(40)),
        head_commit: Some("2".repeat(40)),
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
        let expected = kegari_git::validate_oid(&oid);
        assert_eq!(expected.is_ok(), accepted, "unexpected grammar for {oid:?}");
        for field in ["base_commit", "head_commit"] {
            let mut declaration = workspace.evaluation.clone().unwrap();
            if field == "base_commit" {
                declaration.base_commit = Some(oid.clone());
            } else {
                declaration.head_commit = Some(oid.clone());
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
