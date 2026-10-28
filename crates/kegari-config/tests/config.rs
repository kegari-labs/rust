//! Configuration schema, parsing and policy validation without backend execution.
use kegari_config::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
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
#[test]
fn supplied_null_empty_or_malformed_delivery_policy_never_means_unresolved() {
    for field in ["ci", "production_ci", "no_change_validation", "deployment"] {
        let mut value = serde_json::to_value(configured()).unwrap();
        value["workspaces"]["example"]["repositories"]["101"][field] = Value::Null;
        assert!(
            serde_json::from_value::<Config>(value).is_err(),
            "accepted explicit null {field}"
        );
    }
    for field in ["ci", "production_ci"] {
        let mut config = configured();
        let repository = config
            .workspaces
            .get_mut("example")
            .unwrap()
            .repositories
            .get_mut("101")
            .unwrap();
        if field == "ci" {
            repository.ci = Some(vec![]);
        } else {
            repository.production_ci = Some(vec![]);
        }
        assert!(repository.validate_supplied_policy().is_err());
        let error = if field == "ci" {
            repository.staging_ci().unwrap_err()
        } else {
            repository.production_ci_policy().unwrap_err()
        };
        assert_eq!(error.code, "invalid_config");
        assert!(config.validate().is_err());
        assert!(Config::parse(&toml::to_string(&config).unwrap()).is_err());
    }
    for (field, malformed) in [
        ("ci", json!([{"workflow_id":0,"required_checks":[]}])),
        ("production_ci", json!({})),
        ("no_change_validation", json!({})),
        ("deployment", json!({"steps":[]})),
        ("deployment", json!("unknown")),
    ] {
        let mut value = serde_json::to_value(configured()).unwrap();
        value["workspaces"]["example"]["repositories"]["101"][field] = malformed;
        if let Ok(config) = serde_json::from_value::<Config>(value) {
            assert!(config.validate().is_err());
        }
    }
}

#[test]
fn evaluation_requires_explicit_deployment_free_policy_even_when_other_policy_is_omitted() {
    let mut config = configured();
    let workspace = config.workspaces.get_mut("example").unwrap();
    let workspace_id = workspace.id;
    workspace.independent_review.enabled = true;
    workspace.profiles.independent_review = Some(profile());
    workspace.github.pr_authors = vec!["77".into()];
    workspace.github_connection = format!("evaluation-{workspace_id}-github");
    workspace.linear_connection = format!("evaluation-{workspace_id}-linear");
    let repository = workspace.repositories.get_mut("101").unwrap();
    repository.hostname = format!("github.{workspace_id}.evaluation.invalid");
    repository.ci = None;
    repository.production_ci = None;
    repository.no_change_validation = None;
    workspace.evaluation = Some(EvaluationBinding {
        schema_version: 1,
        suite_manifest: "/private/tmp/evaluation-suite.json".into(),
        suite_digest: "sha256:fb7ab6c0bb6d1d51fc4b003ec620a36ac9746cd0c60f745d255bbdba0113bd07"
            .into(),
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
        connection.endpoint = Some(format!(
            "https://{provider}.{workspace_id}.evaluation.invalid{}",
            if provider == "linear" {
                "/graphql"
            } else {
                "/"
            }
        ));
        config
            .connections
            .insert(format!("evaluation-{workspace_id}-{provider}"), connection);
    }
    config.validate().unwrap();
    Config::parse(&toml::to_string(&config).unwrap()).unwrap();
    for (provider, endpoint) in [
        (
            "github",
            format!("https://github.{workspace_id}.evaluation.invalid"),
        ),
        (
            "github",
            format!("https://github.{workspace_id}.evaluation.invalid/other"),
        ),
        (
            "github",
            format!("https://github.{}.evaluation.invalid/", Uuid::nil()),
        ),
        (
            "linear",
            format!("https://linear.{workspace_id}.evaluation.invalid/graphql/"),
        ),
    ] {
        let mut changed = config.clone();
        changed
            .connections
            .get_mut(&format!("evaluation-{workspace_id}-{provider}"))
            .unwrap()
            .endpoint = Some(endpoint);
        assert_eq!(changed.validate().unwrap_err().code, "invalid_config");
    }
    assert!(
        matches!(config.workspaces["example"].repositories["101"].deployment_policy().unwrap(), Deployment::None(value) if value=="none")
    );
    config
        .workspaces
        .get_mut("example")
        .unwrap()
        .repositories
        .get_mut("101")
        .unwrap()
        .deployment = None;
    let encoded = serde_json::to_value(&config).unwrap();
    assert!(
        encoded["workspaces"]["example"]["repositories"]["101"]
            .get("deployment")
            .is_none()
    );
    assert_eq!(config.validate().unwrap_err().code, "invalid_config");
    assert!(Config::parse(&toml::to_string(&config).unwrap()).is_err());
}

#[test]
fn obsolete_managed_catalog_is_rejected_including_null_and_claimed_qualification() {
    for obsolete in [
        Value::Null,
        json!({}),
        json!({"lifecycle":"foreground_v1"}),
        json!({"qualified":true}),
        json!({"lifecycle":"check_bundle_v1","executables":[{"path":"/private/tmp/tool","digest":"sha256:".to_owned()+&"1".repeat(64)}],"read_roots":[{"path":"/private/tmp/library","directory":true}],"generated_executables":"scratch_build_v1"}),
    ] {
        let mut value = serde_json::to_value(configured()).unwrap();
        value["workspaces"]["example"]["managed_execution"] = obsolete;
        assert!(serde_json::from_value::<Config>(value).is_err());
    }
    let config = configured();
    let text = toml::to_string(&config).unwrap()
        + "\n[workspaces.example.managed_execution]\nlifecycle = 'foreground_v1'\n";
    assert!(Config::parse(&text).is_err());
}

#[test]
fn hosted_proof_routes_require_pinned_caller_and_reusable_workflow_identity() {
    let good = configured();
    let route = good.workspaces["example"].repositories["101"]
        .staging_ci()
        .unwrap()[0]
        .proof
        .clone();
    route.validate().unwrap();
    for field in [
        "workflow_sha",
        "job_workflow_sha",
        "workflow_ref",
        "job_workflow_ref",
        "event_name",
    ] {
        let mut value = serde_json::to_value(&route).unwrap();
        value[field] = json!(match field {
            "workflow_ref" => "owner/repo/../../etc@main",
            "job_workflow_ref" => "owner/repo/.github/workflows/test.yml@main",
            "event_name" => "unreviewed_event",
            _ => "main",
        });
        assert!(
            serde_json::from_value::<HostedProofRoute>(value)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut missing = serde_json::to_value(&good).unwrap();
    missing["workspaces"]["example"]["repositories"]["101"]["ci"][0]
        .as_object_mut()
        .unwrap()
        .remove("proof");
    assert!(serde_json::from_value::<Config>(missing).is_err());
    let mut mismatched = good;
    mismatched
        .workspaces
        .get_mut("example")
        .unwrap()
        .repositories
        .get_mut("101")
        .unwrap()
        .no_change_validation
        .as_mut()
        .unwrap()
        .proof
        .workflow_sha = "1111111111111111111111111111111111111111".into();
    assert!(mismatched.validate().is_err());
}

#[test]
fn unknown_duplicate_and_wrong_typed_config_never_load() {
    let text = toml::to_string(&configured()).unwrap();
    for bad in [
        format!("unexpected = true\n{text}"),
        text.replace(
            "schema_version = 1",
            "schema_version = 1\nschema_version = 1",
        ),
        text.replace("max_agents = 4", "max_agents = \"4\""),
    ] {
        assert!(Config::parse(&bad).is_err());
    }
    let mut config = configured();
    config.runtime.max_agents = 0;
    assert!(config.validate().is_err());
    config = configured();
    config.schema_version = 99;
    assert!(config.validate().is_err());
    config = configured();
    config.workspaces.get_mut("example").unwrap().linear.users = Selector::Ids(vec![]);
    assert!(config.validate().is_err());
}

#[test]
fn selectors_use_category_intersection_creator_or_assignee_and_repository_union() {
    let mut selection = configured().workspaces["example"].linear.clone();
    selection.teams = Selector::Ids(vec!["team-a".into()]);
    selection.projects = Selector::Ids(vec!["project".into()]);
    selection.users = Selector::Ids(vec!["person".into()]);
    assert!(selection.includes("team-a", Some("project"), "person", None));
    assert!(selection.includes("team-a", Some("project"), "other", Some("person")));
    assert!(!selection.includes("team-b", Some("project"), "person", None));
    assert!(!selection.includes("team-a", None, "person", None));
    let github = GithubSelection {
        organizations: vec!["org".into()],
        repositories: vec!["repo".into()],
        pr_authors: vec!["author".into()],
    };
    assert!(github.permits_repository("unlisted", Some("org")));
    assert!(github.permits_repository("repo", None));
    assert!(!github.permits_repository("unlisted", Some("other")));
    assert!(!github.permits_pr_author("reviewer"));
}

#[test]
fn routing_requires_one_label_per_dimension_and_preserves_prior_profile() {
    let config = configured();
    let labels = &config.workspaces["example"].routing_labels;
    let selected: BTreeSet<_> = [id(20), id(21), id(22), id(23)].into_iter().collect();
    let old = labels.resolve(&selected).unwrap();
    assert_eq!(old, profile());
    let mut edited = labels.clone();
    edited.model.insert(id(24), "changed-model".into());
    let mut conflict = selected.clone();
    conflict.insert(id(24));
    assert!(edited.resolve(&conflict).is_err());
    let mut missing = selected.clone();
    missing.remove(&id(22));
    assert!(edited.resolve(&missing).is_err());
    edited.model.insert(id(21), "new-model".into());
    assert_eq!(edited.resolve(&selected).unwrap().model, "new-model");
    assert_eq!(old.model, "fixture-model");
}

#[test]
fn independent_review_and_same_account_caps_require_explicit_configuration() {
    let mut config = configured();
    let mut duplicate = config.connections["codex"].clone();
    duplicate.auth_home = Some("/private/tmp/second-auth".into());
    duplicate.max_agents = Some(1);
    config.connections.insert("alias".into(), duplicate);
    config.validate().unwrap();
    assert_eq!(config.account_limits()["codex-account"], 1);
    let workspace = config.workspaces.get_mut("example").unwrap();
    workspace.independent_review.enabled = true;
    assert!(config.validate().is_err());
    let workspace = config.workspaces.get_mut("example").unwrap();
    workspace.github.pr_authors = vec!["selected-user".into()];
    workspace.profiles.independent_review = Some(profile());
    config.validate().unwrap();
}

#[test]
fn path_expansion_is_host_only_literal_and_has_no_shell_interpolation() {
    let home = Path::new("/private/tmp/home");
    assert_eq!(
        resolve_host_path("~/runs", home, |_| None).unwrap(),
        home.join("runs")
    );
    assert_eq!(
        resolve_host_path("$ROOT", home, |key| (key == "ROOT")
            .then(|| "/private/tmp/runs".into()))
        .unwrap(),
        PathBuf::from("/private/tmp/runs")
    );
    for path in [
        "$MISSING",
        "$HOME/runs",
        "${HOME}",
        "$(touch /tmp/x)",
        "~/../escape",
        "~other/runs",
        "relative",
    ] {
        assert!(resolve_host_path(path, home, |_| None).is_err(), "{path}");
    }
    assert!(resolve_host_path("$ROOT", home, |_| Some("$OTHER".into())).is_err());
}

#[test]
fn configuration_errors_never_echo_supplied_secret_values() {
    let text = format!(
        "schema_version = 1\npassword = \"highly-private-token\"\n{}",
        toml::to_string(&configured()).unwrap()
    );
    assert!(
        !Config::parse(&text)
            .unwrap_err()
            .to_string()
            .contains("highly-private-token")
    );
    let mut config = configured();
    config.connections.get_mut("github").unwrap().credential_ref =
        Some("highly-private-token".into());
    assert!(
        !config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("highly-private-token")
    );
}

#[test]
fn unknown_deployment_keys_are_not_silently_accepted() {
    let mut config = serde_json::to_value(configured()).unwrap();
    config["workspaces"]["example"]["repositories"]["101"]["deployment"] =
        json!({"steps":[],"unrestricted":true});
    assert!(serde_json::from_value::<Config>(config).is_err());
}

#[test]
fn tailscale_address_allowlist_rejects_wildcard_lan_and_public_bind() {
    for value in ["100.64.1.2", "100.127.255.254", "fd7a:115c:a1e0::1"] {
        assert!(is_tailscale_address(value.parse().unwrap()));
    }
    for value in [
        "0.0.0.0",
        "::",
        "192.168.1.10",
        "127.0.0.1",
        "8.8.8.8",
        "100.63.0.1",
        "fd00::1",
    ] {
        assert!(!is_tailscale_address(value.parse().unwrap()));
    }
}

#[test]
fn tailscale_operator_mapping_uses_stable_numeric_identity_not_login_text() {
    let mut config = configured();
    let operator = config.control.local_users.values().next().unwrap().clone();
    for key in ["human@example.invalid", "00042", "0", "-1", "42 "] {
        config.control.tailscale_users =
            std::collections::BTreeMap::from([(key.into(), operator.clone())]);
        assert!(config.validate().is_err(), "accepted {key}");
    }
    config.control.tailscale_users = std::collections::BTreeMap::from([("42".into(), operator)]);
    config.validate().unwrap();
}

#[test]
fn deployment_environment_ids_are_canonical_provider_ids_not_names() {
    for environment in ["production", "0", "088", "+88", "18446744073709551616"] {
        let mut value = serde_json::to_value(configured()).unwrap();
        let repository = &mut value["workspaces"]["example"]["repositories"]["101"];
        repository["deployment"] = json!({"steps":[{"id":"deploy","target_id":"production-target","environment_id":environment,"workflow":repository["no_change_validation"]}]});
        let config: Config = serde_json::from_value(value).unwrap();
        assert!(
            config.validate().is_err(),
            "accepted noncanonical environment {environment}"
        );
    }
}
