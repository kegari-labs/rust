//! Client-device entry wiring. OS account lookup does not probe a runtime, boot
//! identity, credentials or launchd unless the operator selects a local action.
use crate::{
    cli::{GuidanceRequest, LocalActions, LocalCommand, LocalFuture, OutputEnvelope},
    client::{ControlClient, Endpoint, Target, TargetStore},
};
use kegari_config::{Config, Connection, ControlRole, Workspace};
use kegari_platform::{AccountHome, HostIdentity};
use kegari_storage::PrivateDir;
use kegari_types::api::{CommandName, CommandRequest};
use kegari_types::error::{Error, Result};
use kegari_v0::platform::{self, OwnerRecord, ServiceStartState};
use serde_json::json;
use std::{
    collections::BTreeSet,
    io::{BufRead, IsTerminal, Read, Write},
    path::PathBuf,
};
use uuid::Uuid;

pub const UNIX_AUTHORITY: &str = "kegari.local";
pub const UNIX_SOCKET: &str = "control.sock";

/// Client preferences are the only files created while selecting a remote target.
pub fn target_store(account: &AccountHome) -> Result<TargetStore> {
    Ok(TargetStore::new(PrivateDir::create(
        &account.home.join(".kegari"),
    )?))
}
/// Read a local registration without an environment-derived HOME or a boot
/// identity query. Partial registration is an error, never a bootstrap signal.
pub fn local_registration(account: &AccountHome) -> Result<Option<OwnerRecord>> {
    let path = account.home.join(".kegari");
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io("inspect local registration directory", error)),
        Ok(_) => {}
    }
    let root = PrivateDir::open(&path)?;
    let owner = match root.read::<OwnerRecord>("owner.json") {
        Ok(owner) => owner,
        Err(error) if error.code == "record_missing" => {
            for name in ["config.toml", "intent.json"] {
                match root.read_bytes(name, 1024 * 1024) {
                    Ok(_) => {
                        return Err(Error::new(
                            "setup_incomplete",
                            "partial local registration requires explicit recovery",
                        ));
                    }
                    Err(error) if error.code == "record_missing" => (),
                    Err(error) => return Err(error),
                }
            }
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if owner.uid != account.uid || owner.root != path || owner.runtime_id.is_nil() {
        return Err(Error::new(
            "identity_mismatch",
            "local runtime registration differs from this OS account",
        ));
    }
    if !owner.setup_complete || owner.config_revision.is_none() {
        return Err(Error::new(
            "setup_incomplete",
            "local setup is incomplete; preserve its records and reconcile it explicitly",
        ));
    }
    Ok(Some(owner))
}
pub fn registered_target(account: &AccountHome, targets: &TargetStore) -> Result<Option<Target>> {
    let Some(owner) = local_registration(account)? else {
        return Ok(None);
    };
    let target = Target {
        name: "local".into(),
        runtime_id: owner.runtime_id,
        api_version: 1,
        endpoint: Endpoint::Unix {
            socket: owner.root.join(UNIX_SOCKET),
            authority: UNIX_AUTHORITY.into(),
        },
    };
    targets.register_local(target.clone())?;
    Ok(Some(target))
}
pub struct NativeLocalActions {
    account: AccountHome,
}
impl NativeLocalActions {
    pub fn new(account: AccountHome) -> Self {
        Self { account }
    }
    fn host(&self) -> Result<HostIdentity> {
        let host = HostIdentity::discover()?;
        if host.uid != self.account.uid || host.home != self.account.home {
            return Err(Error::new(
                "identity_mismatch",
                "OS account changed during local action",
            ));
        }
        Ok(host)
    }
}
impl LocalActions for NativeLocalActions {
    fn registered_control(&mut self) -> Result<bool> {
        Ok(local_registration(&self.account)?.is_some())
    }
    fn bootstrap<'a>(
        &'a mut self,
        configuration: &'a str,
        request_id: Uuid,
    ) -> LocalFuture<'a, OutputEnvelope> {
        Box::pin(async move {
            let runtime = Config::parse(configuration)?.runtime.id;
            let result = kegari_controller::app::bootstrap(configuration, request_id).await?;
            if result
                .data
                .get("runtime_id")
                .and_then(serde_json::Value::as_str)
                != Some(runtime.to_string().as_str())
            {
                return Err(Error::new(
                    "identity_mismatch",
                    "bootstrap returned a different runtime identity",
                ));
            }
            let mut output = OutputEnvelope::success(request_id, Some(runtime), result.data);
            output.config_revision = result.config_revision;
            output.operation_id = result.operation_id;
            output.state = result.state;
            // The committed local registration, rather than an endpoint response,
            // is the authority for this pin. Failure retains the setup receipt.
            if let Err(error) = target_store(&self.account)
                .and_then(|targets| registered_target(&self.account, &targets))
            {
                output.ok = false;
                output.error = Some(error);
            }
            Ok(output)
        })
    }
    fn service<'a>(
        &'a mut self,
        command: LocalCommand,
        request_id: Uuid,
    ) -> LocalFuture<'a, OutputEnvelope> {
        Box::pin(async move {
            let host = self.host()?;
            match command {
                LocalCommand::ServiceInstall { binary } => {
                    let installed = platform::install_service(&host, &binary).await?;
                    let runtime = local_registration(&self.account)?.map(|owner| owner.runtime_id);
                    let mut output = OutputEnvelope::success(
                        request_id,
                        runtime,
                        json!({"service":installed,"loaded":false}),
                    );
                    output.state = Some("installed".into());
                    Ok(output)
                }
                LocalCommand::ServiceStart => {
                    let result = platform::start_service(&host).await?;
                    let (owner, state) = match result {
                        ServiceStartState::AlreadyRunning(owner) => (owner, "already_running"),
                        ServiceStartState::Ready(owner) => (owner, "start_requested"),
                    };
                    let mut output = OutputEnvelope::success(
                        request_id,
                        Some(owner.runtime_id),
                        json!({"service_state":state,"runtime_build":owner.runtime_build,"next_action":"Inspect status for observed control-host readiness; orchestration intent is preserved"}),
                    );
                    output.state = Some(state.into());
                    Ok(output)
                }
                LocalCommand::ServiceStop => {
                    let targets = target_store(&self.account)?;
                    let target = registered_target(&self.account, &targets)?.ok_or_else(|| {
                        Error::new("target_unavailable", "no local runtime is registered")
                    })?;
                    let client = ControlClient::new(target.clone())?;
                    let mut request = CommandRequest {
                        api_version: 1,
                        runtime_id: target.runtime_id,
                        request_id,
                        command: CommandName::ServiceStop,
                        args: json!({}),
                        expected_config_revision: None,
                        expected_control_generation: None,
                    };
                    let mut output: OutputEnvelope =
                        client.execute(&mut request, None).await.into();
                    if output.ok {
                        match platform::finish_service_stop(&host).await {
                            Ok(()) => {
                                output.state = Some("service_stopped".into());
                                output.data["service_stopped"] = json!(true);
                            }
                            Err(error) => {
                                output.ok = false;
                                output.error = Some(error);
                            }
                        }
                    }
                    Ok(output)
                }
                LocalCommand::TargetList | LocalCommand::TargetAdd(_) => Err(Error::new(
                    "invalid_input",
                    "target preferences are handled by the invoking client",
                )),
            }
        })
    }
    fn guide<'a>(&'a mut self, request: &'a GuidanceRequest) -> LocalFuture<'a, String> {
        Box::pin(async move {
            if !std::io::stdin().is_terminal() {
                return Err(Error::new(
                    "invalid_input",
                    "guided input requires an interactive terminal; use --from and --noninteractive",
                ));
            }
            guided_document(request, std::io::stdin().lock(), std::io::stderr().lock())
        })
    }
    fn guide_initial_setup(&mut self) -> LocalFuture<'_, String> {
        Box::pin(async move {
            if !std::io::stdin().is_terminal() {
                return Err(Error::new(
                    "invalid_input",
                    "guided input requires an interactive terminal; use --from and --noninteractive",
                ));
            }
            if self.registered_control()? {
                return Err(Error::new(
                    "setup_incomplete",
                    "a local registration appeared before guidance; inspect it before configuring the existing runtime",
                ));
            }
            let host = self.host()?;
            guided_initial_setup(&host, std::io::stdin().lock(), std::io::stderr().lock())
        })
    }
    fn open_browser<'a>(&'a mut self, url: &'a str) -> LocalFuture<'a, ()> {
        Box::pin(crate::cli::open_client_browser(url))
    }
}

struct SetupPrompt<R, W> {
    input: R,
    output: W,
    bytes_read: usize,
}
impl<R: BufRead, W: Write> SetupPrompt<R, W> {
    fn ask(&mut self, prompt: &str) -> Result<String> {
        write!(self.output, "{prompt}")
            .and_then(|_| self.output.flush())
            .map_err(|error| Error::io("write setup prompt", error))?;
        let mut bytes = Vec::new();
        let limit = 4096usize.min(crate::cli::MAX_CONFIG.saturating_sub(self.bytes_read));
        let count = (&mut self.input)
            .take((limit + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .map_err(|error| Error::io("read setup answer", error))?;
        if count == 0 {
            return Err(setup_cancelled());
        }
        if count > limit {
            return Err(Error::new(
                "invalid_input",
                "setup answer exceeds its bounded input limit",
            ));
        }
        if bytes.last() != Some(&b'\n') {
            return Err(setup_cancelled());
        }
        self.bytes_read += count;
        let answer = String::from_utf8(bytes)
            .map_err(|_| Error::new("invalid_input", "setup answer must be UTF-8"))?;
        let answer = answer.trim_end_matches(['\r', '\n']);
        if answer.chars().any(char::is_control) {
            return Err(Error::new(
                "invalid_input",
                "setup answer contains control characters",
            ));
        }
        let answer = answer.trim();
        if answer == "/cancel" {
            return Err(setup_cancelled());
        }
        Ok(answer.to_owned())
    }
}
fn setup_cancelled() -> Error {
    Error::new(
        "cancelled",
        "setup cancelled before submission; no registration, credentials or service changes were made",
    )
}

/// Build and review a minimal initial configuration using an already observed
/// host. This pure input/output step does not register, authenticate or persist;
/// bootstrap independently verifies the actual host and applies the typed config.
pub fn guided_initial_setup(
    host: &HostIdentity,
    input: impl BufRead,
    output: impl Write,
) -> Result<String> {
    let mut prompt = SetupPrompt {
        input,
        output,
        bytes_read: 0,
    };
    writeln!(prompt.output,
        "Initial local setup. Enter /cancel at any prompt or end input to cancel.\nDo not enter secrets. The host identity and OS user mapping come from this host; a new runtime UUID will be generated.\nThis initial configuration has no connections, workspaces, remote control, repository selection or release scope. These must be configured explicitly afterward.")
        .map_err(|error| Error::io("write setup introduction", error))?;
    let operator = prompt.ask("Operator label (non-secret, required): ")?;
    if operator.is_empty() || operator.len() > 128 {
        return Err(Error::invalid("operator label must contain 1 to 128 bytes"));
    }
    let linear =
        prompt.ask("Your Linear user UUID (operator-provided; not yet provider-verified): ")?;
    let linear: Uuid = linear
        .parse()
        .map_err(|_| Error::invalid("Linear user identity must be a UUID"))?;
    if linear.is_nil() {
        return Err(Error::invalid("Linear user UUID must not be nil"));
    }
    let agents = prompt.ask("Maximum simultaneous agents (positive integer; no default): ")?;
    let agents: u32 = agents
        .parse()
        .map_err(|_| Error::invalid("maximum agents must be a positive integer"))?;
    if agents == 0 {
        return Err(Error::invalid("maximum agents must be positive"));
    }
    let execution_root =
        prompt.ask("Execution root (absolute path or ~/ path) [~/.kegari/runs]: ")?;
    let execution_root = if execution_root.is_empty() {
        "~/.kegari/runs"
    } else {
        &execution_root
    };
    let defaults = "inspect,configure,lifecycle,correct,accept,release,recover";
    let roles = prompt.ask(&format!(
        "Local roles, comma-separated [{defaults}]; evaluate is optional, configure is required: "
    ))?;
    let roles = if roles.is_empty() { defaults } else { &roles };
    let mut selected = BTreeSet::new();
    for name in roles.split(',').map(str::trim) {
        let role: ControlRole = serde_json::from_value(json!(name))
            .map_err(|_| Error::invalid("local roles must use the listed exact role names"))?;
        if !selected.insert(role) {
            return Err(Error::invalid("local roles must not repeat"));
        }
    }
    if !selected.contains(&ControlRole::Configure) {
        return Err(Error::invalid(
            "initial setup operator requires the configure role",
        ));
    }
    let mut config: Config = serde_json::from_value(json!({
        "schema_version": 1,
        "runtime": {"id": Uuid::new_v4(), "host_id": host.host_id,
            "execution_root": execution_root, "max_agents": agents},
        "connections": {}, "workspaces": {},
        "control": {"local_users": {host.uid.to_string(): {
            "id": operator, "linear_user_id": linear, "roles": selected
        }}}
    }))
    .map_err(|_| Error::invalid("guided setup could not produce the typed configuration"))?;
    // Resolve the displayed path now; guided input never silently adopts an
    // environment value. --from retains the existing documented path handling.
    config.resolve_paths(&host.home, |_| None)?;
    let document = toml::to_string_pretty(&config)
        .map_err(|_| Error::invalid("guided setup configuration could not be encoded"))?;
    Config::parse(&document)?;
    writeln!(prompt.output,
        "\nReview the complete initial configuration (no provider credentials):\n{document}\nThe Linear UUID above is supplied by you; a configured Linear connection must verify the provider identity.\nConfirming submits this configuration to the ordinary setup transaction: local registration and a macOS Keychain attestation key, with STOPPED intent. It does not install/start a service or dispatch agents.\nAfter setup: explicitly install/start the registered service, then use connection add with supported authentication and workspace add with exact IDs, selectors, profiles, repository authority, hosted-validation policy and an explicit release policy. Run status and config check before considering orchestration start. No scope or provider qualification is inferred from this draft.")
        .map_err(|error| Error::io("write setup review", error))?;
    if prompt.ask("Type create to submit this reviewed setup, or /cancel: ")? != "create" {
        return Err(setup_cancelled());
    }
    Ok(document)
}
/// A bounded document editor for the same typed configuration used by --from.
/// Input is never logged or echoed by this function; secrets use separate stdin
/// import, and only non-secret references belong in configuration documents.
pub fn guided_document(
    request: &GuidanceRequest,
    mut input: impl BufRead,
    mut output: impl Write,
) -> Result<String> {
    let fields = match request {
        GuidanceRequest::Setup => {
            "schema_version, [runtime] with explicit host/id/limits, [connections], [workspaces], and [control] operator mappings"
        }
        GuidanceRequest::ConnectionAdd { .. } => {
            "kind, account_id, supported endpoint/credential_ref or Codex auth_home/argv, and optional max_agents"
        }
        GuidanceRequest::WorkspaceAdd { .. } => {
            "id, connection names, optional workflow path (omit for installed generic guidance), exact Linear/GitHub scope, actors, profiles, routing labels, limits, release policy, and repositories"
        }
    };
    writeln!(output, "Enter the typed TOML document for {fields}.\nUse credential references only; import credentials separately with --secret-stdin.\nFinish with a line containing only a period. No changes are sent until the whole document validates.")
        .map_err(|error| Error::io("write configuration guidance", error))?;
    output
        .flush()
        .map_err(|error| Error::io("flush configuration guidance", error))?;
    let mut bytes = Vec::new();
    let mut finished = false;
    loop {
        let mut line = Vec::new();
        let remaining = crate::cli::MAX_CONFIG.saturating_sub(bytes.len());
        let count = (&mut input)
            .take((remaining + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|error| Error::io("read guided configuration", error))?;
        if count == 0 {
            break;
        }
        if line == b".\n" || line == b".\r\n" || line == b"." {
            finished = true;
            break;
        }
        if count > remaining {
            return Err(Error::new(
                "invalid_input",
                "guided configuration exceeds 1 MiB",
            ));
        }
        bytes.extend(line);
    }
    if !finished {
        return Err(Error::new(
            "invalid_input",
            "guided input ended before the terminating period; nothing was submitted",
        ));
    }
    let document = String::from_utf8(bytes)
        .map_err(|_| Error::new("invalid_input", "guided configuration must be UTF-8"))?;
    match request {
        GuidanceRequest::Setup => {
            Config::parse(&document)?;
        }
        GuidanceRequest::ConnectionAdd { kind, .. } => {
            let value: Connection = toml::from_str(&document).map_err(|_| {
                Error::new(
                    "invalid_config",
                    "connection TOML does not match the typed schema",
                )
            })?;
            if value.kind != *kind {
                return Err(Error::new(
                    "invalid_config",
                    "connection kind differs from requested kind",
                ));
            }
        }
        GuidanceRequest::WorkspaceAdd { .. } => {
            let _: Workspace = toml::from_str(&document).map_err(|_| {
                Error::new(
                    "invalid_config",
                    "workspace TOML does not match the typed schema",
                )
            })?;
        }
    }
    Ok(document)
}

/// Minimal diagnostics derived from arguments without echoing their values.
/// Used only when Clap cannot produce a typed invocation.
pub fn output_hints(arguments: &[std::ffi::OsString]) -> (bool, Option<String>, Uuid) {
    let mut json_mode = false;
    let mut target = None;
    let mut request = None;
    let mut args = arguments.iter().skip(1);
    while let Some(value) = args.next().and_then(|value| value.to_str()) {
        match value {
            "--json" => json_mode = true,
            "--target" => {
                target = args
                    .next()
                    .and_then(|value| value.to_str())
                    .map(str::to_owned)
            }
            "--request-id" => {
                request = args
                    .next()
                    .and_then(|value| value.to_str())
                    .and_then(|value| value.parse().ok())
            }
            value if value.starts_with("--target=") => target = Some(value[9..].into()),
            value if value.starts_with("--request-id=") => request = value[13..].parse().ok(),
            _ => (),
        }
    }
    (json_mode, target, request.unwrap_or_else(Uuid::new_v4))
}

pub fn state_root(account: &AccountHome) -> PathBuf {
    account.home.join(".kegari")
}

/// Parse, prepare and execute once. Reads never bootstrap a runtime. A mutation
/// timeout returns the same request identity without replaying the command.
pub async fn run_cli(arguments: Vec<std::ffi::OsString>) -> i32 {
    use crate::cli::{Action, Cli, DeviceInput, write_output};
    use clap::Parser;
    let (json_mode, target_hint, request_hint) = output_hints(&arguments);
    let cli = match Cli::try_parse_from(arguments) {
        Ok(cli) => cli,
        Err(error) => {
            let informational = matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            );
            if informational && !json_mode {
                return if error.print().is_ok() { 0 } else { 3 };
            }
            let response = if informational {
                OutputEnvelope::success(request_hint, None, json!({"help":error.to_string()}))
            } else {
                // Clap diagnostics can echo arbitrary correction/secret-bearing
                // argument values. Never emit those diagnostics into output/logs.
                OutputEnvelope::failure(
                    request_hint,
                    None,
                    Error::new(
                        "invalid_command",
                        "command or arguments are invalid; use --help for the accepted grammar",
                    ),
                )
            };
            let exit = response.exit_code();
            return if write_output(
                &response,
                json_mode,
                target_hint.as_deref(),
                std::io::stdout().lock(),
            )
            .is_ok()
            {
                exit
            } else {
                3
            };
        }
    };
    let json_mode = cli.json;
    let target = cli.target.clone();
    let invocation = match cli.prepare(&mut DeviceInput) {
        Ok(value) => value,
        Err(error) => {
            let response = OutputEnvelope::failure(request_hint, None, error);
            let exit = response.exit_code();
            return if write_output(
                &response,
                json_mode,
                target.as_deref(),
                std::io::stdout().lock(),
            )
            .is_ok()
            {
                exit
            } else {
                3
            };
        }
    };
    let id = invocation.request_id;
    let output = async {
        let account = kegari_platform::account_home()?;
        let targets = target_store(&account)?;
        if invocation.target.is_none()
            && matches!(invocation.action, Action::Server(_) | Action::Setup { .. })
        {
            registered_target(&account, &targets)?;
        }
        let mut local = NativeLocalActions::new(account);
        Ok::<_, Error>(crate::cli::execute(invocation, &targets, &mut local).await)
    }
    .await
    .unwrap_or_else(|error| OutputEnvelope::failure(id, None, error));
    let exit = output.exit_code();
    if write_output(
        &output,
        json_mode,
        target.as_deref(),
        std::io::stdout().lock(),
    )
    .is_err()
    {
        eprintln!("Cannot write command response; retain the request identity for reconciliation.");
        return 3;
    }
    exit
}
