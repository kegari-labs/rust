//! Native control-host composition. Construction requires an actual runtime
//! backend; no permissive scheduler, credential bootstrap or client fallback.
use crate::{
    api::{ApiState, CommandRequest},
    app::{
        ConfigurationControl, HostDispatcher, HostFuture, HostServices, RuntimeInspection,
        WorkflowControl,
    },
    configuration::{ConfigManager, ConfigurationInputs, NativeConfigurationInputs},
    control::{Actor, ControlHost, Receipt},
    digest::canonical_digest,
    error::{Error, Result},
    evidence::AttestationKey,
    housekeeping::{Housekeeping, LogEvent, LogSink, Severity},
    observations::{ObservationCache, ReferenceStore},
    platform::{BinaryIdentity, HostIdentity, NativeProcessProbe, Registration},
    secrets::{Keychain, SecretStore},
    storage::PrivateDir,
    supervisor::{
        DrainState, NativeSupervisor, Supervisor, SupervisorDriver, SupervisorFuture,
        SupervisorLimits,
    },
    transport::{self, ServerHandle, TailscaleClient, TransportAuthentication},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::sync::Notify;
use uuid::Uuid;

pub struct RuntimeDependencies {
    pub configuration: Arc<dyn ConfigurationControl>,
    pub control: Arc<ControlHost>,
    pub registration: Arc<Registration>,
    pub observations: Arc<ObservationCache>,
    pub secrets: Arc<dyn SecretStore>,
    pub key: Arc<AttestationKey>,
    pub housekeeping: Arc<Housekeeping>,
    pub logs: Arc<LogSink>,
    pub inputs: Arc<dyn ConfigurationInputs>,
    pub wake: Arc<Notify>,
}
#[derive(Clone)]
pub struct RuntimeBackend {
    pub workflow: Arc<dyn WorkflowControl>,
    pub driver: Arc<dyn SupervisorDriver>,
}
pub trait HostRuntimeFactory: Send + Sync {
    fn build(&self, dependencies: RuntimeDependencies) -> HostFuture<'_, RuntimeBackend>;
}

/// Called only by the explicit native control-host process entrypoint. Merely
/// running a remote CLI command never invokes this factory or probes this host.
pub async fn run_native(factory: &dyn HostRuntimeFactory) -> Result<()> {
    let host = HostIdentity::discover()?;
    let binary = BinaryIdentity::inspect(
        &std::env::current_exe()
            .map_err(|error| Error::io("resolve installed host binary", error))?,
        true,
    )?;
    let (registration, control, config) = Registration::recover(&host, &binary)?;
    let registration = Arc::new(registration);
    let control = Arc::new(control);
    let logs = Arc::new(LogSink::new(registration.root().child("logs")?));
    let keychain = Arc::new(Keychain::new(config.runtime.id)?);
    let secrets: Arc<dyn SecretStore> = keychain.clone();
    let inputs: Arc<dyn ConfigurationInputs> = Arc::new(NativeConfigurationInputs);
    // TLS prerequisites are independently reported by the transport monitor.
    // Its initial failure cannot remove a valid private Unix control endpoint.
    let authentication = Arc::new(TransportAuthentication::new(config.control.clone(), None));
    let configuration: Arc<dyn ConfigurationControl> = Arc::new(ConfigManager::with_inputs(
        config.clone(),
        registration.clone(),
        control.clone(),
        secrets.clone(),
        authentication.clone(),
        inputs.clone(),
    )?);
    let wake = Arc::new(Notify::new());
    let pending = PendingRuntime {
        configuration: configuration.clone(),
        control: control.clone(),
        registration: registration.clone(),
        secrets,
        keychain,
        logs: logs.clone(),
        inputs,
        wake: wake.clone(),
    };
    let backend = Arc::new(SwitchableBackend::new(
        configuration.clone(),
        registration.root().child("references")?,
    ));
    let mut prerequisite: Option<HostFuture<'_, RuntimeBackend>> =
        Some(Box::pin(pending.build(factory)));
    let mut backend_retry = tokio::time::Instant::now();
    let supervisor = Supervisor::native(NativeSupervisor {
        registration: registration.clone(),
        control: control.clone(),
        host,
        binary,
        poll_ms: config.runtime.poll_ms,
        driver: backend.clone(),
        wake: wake.clone(),
        limits: SupervisorLimits {
            heartbeat: Duration::from_secs(1),
            driver_step: Duration::from_secs(30),
            shutdown: Duration::from_secs(30),
        },
    })?;
    let health = Arc::new(RwLock::new(
        json!({"local":"starting","remote":{"enabled":false},"supervisor":"starting"}),
    ));
    let workflow = Arc::new(HostedWorkflow {
        inner: backend.clone(),
        health: health.clone(),
        logs: logs.clone(),
    });
    let dispatcher = Arc::new(HostDispatcher::new(HostServices {
        control: control.clone(),
        configuration: configuration.clone(),
        workflow,
        owner: registration.clone(),
        wake,
    })?);
    let state = ApiState {
        runtime_id: config.runtime.id,
        expected_authority: "kegari.local".into(),
        authentication: authentication.clone(),
        dispatcher,
    };
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(|error| Error::io("observe host interrupt", error))?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|error| Error::io("observe host termination", error))?;
    reconcile_control_socket(&registration).await?;
    let local = transport::start_unix(registration.root(), "control.sock", state.clone()).await?;
    let mut remote = RemoteListener::new();
    remote
        .refresh(
            configuration.as_ref(),
            authentication.as_ref(),
            &control,
            &state,
        )
        .await;
    let shutdown = supervisor.shutdown_handle();
    let status = supervisor.observe();
    let mut supervisor = tokio::spawn(supervisor.run());
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    log(&logs, config.runtime.id, "control_host", "started", None);
    let mut joined = None;
    let mut failure = None;
    let monitored:Result<()> = async {loop {
        tokio::select! {
            _=interrupt.recv()=>{shutdown.request();break;},
            _=terminate.recv()=>{shutdown.request();break;},
            result=&mut supervisor=>{
                joined=Some(result.map_err(|_|Error::new("control_failed","supervisor task ended without an owned exit result")).and_then(|result|result));
                if !control.snapshot()?.service_stopping && joined.as_ref().is_some_and(Result::is_ok) {failure=Some(Error::new("control_unavailable","supervisor stopped unexpectedly"));}
                break;
            },
            result=async {match prerequisite.as_mut() {Some(build)=>build.await,None=>std::future::pending().await}}, if prerequisite.is_some()=>{
                prerequisite=None;
                backend.install(result)?;
                if backend.ready()? {shutdown.prerequisites_ready();}
                backend_retry=tokio::time::Instant::now()+Duration::from_secs(30);
            },
            _=interval.tick()=>{
                if let Err(error)=registration.verify_control_owner() {failure=Some(error);shutdown.request();break;}
                if local.is_finished() {failure=Some(Error::new("transport_unavailable","private control listener stopped"));shutdown.request();break;}
                if !backend.ready()? && prerequisite.is_none() && tokio::time::Instant::now()>=backend_retry {
                    prerequisite=Some(Box::pin(pending.build(factory)));
                }
                remote.refresh(configuration.as_ref(),authentication.as_ref(),&control,&state).await;
                *health.write().map_err(|_|Error::new("control_failed","host health is unavailable"))?=json!({"local":"listening","remote":remote.health,"supervisor":status.borrow().clone()});
                if control.snapshot()?.service_stopping {break;}
            }
        }
    } Ok(())}.await;
    if let Err(error) = monitored {
        failure = Some(error);
        shutdown.request();
    }
    // ServiceStop already durably proved STOPPED/no local ownership. Signals use
    // the distinct supervisor suspension path and preserve saved desired intent.
    if !control
        .snapshot()
        .is_ok_and(|snapshot| snapshot.service_stopping)
    {
        shutdown.request();
    }
    let local_result = local.shutdown().await;
    let remote_result = remote.shutdown().await;
    let supervisor_result = match joined {
        Some(result) => result,
        None => supervisor.await.map_err(|_| {
            Error::new(
                "control_failed",
                "supervisor task ended without an owned exit result",
            )
        })?,
    };
    let result = failure
        .map_or(Ok(()), Err)
        .and(local_result)
        .and(remote_result)
        .and(supervisor_result);
    log(
        &logs,
        config.runtime.id,
        "control_host",
        if result.is_ok() {
            "exited"
        } else {
            "interrupted_unknown"
        },
        result.as_ref().err(),
    );
    result
}
fn installed_git() -> Result<PathBuf> {
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|path| path.join("git"))
                .find(|path| path.is_file())
        })
        .ok_or_else(|| {
            Error::new(
                "prerequisite",
                "installed Git is absent from the control host PATH",
            )
        })?
        .canonicalize()
        .map_err(|error| Error::io("resolve installed Git", error))
}
struct PendingRuntime {
    configuration: Arc<dyn ConfigurationControl>,
    control: Arc<ControlHost>,
    registration: Arc<Registration>,
    secrets: Arc<dyn SecretStore>,
    keychain: Arc<Keychain>,
    logs: Arc<LogSink>,
    inputs: Arc<dyn ConfigurationInputs>,
    wake: Arc<Notify>,
}
impl PendingRuntime {
    async fn build(&self, factory: &dyn HostRuntimeFactory) -> Result<RuntimeBackend> {
        tokio::time::timeout(Duration::from_secs(35), self.build_inner(factory))
            .await
            .map_err(|_| {
                Error::new(
                    "prerequisite",
                    "runtime prerequisites exceeded their finite observation bound",
                )
            })?
    }
    async fn build_inner(&self, factory: &dyn HostRuntimeFactory) -> Result<RuntimeBackend> {
        let keychain = self.keychain.clone();
        let key = Arc::new(
            tokio::time::timeout(
                Duration::from_secs(5),
                tokio::task::spawn_blocking(move || keychain.load_attestation_key()),
            )
            .await
            .map_err(|_| {
                Error::new(
                    "missing_secret",
                    "existing attestation key lookup timed out; local control remains available",
                )
            })?
            .map_err(|_| {
                Error::new("missing_secret", "existing attestation key lookup failed")
            })??,
        );
        let observations = Arc::new(
            ObservationCache::with_inputs(
                self.configuration.clone(),
                self.secrets.clone(),
                key.clone(),
                self.registration.root().child("references")?,
                self.inputs.clone(),
                self.control.clone(),
            )
            .await?,
        );
        let housekeeping = Arc::new(Housekeeping::new(
            PrivateDir::open(self.registration.root().path())?,
            self.control.clone(),
            Arc::new(NativeProcessProbe),
            installed_git()?,
        )?);
        tokio::time::timeout(
            Duration::from_secs(30),
            factory.build(RuntimeDependencies {
                configuration: self.configuration.clone(),
                control: self.control.clone(),
                registration: self.registration.clone(),
                observations,
                secrets: self.secrets.clone(),
                key,
                housekeeping,
                logs: self.logs.clone(),
                inputs: self.inputs.clone(),
                wake: self.wake.clone(),
            }),
        )
        .await
        .map_err(|_| {
            Error::new(
                "prerequisite",
                "runtime backend construction exceeded its finite bound",
            )
        })?
    }
}
struct SwitchableBackend {
    state: RwLock<std::result::Result<RuntimeBackend, Error>>,
    configuration: Arc<dyn ConfigurationControl>,
    references: PrivateDir,
}
impl SwitchableBackend {
    fn new(configuration: Arc<dyn ConfigurationControl>, references: PrivateDir) -> Self {
        Self {
            state: RwLock::new(Err(Error::new(
                "missing_secret",
                "runtime prerequisites have not been observed",
            ))),
            configuration,
            references,
        }
    }
    fn install(&self, backend: Result<RuntimeBackend>) -> Result<()> {
        let mut state = self.state.write().map_err(|_| {
            Error::new(
                "control_failed",
                "runtime prerequisite state is unavailable",
            )
        })?;
        if state.is_ok() {
            return Err(Error::new(
                "ownership_conflict",
                "live runtime backend cannot be replaced",
            ));
        }
        *state = backend;
        Ok(())
    }
    fn backend(&self) -> Result<RuntimeBackend> {
        self.state
            .read()
            .map_err(|_| {
                Error::new(
                    "control_failed",
                    "runtime prerequisite state is unavailable",
                )
            })?
            .clone()
    }
    fn ready(&self) -> Result<bool> {
        Ok(self
            .state
            .read()
            .map_err(|_| {
                Error::new(
                    "control_failed",
                    "runtime prerequisite state is unavailable",
                )
            })?
            .is_ok())
    }
}
impl WorkflowControl for SwitchableBackend {
    fn inspect(&self) -> HostFuture<'_, RuntimeInspection> {
        Box::pin(async move {
            if let Ok(backend) = self.backend() {
                return backend.workflow.inspect().await;
            }
            let config = self.configuration.snapshot()?;
            let retained =
                ReferenceStore::new(PrivateDir::open(self.references.path())?, config.runtime.id)?
                    .restore(&config)?;
            let projection = crate::reconcile::Reconciler::default()
                .refresh(&config, &BTreeMap::new(), &BTreeMap::new(), &retained)
                .await?;
            Ok(RuntimeInspection {
                projection,
                delivery: BTreeMap::new(),
                outcomes: Default::default(),
                activities: vec![],
                history: vec![],
                release_details: BTreeMap::new(),
                release_history: vec![],
                no_release_history: vec![],
            })
        })
    }
    fn health(&self) -> Result<Value> {
        match self.backend() {
            Ok(backend) => backend.workflow.health(),
            Err(error) => Ok(
                json!({"status":"prerequisite_blocked","error":error,"retry":"read existing key and construct configured runtime only; no bootstrap or alternate account"}),
            ),
        }
    }
    fn command_targets(&self, request: &CommandRequest) -> Result<Vec<String>> {
        self.backend()?.workflow.command_targets(request)
    }
    fn execute(
        &self,
        actor: Actor,
        request: CommandRequest,
        parent: Receipt,
    ) -> HostFuture<'_, Receipt> {
        Box::pin(async move {
            self.backend()?
                .workflow
                .execute(actor, request, parent)
                .await
        })
    }
    fn cleanup_plan(&self, operation: Uuid) -> HostFuture<'_, Value> {
        Box::pin(async move { self.backend()?.workflow.cleanup_plan(operation).await })
    }
}
impl SupervisorDriver for SwitchableBackend {
    fn reconcile_before_start(&self) -> SupervisorFuture<'_, ()> {
        Box::pin(async move { self.backend()?.driver.reconcile_before_start().await })
    }
    fn recover<'a>(&'a self, lease: &'a crate::control::Lease) -> SupervisorFuture<'a, ()> {
        Box::pin(async move { self.backend()?.driver.recover(lease).await })
    }
    fn poll<'a>(
        &'a self,
        lease: &'a crate::control::Lease,
        sequence: u64,
    ) -> SupervisorFuture<'a, ()> {
        Box::pin(async move { self.backend()?.driver.poll(lease, sequence).await })
    }
    fn maintain<'a>(
        &'a self,
        lease: &'a crate::control::Lease,
        snapshot: &'a crate::control::ControlSnapshot,
    ) -> SupervisorFuture<'a, DrainState> {
        Box::pin(async move { self.backend()?.driver.maintain(lease, snapshot).await })
    }
    fn cancel_fenced<'a>(&'a self, operations: &'a [Uuid]) -> SupervisorFuture<'a, ()> {
        Box::pin(async move { self.backend()?.driver.cancel_fenced(operations).await })
    }
    fn exited<'a>(&'a self, lease: &'a crate::control::Lease) -> SupervisorFuture<'a, DrainState> {
        Box::pin(async move { self.backend()?.driver.exited(lease).await })
    }
}
fn log(logs: &LogSink, runtime_id: Uuid, event: &str, outcome: &str, error: Option<&Error>) {
    logs.emit(&LogEvent {
        severity: if error.is_some() {
            Severity::Error
        } else {
            Severity::Info
        },
        runtime_id,
        owner_generation: None,
        workspace_id: None,
        work: None,
        operation_id: None,
        session_id: None,
        revision: None,
        event: event.into(),
        outcome: outcome.into(),
        reason: error.map(|error| error.code.clone()),
    });
}
struct HostedWorkflow {
    inner: Arc<dyn WorkflowControl>,
    health: Arc<RwLock<Value>>,
    logs: Arc<LogSink>,
}
impl WorkflowControl for HostedWorkflow {
    fn inspect(&self) -> HostFuture<'_, RuntimeInspection> {
        self.inner.inspect()
    }
    fn health(&self) -> Result<Value> {
        Ok(
            json!({"runtime":self.inner.health()?,"control_host":self.health.read().map_err(|_|Error::new("control_failed","host health is unavailable"))?.clone(),"logs":self.logs.health()}),
        )
    }
    fn command_targets(&self, request: &CommandRequest) -> Result<Vec<String>> {
        self.inner.command_targets(request)
    }
    fn execute(
        &self,
        actor: Actor,
        request: CommandRequest,
        parent: Receipt,
    ) -> HostFuture<'_, Receipt> {
        self.inner.execute(actor, request, parent)
    }
    fn cleanup_plan(&self, operation: Uuid) -> HostFuture<'_, Value> {
        self.inner.cleanup_plan(operation)
    }
}
struct RemoteListener {
    server: Option<ServerHandle>,
    contract: Option<String>,
    next_retry: tokio::time::Instant,
    health: Value,
}
impl RemoteListener {
    fn new() -> Self {
        Self {
            server: None,
            contract: None,
            next_retry: tokio::time::Instant::now(),
            health: json!({"enabled":false,"status":"not_configured"}),
        }
    }
    async fn shutdown(&mut self) -> Result<()> {
        if let Some(server) = self.server.take() {
            server.shutdown().await?;
        }
        Ok(())
    }
    async fn refresh(
        &mut self,
        configuration: &dyn ConfigurationControl,
        authentication: &TransportAuthentication,
        control: &ControlHost,
        state: &ApiState,
    ) {
        if let Err(error) = self
            .refresh_inner(configuration, authentication, control, state)
            .await
        {
            self.health = json!({"enabled":false,"error":error});
            self.next_retry = tokio::time::Instant::now() + Duration::from_secs(30);
        }
    }
    async fn refresh_inner(
        &mut self,
        configuration: &dyn ConfigurationControl,
        authentication: &TransportAuthentication,
        control: &ControlHost,
        state: &ApiState,
    ) -> Result<()> {
        let config = configuration.snapshot()?;
        let revision = config.revision()?;
        let digest = canonical_digest(1, &config.control.https)?;
        let changed = self.contract.as_deref() != Some(&digest);
        if changed {
            self.shutdown().await?;
            self.contract = Some(digest);
            self.next_retry = tokio::time::Instant::now();
        }
        if let Some(server) = &self.server {
            self.health = serde_json::to_value(server.health.borrow().clone())
                .map_err(|_| Error::invalid("TLS health encoding failed"))?;
            if !server.is_finished() {
                return Ok(());
            }
            self.shutdown().await?;
        }
        let Some(https) = config.control.https.clone() else {
            install_current_authentication(control, configuration, authentication, &config, None)?;
            self.health = json!({"enabled":false,"status":"not_configured"});
            return Ok(());
        };
        if tokio::time::Instant::now() < self.next_retry {
            return Ok(());
        }
        let tailscale = TailscaleClient::installed(https.tailscale_executable.clone().into())?;
        install_current_authentication(
            control,
            configuration,
            authentication,
            &config,
            Some(tailscale.clone()),
        )?;
        let mut remote = state.clone();
        remote.expected_authority = transport::direct_tls_authority(&https)?;
        let server = transport::start_direct_tls(remote, https, tailscale).await?;
        let mut server = Some(server);
        let install = control.serialize_observation(|_| {
            if configuration.snapshot()?.revision()? != revision {
                return Err(Error::new(
                    "stale_config",
                    "TLS configuration changed while listener started",
                ));
            }
            let current = server
                .take()
                .ok_or_else(|| Error::new("ownership_conflict", "TLS handle disappeared"))?;
            self.health = serde_json::to_value(current.health.borrow().clone())
                .map_err(|_| Error::invalid("TLS health encoding failed"))?;
            self.server = Some(current);
            Ok(())
        });
        if let Some(stale) = server {
            stale.shutdown().await?;
        }
        install
    }
}

fn install_current_authentication(
    control: &ControlHost,
    configuration: &dyn ConfigurationControl,
    authentication: &TransportAuthentication,
    expected: &crate::config::Config,
    tailscale: Option<TailscaleClient>,
) -> Result<()> {
    control.serialize_observation(|_| {
        if configuration.snapshot()?.revision()? != expected.revision()? {
            return Err(Error::new(
                "stale_config",
                "TLS configuration changed before authentication installation",
            ));
        }
        authentication.replace_configuration(expected.control.clone(), tailscale)
    })
}

/// Only an exclusively recovered current control registration may remove its
/// dead socket. A responsive listener, non-socket, link or uncertain error is a
/// conflict; no process is adopted or terminated through this pathname.
pub async fn reconcile_control_socket(registration: &Registration) -> Result<()> {
    registration.verify_control_owner()?;
    let path = registration.root().path().join("control.sock");
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Error::io("inspect prior control socket", error)),
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(Error::new(
            "unsafe_storage",
            "prior control endpoint is not an owner-only unlinked socket",
        ));
    }
    match tokio::time::timeout(
        Duration::from_secs(1),
        tokio::net::UnixStream::connect(&path),
    )
    .await
    {
        Ok(Err(error)) if error.raw_os_error() == Some(libc::ECONNREFUSED) => (),
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        _ => {
            return Err(Error::new(
                "ownership_conflict",
                "prior control endpoint is live or its shutdown is unconfirmed",
            ));
        }
    }
    registration.verify_control_owner()?;
    let current = std::fs::symlink_metadata(&path)
        .map_err(|error| Error::io("recheck prior control socket", error))?;
    if !current.file_type().is_socket()
        || current.dev() != metadata.dev()
        || current.ino() != metadata.ino()
    {
        return Err(Error::new(
            "ownership_conflict",
            "prior control endpoint changed during recovery",
        ));
    }
    std::fs::remove_file(path)
        .map_err(|error| Error::io("remove confirmed stale owned control socket", error))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct ReadConfiguration(crate::config::Config);
    impl ConfigurationControl for ReadConfiguration {
        fn authority_fences(&self) -> Result<crate::configuration::AuthorityFences> {
            Ok(Default::default())
        }
        fn snapshot(&self) -> Result<crate::config::Config> {
            Ok(self.0.clone())
        }
        fn execute(
            &self,
            _: Actor,
            _: CommandRequest,
            _: Option<crate::secrets::Secret>,
            _: Receipt,
        ) -> HostFuture<'_, Receipt> {
            Box::pin(async { Err(Error::new("fixture", "unexpected configuration mutation")) })
        }
        fn check(&self, _: Option<Uuid>) -> HostFuture<'_, Value> {
            Box::pin(async { Ok(json!({"fixture":"read_only"})) })
        }
    }
    #[tokio::test]
    async fn unavailable_existing_key_preserves_inspection_but_cannot_admit_or_mutate() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let config:crate::config::Config=serde_json::from_value(json!({"schema_version":1,"runtime":{"id":runtime,"host_id":"fixture","max_agents":1},"connections":{},"workspaces":{},"control":{"local_users":{"501":{"id":"fixture","linear_user_id":Uuid::new_v4(),"roles":["inspect","configure"]}}}})).unwrap();
        config.validate().unwrap();
        let backend = SwitchableBackend::new(
            Arc::new(ReadConfiguration(config)),
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("references")).unwrap(),
        );
        backend
            .install(Err(Error::new(
                "authentication_required",
                "existing fixture Keychain is locked",
            )))
            .unwrap();
        assert!(!backend.ready().unwrap());
        assert_eq!(backend.health().unwrap()["status"], "prerequisite_blocked");
        assert_eq!(
            backend.health().unwrap()["error"]["code"],
            "authentication_required"
        );
        assert_eq!(
            backend.inspect().await.unwrap().projection.runtime_id,
            runtime
        );
        assert_eq!(
            backend.reconcile_before_start().await.unwrap_err().code,
            "authentication_required"
        );
        assert_eq!(
            backend.cleanup_plan(Uuid::new_v4()).await.unwrap_err().code,
            "authentication_required"
        );
        assert!(backend.backend().is_err());
    }
    #[tokio::test]
    async fn stale_listener_install_cannot_restore_removed_operator_roles() {
        use crate::api::{Authentication, Peer};
        let temp = tempfile::tempdir().unwrap();
        let root =
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("runtime")).unwrap();
        let runtime = Uuid::new_v4();
        let control = ControlHost::setup(root, runtime, Uuid::new_v4()).unwrap();
        let old:crate::config::Config=serde_json::from_value(json!({"schema_version":1,"runtime":{"id":runtime,"host_id":"fixture","max_agents":1},"connections":{},"workspaces":{},"control":{"local_users":{"501":{"id":"fixture","linear_user_id":Uuid::new_v4(),"roles":["inspect","configure"]}}}})).unwrap();
        let mut current = old.clone();
        current.control.local_users.get_mut("501").unwrap().roles =
            std::collections::BTreeSet::from([crate::config::ControlRole::Inspect]);
        let authentication = TransportAuthentication::new(current.control.clone(), None);
        let configuration = ReadConfiguration(current);
        assert_eq!(
            install_current_authentication(&control, &configuration, &authentication, &old, None)
                .unwrap_err()
                .code,
            "stale_config"
        );
        let actor = authentication
            .authenticate(Peer::Unix { uid: 501 })
            .await
            .unwrap();
        assert!(!actor.roles.contains("configure"));
        assert!(actor.roles.contains("inspect"));
        install_current_authentication(
            &control,
            &configuration,
            &authentication,
            &configuration.0,
            None,
        )
        .unwrap();
    }
}
