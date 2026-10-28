//! One authenticated command dispatcher for both transports. Inspection consumes
//! a coherent provider projection and sealed evidence, never an agent summary.
use chrono::{Duration, Utc};
use kegari_config::{Config, ControlRole};
use kegari_control_api::{
    api::{DispatchResult, Dispatcher},
    identity::{Actor, Principal},
    snapshot::Snapshots,
};
use kegari_secrets::Secret;
use kegari_types::api::{CommandName, CommandRequest};
use kegari_types::error::{Error, Result};
use kegari_v0::{
    control::{ControlHost, ControlSnapshot, Intent, Lifecycle, Receipt, ReceiptPhase},
    control_command::{
        command_effect_id, mutation_request, parse_id, resolve_workspace, text,
        verify_current_actor,
    },
    reconcile::ParentDelivery,
    runtime_host::{ConfigurationControl, HostFuture, LocalControlOwner, WorkflowControl},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;
use uuid::Uuid;

/// Explicit local-only bootstrap. Acquires registration/control.lock before any
/// credential writes, validates through the same configuration transaction, and
/// leaves durable STOPPED without starting a listener, scheduler or model turn.
pub async fn bootstrap(configuration: &str, request_id: Uuid) -> Result<DispatchResult> {
    use kegari_control_api::transport::TransportAuthentication;
    use kegari_platform::{BinaryIdentity, HostIdentity};
    use kegari_secrets::Keychain;
    use kegari_v0::{configuration::ConfigManager, platform::Registration};
    let host = HostIdentity::discover()?;
    let mut config = Config::parse(configuration)?;
    config.resolve_paths(&host.home, |name| std::env::var(name).ok())?;
    if config.runtime.host_id != host.host_id {
        return Err(Error::new(
            "identity_mismatch",
            "setup configuration does not identify this execution host",
        ));
    }
    let operator = config
        .control
        .local_users
        .get(&host.uid.to_string())
        .ok_or_else(|| {
            Error::new(
                "access_denied",
                "setup requires this OS user's explicit operator mapping",
            )
        })?;
    if !operator.roles.contains(&ControlRole::Configure) {
        return Err(Error::new(
            "access_denied",
            "local setup operator lacks configure authority",
        ));
    }
    let actor = Actor {
        identity: operator.id.clone(),
        roles: ["configure".into()].into(),
        principal: Some(Principal::Unix { uid: host.uid }),
    };
    let binary = BinaryIdentity::inspect(
        &std::env::current_exe().map_err(|e| Error::io("resolve running executable", e))?,
        false,
    )?;
    let (registration, control) =
        Registration::bootstrap(&host, config.runtime.id, binary, request_id)?;
    let registration = Arc::new(registration);
    let control = Arc::new(control);
    let secrets = Arc::new(Keychain::new(config.runtime.id)?);
    secrets.initialize_attestation_key()?;
    let authentication = Arc::new(TransportAuthentication::new(config.control.clone(), None));
    let manager = ConfigManager::new(
        config.clone(),
        registration,
        control.clone(),
        secrets,
        authentication,
    )?;
    let configuration = toml::to_string_pretty(&config)
        .map_err(|_| Error::invalid("setup configuration encoding failed"))?;
    let request = CommandRequest {
        api_version: 1,
        runtime_id: config.runtime.id,
        request_id,
        command: CommandName::Setup,
        args: json!({"configuration":configuration}),
        expected_config_revision: None,
        expected_control_generation: Some(control.snapshot()?.intent.generation),
    };
    let receipt = manager.execute(actor, request, None).await?;
    let current = manager.snapshot()?;
    Ok(DispatchResult {
        config_revision: Some(kegari_v0::config::config_revision(&current)?),
        operation_id: Some(receipt.operation_id),
        state: Some("stopped".into()),
        data: json!({"runtime_id":current.runtime.id,"receipt":receipt,"desired":"STOPPED","next_action":"Explicitly install/start the registered control service, inspect prerequisites, then start orchestration when qualified"}),
    })
}
pub struct HostServices {
    pub control: Arc<ControlHost>,
    pub configuration: Arc<dyn ConfigurationControl>,
    pub workflow: Arc<dyn WorkflowControl>,
    pub owner: Arc<dyn LocalControlOwner>,
    pub wake: Arc<Notify>,
}
pub struct HostDispatcher {
    services: HostServices,
    snapshots: Mutex<Snapshots>,
    snapshot_metadata: Mutex<BTreeMap<Uuid, SnapshotMetadata>>,
    runtime: Uuid,
}
#[derive(Clone)]
struct SnapshotMetadata {
    config_revision: String,
    state: String,
    operation: Option<Uuid>,
    expires_at: chrono::DateTime<Utc>,
}
impl HostDispatcher {
    pub fn new(services: HostServices) -> Result<Self> {
        let runtime = services.configuration.snapshot()?.runtime.id;
        if services.control.snapshot()?.intent.runtime_id != runtime {
            return Err(Error::new(
                "identity_mismatch",
                "control and configuration name different runtimes",
            ));
        }
        Ok(Self {
            services,
            snapshots: Mutex::new(Snapshots::default()),
            snapshot_metadata: Mutex::new(BTreeMap::new()),
            runtime,
        })
    }
    async fn control(&self) -> Result<ControlSnapshot> {
        let control = self.services.control.clone();
        tokio::task::spawn_blocking(move || control.snapshot())
            .await
            .map_err(|_| Error::new("control_failed", "control observation task failed"))?
    }
    fn result(
        &self,
        config: &Config,
        state: &str,
        operation: Option<Uuid>,
        data: Value,
    ) -> Result<DispatchResult> {
        if data.get("snapshot_id").is_some() {
            let page: kegari_types::snapshot::Page =
                serde_json::from_value(data.clone()).map_err(|_| {
                    Error::new("snapshot_invalid", "inspection page metadata is invalid")
                })?;
            let mut metadata = self.snapshot_metadata.lock().map_err(|_| {
                Error::new("snapshot_unavailable", "inspection metadata lock failed")
            })?;
            metadata.retain(|_, m| m.expires_at > Utc::now());
            let recorded = if page.cursor.is_none() {
                let record = SnapshotMetadata {
                    config_revision: kegari_v0::config::config_revision(config)?,
                    state: state.into(),
                    operation,
                    expires_at: page.expires_at,
                };
                metadata.insert(page.snapshot_id, record.clone());
                record
            } else {
                metadata.get(&page.snapshot_id).cloned().ok_or_else(|| {
                    Error::new(
                        "snapshot_expired",
                        "snapshot response metadata expired; restart the complete read",
                    )
                })?
            };
            return Ok(DispatchResult {
                config_revision: Some(recorded.config_revision),
                operation_id: recorded.operation,
                state: Some(recorded.state),
                data,
            });
        }
        Ok(DispatchResult {
            config_revision: Some(kegari_v0::config::config_revision(config)?),
            operation_id: operation,
            state: Some(state.into()),
            data,
        })
    }
    fn page(
        &self,
        actor: &Actor,
        request: &CommandRequest,
        projection: Option<&Value>,
    ) -> Result<Value> {
        let mut filters = request.args.clone();
        let args = filters
            .as_object_mut()
            .ok_or_else(|| Error::new("invalid_input", "inspection arguments must be an object"))?;
        args.remove("snapshot_id");
        args.remove("cursor");
        let binding = json!({"command":request.command,"args":filters,"principal":actor.principal});
        let mut snapshots = self
            .snapshots
            .lock()
            .map_err(|_| Error::new("snapshot_unavailable", "inspection snapshot lock failed"))?;
        let page = match (
            request.args.get("snapshot_id").and_then(Value::as_str),
            request.args.get("cursor").and_then(Value::as_str),
        ) {
            (Some(id), Some(cursor)) => snapshots.page(
                parse_id(id)?,
                parse_id(cursor)?,
                &actor.identity,
                self.runtime,
                &binding,
                Utc::now(),
            )?,
            (None, None) => snapshots.capture(
                &actor.identity,
                self.runtime,
                &binding,
                projection.ok_or_else(|| {
                    Error::new(
                        "snapshot_invalid",
                        "initial read lacks a coherent projection",
                    )
                })?,
                Utc::now(),
            )?,
            _ => {
                return Err(Error::new(
                    "invalid_input",
                    "snapshot continuation requires both IDs",
                ));
            }
        };
        serde_json::to_value(page)
            .map_err(|_| Error::new("snapshot_invalid", "cannot encode complete inspection page"))
    }
    async fn dispatch_command(
        &self,
        actor: Actor,
        request: CommandRequest,
        secret: Option<Secret>,
    ) -> Result<DispatchResult> {
        kegari_control_api::api::validate_command_request(&request, self.runtime)?;
        let config = self.services.configuration.snapshot()?;
        self.services
            .configuration
            .authority_fences()?
            .check_actor(
                &actor,
                kegari_control_api::api::command_role(&request.command, &request.args),
            )?;
        verify_current_actor(
            &config,
            &actor,
            kegari_control_api::api::command_role(&request.command, &request.args),
        )?;
        if secret.is_some() && request.command != CommandName::ConnectionAdd {
            return Err(Error::new(
                "invalid_input",
                "this command cannot receive a credential",
            ));
        }
        let operation = if request.command == CommandName::OperationShow {
            Some(parse_id(text(&request.args, "operation")?)?)
        } else {
            None
        };
        // Continuation consumes only the original immutable projection. It does
        // not refetch providers or combine a new revision with old pages.
        if request.args.get("snapshot_id").is_some() {
            return self.result(
                &config,
                "observed",
                operation,
                self.page(&actor, &request, None)?,
            );
        }
        use CommandName::*;
        match request.command {
            Status => {
                let control = self.control().await?;
                self.result(
                    &config,
                    runtime_state(&control),
                    None,
                    json!({
                        "runtime_version":env!("CARGO_PKG_VERSION"),"runtime_build":kegari_v0::BUILD_ID,
                        "control":control,"owner":self.services.owner.status()?,
                        "health":self.services.workflow.health()?,"observed_at":Utc::now()
                    }),
                )
            }
            Overview | Show | Open => {
                let view = self.services.workflow.inspect().await?;
                if view.projection.runtime_id != self.runtime
                    || view.projection.config_revision
                        != kegari_v0::config::config_revision(&config)?
                {
                    return Err(Error::new(
                        "stale_config",
                        "provider view belongs to another configuration revision",
                    ));
                }
                kegari_v0::inspection::validate_history(
                    self.runtime,
                    &view.release_history,
                    &view.no_release_history,
                )?;
                let workspace = kegari_v0::inspection::resolve_history_workspace(
                    &config,
                    &view.release_history,
                    &view.no_release_history,
                    request.args.get("workspace").and_then(Value::as_str),
                )?;
                let control = self.control().await?;
                let data = if request.command == Overview {
                    serde_json::to_value(kegari_v0::inspection::overview(
                        &view,
                        workspace,
                        &control,
                        Utc::now(),
                        Duration::milliseconds(
                            config
                                .runtime
                                .poll_ms
                                .saturating_mul(2)
                                .min(i64::MAX as u64) as i64,
                        ),
                    )?)
                    .map_err(|_| Error::invalid("overview encoding failed"))?
                } else {
                    let selected = kegari_v0::inspection::resolve_history_parent(
                        &config,
                        &view.projection,
                        &view.release_history,
                        &view.no_release_history,
                        text(&request.args, "parent")?,
                        workspace,
                    )?;
                    let key = &selected.key;
                    let parent = view
                        .projection
                        .parent(selected.workspace, key.issue)
                        .ok()
                        .filter(|parent| parent.key == *key);
                    let releases =
                        kegari_v0::inspection::parent_rows(&view.release_history, &selected);
                    let no_release = kegari_v0::inspection::no_release_parent_rows(
                        &view.no_release_history,
                        &selected,
                    );
                    let proof = parent.and_then(|_| view.delivery.get(key));
                    let evidence =
                        proof
                            .and_then(ParentDelivery::verified_parent)
                            .filter(|verified| {
                                Some(&verified.manifest().work)
                                    == parent.and_then(|parent| parent.work.as_ref())
                            });
                    if request.command == Open {
                        let url = kegari_v0::inspection::observed_history_url(
                            parent,
                            &releases,
                            &no_release,
                        )?;
                        return self.result(&config,"observed",None,json!({"parent":selected.reference(),"url":url,"evidence_token":evidence.map(|p|p.token())}));
                    }
                    let children: BTreeMap<_, _> = proof
                        .map(|p| {
                            p.verified_children()
                                .iter()
                                .map(|(id, c)| (*id, c.evidence()))
                                .collect()
                        })
                        .unwrap_or_default();
                    json!({"parent":parent,"inspection_identity":{"workspace":selected.workspace,"parent":key,"parent_reference":selected.reference()},"children_evidence":children,"evidence_manifest":evidence.map(|p|p.manifest()),
                        "evidence_token":evidence.map(|p|p.token()),"acceptance":proof.and_then(ParentDelivery::verified_acceptance).map(|a|a.acceptance()),"carried_acceptance":proof.and_then(ParentDelivery::carried_acceptance_record),
                        "release":view.release_details.get(&(selected.workspace,key.clone())),"release_history":releases,"no_release_history":no_release,"history":kegari_v0::inspection::history_for_parent(&view.history,selected.workspace,key)})
                };
                self.result(
                    &config,
                    "observed",
                    None,
                    self.page(&actor, &request, Some(&data))?,
                )
            }
            ConnectionList => {
                let connections: Vec<_> = config.connections.iter().map(|(name,c)|json!({"name":name,"kind":c.kind,"account_id":c.account_id,"endpoint":c.endpoint,"credential_configured":c.credential_ref.is_some()||c.auth_home.is_some()})).collect();
                self.result(&config,"observed",None,self.page(&actor,&request,Some(&json!({"connections":connections,"health":self.services.workflow.health()?})))?)
            }
            WorkspaceList => {
                let workspaces: Vec<_> = config.workspaces.iter().map(|(name,w)|json!({"name":name,"id":w.id,"linear_workspace":w.linear_workspace_id,"connections":{"linear":w.linear_connection,"github":w.github_connection,"codex":w.codex_connection},"scope":{"linear":w.linear,"github":w.github},"release_mode":w.release.mode})).collect();
                self.result(&config,"observed",None,self.page(&actor,&request,Some(&json!({"workspaces":workspaces,"health":self.services.workflow.health()?})))?)
            }
            ConfigShow | ConfigCheck => {
                let workspace = resolve_workspace(
                    &config,
                    request.args.get("workspace").and_then(Value::as_str),
                )?;
                let data = if request.command == ConfigCheck {
                    self.services.configuration.check(workspace).await?
                } else {
                    let mut effective = config.clone();
                    if let Some(workspace) = workspace {
                        effective.workspaces.retain(|_, w| w.id == workspace);
                    }
                    json!({"configuration":effective,"revision":kegari_v0::config::config_revision(&config)?,"provenance":"registered execution-host configuration","runtime_build":kegari_v0::BUILD_ID})
                };
                self.result(
                    &config,
                    "observed",
                    None,
                    self.page(&actor, &request, Some(&data))?,
                )
            }
            OperationShow => {
                let id = operation
                    .ok_or_else(|| Error::new("invalid_input", "operation ID required"))?;
                let control = self.services.control.clone();
                let receipt = tokio::task::spawn_blocking(move || control.operation(id))
                    .await
                    .map_err(|_| {
                        Error::new("control_failed", "receipt observation task failed")
                    })??;
                let control = self.control().await?;
                let state = operation_state(&receipt, &control);
                let data = json!({"receipt":receipt,"control":control});
                self.result(
                    &config,
                    state,
                    Some(id),
                    self.page(&actor, &request, Some(&data))?,
                )
            }
            Pause | Resume | Start | Stop => {
                let command = match request.command {
                    Pause => Lifecycle::Pause,
                    Resume => Lifecycle::Resume,
                    Start => Lifecycle::Start,
                    _ if request.args.get("force") == Some(&Value::Bool(true)) => {
                        Lifecycle::ForceStop
                    }
                    _ => Lifecycle::Stop,
                };
                let control = self.services.control.clone();
                let generation = request.expected_control_generation.ok_or_else(|| {
                    Error::new("invalid_input", "expected control generation is required")
                })?;
                let mutation = mutation_request(&actor, &request, vec![self.runtime.to_string()]);
                let configuration = self.services.configuration.clone();
                let receipt = tokio::task::spawn_blocking(move || {
                    control.lifecycle_checked(mutation, command, generation, || {
                        configuration
                            .authority_fences()?
                            .check_actor(&actor, "lifecycle")?;
                        verify_current_actor(&configuration.snapshot()?, &actor, "lifecycle")
                    })
                })
                .await
                .map_err(|_| Error::new("control_failed", "lifecycle task failed"))??;
                self.services.wake.notify_one();
                let control = self.control().await?;
                self.result(
                    &config,
                    operation_state(&receipt, &control),
                    Some(receipt.operation_id),
                    json!({"receipt":receipt,"control":control}),
                )
            }
            ServiceStop => {
                if !matches!(actor.principal, Some(Principal::Unix { .. })) {
                    return Err(Error::new(
                        "access_denied",
                        "service shutdown requires a local authenticated principal",
                    ));
                }
                let control = self.services.control.clone();
                let owner = self.services.owner.clone();
                let mutation = mutation_request(&actor, &request, vec![self.runtime.to_string()]);
                let configuration = self.services.configuration.clone();
                let receipt = tokio::task::spawn_blocking(move || {
                    control.stop_service(mutation, || {
                        configuration
                            .authority_fences()?
                            .check_actor(&actor, "lifecycle")?;
                        verify_current_actor(&configuration.snapshot()?, &actor, "lifecycle")?;
                        owner.reserve_shutdown()
                    })
                })
                .await
                .map_err(|_| Error::new("control_failed", "service shutdown task failed"))??;
                self.services.wake.notify_one();
                self.result(
                    &config,
                    "service_stopping",
                    Some(receipt.operation_id),
                    json!({"receipt":receipt}),
                )
            }
            Setup | ConnectionAdd | ConnectionReconnect | WorkspaceAdd | WorkspaceConfigure => {
                self.queue_mutation(actor, request, secret, &config, true)
                    .await
            }
            Cleanup if request.args.get("apply") != Some(&Value::Bool(true)) => {
                let plan = self
                    .services
                    .workflow
                    .cleanup_plan(parse_id(text(&request.args, "operation")?)?)
                    .await?;
                self.result(
                    &config,
                    "observed",
                    None,
                    self.page(&actor, &request, Some(&plan))?,
                )
            }
            Approve | Correct | Release | OperationReconcile | Retry | ReleaseRecover | Cleanup
            | EvaluationRun => {
                self.queue_mutation(actor, request, None, &config, false)
                    .await
            }
        }
    }

    async fn queue_mutation(
        &self,
        actor: Actor,
        mut request: CommandRequest,
        secret: Option<Secret>,
        config: &Config,
        configuration_command: bool,
    ) -> Result<DispatchResult> {
        let role =
            kegari_control_api::api::command_role(&request.command, &request.args).to_owned();
        let existing = self
            .services
            .control
            .operations()?
            .into_iter()
            .find(|r| r.request_id == request.request_id);
        let targets = if let Some(receipt) = existing {
            receipt.canonical_targets
        } else if configuration_command {
            vec![self.runtime.to_string()]
        } else {
            self.services.workflow.command_targets(&request)?
        };
        let mut mutation = mutation_request(&actor, &request, targets);
        mutation.kind = format!("command/{:?}", request.command);
        mutation.payload = json!({"arguments":request.args,"secret_digest":secret.as_ref().map(|s|kegari_digest::bytes_digest(s.bytes()))});
        let evaluation_trial = if request.command == CommandName::EvaluationRun {
            Some(kegari_control_api::api::evaluation_trial_identity(
                &request,
            )?)
        } else {
            None
        };
        if let Some(identity) = &evaluation_trial {
            mutation
                .expected
                .insert("evaluation_trial".into(), identity.clone());
        }
        let command_id = mutation.request_id;
        let control = self.services.control.clone();
        let command_history = control.clone();
        let configuration = self.services.configuration.clone();
        let authenticated = actor.clone();
        let (receipt, fresh) = tokio::task::spawn_blocking(move || {
            control.queue_command(mutation, &role, || {
                if let Some(identity)=&evaluation_trial {
                    for prior in command_history.operations()? {
                        if prior.kind=="command/EvaluationRun" && prior.request_id!=command_id {
                            let previous=prior.expected.get("evaluation_trial").ok_or_else(||Error::new("record_corrupt","retained evaluation command lacks its exact finite trial identity"))?;
                            if previous==identity {return Err(Error::new("evaluation_trial_exists","this declared repetition already has an original command; inspect it rather than rerun").detail(json!({"operation_id":prior.operation_id})));}
                        }
                    }
                }
                configuration
                    .authority_fences()?
                    .check_actor(&authenticated, &role)?;
                verify_current_actor(&configuration.snapshot()?, &authenticated, &role)
            })
        })
        .await
        .map_err(|_| Error::new("control_failed", "command admission task failed"))??;
        if fresh {
            let operation = receipt.operation_id;
            let parent = receipt.clone();
            request.request_id = command_effect_id(operation);
            let configuration = self.services.configuration.clone();
            let workflow = self.services.workflow.clone();
            let control = self.services.control.clone();
            let wake = self.services.wake.clone();
            // Ownership belongs to the host, not to the HTTP request future.
            // The observer records even a panic or bounded-task timeout as an
            // uncertain operation; neither path creates a replacement request.
            let evaluation = request.command == CommandName::EvaluationRun;
            let job = tokio::spawn(async move {
                let task = async move {
                    if configuration_command {
                        configuration.execute(actor, request, secret, parent).await
                    } else {
                        workflow.execute(actor, request, parent).await
                    }
                };
                if evaluation {
                    // This explicit task is also retained and joined by the actual
                    // runtime driver; its finite session/hook bounds own timeout.
                    return task.await.map(|effect|json!({"effect_operation_id":effect.operation_id,"receipt":effect}));
                }
                match tokio::time::timeout(std::time::Duration::from_secs(300),task).await {
                    Ok(result)=>result.map(|effect|json!({"effect_operation_id":effect.operation_id,"receipt":effect})),
                    Err(_)=>Err(Error::new("operation_uncertain","command task exceeded its bound; reconcile its operation before retrying").detail(json!({"operation_id":operation,"effect_operation_id":command_effect_id(operation)}))),
                }
            });
            tokio::spawn(async move {
                let result=job.await.unwrap_or_else(|_|Err(Error::new("operation_uncertain","command task ended without a verified result").detail(json!({"operation_id":operation,"effect_operation_id":command_effect_id(operation)}))));
                let _ = tokio::task::spawn_blocking(move || {
                    control.complete_command(operation, result)
                })
                .await;
                wake.notify_one();
            });
        }
        let control = self.control().await?;
        self.result(config,operation_state(&receipt,&control),Some(receipt.operation_id),json!({"receipt":receipt,"effect_operation_id":command_effect_id(receipt.operation_id)}))
    }
}

impl Dispatcher for HostDispatcher {
    fn dispatch(
        &self,
        actor: Actor,
        request: CommandRequest,
        secret: Option<Secret>,
    ) -> HostFuture<'_, DispatchResult> {
        Box::pin(self.dispatch_command(actor, request, secret))
    }
}
fn runtime_state(control: &ControlSnapshot) -> &'static str {
    if control.service_stopping {
        "service_stopping"
    } else if control.draining {
        "draining"
    } else {
        match control.intent.desired {
            Intent::Running if control.admission_open => "running",
            Intent::Running => "reconciling",
            Intent::Paused => "paused",
            Intent::Stopped => "stopped",
        }
    }
}
fn operation_state(receipt: &Receipt, control: &ControlSnapshot) -> &'static str {
    if control.background_commands.contains(&receipt.operation_id) {
        return "pending";
    }
    if receipt.kind.starts_with("command/")
        && matches!(receipt.phase, ReceiptPhase::Prepared | ReceiptPhase::Sent)
    {
        return "unknown";
    }
    if receipt
        .observed_result
        .as_ref()
        .is_some_and(|v| v.get("error").is_some())
    {
        return "failed";
    }
    if receipt.kind == "lifecycle" {
        if receipt.request_id != control.intent.last_command_id {
            return "completed";
        }
        return match control.intent.desired {
            Intent::Stopped if control.draining => "draining",
            Intent::Stopped => "stopped",
            Intent::Paused => "completed",
            Intent::Running if control.admission_open => "completed",
            Intent::Running => "reconciling",
        };
    }
    if control.active_operations.iter().any(|p| {
        p.operation_id == receipt.operation_id
            || receipt.expected.get("admission_operation") == Some(&p.operation_id.to_string())
    }) {
        return if control.fenced_operations.iter().any(|id| {
            *id == receipt.operation_id
                || receipt.expected.get("admission_operation") == Some(&id.to_string())
        }) {
            "cancelling"
        } else {
            "running"
        };
    }
    match receipt.phase {
        ReceiptPhase::Prepared => "pending",
        ReceiptPhase::Sent => "unknown",
        ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled => "completed",
    }
}
