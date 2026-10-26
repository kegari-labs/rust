#![cfg(target_os = "macos")]
//! Actual owned Git children with an explicitly synthetic boot identity; no
//! native host qualification, credential access or model/provider invocation.
#[path = "support/git_reuse.rs"]
mod git_reuse;
#[path = "support/pinned_git.rs"]
mod pinned_git;
#[path = "support/process_probe.rs"]
mod process_probe;
use kegari_v0::{
    api::CommandRequest,
    app::{ConfigurationControl, HostFuture},
    config::Config,
    control::{Actor, Intent, Lifecycle, MutationRequest, Receipt},
    delivery::RepositoryContract,
    error::{Error, Result},
    git::{Git, GitLimits, HostGitExecutionScope, RepositoryId, configured_host_read_root},
    host_process::{HostProcessServices, LocalProcessPermit, MetadataRequest},
    platform::{BinaryIdentity, HostIdentity, ProcessProbe, Registration},
    process_records::{ProcessJournal, RecoveryState},
    providers::RepositoryIdentity,
    storage::PrivateDir,
    worker::ExecutionRegistry,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
};
use uuid::Uuid;
fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}
fn config() -> Config {
    let profile =
        json!({"tool":"codex","model":"fixture-model","reasoning":"high","speed":"standard"});
    let proof = json!({"workflow_ref":"fixture/repo/.github/workflows/caller.yml@refs/heads/trusted","workflow_sha":"1111111111111111111111111111111111111111","job_workflow_ref":"fixture/trusted/.github/workflows/proof.yml@1111111111111111111111111111111111111111","job_workflow_sha":"1111111111111111111111111111111111111111","event_name":"workflow_dispatch"});
    let exact = json!({"workflow_id":3,"workflow_revision":"1111111111111111111111111111111111111111","source_input":"source_sha","operation_input":"operation_id","observed_source_output":"source_sha","proof":proof});
    serde_json::from_value(json!({"schema_version":1,"runtime":{"id":id(1),"host_id":"fixture","max_agents":4},"connections":{
        "linear":{"kind":"linear","account_id":id(5),"endpoint":"https://linear.invalid/graphql","credential_ref":"keychain:linear"},
        "github":{"kind":"github","account_id":"7","endpoint":"https://github.invalid/","credential_ref":"keychain:github"},
        "codex":{"kind":"codex","account_id":"codex-fixture","auth_home":"/private/tmp/fixture-auth","max_agents":2}
    },"workspaces":{"example":{"id":id(2),"linear_workspace_id":id(3),"linear_connection":"linear","github_connection":"github","codex_connection":"codex","workflow":"/private/tmp/WORKFLOW.md",
        "linear":{"teams":"all","projects":"all","users":"all","authorization_label_id":id(4),"states":{id(10).to_string():{"prepared":id(11),"running":id(12),"blocked":id(13),"child_done":id(14),"parent_accepted":id(15),"cancelled":id(16)}}},
        "github":{"organizations":[],"repositories":["42"],"pr_authors":["7"]},"actors":{"authorizers":[id(5)],"correctors":[id(5)],"acceptors":[id(5)]},"profiles":{"coordination":profile,"independent_review":profile},
        "routing_labels":{"tool":{id(20).to_string():"codex"},"model":{id(21).to_string():"fixture-model"},"reasoning":{id(22).to_string():"high"},"speed":{id(23).to_string():"standard"}},"max_agents":2,"release":{"mode":"manual","spacing_seconds":0},
        "repositories":{"42":{"source":"/private/tmp/source","local_checks":["local"],"integrated_checks":["e2e"],"ci":[{"workflow_id":1,"required_checks":["suite"],"proof":proof}],"production_ci":[{"workflow_id":2,"required_checks":["suite"],"proof":proof}],"no_change_validation":exact,"deployment":"none"}}
    }},"control":{"local_users":{"501":{"id":"operator","linear_user_id":id(5),"roles":["inspect","configure","lifecycle","correct","accept","release","recover"]}}}})).unwrap()
}
struct ReadConfiguration(Mutex<Config>);
impl ConfigurationControl for ReadConfiguration {
    fn authority_fences(&self) -> Result<kegari_v0::configuration::AuthorityFences> {
        Ok(Default::default())
    }
    fn snapshot(&self) -> Result<Config> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn execute(
        &self,
        _: Actor,
        _: CommandRequest,
        _: Option<kegari_v0::secrets::Secret>,
        _: Receipt,
    ) -> HostFuture<'_, Receipt> {
        Box::pin(async { Err(Error::new("fixture", "unexpected configuration mutation")) })
    }
    fn check(&self, _: Option<Uuid>) -> HostFuture<'_, Value> {
        Box::pin(async { Ok(json!({"fixture":"metadata_only"})) })
    }
}
fn git_run(binary: &Path, root: &Path, args: &[&str]) {
    let output = Command::new(binary)
        .current_dir(root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Disposable Fixture Human")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.test")
        .env("GIT_COMMITTER_NAME", "Disposable Fixture Human")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.test")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
fn stopped_metadata_uses_actual_owned_git_guards_and_refuses_force_and_root_changes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let git_binary = pinned_git::prepare(&root);
    let source = PrivateDir::create(&root.join("source")).unwrap();
    git_run(
        &git_binary,
        source.path(),
        &["init", "--initial-branch=main"],
    );
    std::fs::write(source.path().join("file.txt"), "fixture source\n").unwrap();
    git_run(&git_binary, source.path(), &["add", "file.txt"]);
    git_run(
        &git_binary,
        source.path(),
        &[
            "commit",
            "--signoff",
            "-m",
            "feat: initialize disposable fixture",
        ],
    );
    let probe = Arc::new(process_probe::FixtureBootProbe::new());
    let current = probe.current().unwrap();
    let host = HostIdentity {
        host_id: "fixture".into(),
        boot_id: current.boot_id,
        uid: current.uid,
        user: "fixture".into(),
        home: root.clone(),
    };
    let (registration, control) = Registration::bootstrap_with_probe(
        &host,
        id(1),
        BinaryIdentity::inspect(&std::env::current_exe().unwrap(), false).unwrap(),
        Uuid::new_v4(),
        probe.clone(),
    )
    .unwrap();
    let mut config = config();
    config.runtime.execution_root = root.join("executions").to_string_lossy().into_owned();
    config
        .workspaces
        .get_mut("example")
        .unwrap()
        .repositories
        .get_mut("42")
        .unwrap()
        .source = source.path().to_string_lossy().into_owned();
    registration.complete_setup(&config).unwrap();
    let selected = config.workspaces["example"].repositories["42"].clone();
    let contract = RepositoryContract {
        configuration_name: "42".into(),
        repository: RepositoryId {
            hostname: "github.com".into(),
            id: "42".into(),
        },
        provider: RepositoryIdentity {
            id: 42,
            owner: "fixture".into(),
            name: "repo".into(),
            owner_id: 7,
            organization_id: None,
        },
        configuration: selected,
    };
    let configuration = Arc::new(ReadConfiguration(Mutex::new(config.clone())));
    let control = Arc::new(control);
    let registration = Arc::new(registration);
    let processes = Arc::new(
        ProcessJournal::with_probe(
            registration.root().child("worker-processes").unwrap(),
            id(1),
            probe,
        )
        .unwrap(),
    );
    let executions = Arc::new(ExecutionRegistry::default());
    let services = HostProcessServices {
        control: control.clone(),
        registration,
        configuration: configuration.clone(),
        processes: processes.clone(),
        executions: executions.clone(),
    };
    let make = || {
        LocalProcessPermit::metadata(
            services.clone(),
            MetadataRequest {
                request_id: Uuid::new_v4(),
                work_key: "fixture-canonical-work".into(),
                roots: vec![configured_host_read_root(&config, id(2), &contract).unwrap()],
                git_binary: BinaryIdentity::inspect(&git_binary, true).unwrap(),
            },
        )
        .unwrap()
    };
    let authority = make();
    let operation = authority.identity().operation_id;
    let git = Git::from_pinned_executable(&git_binary)
        .unwrap()
        .with_host_execution_scope(HostGitExecutionScope {
            limits: GitLimits::default(),
            authority: authority.clone(),
        })
        .unwrap();
    let revision = git.revision(source.path(), "HEAD").unwrap();
    assert_eq!(revision.commit.len(), 40);
    let destination = root.join("forbidden-clone-root");
    let clone = kegari_v0::git::IndependentCloneIdentity {
        schema_version: 1,
        runtime_id: id(1),
        workspace_id: id(2),
        directory_id: id(90),
        repository: contract.repository.clone(),
        pull_request: 1,
        head: revision.clone(),
        base: revision.clone(),
        review_identity: kegari_v0::digest::bytes_digest(b"fixture-review"),
        repository_path: destination.join("repo"),
        operator_checkout: source.path().to_owned(),
    };
    assert_eq!(
        git.prepare_independent_review_clone(source.path(), &destination, &clone)
            .unwrap_err()
            .code,
        "scope_violation"
    );
    assert!(
        !destination.exists(),
        "metadata denial must precede all filesystem mutation"
    );

    git.verify_execution_tree(source.path(), &revision, &Default::default())
        .unwrap();
    assert_eq!(control.snapshot().unwrap().intent.desired, Intent::Stopped);
    assert!(control.snapshot().unwrap().active_operations.is_empty());
    assert_eq!(
        control.snapshot().unwrap().pending_local_processes,
        vec![operation]
    );
    assert!(
        processes
            .observe(operation)
            .unwrap()
            .iter()
            .all(|p| matches!(p.state, RecoveryState::OwnedGroupExitRecorded)
                && p.owned_group_exited
                && !p.all_descendants_exited
                && p.binding.lease.is_none()
                && p.binding.control_owner.is_some())
    );
    authority
        .finish(json!({"actual_revision":revision}))
        .unwrap();
    assert!(
        control
            .snapshot()
            .unwrap()
            .pending_local_processes
            .is_empty()
    );
    assert!(executions.active_operations().unwrap().is_empty());
    assert!(
        git.revision(source.path(), "HEAD").is_err(),
        "finished authority cannot start another read"
    );
    assert!(
        !control
            .snapshot()
            .unwrap()
            .fenced_operations
            .contains(&operation)
    );
    let authority = make();
    let operation = authority.identity().operation_id;
    let git = Git::from_pinned_executable(&git_binary)
        .unwrap()
        .with_host_execution_scope(HostGitExecutionScope {
            limits: GitLimits::default(),
            authority: authority.clone(),
        })
        .unwrap();
    control
        .lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: id(1),
                actor: Actor {
                    identity: "fixture-operator".into(),
                    roles: ["lifecycle".into()].into(),
                    principal: None,
                },
                kind: "force".into(),
                canonical_targets: vec![id(1).to_string()],
                expected: BTreeMap::new(),
                payload: json!({}),
            },
            Lifecycle::ForceStop,
            0,
        )
        .unwrap();
    let count = processes.observe(operation).unwrap().len();
    assert!(git.revision(source.path(), "HEAD").is_err());
    assert_eq!(processes.observe(operation).unwrap().len(), count);
    authority
        .finish(json!({"cancelled_before_next_read":true}))
        .unwrap();
    let authority = make();
    let git = Git::from_pinned_executable(&git_binary)
        .unwrap()
        .with_host_execution_scope(HostGitExecutionScope {
            limits: GitLimits::default(),
            authority: authority.clone(),
        })
        .unwrap();
    std::fs::rename(source.path(), root.join("prior-source")).unwrap();
    PrivateDir::create(source.path()).unwrap();
    assert!(
        git.revision(source.path(), "HEAD").is_err(),
        "replaced source inode cannot inherit read authority"
    );
    authority.finish(json!({"root_replaced":true})).unwrap();
}
