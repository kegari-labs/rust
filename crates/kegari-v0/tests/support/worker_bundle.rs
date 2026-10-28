//! Injected command outcomes only. The surrounding fixture uses the actual
//! ScopedWorker, Control and LocalExecutor; this module launches no OS process
//! and never constructs a Control launch token or native qualification proof.
use kegari_execution::ValidatedIsolation;
use kegari_types::error::{Error, Result};
use kegari_v0::{
    control::CheckBundleLaunch,
    execution::{CommandBoundary, CommandCancellation, ExecutionEvidenceClass, StartedCommand},
    execution_host::{BundleCommandLimits, CommandOutcome, ServiceObservation},
};
use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::sync::Notify;

pub(super) fn configuration(
    auth: &std::path::Path,
    workflow: &std::path::Path,
    source: &std::path::Path,
    profile: kegari_config::Profile,
) -> kegari_config::Config {
    use serde_json::json;
    use uuid::Uuid;
    let id = Uuid::from_u128;
    let proof = json!({"workflow_ref":"fixture/repo/.github/workflows/caller.yml@refs/heads/trusted","workflow_sha":"0123456789012345678901234567890123456789","job_workflow_ref":"fixture/trusted/.github/workflows/proof.yml@0123456789012345678901234567890123456789","job_workflow_sha":"0123456789012345678901234567890123456789","event_name":"workflow_dispatch"});
    let config: kegari_config::Config = serde_json::from_value(json!({
        "schema_version":1,"runtime":{"id":id(1),"host_id":"fixture-host","max_agents":2},
        "connections":{
            "linear":{"kind":"linear","account_id":"linear-fixture","endpoint":"https://linear.invalid/graphql","credential_ref":"keychain:fixture-linear"},
            "github":{"kind":"github","account_id":"github-fixture","endpoint":"https://github.invalid/api","credential_ref":"keychain:fixture-github"},
            "codex":{"kind":"codex","account_id":"fixture-account","auth_home":auth,"max_agents":2,"argv":["/usr/bin/true","app-server","--listen","stdio://"]}
        },
        "workspaces":{"fixture":{
            "id":id(3),"linear_workspace_id":id(2),"linear_connection":"linear","github_connection":"github","codex_connection":"codex","workflow":workflow,
            "linear":{"teams":"all","projects":"all","users":"all","authorization_label_id":id(20),"states":{"team":{"prepared":id(30),"running":id(31),"blocked":id(32),"child_done":id(33),"parent_accepted":id(34),"cancelled":id(35)}}},
            "github":{"repositories":["42"]},"actors":{"authorizers":[id(5)],"correctors":[id(5)],"acceptors":[id(5)]},
            "profiles":{"coordination":profile},"routing_labels":{"tool":{id(40).to_string():"codex"},"model":{id(41).to_string():"fixture-model"},"reasoning":{id(42).to_string():"high"},"speed":{id(43).to_string():"standard"}},
            "max_agents":2,"release":{"mode":"manual","spacing_seconds":0},
            "repositories":{"42":{"source":source,"local_checks":["suite"],"integrated_checks":["e2e"],"ci":[{"workflow_id":1,"required_checks":["suite"],"proof":proof}],"production_ci":[{"workflow_id":2,"required_checks":["suite"],"proof":proof}],"no_change_validation":{"workflow_id":1,"workflow_revision":"0123456789012345678901234567890123456789","source_input":"source_sha","operation_input":"operation_id","observed_source_output":"tested_sha","proof":proof},"deployment":"none"}}
        }},"control":{"local_users":{"501":{"id":"fixture-operator","linear_user_id":id(5),"roles":["inspect","configure","lifecycle","correct","accept","release","recover"]}}}
    })).unwrap();
    config.validate().unwrap();
    kegari_v0::config::load_workflows(&config).unwrap();
    config
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Success,
    ReadinessFailure,
    UnexpectedExit,
    UnknownStop,
}
#[derive(Default)]
struct State {
    events: Vec<&'static str>,
    service: Option<Weak<Service>>,
}
pub(super) struct BundleBoundary {
    mode: Mode,
    state: Arc<Mutex<State>>,
}
impl BundleBoundary {
    pub(super) fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            state: Arc::new(Mutex::new(State::default())),
        })
    }
    pub(super) fn events(&self) -> Vec<&'static str> {
        self.state.lock().unwrap().events.clone()
    }
}
#[derive(Default)]
struct ServiceState {
    stopped: bool,
    exited: bool,
    unexpected: bool,
}
struct Service {
    mode: Mode,
    state: Mutex<ServiceState>,
    events: Arc<Mutex<State>>,
    notify: Notify,
    artifact: PathBuf,
}
impl CommandCancellation for Service {
    fn cancel(&self) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        if !state.stopped {
            state.stopped = true;
            self.events.lock().unwrap().events.push("service:stop");
            if !state.unexpected && self.mode != Mode::UnknownStop {
                std::fs::write(&self.artifact, b"written-during-service-stop\n")
                    .map_err(|error| Error::io("write fixture stop artifact", error))?;
            }
            self.notify.notify_one();
        }
        if self.mode == Mode::UnknownStop {
            Err(Error::new(
                "fixture_stop_unknown",
                "injected boundary cannot establish service exit",
            ))
        } else {
            Ok(())
        }
    }
    fn exited(&self) -> Result<bool> {
        Ok(self.state.lock().unwrap().exited)
    }
    fn service_observation(&self) -> Result<ServiceObservation> {
        let state = self.state.lock().unwrap();
        Ok(ServiceObservation {
            leader_running: !state.exited,
            stop_requested: state.stopped,
            kill_escalated: false,
        })
    }
}
struct ServiceCommand {
    service: Arc<Service>,
    outcome: CommandOutcome,
}
impl StartedCommand for ServiceCommand {
    fn cancellation(&self) -> Arc<dyn CommandCancellation> {
        self.service.clone()
    }
    fn wait(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<CommandOutcome>> + Send>> {
        Box::pin(async move {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let notified = self.service.notify.notified();
                    let ended = {
                        let state = self.service.state.lock().unwrap();
                        state.stopped || state.unexpected
                    };
                    if ended {
                        break;
                    }
                    notified.await;
                }
            })
            .await
            .map_err(|_| {
                Error::new("fixture_timeout", "injected service cleanup did not arrive")
            })?;
            if self.service.mode == Mode::UnknownStop {
                self.service
                    .events
                    .lock()
                    .unwrap()
                    .events
                    .push("service:unknown");
                return Err(Error::new(
                    "fixture_stop_unknown",
                    "injected exit remains unknown",
                ));
            }
            let unexpected = {
                let mut state = self.service.state.lock().unwrap();
                state.exited = true;
                state.unexpected
            };
            self.service
                .events
                .lock()
                .unwrap()
                .events
                .push("service:joined");
            let mut outcome = self.outcome;
            if unexpected {
                outcome.exit_code = Some(7);
            }
            Ok(outcome)
        })
    }
}
struct Completed(Mutex<bool>);
impl CommandCancellation for Completed {
    fn cancel(&self) -> Result<()> {
        Ok(())
    }
    fn exited(&self) -> Result<bool> {
        Ok(*self.0.lock().unwrap())
    }
}
struct Immediate {
    cancellation: Arc<Completed>,
    outcome: CommandOutcome,
    events: Arc<Mutex<State>>,
    readiness: bool,
    mode: Mode,
}
impl StartedCommand for Immediate {
    fn cancellation(&self) -> Arc<dyn CommandCancellation> {
        self.cancellation.clone()
    }
    fn wait(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<CommandOutcome>> + Send>> {
        Box::pin(async move {
            *self.cancellation.0.lock().unwrap() = true;
            let service = {
                let mut events = self.events.lock().unwrap();
                events.events.push(if self.readiness {
                    "readiness:joined"
                } else {
                    "check:joined"
                });
                events.service.as_ref().and_then(Weak::upgrade)
            };
            if self.readiness && self.mode == Mode::UnexpectedExit {
                let service = service.unwrap();
                {
                    let mut state = service.state.lock().unwrap();
                    state.unexpected = true;
                    state.exited = true;
                }
                service.notify.notify_one();
            }
            Ok(self.outcome)
        })
    }
}
impl CommandBoundary for BundleBoundary {
    fn evidence_class(&self) -> ExecutionEvidenceClass {
        ExecutionEvidenceClass::DeterministicFixture
    }
    fn start(
        &self,
        _: &ValidatedIsolation,
        _: &[String],
        _: Duration,
        _: usize,
    ) -> Result<Box<dyn StartedCommand>> {
        Err(Error::new(
            "fixture_route",
            "bundle fixture requires actual planned launch",
        ))
    }
    fn start_bundle_member(
        &self,
        isolation: &ValidatedIsolation,
        argv: &[String],
        _: BundleCommandLimits,
        _: CheckBundleLaunch,
    ) -> Result<Box<dyn StartedCommand>> {
        if argv.len() != 2 || argv[0] != "/usr/bin/true" {
            return Err(Error::new(
                "fixture_argv",
                "unexpected declared bundle command",
            ));
        }
        let outcome = CommandOutcome {
            schema_version: 1,
            argv: argv.to_vec(),
            cwd: isolation.plan().cwd.clone(),
            isolation_digest: isolation.digest().into(),
            exit_code: Some(
                if argv[1] == "readiness" && self.mode == Mode::ReadinessFailure {
                    1
                } else {
                    0
                },
            ),
            termination_signal: None,
            timed_out: false,
            stdout: String::new(),
            stderr: String::new(),
            output_truncated: false,
            elapsed_ms: 1,
        };
        match argv[1].as_str() {
            "service" => {
                let service = Arc::new(Service {
                    mode: self.mode,
                    state: Mutex::new(ServiceState::default()),
                    events: self.state.clone(),
                    notify: Notify::new(),
                    artifact: isolation.plan().scratch.join("check.json"),
                });
                let mut state = self.state.lock().unwrap();
                assert!(
                    state.service.is_none(),
                    "original service may start only once"
                );
                state.events.push("service:start");
                state.service = Some(Arc::downgrade(&service));
                Ok(Box::new(ServiceCommand { service, outcome }))
            }
            "readiness" | "check" => {
                let readiness = argv[1] == "readiness";
                let mut state = self.state.lock().unwrap();
                assert!(state.service.is_some());
                state.events.push(if readiness {
                    "readiness:start"
                } else {
                    "check:start"
                });
                drop(state);
                if !readiness {
                    std::fs::write(
                        isolation.plan().scratch.join("check.json"),
                        b"written-during-check\n",
                    )
                    .map_err(|error| Error::io("write fixture check artifact", error))?;
                }
                Ok(Box::new(Immediate {
                    cancellation: Arc::new(Completed(Mutex::new(false))),
                    outcome,
                    events: self.state.clone(),
                    readiness,
                    mode: self.mode,
                }))
            }
            _ => Err(Error::new("fixture_argv", "unsupported declared member")),
        }
    }
}
