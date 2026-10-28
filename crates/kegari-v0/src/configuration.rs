//! Serialized host configuration transactions. Verification is read-only;
//! credential import and durable configuration replacement share the mutation gate.
use crate::{
    api::{CommandName, CommandRequest},
    codex::{CodexConnectionMetadata, CodexProfile, PermissionConfiguration},
    config::{Config, Connection, ControlRole, Deployment, ProviderKind, Speed, Workspace},
    connections::{VerifiedCredential, verify_credential},
    control::{Actor, ControlHost, Intent, MutationRequest, Principal, Receipt},
    digest::{bytes_digest, canonical_digest},
    error::{Error, Result},
    isolation::{ExecutionRole, IsolationPlan},
    platform::{HostIdentity, Registration},
    providers::{AccountIdentity, GithubClient, HttpPolicy, HttpTransport},
    secrets::{Secret, SecretStore, resolve_reference},
    storage::PrivateDir,
    transport::TransportAuthentication,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, RwLock},
    time::Duration,
};
use uuid::Uuid;

/// The production implementation uses actual bounded provider/app-server reads.
/// Tests inject this boundary, never substitute a successful production verifier.
pub trait ConfigurationInputs: Send + Sync {
    fn environment(&self, name: &str) -> Option<String>;
    fn transport(&self, name: &str) -> Option<Arc<dyn HttpTransport>>;
    fn configure_evaluations(&self, config: &Config) -> Result<()> {
        if config.workspaces.values().any(|w| w.evaluation.is_some()) {
            return Err(Error::new(
                "evaluation_unavailable",
                "evaluation requires a concrete reserved provider registry",
            ));
        }
        Ok(())
    }
    fn evaluation_providers(&self) -> Option<Arc<crate::evaluation_provider::EvaluationProviders>> {
        None
    }
    fn codex<'a>(
        &'a self,
        connection: &'a Connection,
        probe_root: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<CodexConnectionMetadata>> + Send + 'a>>;
}
pub struct NativeConfigurationInputs;
impl ConfigurationInputs for NativeConfigurationInputs {
    fn environment(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
    fn transport(&self, name: &str) -> Option<Arc<dyn HttpTransport>> {
        name.starts_with("evaluation-")
            .then(|| crate::evaluation_provider::EvaluationProviders::unconfigured_transport(name))
    }
    fn codex<'a>(
        &'a self,
        connection: &'a Connection,
        probe_root: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<CodexConnectionMetadata>> + Send + 'a>> {
        Box::pin(async move {
            if connection.argv.len() != 4
                || connection.argv[1..] != ["app-server", "--listen", "stdio://"]
            {
                return Err(Error::new(
                    "unsupported_profile",
                    "Codex connection argv differs from the supported stdio app-server contract",
                ));
            }
            let executable = resolve_executable(&connection.argv[0])?;
            let auth_home = PathBuf::from(
                connection
                    .auth_home
                    .as_deref()
                    .ok_or_else(|| Error::invalid("Codex auth home is missing"))?,
            );
            PrivateDir::create(
                probe_root
                    .parent()
                    .ok_or_else(|| Error::invalid("probe directory needs private parent"))?,
            )?;
            let root = PrivateDir::create(probe_root)?;
            let cwd = PrivateDir::create(&root.path().join("empty-context"))?;
            let scratch = PrivateDir::create(&root.path().join("scratch"))?;
            let plan = IsolationPlan {
                schema_version: 1,
                role: ExecutionRole::Coordination,
                execution_root: root.path().into(),
                repository: cwd.path().into(),
                cwd: cwd.path().into(),
                scratch: scratch.path().into(),
                read_roots: vec![],
                resource_writes: vec![],
                source_writes: vec![],
                forbidden_roots: vec![auth_home.clone()],
                managed_execution: None,
            }
            .validate()?;
            let permissions = PermissionConfiguration::for_isolation(&plan, &auth_home)?;
            crate::codex::probe_connection(
                &executable,
                &auth_home,
                &permissions,
                cwd.path(),
                &connection.account_id,
            )
            .await
        })
    }
}
fn resolve_executable(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    let selected = if path.is_absolute() {
        Some(path.to_owned())
    } else if path.components().count() == 1 {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|root| root.join(value))
                .find(|p| p.is_file())
        })
    } else {
        None
    }
    .ok_or_else(|| {
        Error::new(
            "unsupported_provider",
            "configured Codex executable is not installed at an explicit path or host PATH",
        )
    })?;
    crate::platform::BinaryIdentity::inspect(&selected, false).map(|binary| binary.path)
}

#[derive(Clone, Debug, Serialize)]
pub struct ConnectionObservation {
    pub connection: String,
    pub account_id: String,
    pub provider: ProviderKind,
    pub workspace_id: Option<Uuid>,
    pub permission_scopes: Option<Vec<String>>,
    pub protocol_profile: Option<String>,
    pub reused: bool,
}
struct Import {
    name: String,
    keychain_name: String,
    connection: Connection,
    verified: VerifiedCredential,
}
struct Prepared {
    before: String,
    config: Config,
    imports: Vec<Import>,
    observations: Vec<ConnectionObservation>,
    identity_change: bool,
    permission_resolution: FenceResolution,
}

#[derive(Clone)]
pub struct ConfigManager {
    registration: Arc<Registration>,
    control: Arc<ControlHost>,
    config: Arc<RwLock<Config>>,
    secrets: Arc<dyn SecretStore>,
    authentication: Arc<TransportAuthentication>,
    inputs: Arc<dyn ConfigurationInputs>,
    fences: Arc<RwLock<FenceRecord>>,
}
impl ConfigManager {
    pub async fn check(&self, workspace: Option<Uuid>) -> Result<Value> {
        let mut config = self.snapshot()?;
        let revision = config.revision()?;
        if let Some(id) = workspace {
            if !config
                .workspaces
                .values()
                .any(|workspace| workspace.id == id)
            {
                return Err(Error::new(
                    "not_found",
                    "configuration check workspace is not configured",
                ));
            }
            config.workspaces.retain(|_, workspace| workspace.id == id);
            let connections: BTreeSet<_> = config
                .workspaces
                .values()
                .flat_map(|workspace| {
                    [
                        workspace.linear_connection.clone(),
                        workspace.github_connection.clone(),
                        workspace.codex_connection.clone(),
                    ]
                })
                .collect();
            config
                .connections
                .retain(|name, _| connections.contains(name));
        }
        let home = self
            .registration
            .root()
            .path()
            .parent()
            .ok_or_else(|| Error::invalid("registered state root has no host home"))?;
        config.resolve_paths(home, |name| self.inputs.environment(name))?;
        let mut report = verification_report(
            &config,
            self.secrets.as_ref(),
            self.inputs.as_ref(),
            &self.registration.root().path().join("connection-probes"),
        )
        .await?;
        report["config_revision"] = json!(revision);
        Ok(report)
    }
    pub fn new(
        config: Config,
        registration: Arc<Registration>,
        control: Arc<ControlHost>,
        secrets: Arc<dyn SecretStore>,
        authentication: Arc<TransportAuthentication>,
    ) -> Result<Self> {
        Self::with_inputs(
            config,
            registration,
            control,
            secrets,
            authentication,
            Arc::new(NativeConfigurationInputs),
        )
    }
    pub fn with_inputs(
        config: Config,
        registration: Arc<Registration>,
        control: Arc<ControlHost>,
        secrets: Arc<dyn SecretStore>,
        authentication: Arc<TransportAuthentication>,
        inputs: Arc<dyn ConfigurationInputs>,
    ) -> Result<Self> {
        config.validate()?;
        inputs.configure_evaluations(&config)?;
        let owner = registration.snapshot()?;
        let revision = config.revision()?;
        if owner.runtime_id != config.runtime.id
            || owner.host_id != config.runtime.host_id
            || owner
                .config_revision
                .as_ref()
                .is_some_and(|expected| expected != &revision)
        {
            return Err(Error::new(
                "identity_mismatch",
                "configuration manager differs from registered runtime identity or revision",
            ));
        }
        let fences: FenceRecord = match registration.root().read("permission-fences.json") {
            Ok(record) => record,
            Err(error)
                if error.code == "record_missing"
                    && !control
                        .operations()?
                        .iter()
                        .any(|r| r.kind == "permission_fence") =>
            {
                FenceRecord {
                    schema_version: 1,
                    runtime_id: config.runtime.id,
                    entries: BTreeMap::new(),
                    resolutions: BTreeMap::new(),
                }
            }
            Err(error) => return Err(error),
        };
        if fences.schema_version != 1
            || fences.runtime_id != config.runtime.id
            || fences
                .entries
                .iter()
                .any(|(id, entry)| id != &entry.request_id || id.is_nil())
        {
            return Err(Error::new(
                "record_corrupt",
                "permission fence provenance differs from registered runtime",
            ));
        }
        let receipts: BTreeMap<_, _> = control
            .operations()?
            .into_iter()
            .map(|r| (r.request_id, r))
            .collect();
        for receipt in receipts.values().filter(|r| r.kind == "permission_fence") {
            let rejected = receipt.phase == crate::control::ReceiptPhase::Reconciled
                && receipt
                    .observed_result
                    .as_ref()
                    .is_some_and(|v| v["effect"] == "not_sent");
            if !rejected {
                let entry = fences.entries.values().find(|e| fence_operation(e.request_id)==receipt.request_id)
                    .ok_or_else(|| Error::new("permission_fenced", "durable permission proposal is missing its restriction provenance; explicit recovery is required"))?;
                if receipt.expected.get("permission_restriction")
                    != Some(&canonical_digest(1, entry)?)
                {
                    return Err(Error::new(
                        "permission_fenced",
                        "stored permission restriction differs from its immutable receipt",
                    ));
                }
            }
        }
        for entry in fences.entries.values() {
            if !receipts
                .get(&fence_operation(entry.request_id))
                .is_some_and(|r| r.kind == "permission_fence")
            {
                return Err(Error::new(
                    "record_corrupt",
                    "permission restriction has no durable proposal receipt",
                ));
            }
        }
        for (id, resolution) in &fences.resolutions {
            let receipt = receipts.get(id).ok_or_else(|| {
                Error::new(
                    "record_corrupt",
                    "permission resolution has no durable configuration receipt",
                )
            })?;
            if *id != resolution.request_id
                || receipt.kind != "configuration"
                || receipt.expected.get("permission_resolution")
                    != Some(&canonical_digest(1, resolution)?)
                || receipt.expected.get("configuration_after")
                    != Some(&resolution.configuration_after)
                || receipt
                    .observed_result
                    .as_ref()
                    .is_some_and(|v| v["effect"] == "not_sent")
            {
                return Err(Error::new(
                    "permission_fenced",
                    "permission restoration is not bound to an explicitly committed configuration scope",
                ));
            }
        }
        Ok(Self {
            fences: Arc::new(RwLock::new(fences)),
            registration,
            control,
            config: Arc::new(RwLock::new(config)),
            secrets,
            authentication,
            inputs,
        })
    }
    pub fn snapshot(&self) -> Result<Config> {
        self.config
            .read()
            .map(|value| value.clone())
            .map_err(|_| Error::new("invalid_config", "configuration snapshot is unavailable"))
    }
    pub async fn execute(
        &self,
        actor: Actor,
        request: CommandRequest,
        secret: Option<Secret>,
    ) -> Result<Receipt> {
        self.execute_internal(actor, request, secret, None).await
    }
    pub async fn execute_for_command(
        &self,
        actor: Actor,
        request: CommandRequest,
        secret: Option<Secret>,
        parent: Receipt,
    ) -> Result<Receipt> {
        self.execute_internal(actor, request, secret, Some(parent))
            .await
    }
    async fn execute_internal(
        &self,
        actor: Actor,
        request: CommandRequest,
        secret: Option<Secret>,
        parent: Option<Receipt>,
    ) -> Result<Receipt> {
        let current = self.snapshot()?;
        request.validate(current.runtime.id)?;
        if !matches!(
            request.command,
            CommandName::Setup
                | CommandName::ConnectionAdd
                | CommandName::ConnectionReconnect
                | CommandName::WorkspaceAdd
                | CommandName::WorkspaceConfigure
        ) {
            return Err(Error::new(
                "invalid_input",
                "command is not a configuration transaction",
            ));
        }
        require_current_operator(&current, &actor)?;
        if secret.is_some() && request.command != CommandName::ConnectionAdd {
            return Err(Error::new(
                "invalid_input",
                "transient secret input is allowed only for explicit connection import",
            ));
        }
        let secret_digest = secret.as_ref().map(|secret| bytes_digest(secret.bytes()));
        let parent_link=parent.map(|parent|{
            let persisted=self.control.operation(parent.request_id)?;
            let mut submitted=crate::app::mutation_request(&actor,&request,vec![request.runtime_id.to_string()]);
            submitted.kind=format!("command/{:?}",request.command);
            submitted.payload=json!({"arguments":request.args,"secret_digest":secret_digest});
            let expected=canonical_digest(1,&json!({"kind":submitted.kind,"targets":submitted.canonical_targets,"expected":submitted.expected,"payload":submitted.payload,"principal":actor.principal}))?;
            if request.request_id!=crate::app::command_effect_id(parent.request_id)||persisted.phase!=crate::control::ReceiptPhase::Prepared||persisted.actor!=actor.identity||persisted.runtime_id!=request.runtime_id||persisted.kind!=submitted.kind||persisted.payload_digest!=expected||parent.payload_digest!=persisted.payload_digest{return Err(Error::new("request_conflict","configuration effect does not match its authenticated durable command lineage"));}
            Ok((parent.request_id,persisted.payload_digest))
        }).transpose()?;
        match self.control.operation(request.request_id) {
            Ok(_) => (), // A completed/effect-reserved request is replayed below, never reproposed.
            Err(error) if error.code == "record_missing" => {
                self.record_reductions(&current, &actor, &request, parent_link.as_ref())?
            }
            Err(error) => return Err(error),
        }
        self.fences_except(Some(request.request_id))?
            .check_actor(&actor, "configure")?;
        let mut environment_inputs = BTreeMap::new();
        for name in if secret.is_some() {
            BTreeSet::new()
        } else {
            credential_environment_names(&current, &request)?
        } {
            let value = self.inputs.environment(&name).ok_or_else(|| {
                Error::new(
                    "missing_secret",
                    "declared credential environment input is absent",
                )
            })?;
            environment_inputs.insert(name, Secret::new(value.into_bytes())?);
        }
        let environment_digests: BTreeMap<_, _> = environment_inputs
            .iter()
            .map(|(name, value)| (name, bytes_digest(value.bytes())))
            .collect();
        let mut expected = BTreeMap::new();
        if let Some((parent, digest)) = &parent_link {
            expected.insert("parent_command_id".into(), parent.to_string());
            expected.insert("parent_payload_digest".into(), digest.clone());
        }
        if let Some(revision) = &request.expected_config_revision {
            expected.insert("requested_config_revision".into(), revision.clone());
        }
        if let Some(generation) = request.expected_control_generation {
            expected.insert(
                "requested_control_generation".into(),
                generation.to_string(),
            );
        }
        let mut mutation = MutationRequest {
            request_id: request.request_id,
            runtime_id: request.runtime_id,
            actor: actor.clone(),
            kind: "configuration".into(),
            canonical_targets: vec![request.runtime_id.to_string()],
            expected,
            payload: json!({"command":request.command,"args":request.args,"secret_digest":secret_digest,"environment_input_digests":environment_digests}),
        };
        match self.control.operation(request.request_id) {
            Ok(receipt) => {
                for key in [
                    "configuration_before",
                    "configuration_after",
                    "permission_resolution",
                ] {
                    if let Some(value) = receipt.expected.get(key) {
                        mutation.expected.insert(key.into(), value.clone());
                    }
                }
                let control = self.control.clone();
                return tokio::task::spawn_blocking(move || {
                    control.mutate_observed(
                        mutation,
                        None,
                        Some("configure"),
                        |_| {
                            Err(Error::new(
                                "reconciliation_required",
                                "known configuration receipt cannot begin a second effect",
                            ))
                        },
                        |_| {
                            Err(Error::new(
                                "reconciliation_required",
                                "known configuration receipt cannot begin a second effect",
                            ))
                        },
                    )
                })
                .await
                .map_err(|_| Error::new("control_failed", "configuration replay task failed"))?;
            }
            Err(error) if error.code == "record_missing" => {}
            Err(error) => return Err(error),
        }
        let prepared = self
            .prepare(&current, &request, secret, &environment_inputs)
            .await?;
        mutation
            .expected
            .insert("configuration_before".into(), prepared.before.clone());
        mutation
            .expected
            .insert("configuration_after".into(), prepared.config.revision()?);
        mutation.expected.insert(
            "permission_resolution".into(),
            canonical_digest(1, &prepared.permission_resolution)?,
        );
        let manager = self.clone();
        tokio::task::spawn_blocking(move || manager.commit(actor, request, mutation, prepared))
            .await
            .map_err(|_| Error::new("control_failed", "configuration transaction task failed"))?
    }
    async fn prepare(
        &self,
        current: &Config,
        request: &CommandRequest,
        mut secret: Option<Secret>,
        environment_inputs: &BTreeMap<String, Secret>,
    ) -> Result<Prepared> {
        let mut candidate = current.clone();
        let mut verify = BTreeSet::new();
        let mut explicit_import = None;
        let text = |name: &str| {
            request
                .args
                .get(name)
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    Error::new(
                        "invalid_input",
                        format!("configuration command needs {name}"),
                    )
                })
        };
        match request.command {
            CommandName::Setup => {
                candidate = Config::parse(text("configuration")?)?;
                verify.extend(candidate.connections.keys().cloned());
            }
            CommandName::ConnectionAdd => {
                let name = text("name")?.to_owned();
                let mut connection: Connection = toml::from_str(text("connection")?)
                    .map_err(|_| Error::invalid("connection must be strict typed TOML"))?;
                if let Some(reference) =
                    request.args.get("secret_reference").and_then(Value::as_str)
                {
                    if secret.is_some() {
                        return Err(Error::invalid("use only one explicit credential input"));
                    }
                    connection.credential_ref = Some(reference.into());
                }
                if secret.is_some() {
                    if connection.kind == ProviderKind::Codex {
                        return Err(Error::new(
                            "authentication_required",
                            "Codex requires supported isolated account login",
                        ));
                    }
                    connection.credential_ref =
                        Some(format!("keychain:import-{}", request.request_id));
                    explicit_import = Some(name.clone());
                }
                candidate.connections.insert(name.clone(), connection);
                verify.insert(name);
            }
            CommandName::ConnectionReconnect => {
                let name = text("name")?;
                if !candidate.connections.contains_key(name) {
                    return Err(Error::new("not_found", "connection is not configured"));
                }
                verify.insert(name.into());
            }
            CommandName::WorkspaceAdd => {
                let name = text("name")?.to_owned();
                if candidate.workspaces.contains_key(&name) {
                    return Err(Error::new(
                        "already_exists",
                        "workspace name already exists; use configure",
                    ));
                }
                let workspace: Workspace = toml::from_str(text("workspace_configuration")?)
                    .map_err(|_| Error::invalid("workspace must be strict typed TOML"))?;
                verify.extend([
                    workspace.linear_connection.clone(),
                    workspace.github_connection.clone(),
                    workspace.codex_connection.clone(),
                ]);
                candidate.workspaces.insert(name, workspace);
            }
            CommandName::WorkspaceConfigure => {
                let reference = text("workspace")?;
                let names: Vec<_> = candidate
                    .workspaces
                    .iter()
                    .filter(|(name, workspace)| {
                        *name == reference || workspace.id.to_string() == reference
                    })
                    .map(|(name, _)| name.clone())
                    .collect();
                if names.len() != 1 {
                    return Err(Error::new(
                        if names.is_empty() {
                            "not_found"
                        } else {
                            "ambiguous_reference"
                        },
                        "workspace reference must identify exactly one configured workspace",
                    ));
                }
                let workspace: Workspace = toml::from_str(text("workspace_configuration")?)
                    .map_err(|_| Error::invalid("workspace must be strict typed TOML"))?;
                if workspace.id != candidate.workspaces[&names[0]].id {
                    return Err(Error::new(
                        "identity_mismatch",
                        "workspace configure cannot replace its stable runtime workspace UUID",
                    ));
                }
                verify.extend([
                    workspace.linear_connection.clone(),
                    workspace.github_connection.clone(),
                    workspace.codex_connection.clone(),
                ]);
                candidate.workspaces.insert(names[0].clone(), workspace);
            }
            _ => unreachable!(),
        }
        if candidate.runtime.id != current.runtime.id
            || candidate.runtime.host_id != current.runtime.host_id
        {
            return Err(Error::new(
                "identity_mismatch",
                "configuration cannot replace runtime or host identity",
            ));
        }
        let home = self
            .registration
            .root()
            .path()
            .parent()
            .ok_or_else(|| Error::invalid("registered state root has no host home"))?;
        candidate.resolve_paths(home, |name| self.inputs.environment(name))?;
        self.inputs.configure_evaluations(&candidate)?;
        candidate.load_workflows()?;
        let mut imports = Vec::new();
        let mut observations = Vec::new();
        for name in verify {
            let connection = candidate
                .connections
                .get(&name)
                .ok_or_else(|| Error::invalid("workspace references an unavailable connection"))?
                .clone();
            let reused = current.connections.get(&name).is_some_and(|old| {
                old.kind == connection.kind
                    && old.account_id == connection.account_id
                    && old.endpoint == connection.endpoint
            });
            if connection.kind == ProviderKind::Codex {
                let probe = self
                    .registration
                    .root()
                    .path()
                    .join("connection-probes")
                    .join(format!(
                        "{}-{}",
                        request.request_id,
                        canonical_digest(1, &name)?
                            .rsplit(':')
                            .next()
                            .unwrap_or_default()
                    ));
                let metadata = self.inputs.codex(&connection, &probe).await?;
                verify_codex(&candidate, &name, &connection, &metadata)?;
                observations.push(ConnectionObservation {
                    connection: name,
                    account_id: metadata.account_id,
                    provider: ProviderKind::Codex,
                    workspace_id: None,
                    permission_scopes: None,
                    protocol_profile: Some(metadata.protocol_profile),
                    reused,
                });
            } else {
                let reference = connection.credential_ref.as_deref().ok_or_else(|| {
                    Error::new("missing_secret", "provider credential input is absent")
                })?;
                let importing =
                    explicit_import.as_deref() == Some(&name) || reference.starts_with('$');
                let value = if explicit_import.as_deref() == Some(&name) {
                    secret.take().ok_or_else(|| {
                        Error::new(
                            "missing_secret",
                            "explicit credential input was already consumed",
                        )
                    })?
                } else {
                    resolve_reference(reference, self.secrets.as_ref(), |name| {
                        environment_inputs
                            .get(name)
                            .and_then(|secret| std::str::from_utf8(secret.bytes()).ok())
                            .map(str::to_owned)
                    })?
                };
                let verified = verify_credential(
                    &connection,
                    value,
                    Duration::from_millis(candidate.runtime.request_timeout_ms),
                    self.inputs.transport(&name),
                )
                .await?;
                let identity = verified.identity();
                for workspace in candidate
                    .workspaces
                    .values()
                    .filter(|workspace| workspace.linear_connection == name)
                {
                    if identity.workspace_id != Some(workspace.linear_workspace_id) {
                        return Err(Error::new(
                            "identity_mismatch",
                            "Linear account workspace differs from configured workspace",
                        ));
                    }
                }
                observations.push(observation(name.clone(), connection.kind, identity, reused));
                if importing {
                    let keychain_name = format!(
                        "import-{}-{}",
                        request.request_id,
                        canonical_digest(1, &name)?
                            .rsplit(':')
                            .next()
                            .unwrap_or_default()
                    );
                    // VerifiedCredential binds the pre-import connection. Keep it
                    // unchanged until import, then persist this deterministic ref.
                    imports.push(Import {
                        name: name.clone(),
                        keychain_name: keychain_name.clone(),
                        connection: connection.clone(),
                        verified,
                    });
                    candidate.connections.get_mut(&name).unwrap().credential_ref =
                        Some(format!("keychain:{keychain_name}"));
                }
            }
        }
        candidate.validate()?;
        let identity_change = restricted_revision(current)? != restricted_revision(&candidate)?;
        let mut permission_resolution = FenceResolution::new(request, &candidate)?;
        permission_resolution.proposals = self
            .fences
            .read()
            .map_err(|_| Error::new("permission_fenced", "permission fence state unavailable"))?
            .entries
            .keys()
            .copied()
            .collect();
        Ok(Prepared {
            permission_resolution,
            before: current.revision()?,
            config: candidate,
            imports,
            observations,
            identity_change,
        })
    }
    fn commit(
        &self,
        actor: Actor,
        request: CommandRequest,
        mutation: MutationRequest,
        mut prepared: Prepared,
    ) -> Result<Receipt> {
        let parent_command = mutation
            .expected
            .get("parent_command_id")
            .and_then(|value| Uuid::parse_str(value).ok());
        self.control.mutate_observed(mutation,None,Some("configure"),|control|{
            let current=self.snapshot()?;require_current_operator(&current,&actor)?;self.fences_except(Some(request.request_id))?.check_actor(&actor,"configure")?;
            if parent_command.is_some_and(|id|!control.background_commands.contains(&id)){return Err(Error::new("request_conflict","configuration command is no longer owned by the host background queue"));}
            if current.revision()?!=prepared.before || request.expected_config_revision.as_ref().is_some_and(|value|value!=&prepared.before){return Err(Error::new("stale_config","configuration changed after preparation or differs from the client's revision"));}
            if request.expected_control_generation.is_some_and(|generation|generation!=control.intent.generation){return Err(Error::new("stale_generation","runtime intent changed during configuration preparation"));}
            if prepared.identity_change && (control.intent.desired==Intent::Running||control.draining||!control.active_operations.is_empty()||!control.unresolved_operations.is_empty()){return Err(Error::new("configuration_busy","identity or permission replacement requires paused/stopped, drained work and reconciled effects"));}
            Ok(())
        },|_|{
            let mut stored=self.config.write().map_err(|_|Error::new("invalid_config","configuration lock unavailable"))?;
            for import in prepared.imports {
                let connection=prepared.config.connections.get_mut(&import.name).ok_or_else(||Error::invalid("prepared connection disappeared"))?;
                // Verification was against the input reference; restore it only
                // in memory while the sealed import performs its exact check.
                let mut input=import.connection;
                let before_import=import.verified.import(&mut input,&import.keychain_name,self.secrets.as_ref())?;
                if canonical_digest(1,&input)?!=canonical_digest(1,connection)?{return Err(Error::new("identity_mismatch","import result differs from prepared configuration"));}
                if before_import.account_id!=connection.account_id{return Err(Error::new("identity_mismatch","imported identity changed"));}
            }
            if self.registration.snapshot()?.setup_complete {self.registration.commit_configuration(&prepared.before,&prepared.config)?;}else{self.registration.complete_setup(&prepared.config)?;}
            self.authentication.replace_config(prepared.config.control.clone())?;
            let revision=prepared.config.revision()?;*stored=prepared.config;
            self.resolve_fences(prepared.permission_resolution)?;
            Ok((BTreeMap::from([("config_revision".into(),revision.clone())]),json!({"config_revision":revision,"connections":prepared.observations,"mutation_effect":"configuration_committed"})))
        })
    }
}

pub struct InitialPreparation {
    pub config: Config,
    pub report: Value,
}
/// Local bootstrap preflight; no Keychain/configuration/registration writes.
/// Metadata-only Codex probing uses a private empty context, never a model turn.
pub async fn prepare_initial(
    mut config: Config,
    host: &HostIdentity,
    secrets: &dyn SecretStore,
    inputs: &dyn ConfigurationInputs,
    probe_root: &Path,
) -> Result<InitialPreparation> {
    if config.runtime.host_id != host.host_id
        || config
            .control
            .local_users
            .get(&host.uid.to_string())
            .is_none_or(|operator| !operator.roles.contains(&ControlRole::Configure))
    {
        return Err(Error::new(
            "identity_mismatch",
            "initial configuration must identify this execution host and a local configuring operator",
        ));
    }
    config.resolve_paths(&host.home, |name| inputs.environment(name))?;
    let report = verification_report(&config, secrets, inputs, probe_root).await?;
    if report["configuration_admission"] != "verified" {
        return Err(Error::new(
            "configuration_unverified",
            "initial configuration has blocked identity, scope or workflow checks",
        )
        .detail(report));
    }
    Ok(InitialPreparation { config, report })
}
async fn verification_report(
    config: &Config,
    secrets: &dyn SecretStore,
    inputs: &dyn ConfigurationInputs,
    probe_root: &Path,
) -> Result<Value> {
    config.validate()?;
    inputs.configure_evaluations(config)?;
    let mut checks = Vec::new();
    let mut failed = false;
    let mut delivery_blocked = false;
    match config.load_workflows() {
        Ok(workflows) => {
            checks.push(json!({"check":"workflows","status":"verified","count":workflows.len()}))
        }
        Err(error) => {
            failed = true;
            checks.push(json!({"check":"workflows","status":"blocked","error":error}));
        }
    }
    for (name, connection) in &config.connections {
        let observed=async {
            if connection.kind==ProviderKind::Codex {
                let context=probe_root.join(format!("check-{}-{}",Uuid::new_v4(),canonical_digest(1,name)?.rsplit(':').next().unwrap_or_default()));
                let metadata=inputs.codex(connection,&context).await?;verify_codex(config,name,connection,&metadata)?;
                Ok(json!({"connection":name,"status":"verified","account_id":metadata.account_id,"protocol_profile":metadata.protocol_profile,"permission_scopes":null,"qualification":"metadata_only"}))
            }else{
                let reference=connection.credential_ref.as_deref().ok_or_else(||Error::new("missing_secret","connection credential reference is missing"))?;
                let secret=resolve_reference(reference,secrets,|name|inputs.environment(name))?;
                let verified=verify_credential(connection,secret,Duration::from_millis(config.runtime.request_timeout_ms),inputs.transport(name)).await?;
                for workspace in config.workspaces.values().filter(|workspace|workspace.linear_connection==*name){if verified.identity().workspace_id!=Some(workspace.linear_workspace_id){return Err(Error::new("identity_mismatch","Linear credential belongs to a different configured workspace"));}}
                Ok(json!({"connection":name,"status":"verified","identity":verified.identity(),"qualification":"identity_only"}))
            }
        }.await;
        match observed {
            Ok(value) => checks.push(value),
            Err(error) => {
                failed = true;
                checks.push(json!({"connection":name,"status":"blocked","error":error}));
            }
        }
    }
    for (name, workspace) in &config.workspaces {
        let connection = &config.connections[&workspace.github_connection];
        let observed = async {
            let reference = connection.credential_ref.as_deref().ok_or_else(|| {
                Error::new("missing_secret", "GitHub credential reference is missing")
            })?;
            let secret = resolve_reference(reference, secrets, |name| inputs.environment(name))?;
            let client = GithubClient::with_transport(
                HttpPolicy::new(
                    connection
                        .endpoint
                        .as_deref()
                        .ok_or_else(|| Error::invalid("GitHub endpoint is missing"))?,
                    connection.account_id.clone(),
                    Duration::from_millis(config.runtime.request_timeout_ms),
                )?,
                workspace.github.clone(),
                secret.credential()?,
                inputs.transport(&workspace.github_connection),
            )?;
            repository_prerequisites(&client, workspace).await
        }
        .await;
        match observed {
            Ok(value) => {
                let blocked = value
                    .iter()
                    .any(|repository| repository["delivery_prerequisites"]["status"] == "blocked");
                delivery_blocked |= blocked;
                checks.push(json!({"check":"repository_prerequisites","workspace":name,"status":if blocked {"blocked"} else {"verified"},"configuration_admission":"verified","repositories":value}));
            }
            Err(error) => {
                failed = true;
                checks.push(json!({"check":"repository_prerequisites","workspace":name,"status":"blocked","error":error}));
            }
        }
        for (check, reason) in [
            (
                "hosted_workflow_execution",
                "Pinned trusted workflow preparation and actual GitHub-hosted job proof must be qualified against disposable targets; metadata does not prove execution or checkout",
            ),
            (
                "codex_isolation_and_review",
                "The configured Codex profile requires actual filesystem/control-endpoint isolation and same-account review qualification",
            ),
            (
                "integrated_checks_and_deployment",
                "Configured checks, artifacts and deployment bindings require separate bounded qualification; config check executes none of them",
            ),
            (
                "locked_session_credentials",
                "Keychain, Codex and Tailscale locked-session availability require separately observed native qualification",
            ),
        ] {
            checks.push(
                json!({"check":check,"workspace":name,"status":"unverified","reason":reason}),
            );
        }
    }
    Ok(
        json!({"config_revision":config.revision()?,"status":if failed || delivery_blocked {"blocked"}else{"verified"},"configuration_admission":if failed {"blocked"}else{"verified"},"checks":checks,"operational_qualification":"unverified","qualification_reason":"Identity/profile/workflow metadata checks do not execute hooks, tests, agent turns or qualify live isolation, CI, deployment or human evaluation"}),
    )
}

/// Provider metadata preflight only. It neither issues delivery evidence nor
/// authorizes workflow dispatch, merging, tests or any other effect.
pub async fn repository_prerequisites(
    client: &GithubClient,
    workspace: &Workspace,
) -> Result<Vec<Value>> {
    let repositories: BTreeMap<_, _> = client
        .discover_repositories()
        .await?
        .into_iter()
        .map(|r| (r.identity.id.to_string(), r))
        .collect();
    let mut reports = Vec::new();
    for (id, configured) in &workspace.repositories {
        let repository = repositories.get(id).ok_or_else(|| {
            Error::new(
                "repository_inaccessible",
                "configured repository is absent from complete authorized discovery",
            )
        })?;
        if workspace.evaluation.is_some() {
            if repository.archived
                || repository.disabled
                || repository.permissions.get("pull").and_then(Value::as_bool) != Some(true)
            {
                return Err(Error::new(
                    "repository_inaccessible",
                    "declared evaluation repository lacks observed synthetic read access",
                ));
            }
            reports.push(json!({"repository":id,"kind":"declared_synthetic_evaluation","read_access":true,"production_workflows_verified":false,"native_trial_qualified":false}));
            continue;
        }
        configured.validate_supplied_policy()?;
        if repository.archived
            || repository.disabled
            || repository.permissions.get("pull").and_then(Value::as_bool) != Some(true)
        {
            return Err(Error::new(
                "repository_inaccessible",
                "configured repository requires observed read permission and active state",
            ));
        }
        // Read access admits scoped discovery only. Missing downstream policy or
        // delivery metadata must remain visible without preventing discovery.
        let observed = repository_delivery_prerequisites(client, configured, repository).await;
        let mut report = match observed {
            Ok(value) => value,
            Err(error) => {
                json!({"repository_id":repository.identity.id,"repository":repository.identity,"delivery_prerequisites":{"status":"blocked","error":error},"trusted_reusable_execution":"unverified"})
            }
        };
        report["read_access"] = json!(true);
        reports.push(report);
    }
    Ok(reports)
}
async fn repository_delivery_prerequisites(
    client: &GithubClient,
    configured: &crate::config::Repository,
    repository: &crate::providers::Repository,
) -> Result<Value> {
    let unresolved: Vec<_> = [
        ("ci", configured.ci.is_none()),
        ("production_ci", configured.production_ci.is_none()),
        (
            "no_change_validation",
            configured.no_change_validation.is_none(),
        ),
        ("deployment", configured.deployment.is_none()),
    ]
    .into_iter()
    .filter_map(|(field, missing)| missing.then_some(field))
    .collect();
    if !unresolved.is_empty() {
        return Err(Error::new(
            "delivery_policy_unresolved",
            "repository discovery is available; dependent delivery policy remains unresolved",
        )
        .detail(json!({"unresolved_fields":unresolved,"delivery_metadata_observed":false})));
    }
    if repository.permissions.get("push").and_then(Value::as_bool) != Some(true)
        || repository.squash_allowed != Some(true)
    {
        return Err(Error::new(
            "repository_incompatible",
            "delivery requires observed write permission and allowed squash merging",
        ));
    }
    let workflows: BTreeMap<_, _> = client
        .workflows(&repository.identity)
        .await?
        .into_iter()
        .map(|w| (w.id, w))
        .collect();
    let mut routes = configured
        .staging_ci()?
        .iter()
        .chain(configured.production_ci_policy()?)
        .map(|suite| (suite.workflow_id, &suite.proof))
        .collect::<Vec<_>>();
    routes.push((
        configured.no_change_workflow()?.workflow_id,
        &configured.no_change_workflow()?.proof,
    ));
    if let Deployment::Steps { steps } = configured.deployment_policy()? {
        routes.extend(
            steps
                .iter()
                .map(|step| (step.workflow.workflow_id, &step.workflow.proof)),
        );
    }
    let mut pins = BTreeSet::new();
    for (id, proof) in routes {
        let observed = workflows.get(&id).ok_or_else(|| {
            Error::new(
                "workflow_unavailable",
                "configured workflow ID is absent from complete workflow enumeration",
            )
        })?;
        let caller = proof
            .workflow_ref
            .rsplit_once('@')
            .ok_or_else(|| Error::invalid("workflow pin is malformed"))?
            .0;
        let expected_prefix = format!(
            "{}/{}/",
            repository.identity.owner, repository.identity.name
        );
        let expected_path = caller.strip_prefix(&expected_prefix).ok_or_else(|| {
            Error::new(
                "identity_mismatch",
                "workflow proof names another caller repository",
            )
        })?;
        if observed.data.get("state").and_then(Value::as_str) != Some("active")
            || observed.data.get("path").and_then(Value::as_str) != Some(expected_path)
        {
            return Err(Error::new(
                "workflow_unavailable",
                "configured workflow ID/path is disabled or differs from its pinned caller",
            ));
        }
        if pins.insert(proof.workflow_sha.clone()) {
            client
                .commit(&repository.identity, &proof.workflow_sha)
                .await?;
        }
    }
    let mut branches = Vec::new();
    for (branch, suites) in [
        (&configured.staging, configured.staging_ci()?),
        (&configured.main, configured.production_ci_policy()?),
    ] {
        let base = client
            .reference(&repository.identity, &format!("heads/{branch}"))
            .await?;
        let protection = client.protection(&repository.identity, branch).await?;
        let rules = client.branch_rules(&repository.identity, branch).await?;
        let required = suites
            .iter()
            .flat_map(|suite| suite.required_checks.iter().cloned())
            .chain(std::iter::once(configured.review_context.clone()))
            .collect();
        let identity = crate::git::RepositoryId {
            hostname: configured.hostname.clone(),
            id: repository.identity.id.to_string(),
        };
        let policy = crate::delivery::validate_repository_prerequisites(
            &identity,
            repository,
            branch,
            &base,
            &protection,
            &rules,
            &required,
        )?;
        branches.push(
            serde_json::to_value(policy)
                .map_err(|_| Error::invalid("repository policy cannot be encoded"))?,
        );
    }
    Ok(
        json!({"repository_id":repository.identity.id,"repository":repository.identity,"branches":branches,"workflow_count":workflows.len(),"caller_pins_observed":pins,"delivery_prerequisites":{"status":"verified","qualification":"metadata_only"},"trusted_reusable_execution":"unverified"}),
    )
}

fn credential_environment_names(
    current: &Config,
    request: &CommandRequest,
) -> Result<BTreeSet<String>> {
    let text = |name: &str| {
        request
            .args
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::new(
                    "invalid_input",
                    format!("configuration command needs {name}"),
                )
            })
    };
    let connections: Vec<Connection> = match request.command {
        CommandName::Setup => Config::parse(text("configuration")?)?
            .connections
            .into_values()
            .collect(),
        CommandName::ConnectionAdd => {
            let mut connection: Connection = toml::from_str(text("connection")?)
                .map_err(|_| Error::invalid("connection must be strict typed TOML"))?;
            if let Some(reference) = request.args.get("secret_reference").and_then(Value::as_str) {
                connection.credential_ref = Some(reference.into());
            }
            vec![connection]
        }
        CommandName::ConnectionReconnect => vec![
            current
                .connections
                .get(text("name")?)
                .ok_or_else(|| Error::new("not_found", "connection is not configured"))?
                .clone(),
        ],
        CommandName::WorkspaceAdd | CommandName::WorkspaceConfigure => {
            let workspace: Workspace = toml::from_str(text("workspace_configuration")?)
                .map_err(|_| Error::invalid("workspace must be strict typed TOML"))?;
            [
                workspace.linear_connection,
                workspace.github_connection,
                workspace.codex_connection,
            ]
            .iter()
            .map(|name| {
                current.connections.get(name).cloned().ok_or_else(|| {
                    Error::new("not_found", "workspace connection is not configured")
                })
            })
            .collect::<Result<_>>()?
        }
        _ => return Err(Error::new("invalid_input", "not a configuration command")),
    };
    let mut names = BTreeSet::new();
    for connection in connections {
        if let Some(name) = connection
            .credential_ref
            .as_deref()
            .and_then(|reference| reference.strip_prefix('$'))
        {
            if name.is_empty()
                || !name.bytes().enumerate().all(|(i, c)| {
                    c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
                })
            {
                return Err(Error::invalid(
                    "credential environment reference must be a whole variable name",
                ));
            }
            names.insert(name.into());
        }
    }
    Ok(names)
}

impl crate::app::ConfigurationControl for ConfigManager {
    fn authority_fences(&self) -> Result<AuthorityFences> {
        ConfigManager::authority_fences(self)
    }
    fn snapshot(&self) -> Result<Config> {
        ConfigManager::snapshot(self)
    }
    fn execute(
        &self,
        actor: Actor,
        request: CommandRequest,
        secret: Option<Secret>,
        parent: Receipt,
    ) -> crate::app::HostFuture<'_, Receipt> {
        Box::pin(ConfigManager::execute_for_command(
            self, actor, request, secret, parent,
        ))
    }
    fn check(&self, workspace: Option<Uuid>) -> crate::app::HostFuture<'_, Value> {
        Box::pin(ConfigManager::check(self, workspace))
    }
}
fn observation(
    name: String,
    provider: ProviderKind,
    identity: &AccountIdentity,
    reused: bool,
) -> ConnectionObservation {
    ConnectionObservation {
        connection: name,
        account_id: identity.account_id.clone(),
        provider,
        workspace_id: identity.workspace_id,
        permission_scopes: identity.permission_scopes.clone(),
        protocol_profile: None,
        reused,
    }
}
fn require_current_operator(config: &Config, actor: &Actor) -> Result<()> {
    actor.require("configure")?;
    let operator = match actor.principal.as_ref() {
        Some(Principal::Unix { uid }) => config.control.local_users.get(&uid.to_string()),
        Some(Principal::Tailscale { user_id, .. }) if config.control.https.is_some() => {
            config.control.tailscale_users.get(&user_id.to_string())
        }
        _ => None,
    }
    .ok_or_else(|| {
        Error::new(
            "access_denied",
            "configuration mutation requires a currently mapped OS or Tailscale principal",
        )
    })?;
    if operator.id != actor.identity || !operator.roles.contains(&ControlRole::Configure) {
        return Err(Error::new(
            "access_denied",
            "authenticated principal's current configure role was revoked",
        ));
    }
    Ok(())
}
fn restricted_revision(config: &Config) -> Result<String> {
    // Model choices belong to admission snapshots. Authority and path identity
    // replacements require paused/stopped, drained ownership.
    let connections: BTreeMap<_,_> = config.connections.iter().map(|(name,c)| (name,json!({"kind":c.kind,"account_id":c.account_id,"endpoint":c.endpoint,"auth_home":c.auth_home,"argv":c.argv}))).collect();
    let workspaces: BTreeMap<_,_> = config.workspaces.iter().map(|(name,w)| {
        let repositories: BTreeMap<_,_> = w.repositories.iter().map(|(id,r)| (id,json!({"source":r.source,"hostname":r.hostname,"main":r.main,"staging":r.staging,"ci":r.ci,"production_ci":r.production_ci,"no_change_validation":r.no_change_validation,"deployment":r.deployment}))).collect();
        let value = json!({"id":w.id,"linear_workspace_id":w.linear_workspace_id,"linear_connection":w.linear_connection,"github_connection":w.github_connection,"codex_connection":w.codex_connection,"actors":w.actors,"authorization_label_id":w.linear.authorization_label_id,"states":w.linear.states,"organizations":w.github.organizations,"repositories_allowlist":w.github.repositories,"repositories":repositories,"test_resources":w.test_resources});
        (name, value)
    }).collect();
    canonical_digest(
        1,
        &json!({"schema_version":config.schema_version,"runtime_id":config.runtime.id,"host_id":config.runtime.host_id,"execution_root":config.runtime.execution_root,"connections":connections,"workspaces":workspaces,"control":config.control}),
    )
}

fn verify_codex(
    config: &Config,
    name: &str,
    connection: &Connection,
    metadata: &CodexConnectionMetadata,
) -> Result<()> {
    if metadata.account_id != connection.account_id
        || metadata.supported_version != crate::codex::SUPPORTED_CODEX_VERSION
        || metadata.protocol_profile != crate::codex::PROTOCOL_PROFILE
        || metadata.argument_digest.is_none()
        || connection
            .auth_home
            .as_ref()
            .is_none_or(|home| Path::new(home) != metadata.auth_home)
    {
        return Err(Error::new(
            "identity_mismatch",
            "Codex metadata does not bind the configured account, home and supported protocol invocation",
        ));
    }
    for workspace in config
        .workspaces
        .values()
        .filter(|workspace| workspace.codex_connection == name)
    {
        for profile in std::iter::once(&workspace.profiles.coordination)
            .chain(workspace.profiles.independent_review.iter())
        {
            CodexProfile {
                model: profile.model.clone(),
                reasoning: profile.reasoning.clone(),
                speed: match profile.speed {
                    Speed::Standard => "standard",
                    Speed::Fast => "fast",
                }
                .into(),
            }
            .validate_catalog(&metadata.model_catalog)?;
        }
        for model in workspace.routing_labels.model.values() {
            for reasoning in workspace.routing_labels.reasoning.values() {
                for speed in workspace.routing_labels.speed.values() {
                    CodexProfile {
                        model: model.clone(),
                        reasoning: reasoning.clone(),
                        speed: match speed {
                            Speed::Standard => "standard",
                            Speed::Fast => "fast",
                        }
                        .into(),
                    }
                    .validate_catalog(&metadata.model_catalog)?;
                }
            }
        }
    }
    Ok(())
}

/// Monotone operational restrictions from authenticated configuration proposals.
/// They contain only scope identities and provenance digests, never credentials.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityFences {
    pub workspaces: BTreeSet<Uuid>,
    pub denied_roles: BTreeMap<String, BTreeSet<String>>,
}
impl AuthorityFences {
    pub fn check_workspace(&self, workspace: Uuid) -> Result<()> {
        if self.workspaces.contains(&workspace) {
            return Err(Error::new(
                "permission_fenced",
                "a known permission reduction requires a valid explicit configuration for this workspace",
            ));
        }
        Ok(())
    }
    pub fn check_actor(&self, actor: &Actor, role: &str) -> Result<()> {
        let key = principal_mapping(actor.principal.as_ref());
        if key
            .as_ref()
            .and_then(|key| self.denied_roles.get(key))
            .is_some_and(|roles| roles.contains(role))
        {
            return Err(Error::new(
                "access_denied",
                "a later authenticated configuration proposal revoked this principal's role",
            ));
        }
        Ok(())
    }
    fn merge(&mut self, other: &Self) {
        self.workspaces.extend(&other.workspaces);
        for (key, roles) in &other.denied_roles {
            self.denied_roles
                .entry(key.clone())
                .or_default()
                .extend(roles.iter().cloned());
        }
    }
}
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FenceEntry {
    request_id: Uuid,
    candidate_digest: String,
    restrictions: AuthorityFences,
}
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FenceRecord {
    schema_version: u32,
    runtime_id: Uuid,
    entries: BTreeMap<Uuid, FenceEntry>,
    resolutions: BTreeMap<Uuid, FenceResolution>,
}
#[derive(Clone, Debug, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FenceResolution {
    request_id: Uuid,
    configuration_after: String,
    proposals: BTreeSet<Uuid>,
    all_workspaces: bool,
    workspaces: BTreeSet<Uuid>,
    roles: bool,
}
impl FenceResolution {
    fn new(request: &CommandRequest, config: &Config) -> Result<Self> {
        let workspaces = match request.command {
            CommandName::WorkspaceConfigure => config
                .workspaces
                .iter()
                .filter(|(name, w)| {
                    request.args["workspace"]
                        .as_str()
                        .is_some_and(|v| v == name.as_str() || v == w.id.to_string())
                })
                .map(|(_, w)| w.id)
                .collect(),
            CommandName::ConnectionAdd => config
                .workspaces
                .values()
                .filter(|w| {
                    request.args["name"].as_str().is_some_and(|name| {
                        [
                            w.linear_connection.as_str(),
                            w.github_connection.as_str(),
                            w.codex_connection.as_str(),
                        ]
                        .contains(&name)
                    })
                })
                .map(|w| w.id)
                .collect(),
            _ => BTreeSet::new(),
        };
        Ok(Self {
            request_id: request.request_id,
            configuration_after: config.revision()?,
            proposals: BTreeSet::new(),
            all_workspaces: request.command == CommandName::Setup,
            workspaces,
            roles: request.command == CommandName::Setup,
        })
    }
}
fn principal_mapping(principal: Option<&Principal>) -> Option<String> {
    match principal {
        Some(Principal::Unix { uid }) => Some(format!("unix:{uid}")),
        Some(Principal::Tailscale { user_id, .. }) => Some(format!("tailscale:{user_id}")),
        None => None,
    }
}
fn removed_values(before: Option<&Value>, after: Option<&Value>) -> bool {
    match (
        before.and_then(Value::as_array),
        after.and_then(Value::as_array),
    ) {
        (Some(old), Some(new)) if new.iter().all(Value::is_string) => {
            old.iter().any(|value| !new.contains(value))
        }
        _ => false,
    }
}
fn scalar_changed(before: Option<&Value>, after: Option<&Value>) -> bool {
    after.is_some_and(Value::is_string) && before != after
}
fn workspace_reduction(before: &Value, after: &Value) -> bool {
    for key in [
        "id",
        "linear_workspace_id",
        "linear_connection",
        "github_connection",
        "codex_connection",
    ] {
        if scalar_changed(before.get(key), after.get(key)) {
            return true;
        }
    }
    for role in ["authorizers", "correctors", "acceptors"] {
        if removed_values(
            before.get("actors").and_then(|v| v.get(role)),
            after.get("actors").and_then(|v| v.get(role)),
        ) {
            return true;
        }
    }
    for field in ["organizations", "repositories"] {
        if removed_values(
            before.get("github").and_then(|v| v.get(field)),
            after.get("github").and_then(|v| v.get(field)),
        ) {
            return true;
        }
    }
    if scalar_changed(
        before.pointer("/linear/authorization_label_id"),
        after.pointer("/linear/authorization_label_id"),
    ) {
        return true;
    }
    for collection in ["repositories", "test_resources"] {
        if let (Some(old), Some(new)) = (
            before.get(collection).and_then(Value::as_object),
            after.get(collection).and_then(Value::as_object),
        ) {
            for (name, entry) in old {
                let Some(replacement) = new.get(name) else {
                    return true;
                };
                for field in if collection == "repositories" {
                    &["source", "hostname", "main", "staging"][..]
                } else {
                    &["path"][..]
                } {
                    if scalar_changed(entry.get(*field), replacement.get(*field)) {
                        return true;
                    }
                }
                if collection == "test_resources"
                    && ["writable", "disposable"].iter().any(|key| {
                        entry.get(*key) == Some(&Value::Bool(true))
                            && replacement.get(*key) == Some(&Value::Bool(false))
                    })
                {
                    return true;
                }
                if collection == "repositories"
                    && ["ci", "production_ci", "no_change_validation", "deployment"]
                        .iter()
                        .any(|field| {
                            entry.get(*field).is_some()
                                && entry.get(*field) != replacement.get(*field)
                        })
                {
                    return true;
                }
            }
        }
    }
    false
}
/// Parsing TOML as data preserves duplicate-key rejection while permitting a
/// reliable restriction to be observed despite an unrelated typed-field error.
fn proposed_reductions(current: &Config, request: &CommandRequest) -> Result<AuthorityFences> {
    let mut out = AuthorityFences::default();
    // Compare the same TOML representation on both sides. JSON serialization
    // retains nested Option::None as null while TOML omits it; that difference
    // alone is not a permission reduction. Keep the proposal untyped below so
    // a real removal still fences authority despite unrelated invalid fields.
    let current_toml = toml::Value::try_from(current)
        .map_err(|_| Error::invalid("configuration encoding failed"))?;
    let current_value = serde_json::to_value(current_toml)
        .map_err(|_| Error::invalid("configuration encoding failed"))?;
    let parse = |key: &str| -> Option<Value> {
        let text = request.args.get(key)?.as_str()?;
        let value: toml::Value = toml::from_str(text).ok()?;
        serde_json::to_value(value).ok()
    };
    match request.command {
        CommandName::WorkspaceConfigure => {
            let Some(next) = parse("workspace_configuration") else {
                return Ok(out);
            };
            let reference = request
                .args
                .get("workspace")
                .and_then(Value::as_str)
                .unwrap_or("");
            for (name, workspace) in &current.workspaces {
                if (name == reference || workspace.id.to_string() == reference)
                    && workspace_reduction(&current_value["workspaces"][name], &next)
                {
                    out.workspaces.insert(workspace.id);
                }
            }
        }
        CommandName::Setup => {
            let Some(next) = parse("configuration") else {
                return Ok(out);
            };
            if let Some(workspaces) = next.get("workspaces").and_then(Value::as_object) {
                for (name, workspace) in &current.workspaces {
                    if workspaces.get(name).is_none_or(|candidate| {
                        workspace_reduction(&current_value["workspaces"][name], candidate)
                    }) {
                        out.workspaces.insert(workspace.id);
                    }
                }
            }
            for (kind, label) in [("local_users", "unix"), ("tailscale_users", "tailscale")] {
                if let (Some(old), Some(new)) = (
                    current_value["control"]
                        .get(kind)
                        .and_then(Value::as_object),
                    next.get("control")
                        .and_then(|c| c.get(kind))
                        .and_then(Value::as_object),
                ) {
                    for (id, operator) in old {
                        let roles = operator["roles"]
                            .as_array()
                            .ok_or_else(|| Error::invalid("operator roles malformed"))?;
                        let replacement = new.get(id);
                        let changed = replacement.is_none_or(|value| {
                            scalar_changed(operator.get("id"), value.get("id"))
                                || scalar_changed(
                                    operator.get("linear_user_id"),
                                    value.get("linear_user_id"),
                                )
                        });
                        let allowed = replacement
                            .and_then(|v| v.get("roles"))
                            .and_then(Value::as_array);
                        for role in roles.iter().filter_map(Value::as_str) {
                            if changed
                                || allowed.is_some_and(|values| !values.contains(&json!(role)))
                            {
                                out.denied_roles
                                    .entry(format!("{label}:{id}"))
                                    .or_default()
                                    .insert(role.into());
                            }
                        }
                    }
                }
            }
            if current.control.https.is_some()
                && next
                    .get("control")
                    .is_some_and(|c| c.get("https").is_none())
            {
                for (id, operator) in &current.control.tailscale_users {
                    out.denied_roles
                        .entry(format!("tailscale:{id}"))
                        .or_default()
                        .extend(operator.roles.iter().map(|r| {
                            serde_json::to_value(r)
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned()
                        }));
                }
            }
            if scalar_changed(
                current_value.pointer("/runtime/execution_root"),
                next.pointer("/runtime/execution_root"),
            ) {
                out.workspaces
                    .extend(current.workspaces.values().map(|w| w.id));
            }
            if let Some(connections) = next.get("connections").and_then(Value::as_object) {
                for (name, old) in &current.connections {
                    let prior = serde_json::to_value(old)
                        .map_err(|_| Error::invalid("connection encoding"))?;
                    if connections.get(name).is_none_or(|new| {
                        ["kind", "account_id", "endpoint", "auth_home"]
                            .iter()
                            .any(|field| scalar_changed(prior.get(*field), new.get(*field)))
                            || new
                                .get("argv")
                                .is_some_and(|argv| prior.get("argv") != Some(argv))
                    }) {
                        out.workspaces.extend(
                            current
                                .workspaces
                                .values()
                                .filter(|w| {
                                    [
                                        &w.linear_connection,
                                        &w.github_connection,
                                        &w.codex_connection,
                                    ]
                                    .contains(&name)
                                })
                                .map(|w| w.id),
                        );
                    }
                }
            }
        }
        CommandName::ConnectionAdd => {
            let Some(next) = parse("connection") else {
                return Ok(out);
            };
            let name = request
                .args
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("");
            if let Some(old) = current_value["connections"].get(name)
                && (["kind", "account_id", "endpoint", "auth_home"]
                    .iter()
                    .any(|field| scalar_changed(old.get(*field), next.get(*field)))
                    || next
                        .get("argv")
                        .is_some_and(|argv| old.get("argv") != Some(argv)))
            {
                out.workspaces.extend(
                    current
                        .workspaces
                        .values()
                        .filter(|w| {
                            [
                                w.linear_connection.as_str(),
                                w.github_connection.as_str(),
                                w.codex_connection.as_str(),
                            ]
                            .contains(&name)
                        })
                        .map(|w| w.id),
                );
            }
        }
        _ => (),
    }
    Ok(out)
}
impl ConfigManager {
    pub fn authority_fences(&self) -> Result<AuthorityFences> {
        self.fences_except(None)
    }
    fn fences_except(&self, request: Option<Uuid>) -> Result<AuthorityFences> {
        let fences = self.fences.read().map_err(|_| {
            Error::new(
                "permission_fenced",
                "permission restriction state unavailable",
            )
        })?;
        let mut out = AuthorityFences::default();
        for (id, entry) in &fences.entries {
            if Some(*id) != request {
                let mut restrictions = entry.restrictions.clone();
                for resolution in fences
                    .resolutions
                    .values()
                    .filter(|r| r.proposals.contains(id))
                {
                    restrictions.workspaces.retain(|id| {
                        !resolution.all_workspaces && !resolution.workspaces.contains(id)
                    });
                    if resolution.roles {
                        restrictions.denied_roles.clear();
                    }
                }
                out.merge(&restrictions);
            }
        }
        Ok(out)
    }
    fn record_reductions(
        &self,
        current: &Config,
        actor: &Actor,
        request: &CommandRequest,
        parent: Option<&(Uuid, String)>,
    ) -> Result<()> {
        let restrictions = proposed_reductions(current, request)?;
        if restrictions.workspaces.is_empty() && restrictions.denied_roles.is_empty() {
            return Ok(());
        }
        let entry = FenceEntry {
            request_id: request.request_id,
            candidate_digest: canonical_digest(1, &request.args)?,
            restrictions,
        };
        let request_id = fence_operation(request.request_id);
        let mut expected = BTreeMap::from([("configuration_before".into(), current.revision()?)]);
        expected.insert(
            "permission_restriction".into(),
            canonical_digest(1, &entry)?,
        );
        if let Some((id, digest)) = parent {
            expected.insert("parent_command_id".into(), id.to_string());
            expected.insert("parent_payload_digest".into(), digest.clone());
        }
        let mutation = MutationRequest {
            request_id,
            runtime_id: current.runtime.id,
            actor: actor.clone(),
            kind: "permission_fence".into(),
            canonical_targets: vec![current.runtime.id.to_string()],
            expected,
            payload: json!({"command":request.command,"candidate_digest":canonical_digest(1,&request.args)?,"restrictions":entry.restrictions}),
        };
        self.control.record_permission_reduction(
            mutation,
            |snapshot| {
                require_current_operator(&self.snapshot()?, actor)?;
                self.fences_except(Some(request.request_id))?
                    .check_actor(actor, "configure")?;
                if self.snapshot()?.revision()? != current.revision()?
                    || request
                        .expected_config_revision
                        .as_ref()
                        .is_some_and(|r| current.revision().as_ref() != Ok(r))
                {
                    return Err(Error::new(
                        "stale_config",
                        "permission proposal does not bind current configuration",
                    ));
                }
                if request
                    .expected_control_generation
                    .is_some_and(|g| g != snapshot.intent.generation)
                    || parent.is_some_and(|(id, _)| !snapshot.background_commands.contains(id))
                {
                    return Err(Error::new(
                        "stale_generation",
                        "permission proposal command is no longer current",
                    ));
                }
                Ok(())
            },
            |_| {
                let mut stored = self.fences.write().map_err(|_| {
                    Error::new(
                        "permission_fenced",
                        "permission restriction state unavailable",
                    )
                })?;
                stored.entries.insert(request.request_id, entry);
                self.registration
                    .root()
                    .write("permission-fences.json", &*stored)?;
                Ok((
                    BTreeMap::new(),
                    json!({"permission_reductions":"fenced","configuration_committed":false}),
                ))
            },
        )?;
        Ok(())
    }
    fn resolve_fences(&self, resolution: FenceResolution) -> Result<()> {
        let mut stored = self.fences.write().map_err(|_| {
            Error::new(
                "permission_fenced",
                "permission restriction state unavailable",
            )
        })?;
        stored.resolutions.insert(resolution.request_id, resolution);
        self.registration
            .root()
            .write("permission-fences.json", &*stored)
    }
}
fn fence_operation(request: Uuid) -> Uuid {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(format!("kegari:v0:permission-fence:{request}").as_bytes());
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&hash[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}
