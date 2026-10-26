//! Complete command grammar and client-device input handling. Parsing never starts work.
use crate::api::{CommandName, CommandRequest, Envelope};
use crate::client::{API_VERSION, ControlClient, Endpoint, Target, TargetStore};
use crate::config::{Config, Connection, ProviderKind, Workspace};
use crate::error::{Error, Result};
use crate::secrets::Secret;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::future::Future;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use uuid::Uuid;

pub const MAX_CORRECTION: usize = 1024 * 1024;
pub const MAX_CONFIG: usize = 1024 * 1024;
pub const MAX_SECRET: usize = 64 * 1024;
#[derive(Parser, Debug)]
#[command(
    name = "kegari-v0",
    version = concat!(env!("CARGO_PKG_VERSION"), " ", env!("KEGARI_BUILD_ID")),
    about = "Kegari v0 scoped delivery control"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub target: Option<String>,
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    Overview(WorkspaceFilter),
    Status,
    Show(ParentRead),
    Open(Open),
    Approve(ResultMutation),
    Correct(Correct),
    Release(Release),
    Setup(Setup),
    #[command(subcommand)]
    Connection(ConnectionCommand),
    #[command(subcommand)]
    Workspace(WorkspaceCommand),
    #[command(subcommand)]
    Config(ConfigCommand),
    Pause(Lifecycle),
    Resume(Lifecycle),
    Stop(Stop),
    Start(Start),
    #[command(subcommand)]
    Operation(OperationCommand),
    Retry(Retry),
    Cleanup(Cleanup),
    #[command(subcommand)]
    Evaluation(EvaluationCommand),
    #[command(subcommand)]
    Target(TargetCommand),
    #[command(subcommand)]
    Service(ServiceCommand),
}
#[derive(Subcommand, Debug)]
pub enum EvaluationCommand {
    Run(EvaluationRun),
}
#[derive(Args, Debug)]
pub struct EvaluationRun {
    #[arg(long)]
    pub workspace: Uuid,
    #[arg(long)]
    pub suite_digest: String,
    #[arg(long)]
    pub case: String,
    #[arg(long)]
    pub variant: String,
    #[arg(long)]
    pub profile: String,
    #[arg(long)]
    pub repetition: u32,
    #[arg(long)]
    pub expected_config: String,
    #[arg(long)]
    pub expected_control: u64,
    #[command(flatten)]
    pub mutation: Mutation,
}
#[derive(Args, Debug, Default)]
pub struct WorkspaceFilter {
    #[arg(long)]
    pub workspace: Option<String>,
}
#[derive(Args, Debug)]
pub struct ParentRead {
    pub parent: String,
    #[arg(long)]
    pub workspace: Option<String>,
}
#[derive(Args, Debug)]
pub struct Open {
    pub parent: String,
    #[arg(long)]
    pub workspace: Option<String>,
    #[arg(long)]
    pub url_only: bool,
}
#[derive(Args, Debug, Default)]
pub struct Mutation {
    #[arg(long)]
    pub request_id: Option<Uuid>,
}
#[derive(Args, Debug)]
pub struct ResultMutation {
    pub parent: String,
    #[arg(long)]
    pub result: String,
    #[command(flatten)]
    pub mutation: Mutation,
}
#[derive(Args, Debug)]
#[group(required=true,multiple=false,args=["message","message_file","stdin"])]
pub struct CorrectionInput {
    #[arg(long)]
    pub message: Option<String>,
    #[arg(long)]
    pub message_file: Option<PathBuf>,
    #[arg(long)]
    pub stdin: bool,
}
#[derive(Args, Debug)]
pub struct Correct {
    pub parent: String,
    #[command(flatten)]
    pub input: CorrectionInput,
    #[command(flatten)]
    pub mutation: Mutation,
}
#[derive(Args, Debug)]
pub struct Release {
    pub parent: Option<String>,
    #[arg(long)]
    pub result: Option<String>,
    #[command(flatten)]
    pub mutation: Mutation,
    #[command(subcommand)]
    pub action: Option<ReleaseCommand>,
}
#[derive(Subcommand, Debug)]
pub enum ReleaseCommand {
    Recover(ReleaseRecover),
}
#[derive(Clone, Copy, Debug, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    Resume,
    Abandon,
    Resolve,
    Supersede,
}
#[derive(Args, Debug)]
pub struct ReleaseRecover {
    pub parent: String,
    #[arg(long)]
    pub operation: Uuid,
    #[arg(long, value_enum)]
    pub action: RecoveryAction,
    #[arg(long)]
    pub reason: String,
    #[arg(long)]
    pub result: Option<String>,
    #[command(flatten)]
    pub mutation: Mutation,
}
#[derive(Args, Debug)]
pub struct Setup {
    #[arg(long)]
    pub from: Option<PathBuf>,
    #[arg(long)]
    pub noninteractive: bool,
    #[command(flatten)]
    pub mutation: Mutation,
}
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Kind {
    Linear,
    Github,
    Codex,
}
impl Kind {
    fn provider(self) -> ProviderKind {
        match self {
            Self::Linear => ProviderKind::Linear,
            Self::Github => ProviderKind::Github,
            Self::Codex => ProviderKind::Codex,
        }
    }
}
#[derive(Subcommand, Debug)]
pub enum ConnectionCommand {
    List,
    Add(ConnectionAdd),
    Reconnect(ConnectionReconnect),
}
#[derive(Args, Debug)]
pub struct ConnectionAdd {
    pub name: String,
    #[arg(long, value_enum)]
    pub kind: Kind,
    #[arg(long)]
    pub from: Option<PathBuf>,
    #[arg(long)]
    pub secret_stdin: bool,
    #[arg(long)]
    pub noninteractive: bool,
    #[command(flatten)]
    pub mutation: Mutation,
}
#[derive(Args, Debug)]
pub struct ConnectionReconnect {
    pub name: String,
    #[arg(long)]
    pub noninteractive: bool,
    #[command(flatten)]
    pub mutation: Mutation,
}
#[derive(Subcommand, Debug)]
pub enum WorkspaceCommand {
    List,
    Add(WorkspaceAdd),
    Configure(WorkspaceConfigure),
}
#[derive(Args, Debug)]
pub struct WorkspaceAdd {
    pub name: String,
    #[arg(long)]
    pub from: Option<PathBuf>,
    #[arg(long)]
    pub noninteractive: bool,
    #[command(flatten)]
    pub mutation: Mutation,
}
#[derive(Args, Debug)]
pub struct WorkspaceConfigure {
    pub id: String,
    #[arg(long)]
    pub from: PathBuf,
    #[arg(long)]
    pub expected_config: String,
    #[arg(long)]
    pub noninteractive: bool,
    #[command(flatten)]
    pub mutation: Mutation,
}
#[derive(Args, Debug)]
pub struct ConfigFilter {
    #[arg(long)]
    pub workspace: Option<String>,
    #[arg(long)]
    pub noninteractive: bool,
}
#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    Show(ConfigFilter),
    Check(ConfigFilter),
}
#[derive(Args, Debug)]
pub struct Lifecycle {
    #[arg(long)]
    pub expected_control: Option<u64>,
    #[command(flatten)]
    pub mutation: Mutation,
}
#[derive(Args, Debug)]
pub struct Wait {
    #[arg(long)]
    pub wait: bool,
    #[arg(long,requires="wait",value_parser=clap::value_parser!(u64).range(1..=86400))]
    pub timeout: Option<u64>,
}
#[derive(Args, Debug)]
pub struct Start {
    #[command(flatten)]
    pub lifecycle: Lifecycle,
    #[command(flatten)]
    pub waiting: Wait,
}
#[derive(Args, Debug)]
pub struct Stop {
    #[arg(long)]
    pub force: bool,
    #[command(flatten)]
    pub lifecycle: Lifecycle,
    #[command(flatten)]
    pub waiting: Wait,
}
#[derive(Subcommand, Debug)]
pub enum OperationCommand {
    Show {
        operation: Uuid,
    },
    Reconcile {
        operation: Uuid,
        #[command(flatten)]
        mutation: Mutation,
    },
}
#[derive(Args, Debug)]
pub struct Retry {
    pub work: String,
    #[arg(long)]
    pub reason: String,
    #[command(flatten)]
    pub mutation: Mutation,
}
#[derive(Args, Debug)]
pub struct Cleanup {
    #[arg(long)]
    pub operation: Uuid,
    #[arg(long, requires = "plan")]
    pub apply: bool,
    #[arg(long, requires = "apply")]
    pub plan: Option<String>,
    #[command(flatten)]
    pub mutation: Mutation,
}
#[derive(Subcommand, Debug)]
pub enum TargetCommand {
    List,
    Add {
        name: String,
        #[arg(long)]
        url: String,
        #[arg(long)]
        runtime: Uuid,
        #[command(flatten)]
        mutation: Mutation,
    },
}
#[derive(Subcommand, Debug)]
pub enum ServiceCommand {
    Install {
        #[arg(long)]
        binary: PathBuf,
        #[command(flatten)]
        mutation: Mutation,
    },
    Start(Mutation),
    Stop(Mutation),
}

pub trait InputSource {
    fn stdin_is_terminal(&self) -> bool;
    fn read_stdin(&mut self, maximum: usize) -> Result<Vec<u8>>;
    fn read_file(&mut self, path: &Path, maximum: usize) -> Result<Vec<u8>>;
}
pub struct DeviceInput;
fn read_bounded(reader: impl Read, maximum: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read CLI input", e))?;
    if bytes.len() > maximum {
        return Err(Error::new(
            "invalid_input",
            "CLI input exceeds its declared byte limit",
        ));
    }
    Ok(bytes)
}
impl InputSource for DeviceInput {
    fn stdin_is_terminal(&self) -> bool {
        std::io::stdin().is_terminal()
    }
    fn read_stdin(&mut self, maximum: usize) -> Result<Vec<u8>> {
        read_bounded(std::io::stdin().lock(), maximum)
    }
    fn read_file(&mut self, path: &Path, maximum: usize) -> Result<Vec<u8>> {
        let file =
            std::fs::File::open(path).map_err(|e| Error::io("read client-device input file", e))?;
        read_bounded(file, maximum)
    }
}
fn text(bytes: Vec<u8>, name: &str) -> Result<String> {
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::new("invalid_input", format!("{name} must be UTF-8")))?;
    if text.trim().is_empty() || text.contains('\0') {
        return Err(Error::new(
            "invalid_input",
            format!("{name} must be nonempty and contain no NUL"),
        ));
    }
    Ok(text)
}
fn read_from(source: &mut dyn InputSource, path: &Path, limit: usize) -> Result<String> {
    text(
        if path == Path::new("-") {
            source.read_stdin(limit)?
        } else {
            source.read_file(path, limit)?
        },
        "configuration",
    )
}
fn nonempty(value: &str, name: &str) -> Result<()> {
    if value.trim().is_empty() || value.contains('\0') {
        Err(Error::new(
            "invalid_input",
            format!("{name} must be nonempty"),
        ))
    } else {
        Ok(())
    }
}
fn work_reference(value: &str) -> Result<()> {
    nonempty(value, "work reference")?;
    if value.len() > 1024
        || value.chars().any(char::is_control)
        || value.split('/').any(|part| part.trim().is_empty())
        || value.matches('/').count() > 1
    {
        return Err(Error::new(
            "invalid_input",
            "use an unqualified unique work key or WORKSPACE/ISSUE reference",
        ));
    }
    Ok(())
}
#[derive(Clone)]
pub struct ServerCommand {
    pub command: CommandName,
    pub args: Value,
    pub expected_config_revision: Option<String>,
    pub expected_control_generation: Option<u64>,
    pub wait_seconds: Option<u64>,
    pub browser: bool,
    pub url_only: bool,
    pub noninteractive: bool,
}
impl std::fmt::Debug for ServerCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerCommand")
            .field("command", &self.command)
            .field("args", &"[OMITTED]")
            .field("expected_config_revision", &self.expected_config_revision)
            .field(
                "expected_control_generation",
                &self.expected_control_generation,
            )
            .finish_non_exhaustive()
    }
}
impl ServerCommand {
    fn new(command: CommandName, args: Value) -> Self {
        Self {
            command,
            args,
            expected_config_revision: None,
            expected_control_generation: None,
            wait_seconds: None,
            browser: false,
            url_only: false,
            noninteractive: false,
        }
    }
    pub fn request(&self, runtime: Uuid, request_id: Uuid) -> CommandRequest {
        CommandRequest {
            api_version: API_VERSION,
            runtime_id: runtime,
            request_id,
            command: self.command.clone(),
            args: self.args.clone(),
            expected_config_revision: self.expected_config_revision.clone(),
            expected_control_generation: self.expected_control_generation,
        }
    }
}
#[derive(Debug)]
pub enum LocalCommand {
    TargetList,
    TargetAdd(Target),
    ServiceInstall { binary: PathBuf },
    ServiceStart,
    ServiceStop,
}
#[derive(Debug)]
pub enum GuidanceRequest {
    Setup,
    ConnectionAdd { name: String, kind: ProviderKind },
    WorkspaceAdd { name: String },
}
pub enum Action {
    Server(ServerCommand),
    Setup {
        configuration: String,
        noninteractive: bool,
    },
    Local(LocalCommand),
    Guided(GuidanceRequest),
}
impl std::fmt::Debug for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Server(command) => command.fmt(f),
            Self::Setup { .. } => f.write_str("Setup([OMITTED])"),
            Self::Local(command) => command.fmt(f),
            Self::Guided(request) => request.fmt(f),
        }
    }
}
pub struct PreparedInvocation {
    pub request_id: Uuid,
    pub target: Option<String>,
    pub json: bool,
    pub action: Action,
    pub secret: Option<Secret>,
}
impl std::fmt::Debug for PreparedInvocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedInvocation")
            .field("request_id", &self.request_id)
            .field("target", &self.target)
            .field("json", &self.json)
            .field("action", &self.action)
            .field("secret", &self.secret)
            .finish()
    }
}
impl Cli {
    pub fn prepare(self, source: &mut dyn InputSource) -> Result<PreparedInvocation> {
        let mut request_id = None;
        let mut secret = None;
        let command = self
            .command
            .unwrap_or(Command::Overview(WorkspaceFilter::default()));
        let action = match command {
            Command::Overview(filter) => Action::Server(ServerCommand::new(
                CommandName::Overview,
                filter_args(filter.workspace),
            )),
            Command::Status => Action::Server(ServerCommand::new(CommandName::Status, json!({}))),
            Command::Show(parent) => {
                work_reference(&parent.parent)?;
                let mut args = filter_args(parent.workspace);
                args["parent"] = json!(parent.parent);
                Action::Server(ServerCommand::new(CommandName::Show, args))
            }
            Command::Open(parent) => {
                work_reference(&parent.parent)?;
                let mut args = filter_args(parent.workspace);
                args["parent"] = json!(parent.parent);
                let mut command = ServerCommand::new(CommandName::Open, args);
                command.browser = !parent.url_only;
                command.url_only = parent.url_only;
                Action::Server(command)
            }
            Command::Approve(value) => {
                work_reference(&value.parent)?;
                nonempty(&value.result, "result token")?;
                request_id = value.mutation.request_id;
                Action::Server(ServerCommand::new(
                    CommandName::Approve,
                    json!({"parent":value.parent,"result":value.result}),
                ))
            }
            Command::Correct(value) => {
                work_reference(&value.parent)?;
                request_id = value.mutation.request_id;
                let message = match (
                    value.input.message,
                    value.input.message_file,
                    value.input.stdin,
                ) {
                    (Some(message), None, false) => {
                        if message.len() > MAX_CORRECTION {
                            return Err(Error::new("invalid_input", "correction exceeds 1 MiB"));
                        }
                        text(message.into_bytes(), "correction")?
                    }
                    (None, Some(path), false) => {
                        text(source.read_file(&path, MAX_CORRECTION)?, "correction")?
                    }
                    (None, None, true) => text(source.read_stdin(MAX_CORRECTION)?, "correction")?,
                    _ => {
                        return Err(Error::new(
                            "invalid_input",
                            "exactly one correction input is required",
                        ));
                    }
                };
                Action::Server(ServerCommand::new(
                    CommandName::Correct,
                    json!({"parent":value.parent,"message":message}),
                ))
            }
            Command::Release(value) => {
                if let Some(ReleaseCommand::Recover(recovery)) = value.action {
                    if value.parent.is_some()
                        || value.result.is_some()
                        || value.mutation.request_id.is_some()
                    {
                        return Err(Error::new(
                            "invalid_input",
                            "release options must belong to either admission or recovery",
                        ));
                    }
                    work_reference(&recovery.parent)?;
                    not_nil(recovery.operation)?;
                    nonempty(&recovery.reason, "recovery reason")?;
                    if matches!(recovery.action, RecoveryAction::Supersede)
                        && recovery
                            .result
                            .as_ref()
                            .is_none_or(|result| result.trim().is_empty())
                    {
                        return Err(Error::new(
                            "invalid_input",
                            "supersede requires the current corrected result token",
                        ));
                    }
                    request_id = recovery.mutation.request_id;
                    let mut args = json!({"parent":recovery.parent,"operation":recovery.operation,"action":recovery.action,"reason":recovery.reason});
                    if let Some(result) = recovery.result {
                        nonempty(&result, "result token")?;
                        args["result"] = json!(result);
                    }
                    Action::Server(ServerCommand::new(CommandName::ReleaseRecover, args))
                } else {
                    let parent = value
                        .parent
                        .ok_or_else(|| Error::new("invalid_input", "release requires a parent"))?;
                    work_reference(&parent)?;
                    let result = value.result.ok_or_else(|| {
                        Error::new(
                            "invalid_input",
                            "release requires the exact accepted result token",
                        )
                    })?;
                    nonempty(&result, "result token")?;
                    request_id = value.mutation.request_id;
                    Action::Server(ServerCommand::new(
                        CommandName::Release,
                        json!({"parent":parent,"result":result}),
                    ))
                }
            }
            Command::Setup(value) => {
                request_id = value.mutation.request_id;
                match value.from {
                    Some(path) => {
                        let configuration = read_from(source, &path, MAX_CONFIG)?;
                        Config::parse(&configuration)?;
                        Action::Setup {
                            configuration,
                            noninteractive: value.noninteractive,
                        }
                    }
                    None => {
                        require_guidance(source, value.noninteractive)?;
                        Action::Guided(GuidanceRequest::Setup)
                    }
                }
            }
            Command::Connection(ConnectionCommand::List) => {
                Action::Server(ServerCommand::new(CommandName::ConnectionList, json!({})))
            }
            Command::Connection(ConnectionCommand::Reconnect(value)) => {
                nonempty(&value.name, "connection name")?;
                request_id = value.mutation.request_id;
                let mut command = ServerCommand::new(
                    CommandName::ConnectionReconnect,
                    json!({"name":value.name}),
                );
                command.noninteractive = value.noninteractive;
                Action::Server(command)
            }
            Command::Connection(ConnectionCommand::Add(value)) => {
                nonempty(&value.name, "connection name")?;
                request_id = value.mutation.request_id;
                if value.secret_stdin && value.from.as_deref() == Some(Path::new("-")) {
                    return Err(Error::new(
                        "invalid_input",
                        "configuration and secret must not share stdin",
                    ));
                }
                let action = match value.from {
                    Some(path) => {
                        let connection = read_from(source, &path, MAX_CONFIG)?;
                        let parsed: Connection = toml::from_str(&connection).map_err(|_| {
                            Error::new(
                                "invalid_config",
                                "connection input must be typed connection TOML",
                            )
                        })?;
                        if parsed.kind != value.kind.provider() {
                            return Err(Error::new(
                                "invalid_config",
                                "connection kind differs from --kind",
                            ));
                        }
                        let mut command = ServerCommand::new(
                            CommandName::ConnectionAdd,
                            json!({"name":value.name,"connection":connection}),
                        );
                        command.noninteractive = value.noninteractive;
                        Action::Server(command)
                    }
                    None => {
                        require_guidance(source, value.noninteractive)?;
                        Action::Guided(GuidanceRequest::ConnectionAdd {
                            name: value.name,
                            kind: value.kind.provider(),
                        })
                    }
                };
                if value.secret_stdin {
                    if source.stdin_is_terminal() {
                        return Err(Error::new(
                            "invalid_input",
                            "--secret-stdin requires protected non-terminal input; terminal echo is not disabled",
                        ));
                    }
                    let mut bytes = source.read_stdin(MAX_SECRET)?;
                    if bytes.last() == Some(&b'\n') {
                        bytes.pop();
                        if bytes.last() == Some(&b'\r') {
                            bytes.pop();
                        }
                    }
                    std::str::from_utf8(&bytes)
                        .map_err(|_| Error::new("invalid_input", "secret input must be UTF-8"))?;
                    secret = Some(Secret::new(bytes)?);
                }
                action
            }
            Command::Workspace(WorkspaceCommand::List) => {
                Action::Server(ServerCommand::new(CommandName::WorkspaceList, json!({})))
            }
            Command::Workspace(WorkspaceCommand::Add(value)) => {
                nonempty(&value.name, "workspace name")?;
                request_id = value.mutation.request_id;
                match value.from {
                    Some(path) => {
                        let configuration = workspace_input(source, &path)?;
                        let mut command = ServerCommand::new(
                            CommandName::WorkspaceAdd,
                            json!({"name":value.name,"workspace_configuration":configuration}),
                        );
                        command.noninteractive = value.noninteractive;
                        Action::Server(command)
                    }
                    None => {
                        require_guidance(source, value.noninteractive)?;
                        Action::Guided(GuidanceRequest::WorkspaceAdd { name: value.name })
                    }
                }
            }
            Command::Workspace(WorkspaceCommand::Configure(value)) => {
                nonempty(&value.id, "workspace identity")?;
                nonempty(&value.expected_config, "expected config revision")?;
                request_id = value.mutation.request_id;
                let configuration = workspace_input(source, &value.from)?;
                let mut command = ServerCommand::new(
                    CommandName::WorkspaceConfigure,
                    json!({"workspace":value.id,"workspace_configuration":configuration}),
                );
                command.expected_config_revision = Some(value.expected_config);
                command.noninteractive = value.noninteractive;
                Action::Server(command)
            }
            Command::Config(ConfigCommand::Show(filter)) => Action::Server(ServerCommand::new(
                CommandName::ConfigShow,
                filter_args(filter.workspace),
            )),
            Command::Config(ConfigCommand::Check(filter)) => Action::Server(ServerCommand::new(
                CommandName::ConfigCheck,
                filter_args(filter.workspace),
            )),
            Command::Pause(value) => {
                request_id = value.mutation.request_id;
                let mut command = ServerCommand::new(CommandName::Pause, json!({}));
                command.expected_control_generation = value.expected_control;
                Action::Server(command)
            }
            Command::Resume(value) => {
                request_id = value.mutation.request_id;
                let mut command = ServerCommand::new(CommandName::Resume, json!({}));
                command.expected_control_generation = value.expected_control;
                Action::Server(command)
            }
            Command::Start(value) => {
                request_id = value.lifecycle.mutation.request_id;
                Action::Server(lifecycle(
                    CommandName::Start,
                    value.lifecycle.expected_control,
                    value.waiting,
                    false,
                ))
            }
            Command::Stop(value) => {
                request_id = value.lifecycle.mutation.request_id;
                Action::Server(lifecycle(
                    CommandName::Stop,
                    value.lifecycle.expected_control,
                    value.waiting,
                    value.force,
                ))
            }
            Command::Operation(OperationCommand::Show { operation }) => {
                not_nil(operation)?;
                Action::Server(ServerCommand::new(
                    CommandName::OperationShow,
                    json!({"operation":operation}),
                ))
            }
            Command::Operation(OperationCommand::Reconcile {
                operation,
                mutation,
            }) => {
                not_nil(operation)?;
                request_id = mutation.request_id;
                Action::Server(ServerCommand::new(
                    CommandName::OperationReconcile,
                    json!({"operation":operation}),
                ))
            }
            Command::Evaluation(EvaluationCommand::Run(value)) => {
                not_nil(value.workspace)?;
                crate::evidence::validate_digest(&value.suite_digest)?;
                nonempty(&value.expected_config, "expected config")?;
                if value.repetition == 0 {
                    return Err(Error::new(
                        "invalid_input",
                        "evaluation repetition must be positive",
                    ));
                }
                request_id = value.mutation.request_id;
                let mut command = ServerCommand::new(
                    CommandName::EvaluationRun,
                    json!({"workspace":value.workspace,"suite_digest":value.suite_digest,"case":value.case,"variant":value.variant,"profile":value.profile,"repetition":value.repetition}),
                );
                command.expected_config_revision = Some(value.expected_config);
                command.expected_control_generation = Some(value.expected_control);
                Action::Server(command)
            }
            Command::Retry(value) => {
                work_reference(&value.work)?;
                nonempty(&value.reason, "retry reason")?;
                request_id = value.mutation.request_id;
                Action::Server(ServerCommand::new(
                    CommandName::Retry,
                    json!({"work":value.work,"reason":value.reason}),
                ))
            }
            Command::Cleanup(value) => {
                not_nil(value.operation)?;
                if value.apply != value.plan.is_some() {
                    return Err(Error::new(
                        "invalid_input",
                        "cleanup apply requires its unchanged inspected plan",
                    ));
                }
                request_id = value.mutation.request_id;
                let mut args = json!({"operation":value.operation,"apply":value.apply});
                if let Some(plan) = value.plan {
                    nonempty(&plan, "cleanup plan")?;
                    args["plan"] = json!(plan);
                }
                Action::Server(ServerCommand::new(CommandName::Cleanup, args))
            }
            Command::Target(TargetCommand::List) => {
                local_only(&self.target)?;
                Action::Local(LocalCommand::TargetList)
            }
            Command::Target(TargetCommand::Add {
                name,
                url,
                runtime,
                mutation,
            }) => {
                local_only(&self.target)?;
                request_id = mutation.request_id;
                let target = Target {
                    name,
                    runtime_id: runtime,
                    api_version: API_VERSION,
                    endpoint: Endpoint::Https { origin: url },
                };
                target.validate()?;
                Action::Local(LocalCommand::TargetAdd(target))
            }
            Command::Service(ServiceCommand::Install { binary, mutation }) => {
                local_only(&self.target)?;
                if !binary.is_absolute() {
                    return Err(Error::new(
                        "invalid_input",
                        "service install needs an absolute installed binary path",
                    ));
                }
                request_id = mutation.request_id;
                Action::Local(LocalCommand::ServiceInstall { binary })
            }
            Command::Service(ServiceCommand::Start(mutation)) => {
                local_only(&self.target)?;
                request_id = mutation.request_id;
                Action::Local(LocalCommand::ServiceStart)
            }
            Command::Service(ServiceCommand::Stop(mutation)) => {
                local_only(&self.target)?;
                request_id = mutation.request_id;
                Action::Local(LocalCommand::ServiceStop)
            }
        };
        let request_id = request_id.unwrap_or_else(Uuid::new_v4);
        not_nil(request_id)?;
        Ok(PreparedInvocation {
            request_id,
            target: self.target,
            json: self.json,
            action,
            secret,
        })
    }
}
fn filter_args(workspace: Option<String>) -> Value {
    workspace.map_or_else(|| json!({}), |workspace| json!({"workspace":workspace}))
}
fn require_guidance(source: &dyn InputSource, noninteractive: bool) -> Result<()> {
    if noninteractive || !source.stdin_is_terminal() {
        Err(Error::new(
            "invalid_config",
            "noninteractive setup/configuration needs complete typed input; no prompt was opened",
        ))
    } else {
        Ok(())
    }
}
fn workspace_input(source: &mut dyn InputSource, path: &Path) -> Result<String> {
    let text = read_from(source, path, MAX_CONFIG)?;
    let _: Workspace = toml::from_str(&text).map_err(|_| {
        Error::new(
            "invalid_config",
            "workspace input must be typed workspace TOML",
        )
    })?;
    Ok(text)
}
fn not_nil(id: Uuid) -> Result<()> {
    if id.is_nil() {
        Err(Error::new("invalid_input", "UUID must not be nil"))
    } else {
        Ok(())
    }
}
fn local_only(target: &Option<String>) -> Result<()> {
    if target.is_some() {
        Err(Error::new(
            "invalid_input",
            "this client/local-service command cannot target a remote runtime",
        ))
    } else {
        Ok(())
    }
}
fn lifecycle(
    name: CommandName,
    generation: Option<u64>,
    waiting: Wait,
    force: bool,
) -> ServerCommand {
    let seconds = waiting.timeout.unwrap_or(30);
    let mut args = json!({"wait":waiting.wait,"timeout_seconds":seconds});
    if name == CommandName::Stop {
        args["force"] = json!(force);
    }
    let mut command = ServerCommand::new(name, args);
    command.expected_control_generation = generation;
    command.wait_seconds = waiting.wait.then_some(seconds);
    command
}

/// Client-only output has no fabricated runtime identity before registration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputEnvelope {
    pub api_version: u32,
    pub request_id: Uuid,
    pub runtime_id: Option<Uuid>,
    pub config_revision: Option<String>,
    pub observed_at: chrono::DateTime<chrono::Utc>,
    pub ok: bool,
    pub operation_id: Option<Uuid>,
    pub state: Option<String>,
    pub data: Value,
    pub error: Option<Error>,
}
impl From<Envelope> for OutputEnvelope {
    fn from(value: Envelope) -> Self {
        Self {
            api_version: value.api_version,
            request_id: value.request_id,
            runtime_id: Some(value.runtime_id),
            config_revision: value.config_revision,
            observed_at: value.observed_at,
            ok: value.ok,
            operation_id: value.operation_id,
            state: value.state,
            data: value.data,
            error: value.error,
        }
    }
}
impl OutputEnvelope {
    pub fn success(request_id: Uuid, runtime: Option<Uuid>, data: Value) -> Self {
        Self {
            api_version: API_VERSION,
            request_id,
            runtime_id: runtime,
            config_revision: None,
            observed_at: chrono::Utc::now(),
            ok: true,
            operation_id: None,
            state: None,
            data,
            error: None,
        }
    }
    pub fn failure(request_id: Uuid, runtime: Option<Uuid>, error: Error) -> Self {
        Self {
            ok: false,
            error: Some(error),
            ..Self::success(request_id, runtime, Value::Null)
        }
    }
    pub fn exit_code(&self) -> i32 {
        if let Some(error) = &self.error {
            return error.exit_code();
        }
        if self
            .state
            .as_deref()
            .is_some_and(|state| ["failed", "partial", "unknown", "uncertain"].contains(&state))
        {
            return 7;
        }
        if self.ok { 0 } else { 7 }
    }
}
pub type LocalFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
/// Host-specific operations stay on the invoking device. The implementation must
/// distinguish absent setup from partial/stale registration and hold control.lock
/// during bootstrap. These interfaces confer no permission by themselves.
pub trait LocalActions {
    fn registered_control(&mut self) -> Result<bool>;
    fn bootstrap<'a>(
        &'a mut self,
        configuration: &'a str,
        request_id: Uuid,
    ) -> LocalFuture<'a, OutputEnvelope>;
    fn service<'a>(
        &'a mut self,
        command: LocalCommand,
        request_id: Uuid,
    ) -> LocalFuture<'a, OutputEnvelope>;
    fn guide<'a>(&'a mut self, request: &'a GuidanceRequest) -> LocalFuture<'a, String>;
    /// Initial local guidance may derive the invoking host's identity. Existing
    /// and remote configurations must continue through ordinary typed guidance.
    fn guide_initial_setup(&mut self) -> LocalFuture<'_, String> {
        self.guide(&GuidanceRequest::Setup)
    }
    fn open_browser<'a>(&'a mut self, url: &'a str) -> LocalFuture<'a, ()>;
}
fn finish_guidance(request: GuidanceRequest, configuration: String) -> Result<Action> {
    match request {
        GuidanceRequest::Setup => {
            Config::parse(&configuration)?;
            Ok(Action::Setup {
                configuration,
                noninteractive: false,
            })
        }
        GuidanceRequest::ConnectionAdd { name, kind } => {
            let parsed: Connection = toml::from_str(&configuration).map_err(|_| {
                Error::new(
                    "invalid_config",
                    "guided connection must use the same typed TOML schema",
                )
            })?;
            if parsed.kind != kind {
                return Err(Error::new(
                    "invalid_config",
                    "guided connection kind differs from requested kind",
                ));
            }
            Ok(Action::Server(ServerCommand::new(
                CommandName::ConnectionAdd,
                json!({"name":name,"connection":configuration}),
            )))
        }
        GuidanceRequest::WorkspaceAdd { name } => {
            let _: Workspace = toml::from_str(&configuration).map_err(|_| {
                Error::new(
                    "invalid_config",
                    "guided workspace must use the same typed TOML schema",
                )
            })?;
            Ok(Action::Server(ServerCommand::new(
                CommandName::WorkspaceAdd,
                json!({"name":name,"workspace_configuration":configuration}),
            )))
        }
    }
}
/// Execute one prepared invocation. Main owns paths, terminal detection and output.
/// This function never installs a service unless that explicit local command was parsed.
pub async fn execute(
    mut invocation: PreparedInvocation,
    targets: &TargetStore,
    local: &mut dyn LocalActions,
) -> OutputEnvelope {
    let id = invocation.request_id;
    let mut initial_local_setup = false;
    if let Action::Guided(request) = invocation.action {
        initial_local_setup =
            if matches!(request, GuidanceRequest::Setup) && invocation.target.is_none() {
                match local.registered_control() {
                    Ok(registered) => !registered,
                    Err(error) => return OutputEnvelope::failure(id, None, error),
                }
            } else {
                false
            };
        let guided = if initial_local_setup {
            local.guide_initial_setup().await
        } else {
            local.guide(&request).await
        };
        invocation.action = match guided.and_then(|text| finish_guidance(request, text)) {
            Ok(action) => action,
            Err(error) => return OutputEnvelope::failure(id, None, error),
        };
    }
    let command = match invocation.action {
        Action::Local(LocalCommand::TargetList) => {
            return match targets.list() {
                Ok(values) => OutputEnvelope::success(id, None, json!({"targets":values})),
                Err(error) => OutputEnvelope::failure(id, None, error),
            };
        }
        Action::Local(LocalCommand::TargetAdd(target)) => {
            // The operator's explicit pin is verified before saving a preference.
            let client = match ControlClient::new(target.clone()) {
                Ok(client) => client,
                Err(error) => return OutputEnvelope::failure(id, Some(target.runtime_id), error),
            };
            let request =
                ServerCommand::new(CommandName::Status, json!({})).request(target.runtime_id, id);
            let observation = client.read_complete(&request).await;
            if !observation.ok {
                return observation.into();
            }
            return match targets.add(target.clone()) {
                Ok(()) => OutputEnvelope::success(
                    id,
                    Some(target.runtime_id),
                    json!({"target":target,"verified":true}),
                ),
                Err(error) => OutputEnvelope::failure(id, Some(target.runtime_id), error),
            };
        }
        Action::Local(command) => {
            return local
                .service(command, id)
                .await
                .unwrap_or_else(|error| OutputEnvelope::failure(id, None, error));
        }
        Action::Setup {
            configuration,
            noninteractive,
        } => {
            if invocation.target.is_none() {
                match local.registered_control() {
                    Ok(false) => {
                        return local
                            .bootstrap(&configuration, id)
                            .await
                            .unwrap_or_else(|error| OutputEnvelope::failure(id, None, error));
                    }
                    Ok(true) if initial_local_setup => {
                        return OutputEnvelope::failure(
                            id,
                            None,
                            Error::new(
                                "setup_incomplete",
                                "a local registration appeared during initial setup review; the generated draft was not submitted",
                            )
                            .next("Inspect the registered runtime before explicitly configuring it; do not retry the initial draft as a replacement"),
                        );
                    }
                    Ok(true) => {}
                    Err(error) => return OutputEnvelope::failure(id, None, error),
                }
            }
            let mut command =
                ServerCommand::new(CommandName::Setup, json!({"configuration":configuration}));
            command.noninteractive = noninteractive;
            command
        }
        Action::Server(command) => command,
        Action::Guided(_) => {
            return OutputEnvelope::failure(
                id,
                None,
                Error::new(
                    "invalid_input",
                    "guided input did not yield a concrete typed command",
                ),
            );
        }
    };
    let target = match targets.select(invocation.target.as_deref()) {
        Ok(target) => target,
        Err(error) => return OutputEnvelope::failure(id, None, error),
    };
    let runtime = target.runtime_id;
    let client = match ControlClient::new(target) {
        Ok(client) => client,
        Err(error) => return OutputEnvelope::failure(id, Some(runtime), error),
    };
    let mut request = command.request(runtime, id);
    let mut response = client
        .execute(&mut request, invocation.secret.as_ref())
        .await;
    if let Some(seconds) = command.wait_seconds
        && response.ok
    {
        response = client.wait_operation(response, seconds).await;
    }
    if command.command == CommandName::Open && response.ok {
        let url = response
            .data
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned);
        match url
            .as_deref()
            .ok_or_else(|| {
                Error::new(
                    "incomplete_snapshot",
                    "parent open response lacks its Linear URL",
                )
            })
            .and_then(validate_linear_url)
        {
            Ok(()) => {
                if command.browser
                    && local
                        .open_browser(url.as_deref().expect("validated URL"))
                        .await
                        .is_err()
                {
                    response.ok = false;
                    response.error=Some(Error::new("target_unavailable","Linear URL was obtained but the reviewing client's browser could not open it; workflow was unchanged").next("Open the returned Linear URL manually"));
                }
            }
            Err(error) => {
                response.ok = false;
                response.error = Some(error);
            }
        }
    }
    response.into()
}
pub fn validate_linear_url(text: &str) -> Result<()> {
    let url =
        url::Url::parse(text).map_err(|_| Error::new("invalid_input", "invalid Linear URL"))?;
    if url.scheme() != "https"
        || url.host_str() != Some("linear.app")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::new(
            "invalid_input",
            "open requires the authoritative HTTPS Linear URL",
        ));
    }
    Ok(())
}
/// Native client browser operation; no shell, and never requested on the remote host.
pub async fn open_client_browser(url: &str) -> Result<()> {
    validate_linear_url(url)?;
    #[cfg(target_os = "macos")]
    let mut command = tokio::process::Command::new("/usr/bin/open");
    #[cfg(not(target_os = "macos"))]
    let mut command = tokio::process::Command::new("xdg-open");
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let cutoff = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let result = tokio::time::timeout_at(tokio::time::Instant::from_std(cutoff), async {
        let mut child = crate::process_spawn::spawn_tokio(&mut command, Some(cutoff))?;
        child.wait().await
    })
    .await
    .map_err(|_| Error::new("timeout", "client browser launch timed out"))?
    .map_err(|error| {
        if error.kind() == std::io::ErrorKind::TimedOut {
            Error::new("timeout", "client browser launch timed out")
        } else {
            Error::new(
                "target_unavailable",
                "client browser launcher is unavailable",
            )
        }
    })?;
    if !result.success() {
        return Err(Error::new(
            "target_unavailable",
            "client browser launch failed",
        ));
    }
    Ok(())
}
/// Emits exactly one JSON envelope on stdout. Callers send parser diagnostics only
/// to stderr; never print input TOML, correction bodies or credential values there.
pub fn write_output(
    output: &OutputEnvelope,
    json_mode: bool,
    target: Option<&str>,
    mut writer: impl Write,
) -> Result<()> {
    if json_mode {
        serde_json::to_writer(&mut writer, output)
            .map_err(|_| Error::new("io_error", "cannot write JSON response"))?;
        writer
            .write_all(b"\n")
            .map_err(|e| Error::io("finish JSON response", e))?;
    } else {
        writer
            .write_all(render_human(output, target).as_bytes())
            .map_err(|e| Error::io("write human response", e))?;
    }
    Ok(())
}
fn printable(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '\u{fffd}'
            } else {
                c
            }
        })
        .collect()
}
/// The overview's four sections are always present in normative order. Complex
/// detail fields remain fully rendered as safe JSON rather than being truncated.
pub fn render_human(output: &OutputEnvelope, target: Option<&str>) -> String {
    let runtime = output
        .runtime_id
        .map_or_else(|| "none (client-only)".into(), |id| id.to_string());
    let mut text = format!(
        "Target: {}\nRuntime: {}\nRequest: {}\n",
        printable(target.unwrap_or("local")),
        runtime,
        output.request_id
    );
    if let Some(operation) = output.operation_id {
        text.push_str(&format!("Operation: {operation}\n"));
    }
    if let Some(state) = &output.state {
        text.push_str(&format!("State: {}\n", printable(state)));
    }
    if let Some(error) = &output.error {
        text.push_str(&format!(
            "Result: {} — {}\n",
            printable(&error.code),
            printable(&error.message)
        ));
        if let Some(next) = &error.next_action {
            text.push_str(&format!("Next action: {}\n", printable(next)));
        }
        if !error.details.is_null() {
            text.push_str(&format!("Details: {}\n", error.details));
        }
    } else {
        text.push_str("Result: acknowledged; observed state is reported below\n");
    }
    if let Some(object) = output.data.as_object() {
        let keys = [
            ("ready_for_release", "Ready for release"),
            ("ready_for_review", "Ready for review"),
            ("blocked", "Blocked"),
            ("running", "Running"),
        ];
        if keys.iter().any(|(key, _)| object.contains_key(*key)) {
            for (key, label) in keys {
                text.push_str(&format!("\n{label}\n"));
                match object.get(key).and_then(Value::as_array) {
                    Some(rows) if !rows.is_empty() => {
                        for row in rows {
                            text.push_str(&format!(
                                "{}\n",
                                serde_json::to_string_pretty(row)
                                    .unwrap_or_else(|_| "unavailable".into())
                            ));
                        }
                    }
                    _ => text.push_str("(none)\n"),
                }
            }
            for (key, value) in object {
                if ![
                    "ready_for_release",
                    "ready_for_review",
                    "blocked",
                    "running",
                ]
                .contains(&key.as_str())
                {
                    text.push_str(&format!(
                        "\n{}: {}\n",
                        printable(key),
                        serde_json::to_string_pretty(value)
                            .unwrap_or_else(|_| "unavailable".into())
                    ));
                }
            }
            return text;
        }
    }
    if !output.data.is_null() {
        text.push_str(&format!(
            "{}\n",
            serde_json::to_string_pretty(&output.data).unwrap_or_else(|_| "unavailable".into())
        ));
    }
    text
}
