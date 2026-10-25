//! Deterministic production-path coverage for V0-T01, T33 and T34.
//! These do not substitute for provider discovery/sandbox or actual-agent trials.
use kegari_v0::config::*;
use kegari_v0::digest::{bytes_digest, canonical_digest};
use kegari_v0::instructions::*;
use kegari_v0::workflow::*;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
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
    assert_eq!(config.revision().unwrap(), parsed.revision().unwrap());
}

#[test]
fn hosted_proof_routes_require_pinned_caller_and_reusable_workflow_identity() {
    let good = configured();
    let route = good.workspaces["example"].repositories["101"].ci[0]
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
fn invalid_reload_fences_mutations_and_does_not_change_valid_snapshot() {
    let config = configured();
    let before = config.revision().unwrap();
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
    assert_eq!(before, config.revision().unwrap());
    assert!(matches!(
        inspect_reload(&before, &toml::to_string(&config).unwrap()),
        ConfigReload::Unchanged
    ));
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
    assert!(assembled.prompt.contains("Repository work for Fixture"));
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
    let before = config.revision().unwrap();
    config.workspaces.get_mut("example").unwrap().workflow = "$WORKFLOW".into();
    let symbolic = config.revision().unwrap();
    assert!(config.resolve_paths(fixture.path(), |_| None).is_err());
    assert_eq!(symbolic, config.revision().unwrap());
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
    assert!(config.load_workflows().is_err());
    std::fs::write(&workflow,"---\nchecks:\n  local:\n    local: {argv: ['true'], result_artifact: local.json}\n  integrated:\n    e2e: {argv: ['true'], result_artifact: e2e.json}\n---\nTask").unwrap();
    assert_eq!(config.load_workflows().unwrap().len(), 1);
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
