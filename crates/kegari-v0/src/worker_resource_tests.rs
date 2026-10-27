//! Actual worker/Control/journal inspection. Filesystem fixtures only; no native
//! execution, provider request or environment-admission capability is fabricated.
use super::*;
use crate::validation::{DiscoveryReadContent, DiscoveryReadReference};

fn ready(mut fixture: Fixture) -> Fixture {
    fixture
        .worker
        .bind_inspection_deadline(std::time::Instant::now() + StdDuration::from_secs(20));
    fixture
}
fn resources() -> Fixture {
    ready(Fixture::configured(
        AgentRole::Implementation,
        "",
        None,
        None,
        true,
        false,
        true,
    ))
}
async fn environment(f: &mut Fixture) -> Value {
    let call = f.call("context_read", json!({"key":"execution_environment"}));
    f.worker.call(&call).await.unwrap()
}
fn handle(env: &Value, kind: &str, name: Option<&str>) -> String {
    env["resources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["kind"] == kind && name.is_none_or(|n| v["name"] == n))
        .unwrap()["handle"]
        .as_str()
        .unwrap()
        .into()
}
async fn inspect(f: &mut Fixture, tool: &str, handle: &str, path: &str) -> Result<Value> {
    let call = f.call(tool, json!({"handle":handle,"path":path}));
    f.worker.call(&call).await
}
fn latest(f: &Fixture) -> DiscoveryReadReference {
    serde_json::from_value(f.worker.validation_evidence().unwrap()["reads"][0]["reference"].clone())
        .unwrap()
}
async fn propose(f: &mut Fixture, reference: &DiscoveryReadReference) -> Result<Value> {
    let call=f.call("validation_propose",json!({"previous":null,"reason":"actual inspected resource establishes a validation obligation","supporting_reads":[reference.digest().unwrap()],
        "requirements":[{"id":"observed-behavior","statement":"validate the requested behavior","rationale":"actual generic resource inspection","argv":["/usr/bin/true"],"cwd":".","services":[],"artifacts":[]}]}));
    f.worker.call(&call).await
}
#[tokio::test]
async fn actual_resources_are_listed_without_probes_and_settle_exact_nonempty_and_empty_results() {
    let mut f = resources();
    let env = environment(&mut f).await;
    assert_eq!(env["resources"].as_array().unwrap().len(), 3);
    assert!(
        env["resources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.get("root").is_none())
    );
    assert_eq!(env["environment"]["available"], true);
    assert_eq!(
        env["environment"]["observation"]["native_prerequisite"]["code"],
        "managed_profile_unqualified"
    );
    let data = handle(&env, "test_resource", Some("data"));
    let listing = inspect(&mut f, "resource_list", &data, ".").await.unwrap();
    assert_eq!(listing["complete"], true);
    assert_eq!(listing["entries"][0]["name"], "custom.rules");
    assert_eq!(listing["entries"][1]["name"], "empty");
    assert!(matches!(
        latest(&f).content,
        DiscoveryReadContent::Directory { .. }
    ));
    let empty = inspect(&mut f, "resource_list", &data, "empty")
        .await
        .unwrap();
    assert_eq!(empty["complete"], true);
    assert_eq!(empty["entries"], json!([]));
    let recipe = handle(&env, "test_resource", Some("recipe"));
    let value = inspect(&mut f, "resource_read", &recipe, ".")
        .await
        .unwrap();
    assert_eq!(value["content"], "custom build instructions\n");
    let reference = latest(&f);
    assert!(matches!(&reference.content,DiscoveryReadContent::Resource{path,..} if path=="."));
    let finished: Value = f.worker.journal.dir.read(&reference.journal.leaf).unwrap();
    assert_eq!(finished["kind"], "tool_finished");
    assert_eq!(
        reference.result_digest,
        canonical_digest(1, &value).unwrap()
    );
    propose(&mut f, &reference).await.unwrap();
    assert!(f.boundary.last_cancel.lock().unwrap().is_none());
    assert!(f.worker.take_validation_plan().is_some());
}
#[tokio::test]
async fn exact_handles_paths_and_file_grants_refuse_widening_links_and_git() {
    let mut f = resources();
    let env = environment(&mut f).await;
    let recipe = handle(&env, "test_resource", Some("recipe"));
    for path in [
        "../unfamiliar-resource/custom.rules",
        "custom.rules",
        "./",
        "/etc/passwd",
    ] {
        assert!(
            inspect(&mut f, "resource_read", &recipe, path)
                .await
                .is_err()
        );
    }
    assert!(
        inspect(&mut f, "resource_list", &recipe, ".")
            .await
            .is_err()
    );
    assert!(
        inspect(&mut f, "resource_read", "/usr/bin", ".")
            .await
            .is_err()
    );
    let repo = handle(&env, "primary_repository", None);
    for path in [
        ".git/config",
        ".GIT/config",
        "../auth",
        "a.txt/..",
        "./a.txt",
        "a.txt//x",
    ] {
        assert!(inspect(&mut f, "resource_read", &repo, path).await.is_err());
    }
    let root = f.input.isolation.plan().repository.clone();
    std::os::unix::fs::symlink("a.txt", root.join("alias")).unwrap();
    std::os::unix::fs::symlink(
        &f.input.isolation.plan().forbidden_roots[0],
        root.join("outside"),
    )
    .unwrap();
    let fifo = std::ffi::CString::new(root.join("pipe").to_str().unwrap()).unwrap();
    // Ordinary owned FIFO only, with no writer or spawned process. Both public
    // readers must reach fstat/type refusal without waiting for a counterpart.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let source_fifo = f.call("source_read", json!({"path":"pipe"}));
    assert_eq!(
        f.worker.call(&source_fifo).await.unwrap_err().code,
        "tool_limit"
    );
    // Preserve the source reader's independent hard-link guard: install that
    // adversarial input only after the FIFO reaches its own file-type check.
    std::fs::hard_link(root.join("a.txt"), root.join("hard")).unwrap();
    for path in ["alias", "outside/WORKFLOW.md", "hard", "pipe"] {
        assert!(inspect(&mut f, "resource_read", &repo, path).await.is_err());
    }
    let listing = inspect(&mut f, "resource_list", &repo, ".").await.unwrap();
    assert!(
        listing["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["name"] != ".git")
    );
    assert!(f.boundary.last_cancel.lock().unwrap().is_none());
}
#[tokio::test]
async fn resource_and_listing_changes_refuse_proposal_and_failed_reply_is_not_evidence() {
    let mut f = ready(Fixture::without_checks(AgentRole::Implementation));
    let env = environment(&mut f).await;
    let repo = handle(&env, "primary_repository", None);
    inspect(&mut f, "resource_read", &repo, "a.txt")
        .await
        .unwrap();
    let original = latest(&f);
    std::fs::write(
        f.input.isolation.plan().repository.join("a.txt"),
        b"changed\n",
    )
    .unwrap();
    assert!(propose(&mut f, &original).await.is_err());
    std::fs::write(
        f.input.isolation.plan().repository.join("a.txt"),
        b"original\n",
    )
    .unwrap();
    inspect(&mut f, "resource_list", &repo, ".").await.unwrap();
    let original = latest(&f);
    std::fs::write(
        f.input.isolation.plan().repository.join("new-unlisted"),
        b"new",
    )
    .unwrap();
    assert!(propose(&mut f, &original).await.is_err());
    let count = f.worker.validation_evidence().unwrap()["total_reads"].clone();
    let next = *f.worker.journal.next.lock().unwrap();
    std::fs::create_dir(
        f.worker
            .journal
            .dir
            .path()
            .join(format!("event-{:08}.json", next + 1)),
    )
    .unwrap();
    assert!(
        inspect(&mut f, "resource_read", &repo, "a.txt")
            .await
            .is_err()
    );
    assert_eq!(
        f.worker.validation_evidence().unwrap()["total_reads"],
        count
    );
    assert!(f.worker.take_validation_plan().is_none());
}
#[tokio::test]
async fn oversized_listing_escaped_read_and_shared_budget_never_yield_partial_facts() {
    let mut f = ready(Fixture::without_checks(AgentRole::Implementation));
    let env = environment(&mut f).await;
    let repo = handle(&env, "primary_repository", None);
    let root = f.input.isolation.plan().repository.clone();
    std::fs::create_dir(root.join("large")).unwrap();
    for i in 0..257 {
        std::fs::write(root.join("large").join(i.to_string()), b"").unwrap();
    }
    let count = f.worker.validation_evidence().unwrap()["total_reads"].clone();
    assert_eq!(
        inspect(&mut f, "resource_list", &repo, "large")
            .await
            .unwrap_err()
            .code,
        "tool_limit"
    );
    std::fs::write(root.join("escaped"), vec![b'\t'; 48000]).unwrap();
    assert_eq!(
        inspect(&mut f, "resource_read", &repo, "escaped")
            .await
            .unwrap_err()
            .code,
        "tool_limit"
    );
    assert_eq!(
        f.worker.validation_evidence().unwrap()["total_reads"],
        count
    );
    // Failure reservations are never refunded; no sleeps or actual processes.
    for _ in 0..180 {
        assert!(
            inspect(&mut f, "resource_read", &repo, "escaped")
                .await
                .is_err()
        );
    }
    assert_eq!(
        inspect(&mut f, "resource_read", &repo, "a.txt")
            .await
            .unwrap_err()
            .code,
        "tool_limit"
    );
}
#[tokio::test]
async fn original_cutoff_cannot_be_renewed_and_readonly_roles_never_gain_commands() {
    let mut f = Fixture::without_checks(AgentRole::Implementation);
    let call = f.call("context_read", json!({"key":"execution_environment"}));
    assert_eq!(
        f.worker.call(&call).await.unwrap_err().code,
        "inspection_unavailable"
    );
    f.worker.bind_inspection_deadline(std::time::Instant::now());
    f.worker
        .bind_inspection_deadline(std::time::Instant::now() + StdDuration::from_secs(60));
    let call = f.call("context_read", json!({"key":"execution_environment"}));
    assert_eq!(f.worker.call(&call).await.unwrap_err().code, "timeout");
    for role in [AgentRole::Review, AgentRole::IndependentReview] {
        let mut f = ready(Fixture::new(role));
        let env = environment(&mut f).await;
        let repo = handle(&env, "primary_repository", None);
        let result = inspect(&mut f, "resource_read", &repo, "a.txt")
            .await
            .unwrap();
        assert_eq!(result["content"], "original\n");
        assert_eq!(f.worker.validation_evidence().unwrap()["total_reads"], 0);
        let call = f.call("command_run", json!({"argv":["/usr/bin/true"],"cwd":"."}));
        assert!(f.worker.call(&call).await.is_err());
        if role == AgentRole::IndependentReview {
            assert_eq!(env["environment"]["available"], false);
        }
    }
}

#[tokio::test]
async fn actual_revocation_after_bytes_or_environment_observation_prevents_settled_evidence() {
    for environment_call in [false, true] {
        let mut f = ready(Fixture::without_checks(AgentRole::Implementation));
        let env = environment(&mut f).await;
        let repo = handle(&env, "primary_repository", None);
        let count = f.worker.validation_evidence().unwrap()["total_reads"].clone();
        let host = f.host.clone();
        f.worker.inspection_observed = Some(Arc::new(move || {
            host.revoked.store(true, Ordering::SeqCst);
        }));
        let call = if environment_call {
            f.call("context_read", json!({"key":"execution_environment"}))
        } else {
            f.call("resource_read", json!({"handle":repo,"path":"a.txt"}))
        };
        assert_eq!(
            f.worker.call(&call).await.unwrap_err().code,
            "access_denied"
        );
        assert_eq!(
            f.worker.validation_evidence().unwrap()["total_reads"],
            count
        );
        assert!(f.worker.take_validation_plan().is_none());
        assert!(f.boundary.last_cancel.lock().unwrap().is_none());
    }
}
