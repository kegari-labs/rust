#[path = "worker_bundle.rs"]
#[allow(dead_code)] // Also included by targets that exercise only ordinary checks.
mod worker_bundle;
use chrono::{Duration, Utc};
use kegari_v0::{
    authority::*,
    codex::*,
    config::{Profile, ReleaseMode, Speed},
    control::*,
    digest::*,
    error::{Error, Result},
    evaluation::TrialHost,
    evidence::{Generation, WorkIdentity},
    execution::*,
    git::{Git, RepositoryId},
    instructions::*,
    isolation::*,
    providers::ProviderBroker,
    reconcile::*,
    storage::PrivateDir,
    worker::*,
    workflow::Workflow,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration as StdDuration,
};
use tempfile::TempDir;
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}
fn profile() -> Profile {
    Profile {
        tool: "codex".into(),
        model: "fixture-model".into(),
        reasoning: "high".into(),
        speed: Speed::Standard,
    }
}
fn git_path() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("git"))
        .find(|p| p.is_file())
        .unwrap()
        .canonicalize()
        .unwrap()
}
fn git_run(path: &Path, args: &[&str]) {
    let r = kegari_v0::process_group::OwnedGroup::spawn_std(
        Command::new(git_path())
            .current_dir(path)
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
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()),
    )
    .expect("spawn owned disposable fixture")
    .0
    .wait_with_output()
    .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
}
struct Host {
    authority: Mutex<WorkerAuthority>,
    revoked: AtomicBool,
    owner: AtomicBool,
    rates: AtomicBool,
    waiting: AtomicBool,
}
impl WorkerHost for Host {
    fn observed_session(&self, _: Uuid, _: &SessionState) -> Result<()> {
        Ok(())
    }
    fn current_authority(&self, _: &WorkerAuthority) -> Result<WorkerAuthority> {
        if self.revoked.load(Ordering::SeqCst) {
            return Err(Error::new("access_denied", "fixture revoked authorization"));
        }
        if self.waiting.load(Ordering::SeqCst) {
            return Err(Error::new(
                "correction_classification_pending",
                "fixture native input pending",
            ));
        }
        Ok(self.authority.lock().unwrap().clone())
    }
    fn correction_waiting(&self, _: Uuid) -> Result<bool> {
        Ok(self.waiting.load(Ordering::SeqCst))
    }
    fn verify_owner(&self, _: &Lease) -> Result<()> {
        if !self.owner.load(Ordering::SeqCst) {
            return Err(Error::new("ownership_conflict", "fixture owner exited"));
        }
        Ok(())
    }
    fn rate_limits(&self, _: Uuid, _: &Value) -> Result<()> {
        self.rates.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn reconcile_prior(&self, _: Uuid, prior: Option<&Value>) -> Result<()> {
        if prior.is_some_and(|p| p["status"] == "unknown") {
            Err(Error::new(
                "reconciliation_required",
                "unknown provider effect",
            ))
        } else {
            Ok(())
        }
    }
    fn observed_state(&self, _: Uuid) -> Result<Value> {
        Ok(json!({"delivery":"pending"}))
    }
    fn correction_event(&self, _: Uuid, watermark: u64, _: &str) -> Result<String> {
        Ok(format!("fixture-correction:{watermark}"))
    }
}
#[derive(Default)]
struct Sink(Mutex<Vec<Value>>);
impl OutcomeSink for Sink {
    fn record(&self, outcome: &WorkerOutcome) -> Result<String> {
        self.0
            .lock()
            .unwrap()
            .push(serde_json::to_value(outcome).unwrap());
        Ok(format!("pending:{}", outcome.proposal_id))
    }
}
struct Boundary {
    fail: AtomicBool,
    block: AtomicBool,
    unknown: AtomicBool,
    last_cancel: Mutex<Option<Arc<CancelFlag>>>,
    bundle: Option<Arc<worker_bundle::BundleBoundary>>,
}
struct CancelFlag {
    cancelled: AtomicBool,
    exited: AtomicBool,
}
impl CommandCancellation for CancelFlag {
    fn cancel(&self) -> Result<()> {
        self.cancelled.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn exited(&self) -> Result<bool> {
        Ok(self.exited.load(Ordering::SeqCst))
    }
}
struct FixtureCommand {
    receipt: CommandOutcome,
    block: bool,
    cancel: Arc<CancelFlag>,
    unknown: bool,
}
impl StartedCommand for FixtureCommand {
    fn cancellation(&self) -> Arc<dyn CommandCancellation> {
        self.cancel.clone()
    }
    fn wait(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<CommandOutcome>> + Send>> {
        Box::pin(async move {
            if self.unknown {
                return Err(Error::new(
                    "operation_uncertain",
                    "fixture wait lost its exit observation",
                ));
            }
            let mut receipt = self.receipt;
            if self.block {
                while !self.cancel.cancelled.load(Ordering::SeqCst) {
                    tokio::time::sleep(StdDuration::from_millis(5)).await;
                }
                receipt.exit_code = Some(130);
            }
            self.cancel.exited.store(true, Ordering::SeqCst);
            Ok(receipt)
        })
    }
}
impl CommandBoundary for Boundary {
    fn evidence_class(&self) -> ExecutionEvidenceClass {
        ExecutionEvidenceClass::DeterministicFixture
    }
    fn start_bundle_member(
        &self,
        isolation: &ValidatedIsolation,
        argv: &[String],
        limits: BundleCommandLimits,
        launch: CheckBundleLaunch,
    ) -> Result<Box<dyn StartedCommand>> {
        self.bundle
            .as_ref()
            .ok_or_else(|| Error::new("unsupported_check_bundle", "fixture has no bundle"))?
            .start_bundle_member(isolation, argv, limits, launch)
    }
    fn start(
        &self,
        b: &ValidatedIsolation,
        argv: &[String],
        _: StdDuration,
        _: usize,
    ) -> Result<Box<dyn StartedCommand>> {
        std::fs::write(
            b.plan().scratch.join("check.json"),
            format!("{{\"fixture\":\"{}\"}}", Uuid::new_v4()),
        )
        .unwrap();
        let cancel = Arc::new(CancelFlag {
            cancelled: AtomicBool::new(false),
            exited: AtomicBool::new(false),
        });
        *self.last_cancel.lock().unwrap() = Some(cancel.clone());
        Ok(Box::new(FixtureCommand {
            receipt: CommandOutcome {
                schema_version: 1,
                elapsed_ms: 1,
                argv: argv.to_vec(),
                cwd: b.plan().cwd.clone(),
                exit_code: Some(if self.fail.load(Ordering::SeqCst) {
                    1
                } else {
                    0
                }),
                termination_signal: None,
                timed_out: false,
                stdout: "deterministic fixture".into(),
                stderr: String::new(),
                output_truncated: false,
                isolation_digest: b.digest().into(),
            },
            block: self.block.load(Ordering::SeqCst),
            cancel,
            unknown: self.unknown.load(Ordering::SeqCst),
        }))
    }
}
struct Fixture {
    _temp: TempDir,
    worker: ScopedWorker,
    contract: Arc<WorkerContract>,
    host: Arc<Host>,
    sink: Arc<Sink>,
    control: Arc<ControlHost>,
    state: SessionState,
    boundary: Arc<Boundary>,
    input: ContractInput,
    executions: Arc<ExecutionRegistry>,
    parent: ParentProjection,
    permit: Permit,
    execution: Option<Arc<WorkerExecution>>,
}
type ConfiguredContract =
    fn(ContractInput, &kegari_v0::config::Workspace) -> Result<WorkerContract>;
impl Fixture {
    fn new(role: AgentRole) -> Self {
        Self::with_hooks(role, "")
    }
    fn with_hooks(role: AgentRole, hooks: &str) -> Self {
        Self::configured(role, hooks, None, None, false)
    }
    #[allow(dead_code)] // Shared prelude also serves the unchanged worker target.
    fn with_bundle(mode: worker_bundle::Mode, configured: ConfiguredContract) -> Self {
        Self::configured(
            AgentRole::Implementation,
            "",
            Some(mode),
            Some(configured),
            false,
        )
    }
    #[allow(dead_code)]
    fn without_checks(role: AgentRole) -> Self {
        Self::configured(role, "", None, None, true)
    }
    fn configured(
        role: AgentRole,
        hooks: &str,
        bundle: Option<worker_bundle::Mode>,
        configured: Option<ConfiguredContract>,
        without_checks: bool,
    ) -> Self {
        let temp = TempDir::new().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let source = base.join("operator");
        let run = base.join("execution");
        let repo = run.join("repository");
        let scratch = run.join("scratch");
        let private = base.join("control");
        let auth = base.join("auth");
        for p in [&source, &run, &scratch] {
            std::fs::create_dir(p).unwrap();
        }
        PrivateDir::create(&auth).unwrap();
        git_run(&source, &["init", "-b", "main"]);
        std::fs::write(source.join("a.txt"), "original\n").unwrap();
        let main = if bundle.is_some() { ", \"check\"" } else { "" };
        let services = if bundle.is_some() {
            "      services:\n        - name: database\n          argv: [\"/usr/bin/true\", \"service\"]\n          readiness:\n            argv: [\"/usr/bin/true\", \"readiness\"]\n            timeout_ms: 3000\n          max_lifetime_ms: 10000\n"
        } else {
            ""
        };
        // The configured workspace requires an integrated obligation too. It is
        // declared for config/workflow fidelity but is not run by these local
        // bundle tests; ordinary fixtures retain their exact previous workflow.
        let integrated = if bundle.is_some() {
            format!(
                "  integrated:\n    e2e:\n      argv: [\"/usr/bin/true\"]\n      result_artifact: \"{}/integrated.json\"\n",
                scratch.display()
            )
        } else {
            String::new()
        };
        let workflow_text = if without_checks {
            format!(
                "---\nschema_version: 1\n{hooks}checks: {{}}\n---\nOutcome: {{{{ issue | json }}}}\n"
            )
        } else {
            format!(
                "---\nschema_version: 1\n{hooks}checks:\n  local:\n    suite:\n      argv: [\"/usr/bin/true\"{main}]\n      result_artifact: \"{}/check.json\"\n{services}{integrated}---\nOutcome: {{{{ issue | json }}}}\n",
                scratch.display()
            )
        };
        std::fs::write(source.join("WORKFLOW.md"), workflow_text).unwrap();
        git_run(&source, &["add", "."]);
        git_run(
            &source,
            &[
                "commit",
                "--signoff",
                "-m",
                "test: create disposable fixture",
            ],
        );
        git_run(
            &run,
            &[
                "clone",
                "--local",
                "--no-hardlinks",
                source.to_str().unwrap(),
                repo.to_str().unwrap(),
            ],
        );
        let workflow = Workflow::load(&repo.join("WORKFLOW.md")).unwrap();
        let pinned_git_path = pinned_git::prepare(temp.path());
        let git = Arc::new(Git::new(&pinned_git_path).unwrap());
        let revision = git.revision(&repo, "HEAD").unwrap();
        let child = role.permits_source_writes();
        let configured_workspace = bundle.map(|_| {
            worker_bundle::configuration(
                &auth,
                workflow.path.as_deref().unwrap(),
                &source,
                profile(),
            )
        });
        let work = WorkIdentity {
            linear_endpoint: "https://linear.invalid/graphql".into(),
            linear_workspace: id(2),
            parent: id(10),
            child: child.then_some(id(11)),
            generation: Generation {
                authorization: bytes_digest(b"a"),
                definition: bytes_digest(b"d"),
                correction: bytes_digest(b"c"),
                scope: bytes_digest(b"s"),
            },
        };
        let scope = ChildScope {
            schema_version: 1,
            kind: "child".into(),
            responsibility: match role {
                AgentRole::Testing => Responsibility::Testing,
                AgentRole::Documentation => Responsibility::Documentation,
                _ => Responsibility::Implementation,
            },
            write_repository: Some("42".into()),
            write_paths: vec!["a.txt".into()],
            read_repositories: vec![],
            test_resources: vec![],
        };
        let parent = ParentProjection {
            workspace_id: id(3),
            workspace_name: "fixture".into(),
            parent_reference: "fixture/10".into(),
            key: IssueKey::new("https://linear.invalid/graphql", id(2), id(10)).unwrap(),
            issue: None,
            included: true,
            observed_at: Utc::now(),
            required_children: BTreeSet::from([id(11)]),
            children: BTreeMap::from([(
                id(11),
                ChildProjection {
                    parent_reference: "fixture/10".into(),
                    key: IssueKey::new("https://linear.invalid/graphql", id(2), id(11)).unwrap(),
                    issue: None,
                    scope: Some(scope),
                    profile: Some(profile()),
                    work: Some(work.clone()),
                    blockers: vec![],
                    prerequisites: BTreeSet::new(),
                },
            )]),
            observations: BTreeMap::new(),
            metadata: None,
            authorization_candidate: None,
            authorization_published: true,
            corrections: Corrections::default(),
            observation_revision: 1,
            disappeared_children: BTreeMap::new(),
            work: Some(work.clone()),
            blockers: vec![],
            prerequisites: BTreeSet::new(),
            source_digest: bytes_digest(b"sources"),
            config_revision: configured_workspace.as_ref().map_or_else(
                || bytes_digest(b"config"),
                |config| config.revision().unwrap(),
            ),
            drain_config_revision: bytes_digest(b"config-drain"),
            workflow_digest: workflow.digest.clone(),
            coordination_profile: profile(),
            release_mode: ReleaseMode::Manual,
        };
        let mut authority = WorkerAuthority::Authorized(
            parent
                .fresh_gate(
                    work.child,
                    if child {
                        GatePurpose::NewOperation
                    } else {
                        GatePurpose::Coordination
                    },
                    &DependencyOutcomes::default(),
                    Utc::now(),
                    Duration::seconds(30),
                )
                .unwrap(),
        );
        let mut review = None;
        if role == AgentRole::IndependentReview {
            use kegari_v0::providers::{PullRequest, Repository, RepositoryIdentity};
            let repository = RepositoryIdentity {
                id: 42,
                owner: "fixture".into(),
                name: "repo".into(),
                owner_id: 7,
                organization_id: None,
            };
            let pr = PullRequest {
                repository_id: 42,
                id: 40,
                number: 1,
                author_id: 7,
                state: "open".into(),
                draft: false,
                head_sha: revision.commit.clone(),
                base_sha: revision.commit.clone(),
                head_ref: "feature".into(),
                base_ref: "main".into(),
                head_repository_id: Some(42),
                merged: Some(false),
                merge_commit_sha: None,
                url: "https://github.invalid/fixture/repo/pull/1".into(),
            };
            let projection = RuntimeProjection {
                runtime_id: id(1),
                config_revision: bytes_digest(b"config"),
                observed_at: Utc::now(),
                workspaces: BTreeMap::from([(
                    id(3),
                    WorkspaceProjection {
                        workspace_id: id(3),
                        workspace_name: "fixture".into(),
                        complete: true,
                        observed_at: Utc::now(),
                        parents: BTreeMap::new(),
                        repositories: BTreeMap::from([(
                            42,
                            Repository {
                                identity: repository.clone(),
                                default_branch: "main".into(),
                                private: true,
                                archived: false,
                                disabled: false,
                                squash_allowed: Some(true),
                                permissions: json!({"pull":true}),
                            },
                        )]),
                        independent_jobs: vec![IndependentJob {
                            workspace_id: id(3),
                            repository_id: 42,
                            pull_request: pr,
                            pull_request_text: Some(kegari_v0::providers::PullRequestText {
                                title: "Fixture independent review".into(),
                                body: None,
                            }),
                            review_identity: bytes_digest(b"independent-review"),
                            phase: "review".into(),
                        }],
                        error: None,
                        config_revision: bytes_digest(b"config"),
                        workflow_digest: workflow.digest.clone(),
                        independent_review_profile: Some(profile()),
                    },
                )]),
            };
            authority = WorkerAuthority::Independent(
                projection
                    .independent_gate(id(3), 42, 1, Utc::now(), Duration::seconds(30))
                    .unwrap(),
            );
            review = Some(ReviewTarget {
                repository,
                pull_request: 1,
                head: revision.commit.clone(),
                base: revision.commit.clone(),
            });
        }
        let execution_role = if child {
            ExecutionRole::Child
        } else if role == AgentRole::Validation {
            ExecutionRole::Validation
        } else if role == AgentRole::IndependentReview {
            ExecutionRole::IndependentReview
        } else if role == AgentRole::Review {
            ExecutionRole::Review
        } else {
            ExecutionRole::Coordination
        };
        let mut plan = IsolationPlan {
            managed_execution: None,
            schema_version: 1,
            role: execution_role,
            execution_root: run.clone(),
            repository: repo.clone(),
            cwd: repo.clone(),
            scratch: scratch.clone(),
            read_roots: vec![PathGrant {
                path: repo.clone(),
                directory: true,
            }],
            resource_writes: vec![],
            source_writes: if child {
                vec![PathGrant {
                    path: repo.join("a.txt"),
                    directory: false,
                }]
            } else {
                vec![]
            },
            forbidden_roots: vec![source.clone(), private.clone(), auth.clone()],
        };
        if role.permits_repository_execution() {
            plan.managed_execution = Some(BoundManagedExecution::automatic(&plan).unwrap());
        }
        let isolation = plan.validate().unwrap();
        let context = json!({"issue":{"body":"fixture"},"parent":{},"children":[{"id":id(11)}],"workspace":{"id":id(3)},"scope":{},"dependencies":[],"corrections":[],"evidence":{},"attempt":1,"phase":"work","profile":profile(),"operation":{"id":id(4)}});
        let work_key = authority.work_key().unwrap();
        let generation = match &authority {
            WorkerAuthority::Authorized(_) => work.digest().unwrap(),
            WorkerAuthority::Independent(g) => g.review_identity().into(),
        };
        let assembled = assemble(AssemblyInput {
            workflow: &workflow,
            role,
            phase: "work",
            context: &context,
            workspace_id: id(3),
            operation_id: id(4),
            work_generation: &generation,
            correction_revision: &work.generation.correction,
            profile: &profile(),
            cwd: &repo,
            codex_home: &auth,
            codex_version: kegari_v0::instructions::SUPPORTED_CODEX_VERSION,
            repository: Some(RepositoryContext {
                root: repo.clone(),
                revision: revision.commit.clone(),
                scope_paths: vec!["a.txt".into()],
            }),
            required_context: &[],
            project_doc_max_bytes: 32768,
            permissions: PermissionManifest {
                read_roots: vec![repo.clone()],
                resource_write_paths: vec![],
                source_write_paths: if child {
                    vec![repo.join("a.txt")]
                } else {
                    vec![]
                },
                scratch_roots: vec![scratch],
                repository_code_execution: role.permits_repository_execution(),
                enforcement_profile: isolation.digest().into(),
                network: WorkerNetwork::BrokerOnly,
            },
            tools: tool_definitions(role, child)
                .into_iter()
                .map(|(e, _)| e)
                .collect(),
            provider_discovery: ProviderDiscovery::Unavailable {
                reason: "fixture no live Codex".into(),
            },
            known_secrets: &[],
        })
        .unwrap();
        let repository = RepositoryId {
            hostname: "github.invalid".into(),
            id: "42".into(),
        };
        let input = ContractInput {
            assembled,
            authority: authority.clone(),
            isolation: isolation.clone(),
            workflow,
            context,
            review,
            account_id: "fixture-account".into(),
            repository: Some(repository.clone()),
            read_resources: BTreeMap::new(),
        };
        let contract = Arc::new(match (&configured_workspace, configured) {
            (Some(config), Some(construct)) => {
                construct(input.clone(), &config.workspaces["fixture"]).unwrap()
            }
            (None, None) => WorkerContract::new(input.clone()).unwrap(),
            _ => panic!("bundle fixture must use the original configured contract constructor"),
        });
        let dir = PrivateDir::create(&private).unwrap();
        let control = Arc::new(
            ControlHost::setup(dir.child("runtime").unwrap(), id(1), Uuid::new_v4()).unwrap(),
        );
        control
            .lifecycle(
                MutationRequest {
                    request_id: Uuid::new_v4(),
                    runtime_id: id(1),
                    actor: Actor {
                        principal: None,
                        identity: "fixture-operator".into(),
                        roles: BTreeSet::from(["lifecycle".into()]),
                    },
                    kind: "start".into(),
                    canonical_targets: vec![id(1).to_string()],
                    expected: BTreeMap::new(),
                    payload: Value::Null,
                },
                Lifecycle::Start,
                0,
            )
            .unwrap();
        let lease = Lease {
            runtime_id: id(1),
            owner_generation: 1,
            owner_id: id(5),
        };
        control.attach_reconciled(lease.clone()).unwrap();
        let permit = control
            .admit(
                &lease,
                id(4),
                work_key.clone(),
                contract.scope_digest().into(),
                || Ok(()),
            )
            .unwrap();
        let host = Arc::new(Host {
            authority: Mutex::new(authority),
            revoked: AtomicBool::new(false),
            owner: AtomicBool::new(true),
            rates: AtomicBool::new(false),
            waiting: AtomicBool::new(false),
        });
        let sink = Arc::new(Sink::default());
        let boundary = Arc::new(Boundary {
            fail: AtomicBool::new(false),
            block: AtomicBool::new(false),
            unknown: AtomicBool::new(false),
            last_cancel: Mutex::new(None),
            bundle: bundle.map(worker_bundle::BundleBoundary::new),
        });
        let execution = if role.permits_repository_execution() {
            Some(Arc::new(WorkerExecution {
                git: git.clone(),
                git_executable: pinned_git_path.clone(),
                boundary: boundary.clone(),
                ownership: Arc::new(
                    OwnedDirectory::register(
                        DirectoryIdentity {
                            schema_version: 1,
                            runtime_id: id(1),
                            directory_id: id(6),
                            repository: repository.clone(),
                            work: work.clone(),
                            repository_path: repo,
                            operator_checkout: source,
                        },
                        &isolation,
                    )
                    .unwrap(),
                ),
                binding: ExecutionBinding {
                    runtime_id: id(1),
                    operation_id: id(4),
                    work,
                    repository,
                    revision,
                    candidate_vector_digest: None,
                    isolation_digest: isolation.digest().into(),
                },
                limits: ExecutionLimits {
                    output_bytes: 32768,
                    ..Default::default()
                },
                check_outputs: BTreeMap::new(),
            }))
        } else {
            None
        };
        let executions = Arc::new(ExecutionRegistry::default());
        let mut worker = ScopedWorker::new(
            contract.clone(),
            permit.clone(),
            WorkerServices {
                control: control.clone(),
                providers: ProviderBroker::default(),
                host: host.clone(),
                sink: sink.clone(),
                execution: execution.clone(),
                journal: dir.child("worker").unwrap(),
                executions: executions.clone(),
            },
        )
        .unwrap();
        let mut state = SessionState::new(
            SessionBinding {
                schema_version: 1,
                account_id: "fixture-account".into(),
                work_identity: work_key,
                operation_id: id(4).to_string(),
                generation,
                instruction_digest: contract.instructions().manifest_digest.clone(),
                isolation_digest: isolation.digest().into(),
                profile: CodexProfile {
                    model: "fixture-model".into(),
                    reasoning: "high".into(),
                    speed: "standard".into(),
                },
                cwd: isolation.plan().cwd.clone(),
            },
            3,
        )
        .unwrap();
        state.thread_id = Some("thread".into());
        state.turn_id = Some("turn".into());
        state.turns_started = 1;
        worker.checkpoint(&state, None).unwrap();
        Self {
            _temp: temp,
            worker,
            contract,
            host,
            sink,
            control,
            state,
            boundary,
            input,
            executions,
            parent,
            permit,
            execution,
        }
    }
    fn call(&self, name: &str, args: Value) -> ToolCall {
        ToolCall {
            thread_id: "thread".into(),
            turn_id: "turn".into(),
            call_id: Uuid::new_v4().to_string(),
            name: name.into(),
            arguments: args,
        }
    }
}
