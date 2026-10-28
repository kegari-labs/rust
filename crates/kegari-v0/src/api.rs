//! Versioned Axum CLI protocol, shared by authenticated Unix and direct TLS
//! listeners. Connection identity must come from the accepting transport.
use crate::control::Actor;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Path, Query, Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use kegari_types::api::{CommandName, CommandRequest, Envelope};
use kegari_types::error::{Error, Result};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap, future::Future, net::SocketAddr, pin::Pin, sync::Arc, time::Duration,
};
use tokio::time::{Instant, timeout_at};
use uuid::Uuid;

pub const MAX_BODY: usize = 8 * 1024 * 1024;
pub const MAX_HEADERS: usize = 32 * 1024;
pub const REQUEST_DEADLINE: Duration = Duration::from_secs(10);

/// Not deserializable: an HTTP body/header cannot construct a peer identity.
#[derive(Debug, Clone)]
pub enum Peer {
    Unix { uid: u32 },
    DirectTls { address: SocketAddr },
}
pub trait Authentication: Send + Sync {
    fn authenticate(&self, peer: Peer) -> Pin<Box<dyn Future<Output = Result<Actor>> + Send + '_>>;
}
pub trait Dispatcher: Send + Sync {
    fn dispatch(
        &self,
        actor: Actor,
        request: CommandRequest,
        secret: Option<kegari_secrets::Secret>,
    ) -> Pin<Box<dyn Future<Output = Result<DispatchResult>> + Send + '_>>;
}
pub fn command_mutates(command: &CommandName, args: &Value) -> bool {
    match command {
        CommandName::Overview
        | CommandName::Status
        | CommandName::Show
        | CommandName::Open
        | CommandName::ConnectionList
        | CommandName::WorkspaceList
        | CommandName::ConfigShow
        | CommandName::ConfigCheck
        | CommandName::OperationShow => false,
        CommandName::Cleanup => args.get("apply") == Some(&Value::Bool(true)),
        _ => true,
    }
}
pub fn command_role(command: &CommandName, args: &Value) -> &'static str {
    match command {
        CommandName::EvaluationRun => "evaluate",
        CommandName::Approve => "accept",
        CommandName::Correct => "correct",
        CommandName::Release => "release",
        CommandName::Setup
        | CommandName::ConnectionAdd
        | CommandName::ConnectionReconnect
        | CommandName::WorkspaceAdd
        | CommandName::WorkspaceConfigure => "configure",
        CommandName::Pause
        | CommandName::Resume
        | CommandName::Stop
        | CommandName::Start
        | CommandName::ServiceStop => "lifecycle",
        CommandName::OperationReconcile | CommandName::Retry | CommandName::ReleaseRecover => {
            "recover"
        }
        CommandName::Cleanup if command_mutates(command, args) => "recover",
        _ => "inspect",
    }
}
pub fn validate_command_request(request: &CommandRequest, runtime: Uuid) -> Result<()> {
    if request.api_version != 1 {
        return Err(Error::new(
            "incompatible_version",
            "CLI protocol major must be 1",
        ));
    }
    if request.runtime_id != runtime || request.request_id.is_nil() {
        return Err(Error::new(
            "identity_mismatch",
            "request runtime or request ID is invalid",
        ));
    }
    let args = request
        .args
        .as_object()
        .ok_or_else(|| Error::new("invalid_input", "command args must be an object"))?;
    let allowed: &[&str] = match request.command {
        CommandName::Status | CommandName::ConnectionList | CommandName::WorkspaceList => {
            &["snapshot_id", "cursor"]
        }
        CommandName::Overview => &["workspace", "snapshot_id", "cursor"],
        CommandName::Show | CommandName::Open => &["parent", "workspace", "snapshot_id", "cursor"],
        CommandName::Approve | CommandName::Release => &["parent", "result"],
        CommandName::Correct => &["parent", "message"],
        CommandName::Setup => &["configuration"],
        CommandName::ConnectionAdd => &["name", "connection", "secret_reference"],
        CommandName::ConnectionReconnect => &["name"],
        CommandName::WorkspaceAdd => &["name", "workspace_configuration"],
        CommandName::WorkspaceConfigure => &["workspace", "workspace_configuration"],
        CommandName::ConfigShow | CommandName::ConfigCheck => {
            &["workspace", "snapshot_id", "cursor"]
        }
        CommandName::Pause | CommandName::Resume | CommandName::ServiceStop => &[],
        CommandName::Start => &["wait", "timeout_seconds"],
        CommandName::Stop => &["force", "wait", "timeout_seconds"],
        CommandName::OperationShow => &["operation", "snapshot_id", "cursor"],
        CommandName::OperationReconcile => &["operation"],
        CommandName::Retry => &["work", "reason"],
        CommandName::ReleaseRecover => &["parent", "operation", "action", "reason", "result"],
        CommandName::Cleanup => &["operation", "apply", "plan", "snapshot_id", "cursor"],
        CommandName::EvaluationRun => &[
            "workspace",
            "suite_digest",
            "case",
            "variant",
            "profile",
            "repetition",
        ],
    };
    if args.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(Error::new(
            "invalid_input",
            "unknown or forbidden command argument",
        ));
    }
    let text = |key: &str| -> Result<&str> {
        args.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Error::new("invalid_input", format!("command requires nonempty {key}")))
    };
    // Validate optional fields before any dispatcher can interpret missing
    // strings as an absent filter and accidentally broaden an inspection.
    for key in allowed {
        if args.contains_key(*key)
            && !["force", "wait", "apply", "timeout_seconds", "repetition"].contains(key)
        {
            text(key)?;
        }
    }
    if args.contains_key("snapshot_id") != args.contains_key("cursor") {
        return Err(Error::new(
            "invalid_input",
            "snapshot continuation requires both snapshot_id and cursor",
        ));
    }
    for key in ["snapshot_id", "cursor"] {
        if args.contains_key(key) {
            Uuid::parse_str(text(key)?)
                .map_err(|_| Error::new("invalid_input", "snapshot identities must be UUIDs"))?;
        }
    }
    for key in ["parent", "name", "work", "reason"] {
        if allowed.contains(&key)
            && !matches!(
                request.command,
                CommandName::Overview | CommandName::ConfigShow | CommandName::ConfigCheck
            )
        {
            text(key)?;
        }
    }
    if request.command == CommandName::EvaluationRun {
        for key in ["workspace", "suite_digest", "case", "variant", "profile"] {
            text(key)?;
        }
        Uuid::parse_str(text("workspace")?).map_err(|_| {
            Error::new(
                "invalid_input",
                "evaluation workspace must be canonical UUID",
            )
        })?;
        crate::evidence::validate_digest(text("suite_digest")?)?;
        if args
            .get("repetition")
            .and_then(Value::as_u64)
            .is_none_or(|n| n == 0 || n > u32::MAX as u64)
            || request.expected_config_revision.is_none()
            || request.expected_control_generation.is_none()
        {
            return Err(Error::new(
                "invalid_input",
                "evaluation needs exact declared repetition and expected config/control revisions",
            ));
        }
    }
    if matches!(request.command, CommandName::Approve | CommandName::Release) {
        text("result")?;
    }
    if request.command == CommandName::Correct && text("message")?.len() > 1024 * 1024 {
        return Err(Error::new("invalid_input", "correction exceeds 1 MiB"));
    }
    match request.command {
        CommandName::Setup => {
            text("configuration")?;
        }
        CommandName::ConnectionAdd => {
            text("connection")?;
        }
        CommandName::WorkspaceAdd => {
            text("workspace_configuration")?;
        }
        CommandName::WorkspaceConfigure => {
            text("workspace")?;
            text("workspace_configuration")?;
        }
        _ => (),
    }
    if allowed.contains(&"operation") {
        Uuid::parse_str(text("operation")?)
            .map_err(|_| Error::new("invalid_input", "operation must be a UUID"))?;
    }
    for key in ["force", "wait", "apply"] {
        if args.get(key).is_some_and(|v| !v.is_boolean()) {
            return Err(Error::new(
                "invalid_input",
                format!("{key} must be boolean"),
            ));
        }
    }
    if args
        .get("timeout_seconds")
        .is_some_and(|v| v.as_u64().is_none_or(|n| n == 0))
    {
        return Err(Error::new(
            "invalid_input",
            "timeout_seconds must be positive",
        ));
    }
    if request.command == CommandName::Cleanup && command_mutates(&request.command, &request.args) {
        text("plan")?;
        if args.contains_key("snapshot_id") {
            return Err(Error::new(
                "invalid_input",
                "cleanup application cannot carry inspection pagination",
            ));
        }
    }
    if request.command == CommandName::ReleaseRecover {
        let action = text("action")?;
        if !["resume", "abandon", "resolve", "supersede"].contains(&action) {
            return Err(Error::new(
                "invalid_input",
                "unsupported release recovery action",
            ));
        }
        if action == "supersede" {
            text("result")?;
        }
    }
    if matches!(
        request.command,
        CommandName::Pause | CommandName::Resume | CommandName::Stop | CommandName::Start
    ) && request.expected_control_generation.is_none()
    {
        return Err(Error::new(
            "invalid_input",
            "lifecycle command requires the client's observed control generation",
        ));
    }
    if request.command == CommandName::WorkspaceConfigure
        && request.expected_config_revision.is_none()
    {
        return Err(Error::new(
            "invalid_input",
            "workspace change requires expected config revision",
        ));
    }
    Ok(())
}
#[derive(Debug, Clone)]
pub struct DispatchResult {
    pub config_revision: Option<String>,
    pub operation_id: Option<Uuid>,
    pub state: Option<String>,
    pub data: Value,
}
#[derive(Clone)]
pub struct ApiState {
    pub runtime_id: Uuid,
    pub expected_authority: String,
    pub authentication: Arc<dyn Authentication>,
    pub dispatcher: Arc<dyn Dispatcher>,
}
#[derive(Clone)]
struct VerifiedRequest {
    actor: Actor,
    deadline: Instant,
    local: bool,
}

fn status(error: &Error) -> StatusCode {
    match error.code.as_str() {
        "invalid_input" | "invalid_command" => StatusCode::BAD_REQUEST,
        "authentication_required" => StatusCode::UNAUTHORIZED,
        "access_denied" => StatusCode::FORBIDDEN,
        "unknown_resource" => StatusCode::NOT_FOUND,
        "body_too_large" => StatusCode::PAYLOAD_TOO_LARGE,
        "rate_limited" | "capacity" => StatusCode::TOO_MANY_REQUESTS,
        "stale_result"
        | "stale_config"
        | "stale_control"
        | "request_conflict"
        | "ownership_conflict"
        | "identity_mismatch"
        | "incompatible_version"
        | "snapshot_expired" => StatusCode::CONFLICT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}
fn response(
    runtime: Uuid,
    request_id: Uuid,
    result: Result<DispatchResult>,
    uncertain: bool,
) -> Response {
    let (status, envelope) = match result {
        Ok(result) => (
            StatusCode::OK,
            Envelope {
                api_version: 1,
                request_id,
                runtime_id: runtime,
                config_revision: result.config_revision,
                observed_at: Utc::now(),
                ok: true,
                operation_id: result.operation_id,
                state: result.state,
                data: result.data,
                error: None,
            },
        ),
        Err(error) => {
            let uncertain = uncertain || error.code == "operation_uncertain";
            let operation_id = error
                .details
                .get("operation_id")
                .and_then(Value::as_str)
                .and_then(|id| Uuid::parse_str(id).ok())
                .or(if uncertain { Some(request_id) } else { None });
            (
                status(&error),
                Envelope {
                    api_version: 1,
                    request_id,
                    runtime_id: runtime,
                    config_revision: None,
                    observed_at: Utc::now(),
                    ok: false,
                    operation_id,
                    state: if uncertain {
                        Some("uncertain".into())
                    } else {
                        None
                    },
                    data: Value::Null,
                    error: Some(error),
                },
            )
        }
    };
    match serde_json::to_vec(&envelope) {
        Ok(body) if body.len() <= MAX_BODY => (
            status,
            [
                (header::CONTENT_TYPE, "application/json"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            body,
        )
            .into_response(),
        _ => {
            let error = Error::new(
                "body_too_large",
                "response requires complete snapshot pagination",
            );
            let fallback = Envelope {
                data: Value::Null,
                error: Some(error),
                ok: false,
                ..envelope
            };
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::to_vec(&fallback).expect("fixed envelope serializes"),
            )
                .into_response()
        }
    }
}
pub fn validate_headers(
    headers: &HeaderMap,
    method: &Method,
    runtime: Uuid,
    authority: &str,
) -> Result<()> {
    if headers
        .iter()
        .map(|(key, value)| key.as_str().len() + value.as_bytes().len() + 4)
        .sum::<usize>()
        > MAX_HEADERS
    {
        return Err(Error::new("body_too_large", "headers exceed 32 KiB"));
    }
    if headers.contains_key(header::ORIGIN) || method == Method::OPTIONS {
        return Err(Error::new(
            "access_denied",
            "browser-origin and preflight requests are not supported",
        ));
    }
    if headers.keys().any(|key| {
        matches!(key.as_str(), "forwarded" | "x-forwarded-for")
            || key.as_str().starts_with("tailscale-user-")
    }) {
        return Err(Error::new(
            "access_denied",
            "caller-supplied identity headers are not accepted",
        ));
    }
    let single = |key: &str| -> Option<&str> {
        let values: Vec<_> = headers.get_all(key).iter().collect();
        if values.len() != 1 {
            return None;
        }
        values[0].to_str().ok()
    };
    if single("x-kegari-protocol") != Some("1") {
        return Err(Error::new(
            "incompatible_version",
            "X-Kegari-Protocol must be 1",
        ));
    }
    if single("host") != Some(authority)
        || single("x-kegari-runtime") != Some(runtime.to_string().as_str())
    {
        return Err(Error::new(
            "identity_mismatch",
            "Host or expected runtime identity differs from selected target",
        ));
    }
    if method == Method::POST
        && single("content-type")
            .is_none_or(|v| v.split(';').next().map(str::trim) != Some("application/json"))
    {
        return Err(Error::new(
            "invalid_input",
            "POST commands require application/json",
        ));
    }
    if method != Method::GET && method != Method::POST {
        return Err(Error::new(
            "invalid_command",
            "only the documented GET and POST routes are supported",
        ));
    }
    Ok(())
}
async fn guard(State(state): State<ApiState>, mut request: Request, next: Next) -> Response {
    let id = Uuid::new_v4();
    let deadline = Instant::now() + REQUEST_DEADLINE;
    if let Err(error) = validate_headers(
        request.headers(),
        request.method(),
        state.runtime_id,
        &state.expected_authority,
    ) {
        return response(state.runtime_id, id, Err(error), false);
    }
    let Some(peer) = request.extensions().get::<Peer>().cloned() else {
        return response(
            state.runtime_id,
            id,
            Err(Error::new(
                "authentication_required",
                "accepting transport did not supply verified peer metadata",
            )),
            false,
        );
    };
    let local = matches!(peer, Peer::Unix { .. });
    let actor = match timeout_at(deadline, state.authentication.authenticate(peer)).await {
        Ok(Ok(actor)) => actor,
        Ok(Err(error)) => return response(state.runtime_id, id, Err(error), false),
        Err(_) => {
            return response(
                state.runtime_id,
                id,
                Err(Error::new(
                    "timeout",
                    "peer identity verification timed out",
                )),
                false,
            );
        }
    };
    request.extensions_mut().insert(VerifiedRequest {
        actor,
        deadline,
        local,
    });
    if request.method() == Method::GET {
        let (parts, body) = request.into_parts();
        match timeout_at(deadline, to_bytes(body, MAX_BODY)).await {
            Ok(Ok(bytes)) if bytes.is_empty() => {
                request = Request::from_parts(parts, Body::empty())
            }
            Ok(Ok(_)) => {
                return response(
                    state.runtime_id,
                    id,
                    Err(Error::new(
                        "invalid_input",
                        "GET control requests do not accept a body",
                    )),
                    false,
                );
            }
            Ok(Err(_)) => {
                return response(
                    state.runtime_id,
                    id,
                    Err(Error::new(
                        "body_too_large",
                        "request body exceeds 8 MiB or is unreadable",
                    )),
                    false,
                );
            }
            Err(_) => {
                return response(
                    state.runtime_id,
                    id,
                    Err(Error::new("timeout", "request body deadline elapsed")),
                    false,
                );
            }
        }
    }
    next.run(request).await
}
async fn dispatch(
    state: ApiState,
    verified: VerifiedRequest,
    command: CommandRequest,
    secret: Option<kegari_secrets::Secret>,
) -> Response {
    let id = command.request_id;
    if matches!(
        command.command,
        CommandName::ServiceStop | CommandName::EvaluationRun
    ) && !verified.local
    {
        return response(
            state.runtime_id,
            id,
            Err(Error::new(
                "access_denied",
                "this command requires the registered host's Unix peer",
            )),
            false,
        );
    }
    let mutating = command_mutates(&command.command, &command.args);
    if let Err(error) = validate_command_request(&command, state.runtime_id).and_then(|()| {
        verified
            .actor
            .require(command_role(&command.command, &command.args))
    }) {
        return response(state.runtime_id, id, Err(error), false);
    }
    match timeout_at(
        verified.deadline,
        state.dispatcher.dispatch(verified.actor, command, secret),
    )
    .await
    {
        Ok(result) => response(state.runtime_id, id, result, false),
        Err(_) => response(
            state.runtime_id,
            id,
            Err(Error::new(
                "timeout",
                "request deadline elapsed; inspect this request ID before repeating a mutation",
            )),
            mutating,
        ),
    }
}
async fn commands(State(state): State<ApiState>, request: Request) -> Response {
    let verified = request
        .extensions()
        .get::<VerifiedRequest>()
        .cloned()
        .expect("guard installs identity");
    let id = Uuid::new_v4();
    let body = match timeout_at(verified.deadline, to_bytes(request.into_body(), MAX_BODY)).await {
        Ok(Ok(body)) => body,
        Ok(Err(_)) => {
            return response(
                state.runtime_id,
                id,
                Err(Error::new(
                    "body_too_large",
                    "request body exceeds 8 MiB or is unreadable",
                )),
                false,
            );
        }
        Err(_) => {
            return response(
                state.runtime_id,
                id,
                Err(Error::new("timeout", "request body deadline elapsed")),
                false,
            );
        }
    };
    let mut command: CommandRequest = match serde_json::from_slice(&body) {
        Ok(command) => command,
        Err(_) => {
            return response(
                state.runtime_id,
                id,
                Err(Error::new(
                    "invalid_input",
                    "invalid closed command envelope",
                )),
                false,
            );
        }
    };
    let secret = command
        .args
        .as_object_mut()
        .and_then(|args| args.remove("secret_input"));
    let secret = match secret {
        Some(value) if command.command == CommandName::ConnectionAdd => {
            match value
                .as_str()
                .ok_or_else(|| Error::new("invalid_input", "secret input must be UTF-8 text"))
                .and_then(|text| kegari_secrets::Secret::new(text.as_bytes().to_vec()))
            {
                Ok(secret) => Some(secret),
                Err(error) => {
                    return response(state.runtime_id, command.request_id, Err(error), false);
                }
            }
        }
        Some(_) => {
            return response(
                state.runtime_id,
                command.request_id,
                Err(Error::new(
                    "invalid_input",
                    "this command cannot import a credential",
                )),
                false,
            );
        }
        None => None,
    };
    dispatch(state, verified, command, secret).await
}
async fn read(state: ApiState, request: Request, name: CommandName, args: Value) -> Response {
    let verified = request
        .extensions()
        .get::<VerifiedRequest>()
        .cloned()
        .expect("guard installs identity");
    let runtime = state.runtime_id;
    dispatch(
        state,
        verified,
        CommandRequest {
            api_version: 1,
            runtime_id: runtime,
            request_id: Uuid::new_v4(),
            command: name,
            args,
            expected_config_revision: None,
            expected_control_generation: None,
        },
        None,
    )
    .await
}
type QueryArgs =
    std::result::Result<Query<Vec<(String, String)>>, axum::extract::rejection::QueryRejection>;
fn unique_query(query: QueryArgs) -> Result<BTreeMap<String, String>> {
    let Query(pairs) = query.map_err(|_| Error::new("invalid_input", "invalid query arguments"))?;
    let mut result = BTreeMap::new();
    for (key, value) in pairs {
        if result.insert(key, value).is_some() {
            return Err(Error::new("invalid_input", "duplicate query argument"));
        }
    }
    Ok(result)
}
async fn status_route(
    State(state): State<ApiState>,
    query: QueryArgs,
    request: Request,
) -> Response {
    match unique_query(query) {
        Ok(query) => read(state, request, CommandName::Status, json!(query)).await,
        Err(error) => response(state.runtime_id, Uuid::new_v4(), Err(error), false),
    }
}
async fn overview(State(state): State<ApiState>, query: QueryArgs, request: Request) -> Response {
    match unique_query(query) {
        Ok(query) => read(state, request, CommandName::Overview, json!(query)).await,
        Err(error) => response(state.runtime_id, Uuid::new_v4(), Err(error), false),
    }
}
async fn parent(
    State(state): State<ApiState>,
    path: std::result::Result<Path<(String, String)>, axum::extract::rejection::PathRejection>,
    query: QueryArgs,
    request: Request,
) -> Response {
    match (path, unique_query(query)) {
        (Ok(Path((workspace, issue))), Ok(mut query))
            if !query.contains_key("parent") && !query.contains_key("workspace") =>
        {
            query.insert("workspace".into(), workspace);
            query.insert("parent".into(), issue);
            read(state, request, CommandName::Show, json!(query)).await
        }
        _ => response(
            state.runtime_id,
            Uuid::new_v4(),
            Err(Error::new("invalid_input", "invalid parent path/query")),
            false,
        ),
    }
}
async fn operation(
    State(state): State<ApiState>,
    path: std::result::Result<Path<String>, axum::extract::rejection::PathRejection>,
    query: QueryArgs,
    request: Request,
) -> Response {
    match (path, unique_query(query)) {
        (Ok(Path(id)), Ok(mut query)) if !query.contains_key("operation") => {
            query.insert("operation".into(), id);
            read(state, request, CommandName::OperationShow, json!(query)).await
        }
        _ => response(
            state.runtime_id,
            Uuid::new_v4(),
            Err(Error::new("invalid_input", "invalid operation path")),
            false,
        ),
    }
}
async fn unknown(State(state): State<ApiState>) -> Response {
    response(
        state.runtime_id,
        Uuid::new_v4(),
        Err(Error::new("unknown_resource", "unknown control route")),
        false,
    )
}
async fn unknown_method(State(state): State<ApiState>) -> Response {
    response(
        state.runtime_id,
        Uuid::new_v4(),
        Err(Error::new(
            "invalid_command",
            "HTTP method is not supported by this route",
        )),
        false,
    )
}
pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/v1/status", get(status_route))
        .route("/v1/overview", get(overview))
        .route("/v1/parents/{workspace}/{issue}", get(parent))
        .route("/v1/operations/{id}", get(operation))
        .route("/v1/commands", post(commands))
        .fallback(unknown)
        .method_not_allowed_fallback(unknown_method)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

/// Per-connection transport code inserts this extension from OS/TCP metadata;
/// request parsers never accept a peer supplied in JSON or a header.
pub fn connection_router(state: ApiState, peer: Peer) -> Router {
    router(state).layer(axum::Extension(peer))
}

/// Identity only: a declared repetition is a finite trial slot, not authority.
/// The actual command receipt retains this before any evaluator task starts.
pub(crate) fn evaluation_trial_identity(request: &CommandRequest) -> Result<String> {
    validate_command_request(request, request.runtime_id)?;
    if request.command != CommandName::EvaluationRun {
        return Err(Error::invalid("not an evaluation command"));
    }
    let workspace = Uuid::parse_str(
        request.args["workspace"]
            .as_str()
            .ok_or_else(|| Error::invalid("evaluation workspace missing"))?,
    )
    .map_err(|_| Error::invalid("evaluation workspace invalid"))?;
    kegari_digest::canonical_digest(
        1,
        &serde_json::json!({"runtime":request.runtime_id,"workspace":workspace,"suite":request.args["suite_digest"],"case":request.args["case"],"variant":request.args["variant"],"profile":request.args["profile"],"repetition":request.args["repetition"]}),
    )
}
