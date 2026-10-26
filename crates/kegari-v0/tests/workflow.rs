//! Workflow data and filesystem-resolution components only: no process launch,
//! service readiness, Control admission or native qualification is exercised.
use kegari_v0::workflow::{
    CheckCommand, MAX_CHECK_SERVICES, MAX_SERVICE_ARGUMENT_BYTES, MAX_SERVICE_ARGV,
    MAX_SERVICE_COMMAND_BYTES, MAX_SERVICE_LIFETIME_MS, Workflow,
};
use serde_json::{Value, json};
use std::path::Path;

fn service(name: &str) -> Value {
    json!({
        "name": name,
        "argv": ["service", "--foreground", "literal {{ issue.id }}"],
        "readiness": {"argv": ["ready", "--once", ""], "timeout_ms": 1000},
        "max_lifetime_ms": 5000
    })
}
fn check() -> Value {
    json!({"argv":["check", "--result", "result.json"],
        "result_artifact":"result.json", "services":[service("database")]})
}
fn source(command: Value, kind: &str) -> String {
    let checks = serde_json::Map::from_iter([(kind.to_owned(), json!({"suite":command}))]);
    format!(
        "---\n{}\n---\nCheck the declared result.\n",
        json!({"schema_version":1,"checks":checks})
    )
}
fn parse(command: Value) -> kegari_v0::error::Result<Workflow> {
    Workflow::parse(&source(command, "local"), "/configured/WORKFLOW.md")
}

#[test]
fn empty_service_declaration_preserves_original_command_and_resolved_bytes() {
    #[derive(serde::Serialize)]
    struct LegacyResolved<'a> {
        cwd: &'a Path,
        argv: &'a [String],
        artifact_roots: &'a [std::path::PathBuf],
        result_artifact: &'a Path,
    }
    let legacy = r#"{"argv":["check"],"result_artifact":"result.json"}"#;
    let omitted: CheckCommand = serde_json::from_str(legacy).unwrap();
    let explicit: CheckCommand = serde_json::from_value(json!({
        "argv":["check"], "result_artifact":"result.json", "services":[]
    }))
    .unwrap();
    for command in [omitted, explicit] {
        assert!(command.services.is_empty());
        assert_eq!(serde_json::to_string(&command).unwrap(), legacy);
        let temp = tempfile::tempdir().unwrap();
        let cwd = temp.path().canonicalize().unwrap();
        let workflow = Workflow::parse("task", "/configured/WORKFLOW.md").unwrap();
        let resolved = workflow
            .resolve_check(&command, &cwd, std::slice::from_ref(&cwd))
            .unwrap();
        assert!(resolved.services.is_empty());
        assert_eq!(
            serde_json::to_vec(&resolved).unwrap(),
            serde_json::to_vec(&LegacyResolved {
                cwd: &resolved.cwd,
                argv: &resolved.argv,
                artifact_roots: &resolved.artifact_roots,
                result_artifact: &resolved.result_artifact,
            })
            .unwrap()
        );
    }
}

#[test]
fn bundle_resolution_retains_service_order_literal_argv_and_actual_check_scope() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let cwd = root.join("candidate");
    let workflow_dir = root.join("workflow");
    std::fs::create_dir(&cwd).unwrap();
    std::fs::create_dir(&workflow_dir).unwrap();
    for kind in ["local", "integrated"] {
        let mut declared = check();
        declared["services"] = json!([service("z-primary"), service("a-helper")]);
        let workflow = Workflow::parse(
            &source(declared.clone(), kind),
            workflow_dir.join("WORKFLOW.md"),
        )
        .unwrap();
        let configured = if kind == "local" {
            &workflow.front_matter.checks.local["suite"]
        } else {
            &workflow.front_matter.checks.integrated["suite"]
        };
        let resolved = workflow
            .resolve_check(configured, &cwd, std::slice::from_ref(&cwd))
            .unwrap();
        assert_eq!(resolved.cwd, cwd);
        assert_eq!(resolved.artifact_roots, vec![cwd.clone()]);
        assert_eq!(resolved.result_artifact, cwd.join("result.json"));
        assert_eq!(resolved.argv, configured.argv);
        assert_eq!(resolved.services, configured.services);
        assert_eq!(resolved.services[0].name, "z-primary");
        assert_eq!(resolved.services[1].name, "a-helper");
        assert_eq!(resolved.services[0].argv[2], "literal {{ issue.id }}");
        assert_eq!(resolved.services[0].readiness.argv[2], "");
        assert_eq!(
            serde_json::to_value(&resolved).unwrap()["services"],
            declared["services"]
        );
        let mut changed = declared;
        changed["services"][0]["readiness"]["timeout_ms"] = json!(999);
        assert_ne!(
            workflow.digest,
            Workflow::parse(&source(changed, kind), workflow_dir.join("WORKFLOW.md"))
                .unwrap()
                .digest
        );
    }
}

#[test]
fn service_schema_rejects_missing_unknown_and_duplicate_fields() {
    for key in ["cwd", "env", "stop", "url", "network", "output_limit"] {
        let mut declared = check();
        declared["services"][0][key] = json!("unrequested");
        assert!(parse(declared).is_err(), "unexpected service field {key}");
    }
    for key in ["name", "argv", "readiness", "max_lifetime_ms"] {
        let mut declared = check();
        declared["services"][0].as_object_mut().unwrap().remove(key);
        assert!(parse(declared).is_err(), "missing service field {key}");
    }
    for key in ["argv", "timeout_ms"] {
        let mut declared = check();
        declared["services"][0]["readiness"]
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(parse(declared).is_err(), "missing readiness field {key}");
    }
    let mut declared = check();
    declared["services"][0]["readiness"]["attempts"] = json!(2);
    assert!(parse(declared).is_err());
    let duplicate = "---\nchecks:\n  local:\n    suite:\n      argv: [check]\n      result_artifact: result.json\n      services:\n        - name: database\n          argv: [service]\n          max_lifetime_ms: 5000\n          readiness:\n            argv: [ready]\n            timeout_ms: 1000\n            timeout_ms: 2000\n---\ntask\n";
    assert!(Workflow::parse(duplicate, "/workflow").is_err());
    for services in [Value::Null, json!({}), json!("service")] {
        let mut declared = check();
        declared["services"] = services;
        assert!(parse(declared).is_err());
    }
    let unsupported =
        source(check(), "local").replace("\"schema_version\":1", "\"schema_version\":2");
    assert!(Workflow::parse(&unsupported, "/workflow").is_err());
}

#[test]
fn service_inventory_names_and_time_bounds_are_finite_and_exact() {
    let mut declared = check();
    declared["services"] = Value::Array(
        (0..MAX_CHECK_SERVICES)
            .map(|index| service(&format!("service-{index}")))
            .collect(),
    );
    assert!(parse(declared.clone()).is_ok());
    declared["services"]
        .as_array_mut()
        .unwrap()
        .push(service("extra"));
    assert!(parse(declared).is_err());
    let mut duplicate = check();
    duplicate["services"] = json!([service("same"), service("same")]);
    assert!(parse(duplicate).is_err());
    for name in [
        "".into(),
        "white space".into(),
        "../escape".into(),
        "é".into(),
        "a".repeat(65),
    ] {
        let mut declared = check();
        declared["services"][0]["name"] = json!(name);
        assert!(parse(declared).is_err());
    }
    let mut maximum = check();
    maximum["services"][0]["name"] = json!("a".repeat(64));
    maximum["services"][0]["max_lifetime_ms"] = json!(MAX_SERVICE_LIFETIME_MS);
    maximum["services"][0]["readiness"]["timeout_ms"] = json!(MAX_SERVICE_LIFETIME_MS);
    assert!(parse(maximum).is_ok());
    for (readiness, lifetime) in [
        (0, 5000),
        (1, 0),
        (5001, 5000),
        (1, MAX_SERVICE_LIFETIME_MS + 1),
    ] {
        let mut declared = check();
        declared["services"][0]["max_lifetime_ms"] = json!(lifetime);
        declared["services"][0]["readiness"]["timeout_ms"] = json!(readiness);
        assert!(parse(declared).is_err());
    }
}

#[test]
fn bundle_main_start_and_readiness_argv_share_bounded_validation() {
    for pointer in ["/argv", "/services/0/argv", "/services/0/readiness/argv"] {
        for argv in [
            vec![],
            vec![" ".into()],
            vec!["command".into(), "nul\0argument".into()],
            vec!["x".repeat(MAX_SERVICE_ARGUMENT_BYTES + 1)],
            vec!["x".into(); MAX_SERVICE_ARGV + 1],
            vec![
                "x".repeat(MAX_SERVICE_ARGUMENT_BYTES);
                MAX_SERVICE_COMMAND_BYTES / MAX_SERVICE_ARGUMENT_BYTES + 1
            ],
        ] {
            let mut declared = check();
            *declared.pointer_mut(pointer).unwrap() = json!(argv);
            assert!(
                parse(declared).is_err(),
                "invalid argv accepted at {pointer}"
            );
        }
        let mut declared = check();
        *declared.pointer_mut(pointer).unwrap() = json!(vec![
            "x".repeat(MAX_SERVICE_ARGUMENT_BYTES);
            MAX_SERVICE_COMMAND_BYTES
                / MAX_SERVICE_ARGUMENT_BYTES
        ]);
        assert!(
            parse(declared).is_ok(),
            "exact byte bound refused at {pointer}"
        );
        let mut declared = check();
        *declared.pointer_mut(pointer).unwrap() = json!(vec!["x"; MAX_SERVICE_ARGV]);
        assert!(
            parse(declared).is_ok(),
            "exact argument count refused at {pointer}"
        );
    }
    // The new service-bundle bounds do not silently narrow old no-service argv.
    let legacy = json!({"argv":vec!["x"; MAX_SERVICE_ARGV + 1], "result_artifact":"result.json"});
    assert!(parse(legacy).is_ok());
}

#[test]
fn typed_resolution_rechecks_services_and_retains_artifact_containment() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().canonicalize().unwrap();
    let workflow = parse(check()).unwrap();
    let original = &workflow.front_matter.checks.local["suite"];
    let mut invalid = original.clone();
    invalid.services[0].readiness.timeout_ms = 0;
    assert!(
        workflow
            .resolve_check(&invalid, &cwd, std::slice::from_ref(&cwd))
            .is_err()
    );
    for path in ["../outside/result.json", "/outside/result.json"] {
        let mut invalid = original.clone();
        invalid.result_artifact = path.into();
        assert!(
            workflow
                .resolve_check(&invalid, &cwd, std::slice::from_ref(&cwd))
                .is_err()
        );
    }
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), cwd.join("linked")).unwrap();
        let mut invalid = original.clone();
        invalid.result_artifact = "linked/result.json".into();
        assert!(
            workflow
                .resolve_check(&invalid, &cwd, std::slice::from_ref(&cwd))
                .is_err()
        );
    }
}
