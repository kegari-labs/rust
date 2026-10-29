//! Codex path-scope validation uses only disposable filesystem fixtures.
use kegari_execution::*;
use std::path::Path;

const TEST_BUILD: &str = "execution-integration-test-build";

fn fixture() -> (tempfile::TempDir, IsolationPlan) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for d in [
        "repository",
        "repository/src",
        "scratch",
        "privileged",
        "auth",
    ] {
        std::fs::create_dir(root.join(d)).unwrap();
    }
    let plan = IsolationPlan {
        schema_version: 1,
        role: ExecutionRole::Child,
        execution_root: root.clone(),
        repository: root.join("repository"),
        cwd: root.join("repository"),
        scratch: root.join("scratch"),
        read_roots: vec![],
        resource_writes: vec![],
        source_writes: vec![PathGrant {
            path: root.join("repository/src"),
            directory: true,
        }],
        forbidden_roots: vec![root.join("privileged")],
    };
    (temp, plan)
}

#[test]
fn role_scope_rejects_unpermitted_source_writes() {
    let (_temp, mut plan) = fixture();
    plan.validate(TEST_BUILD).unwrap();
    plan.role = ExecutionRole::Coordination;
    assert!(plan.validate(TEST_BUILD).is_err());
    plan.source_writes.clear();
    plan.validate(TEST_BUILD).unwrap();
    plan.role = ExecutionRole::IndependentReview;
    plan.validate(TEST_BUILD).unwrap();
}

#[test]
fn artifact_resolution_uses_candidate_and_refuses_symlink_escape() {
    let (_temp, plan) = fixture();
    let isolated = plan.validate(TEST_BUILD).unwrap();
    std::fs::write(plan.cwd.join("results.json"), "candidate").unwrap();
    std::fs::write(plan.execution_root.join("results.json"), "workflow-copy").unwrap();
    assert_eq!(
        isolated.artifact(Path::new("results.json")).unwrap(),
        plan.cwd.join("results.json")
    );
    assert!(isolated.artifact(Path::new("../results.json")).is_err());
    assert!(
        isolated
            .artifact(&plan.execution_root.join("results.json"))
            .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&plan.forbidden_roots[0], plan.cwd.join("escape")).unwrap();
        assert!(isolated.artifact(Path::new("escape/secret")).is_err());
    }
}

#[test]
fn identity_directory_names_do_not_collide_or_escape() {
    let a = execution_name("../../same title", "issue-one", "repo", "g1").unwrap();
    let b = execution_name("../../same title", "issue-two", "repo", "g1").unwrap();
    assert_ne!(a, b);
    assert!(!a.contains('/'));
    assert!(!a.starts_with('.'));
    assert!(
        a.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    );
}

#[test]
fn writable_directory_rejects_hardlinks_to_unrelated_files() {
    let (_temp, plan) = fixture();
    let outside = plan.forbidden_roots[0].join("secret");
    std::fs::write(&outside, "private").unwrap();
    std::fs::hard_link(outside, plan.repository.join("src/linked")).unwrap();
    assert_eq!(
        plan.validate(TEST_BUILD).unwrap_err().code,
        "scope_violation"
    );
}

#[test]
fn test_resource_writes_are_separate_exact_grants_and_never_reopen_source_or_control() {
    let (_temp, mut plan) = fixture();
    let resource = plan.execution_root.join("disposable-resource");
    std::fs::create_dir(&resource).unwrap();
    std::fs::write(resource.join("data"), "fixture").unwrap();
    let grant = PathGrant {
        path: resource.clone(),
        directory: true,
    };
    plan.resource_writes.push(grant.clone());
    assert_eq!(
        plan.validate(TEST_BUILD).unwrap_err().code,
        "scope_violation"
    );
    plan.read_roots.push(grant.clone());
    let boundary = plan.validate(TEST_BUILD).unwrap();
    assert_eq!(boundary.plan().resource_writes, vec![grant]);
    plan.role = ExecutionRole::Coordination;
    plan.source_writes.clear();
    assert_eq!(
        plan.validate(TEST_BUILD).unwrap_err().code,
        "scope_violation"
    );
    plan.role = ExecutionRole::Validation;
    plan.validate(TEST_BUILD).unwrap();
    let original = plan.resource_writes.clone();
    for path in [&plan.repository, &plan.scratch, &plan.forbidden_roots[0]] {
        let mut unsafe_plan = plan.clone();
        let grant = PathGrant {
            path: path.clone(),
            directory: true,
        };
        unsafe_plan.read_roots.push(grant.clone());
        unsafe_plan.resource_writes = vec![grant];
        assert_eq!(
            unsafe_plan.validate(TEST_BUILD).unwrap_err().code,
            "scope_violation"
        );
    }
    plan.resource_writes = original;
    std::os::unix::fs::symlink(&plan.repository, resource.join("escape")).unwrap();
    assert_eq!(
        plan.validate(TEST_BUILD).unwrap_err().code,
        "scope_violation"
    );
}

#[test]
fn isolation_records_preserve_exact_fields_defaults_and_unknown_field_refusal() {
    let grant = PathGrant {
        path: "/fixture/read".into(),
        directory: true,
    };
    assert_eq!(
        serde_json::to_string(&grant).unwrap(),
        r#"{"path":"/fixture/read","directory":true}"#
    );
    assert_eq!(
        serde_json::to_string(&ExecutionRole::IndependentReview).unwrap(),
        "\"independent_review\""
    );
    let plan = IsolationPlan {
        schema_version: 1,
        role: ExecutionRole::Child,
        execution_root: "/fixture".into(),
        repository: "/fixture/repository".into(),
        cwd: "/fixture/repository".into(),
        scratch: "/fixture/scratch".into(),
        read_roots: vec![grant],
        source_writes: vec![],
        resource_writes: vec![],
        forbidden_roots: vec!["/fixture/forbidden".into()],
    };
    let encoded = serde_json::to_string(&plan).unwrap();
    assert_eq!(
        encoded,
        r#"{"schema_version":1,"role":"child","execution_root":"/fixture","repository":"/fixture/repository","cwd":"/fixture/repository","scratch":"/fixture/scratch","read_roots":[{"path":"/fixture/read","directory":true}],"source_writes":[],"resource_writes":[],"forbidden_roots":["/fixture/forbidden"]}"#
    );
    let mut omitted = serde_json::to_value(&plan).unwrap();
    omitted.as_object_mut().unwrap().remove("resource_writes");
    let decoded: IsolationPlan = serde_json::from_value(omitted).unwrap();
    assert!(decoded.resource_writes.is_empty());
    assert_eq!(serde_json::to_string(&decoded).unwrap(), encoded);
    for (field, value) in [
        ("authority", serde_json::json!(true)),
        ("managed_execution", serde_json::Value::Null),
        ("resource_writes", serde_json::Value::Null),
        ("role", serde_json::json!("unknown")),
    ] {
        let mut invalid = serde_json::to_value(&plan).unwrap();
        invalid[field] = value;
        assert!(serde_json::from_value::<IsolationPlan>(invalid).is_err());
    }
    let mut invalid = serde_json::to_value(&plan).unwrap();
    invalid["read_roots"][0]["authority"] = serde_json::json!(true);
    assert!(serde_json::from_value::<IsolationPlan>(invalid).is_err());
    let mut missing = serde_json::to_value(&plan).unwrap();
    missing.as_object_mut().unwrap().remove("schema_version");
    assert!(serde_json::from_value::<IsolationPlan>(missing).is_err());
    // Deserialization remains a data operation; paths need not exist until validation.
    assert!(serde_json::from_str::<IsolationPlan>(&encoded).is_ok());
}
