//! CLI-only pinned control transport. No redirects, proxies, retries or fallback.
use chrono::Utc;
use kegari_control_api::api::MAX_BODY;
use kegari_control_api::api::MAX_HEADERS;
use kegari_control_api::api::REQUEST_DEADLINE;
use kegari_control_api::snapshot;
use kegari_secrets::Secret;
use kegari_storage::PrivateDir;
use kegari_types::api::{CommandName, CommandRequest, Envelope};
use kegari_types::error::{Error, Result};
use kegari_types::snapshot::Page;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tokio::time::{Instant, timeout_at};
use uuid::Uuid;

pub const API_VERSION: u32 = 1;
pub const MAX_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_SNAPSHOT_PAGES: usize = 8192;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
pub enum Endpoint {
    Unix { socket: PathBuf, authority: String },
    Https { origin: String },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub name: String,
    pub runtime_id: Uuid,
    pub api_version: u32,
    pub endpoint: Endpoint,
}
impl Target {
    pub fn validate(&self) -> Result<Url> {
        if self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
            || self.runtime_id.is_nil()
            || self.api_version != API_VERSION
        {
            return Err(Error::new(
                "invalid_input",
                "target needs name, pinned runtime UUID and supported API version",
            ));
        }
        let url = match &self.endpoint {
            Endpoint::Https { origin } => {
                let url = Url::parse(origin)
                    .map_err(|_| Error::new("invalid_input", "target must be an HTTPS origin"))?;
                if url.scheme() != "https"
                    || url.host_str().is_none()
                    || url.path() != "/"
                    || url.query().is_some()
                    || url.fragment().is_some()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(Error::new(
                        "invalid_input",
                        "target must be a credential-free HTTPS origin without path/query/fragment",
                    ));
                }
                url
            }
            Endpoint::Unix { socket, authority } => {
                if !socket.is_absolute()
                    || socket.components().any(|component| {
                        matches!(component, Component::ParentDir | Component::CurDir)
                    })
                    || authority.contains(['/', '@', '?', '#'])
                {
                    return Err(Error::new(
                        "invalid_input",
                        "local target needs an absolute protected socket and explicit authority",
                    ));
                }
                let url = Url::parse(&format!("http://{authority}/"))
                    .map_err(|_| Error::new("invalid_input", "invalid local authority"))?;
                if url.host_str().is_none() || authority.is_empty() {
                    return Err(Error::new("invalid_input", "invalid local authority"));
                }
                url
            }
        };
        Ok(url)
    }
    pub fn remote(&self) -> bool {
        matches!(self.endpoint, Endpoint::Https { .. })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetRecords {
    schema_version: u32,
    local: Option<Target>,
    named: BTreeMap<String, Target>,
}
pub struct TargetStore {
    directory: PrivateDir,
}
impl TargetStore {
    pub fn new(directory: PrivateDir) -> Self {
        Self { directory }
    }
    fn records(&self) -> Result<TargetRecords> {
        match std::fs::symlink_metadata(self.directory.path().join("clients.toml")) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TargetRecords {
                    schema_version: 1,
                    local: None,
                    named: BTreeMap::new(),
                });
            }
            Err(error) => return Err(Error::io("inspect client target record", error)),
            Ok(_) => {}
        }
        let bytes = self.directory.read_bytes("clients.toml", 1_048_576)?;
        let records: TargetRecords = toml::from_str(
            std::str::from_utf8(&bytes)
                .map_err(|_| Error::new("invalid_config", "client preferences are not UTF-8"))?,
        )
        .map_err(|_| Error::new("invalid_config", "client preferences must be typed TOML"))?;
        if records.schema_version != 1 {
            return Err(Error::new(
                "incompatible_version",
                "unsupported client target configuration",
            ));
        }
        for target in records.named.values().chain(records.local.iter()) {
            target.validate()?;
        }
        if records
            .named
            .iter()
            .any(|(name, target)| name != &target.name || !target.remote())
            || records.local.as_ref().is_some_and(Target::remote)
        {
            return Err(Error::new(
                "invalid_config",
                "target record name/transport differs from its registration",
            ));
        }
        Ok(records)
    }
    pub fn select(&self, name: Option<&str>) -> Result<Target> {
        let records = self.records()?;
        match name {
            Some(name) => records.named.get(name).cloned().ok_or_else(|| {
                Error::new(
                    "target_unavailable",
                    "the explicitly selected target is not configured; no fallback is allowed",
                )
            }),
            None => records.local.ok_or_else(|| {
                Error::new(
                    "target_unavailable",
                    "no local runtime is configured; inspect explicit setup requirements with `kegari-v0 setup --help`",
                )
                .next("Run `kegari-v0 setup --help` to inspect setup; this read does not register a runtime or start a service")
            }),
        }
    }
    /// Pins are supplied explicitly, never discovered from a mismatching response.
    pub fn add(&self, target: Target) -> Result<()> {
        target.validate()?;
        if !target.remote() {
            return Err(Error::new("invalid_input", "named targets must use HTTPS"));
        }
        let _lock = self.directory.section_lock("targets.lock")?;
        let mut records = self.records()?;
        if let Some(old) = records.named.get(&target.name) {
            if old == &target {
                return Ok(());
            }
            return Err(Error::new(
                "identity_mismatch",
                "target already pins another origin/runtime; explicit replacement and reconciliation are required",
            ));
        }
        records.named.insert(target.name.clone(), target);
        self.save(&records)
    }
    /// Used by explicit local registration; never by remote target recovery.
    pub fn register_local(&self, target: Target) -> Result<()> {
        target.validate()?;
        if target.remote() {
            return Err(Error::new(
                "invalid_input",
                "local registration must use the protected Unix socket",
            ));
        }
        let _lock = self.directory.section_lock("targets.lock")?;
        let mut records = self.records()?;
        if records.local.as_ref().is_some_and(|old| old != &target) {
            return Err(Error::new(
                "identity_mismatch",
                "local runtime registration cannot silently replace an existing runtime",
            ));
        }
        records.local = Some(target);
        self.save(&records)
    }
    pub fn list(&self) -> Result<Vec<Target>> {
        let records = self.records()?;
        Ok(records
            .local
            .into_iter()
            .chain(records.named.into_values())
            .collect())
    }
    fn save(&self, records: &TargetRecords) -> Result<()> {
        let text = toml::to_string_pretty(records)
            .map_err(|_| Error::invalid("client preferences encoding failed"))?;
        self.directory.write_bytes("clients.toml", text.as_bytes())
    }
}

pub struct ControlClient {
    target: Target,
    origin: Url,
    http: Client,
}
impl std::fmt::Debug for ControlClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ControlClient")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}
fn validate_socket(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
        let metadata = std::fs::symlink_metadata(path).map_err(|_| {
            Error::new(
                "target_unavailable",
                "registered local control socket is unavailable",
            )
        })?;
        if !metadata.file_type().is_socket()
            || metadata.permissions().mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(Error::new(
                "ownership_conflict",
                "local control socket is not a private socket owned by this user",
            ));
        }
        let parent = path
            .parent()
            .ok_or_else(|| Error::new("invalid_input", "socket has no parent"))?;
        PrivateDir::open(parent)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(Error::new(
            "prerequisite",
            "Unix control transport is unavailable on this host",
        ))
    }
}
fn contains_secret(value: &Value, secret: &str) -> bool {
    match value {
        Value::String(text) => text.contains(secret),
        Value::Array(values) => values.iter().any(|value| contains_secret(value, secret)),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key.contains(secret) || contains_secret(value, secret)),
        _ => false,
    }
}
impl ControlClient {
    pub fn new(target: Target) -> Result<Self> {
        let origin = target.validate()?;
        let mut builder = Client::builder()
            .use_rustls_tls()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .retry(reqwest::retry::never())
            .http1_only()
            .connect_timeout(REQUEST_DEADLINE)
            .timeout(REQUEST_DEADLINE)
            .user_agent(format!("kegari-v0/{}", env!("CARGO_PKG_VERSION")));
        match &target.endpoint {
            Endpoint::Https { .. } => {
                builder = builder.https_only(true);
            }
            Endpoint::Unix { socket, .. } => {
                validate_socket(socket)?;
                #[cfg(unix)]
                {
                    builder = builder.unix_socket(socket.clone());
                }
            }
        }
        let http = builder.build().map_err(|_| {
            Error::new(
                "prerequisite",
                "cannot initialize verified control transport",
            )
        })?;
        Ok(Self {
            target,
            origin,
            http,
        })
    }
    pub fn target(&self) -> &Target {
        &self.target
    }
    fn failure(&self, request: &CommandRequest, error: Error, uncertain: bool) -> Envelope {
        let error=error.detail(json!({"request_id":request.request_id,"target":self.target.name,"outcome_unknown":uncertain}));
        Envelope {
            api_version: API_VERSION,
            request_id: request.request_id,
            runtime_id: self.target.runtime_id,
            config_revision: None,
            observed_at: Utc::now(),
            ok: false,
            operation_id: None,
            state: uncertain.then(|| "uncertain".into()),
            data: Value::Null,
            error: Some(error),
        }
    }
    /// Exactly one transmission. A missing acknowledgement is never retried here.
    pub async fn send(&self, request: &CommandRequest, secret: Option<&Secret>) -> Envelope {
        if request.runtime_id != self.target.runtime_id
            || request.api_version != self.target.api_version
        {
            return self.failure(
                request,
                Error::new(
                    "identity_mismatch",
                    "request is not bound to selected target",
                ),
                false,
            );
        }
        if let Err(error) =
            kegari_control_api::api::validate_command_request(request, self.target.runtime_id)
        {
            return self.failure(request, error, false);
        }
        if let Endpoint::Unix { socket, .. } = &self.target.endpoint
            && let Err(error) = validate_socket(socket)
        {
            return self.failure(request, error, false);
        }
        if secret.is_some() && request.command != CommandName::ConnectionAdd {
            return self.failure(
                request,
                Error::new(
                    "invalid_input",
                    "transient credential input is only valid for connection add",
                ),
                false,
            );
        }
        let mut body = match serde_json::to_value(request) {
            Ok(body) => body,
            Err(_) => {
                return self.failure(
                    request,
                    Error::new("invalid_input", "cannot encode command"),
                    false,
                );
            }
        };
        if let Some(secret) = secret {
            let value = match std::str::from_utf8(secret.bytes()) {
                Ok(value) => value,
                Err(_) => {
                    return self.failure(
                        request,
                        Error::new("invalid_input", "secret input must be UTF-8"),
                        false,
                    );
                }
            };
            body["args"]["secret_input"] = json!(value);
        }
        let mut encoded = match serde_json::to_vec(&body) {
            Ok(value) => value,
            Err(_) => {
                return self.failure(
                    request,
                    Error::new("invalid_input", "cannot encode command"),
                    false,
                );
            }
        };
        // The transient secret is injected only in this bounded, verified transport body.
        // Neither CommandRequest nor a recovery receipt contains its plaintext.
        if encoded.len() > MAX_BODY {
            encoded.fill(0);
            return self.failure(
                request,
                Error::new("invalid_input", "command body exceeds 8 MiB"),
                false,
            );
        }
        let mut url = self.origin.clone();
        url.set_path("/v1/commands");
        let authority = match &self.target.endpoint {
            Endpoint::Unix { authority, .. } => authority.clone(),
            Endpoint::Https { .. } => {
                self.origin[url::Position::BeforeHost..url::Position::AfterPort].to_owned()
            }
        };
        let mutating = kegari_control_api::api::command_mutates(&request.command, &request.args);
        let sent = self
            .http
            .post(url)
            .header("host", authority)
            .header("x-kegari-protocol", "1")
            .header("x-kegari-runtime", self.target.runtime_id.to_string())
            .header("content-type", "application/json")
            .body(encoded)
            .send()
            .await;
        // Drop the extra JSON value as soon as transmission completes. It is never logged.
        drop(body);
        let mut response=match sent {Ok(response)=>response,Err(error)=>return self.failure(request,Error::new(if error.is_timeout(){"timeout"}else{"target_unavailable"},"selected target did not return a verified acknowledgement; inspect the retained request ID before any mutation retry"),mutating)};
        let status = response.status();
        if status.is_redirection() {
            return self.failure(
                request,
                Error::new(
                    "identity_mismatch",
                    "control redirects are forbidden; selected target was not followed",
                ),
                mutating,
            );
        }
        if response
            .headers()
            .iter()
            .map(|(name, value)| name.as_str().len() + value.len() + 4)
            .sum::<usize>()
            > MAX_HEADERS
        {
            return self.failure(
                request,
                Error::new("incomplete_snapshot", "response headers exceed 32 KiB"),
                mutating,
            );
        }
        if response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(';').next())
            .map(str::trim)
            != Some("application/json")
        {
            return self.failure(
                request,
                Error::new(
                    "incompatible_version",
                    "target did not return the JSON control envelope",
                ),
                mutating,
            );
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BODY as u64)
        {
            return self.failure(
                request,
                Error::new(
                    "incomplete_snapshot",
                    "response exceeds 8 MiB; pagination is required",
                ),
                mutating,
            );
        }
        let mut bytes = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) if bytes.len().saturating_add(chunk.len()) <= MAX_BODY => {
                    bytes.extend_from_slice(&chunk)
                }
                Ok(Some(_)) => {
                    return self.failure(
                        request,
                        Error::new("incomplete_snapshot", "streamed response exceeds 8 MiB"),
                        mutating,
                    );
                }
                Ok(None) => break,
                Err(error) => {
                    return self.failure(
                        request,
                        Error::new(
                            if error.is_timeout() {
                                "timeout"
                            } else {
                                "target_unavailable"
                            },
                            "response ended before a complete verified acknowledgement",
                        ),
                        mutating,
                    );
                }
            }
        }
        let mut envelope: Envelope = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) => {
                return self.failure(
                    request,
                    Error::new(
                        "incompatible_version",
                        "response is not a complete versioned control envelope",
                    ),
                    mutating,
                );
            }
        };
        if let Some(secret) = secret
            && let Ok(secret) = std::str::from_utf8(secret.bytes())
            && serde_json::to_value(&envelope)
                .ok()
                .is_some_and(|value| contains_secret(&value, secret))
        {
            return self.failure(
                request,
                Error::new(
                    "secret_disclosure",
                    "target response contained credential material; the response was omitted",
                ),
                mutating,
            );
        }
        if envelope.runtime_id != self.target.runtime_id
            || envelope.api_version != self.target.api_version
        {
            return self.failure(request,Error::new("identity_mismatch","response runtime/API differs from selected pin; no repinning or fallback occurred"),mutating);
        }
        if envelope.ok != status.is_success() || envelope.ok == envelope.error.is_some() {
            return self.failure(
                request,
                Error::new(
                    "incompatible_version",
                    "HTTP status and envelope result disagree",
                ),
                mutating,
            );
        }
        if envelope.request_id != request.request_id {
            // Header/auth rejection can occur before the server reads the body ID.
            // Preserve our already-created ID without treating that error as acknowledgement.
            if !envelope.ok
                && envelope.operation_id.is_none()
                && envelope.state.is_none()
                && envelope.error.as_ref().is_some_and(|error| {
                    [
                        "authentication_required",
                        "access_denied",
                        "invalid_input",
                        "body_too_large",
                        "rate_limited",
                    ]
                    .contains(&error.code.as_str())
                })
            {
                envelope.request_id = request.request_id;
            } else {
                return self.failure(
                    request,
                    Error::new(
                        "request_conflict",
                        "target acknowledgement refers to another request ID",
                    ),
                    mutating,
                );
            }
        }
        envelope
    }
    /// Retrieve every immutable page/chunk before returning one complete envelope.
    /// Expiry restarts a read at most once; mutations are never paginated/replayed.
    pub async fn read_complete(&self, request: &CommandRequest) -> Envelope {
        if kegari_control_api::api::command_mutates(&request.command, &request.args) {
            return self.failure(
                request,
                Error::new(
                    "invalid_input",
                    "snapshot retrieval cannot submit a mutation",
                ),
                false,
            );
        }
        if let Err(error) =
            kegari_control_api::api::validate_command_request(request, self.target.runtime_id)
        {
            return self.failure(request, error, false);
        }
        let deadline = Instant::now() + Duration::from_secs(300);
        for restart in 0..=1 {
            let mut current = request.clone();
            current
                .args
                .as_object_mut()
                .expect("validated read args")
                .remove("snapshot_id");
            current
                .args
                .as_object_mut()
                .expect("validated read args")
                .remove("cursor");
            let mut pages = Vec::new();
            let mut seen = BTreeSet::new();
            let mut first: Option<Envelope> = None;
            let mut total = 0usize;
            loop {
                let response = match timeout_at(deadline, self.send(&current, None)).await {
                    Ok(response) => response,
                    Err(_) => {
                        return self.failure(
                            request,
                            Error::new(
                                "incomplete_snapshot",
                                "complete snapshot read exceeded its finite deadline",
                            ),
                            false,
                        );
                    }
                };
                if !response.ok {
                    if response
                        .error
                        .as_ref()
                        .is_some_and(|e| e.code == "snapshot_expired")
                        && restart == 0
                    {
                        break;
                    }
                    return response;
                }
                if response.data.get("snapshot_id").is_none() {
                    let snapshot_required =
                        !matches!(request.command, CommandName::Status | CommandName::Open);
                    return if pages.is_empty() && !snapshot_required {
                        response
                    } else {
                        self.failure(request, Error::new("incomplete_snapshot", "collection/detail response lacks its required immutable snapshot identity"), false)
                    };
                }
                let page: Page = match serde_json::from_value(response.data.clone()) {
                    Ok(page) => page,
                    Err(_) => {
                        return self.failure(
                            request,
                            Error::new("incomplete_snapshot", "malformed snapshot page"),
                            false,
                        );
                    }
                };
                total = total.saturating_add(
                    serde_json::to_vec(&page).map_or(MAX_SNAPSHOT_BYTES + 1, |bytes| bytes.len()),
                );
                if pages.len() >= MAX_SNAPSHOT_PAGES
                    || total > MAX_SNAPSHOT_BYTES
                    || !seen.insert(page.cursor)
                    || page.expires_at <= Utc::now()
                    || first
                        .as_ref()
                        .is_some_and(|first| first.config_revision != response.config_revision)
                {
                    return self.failure(request,Error::new("incomplete_snapshot","snapshot exceeded bounds, expired, repeated a cursor or changed revision"),false);
                }
                if first.is_none() {
                    first = Some(response);
                }
                let next = page.next_cursor;
                let snapshot_id = page.snapshot_id;
                pages.push(page);
                match next {
                    Some(cursor) => {
                        current.request_id = Uuid::new_v4();
                        current.args["snapshot_id"] = json!(snapshot_id);
                        current.args["cursor"] = json!(cursor);
                    }
                    None => {
                        let data = match snapshot::assemble(&pages) {
                            Ok(data) => data,
                            Err(error) => return self.failure(request, error, false),
                        };
                        let mut response = first.expect("first complete page");
                        response.data = data;
                        response.observed_at = pages[0].observed_at;
                        response.request_id = request.request_id;
                        return response;
                    }
                }
            }
        }
        self.failure(
            request,
            Error::new(
                "incomplete_snapshot",
                "snapshot expired again; no partial view was emitted",
            ),
            false,
        )
    }
    /// Fetch the current generation once when absent, then submit once. A stale
    /// generation is returned to the operator without updating and resending it.
    pub async fn execute(&self, request: &mut CommandRequest, secret: Option<&Secret>) -> Envelope {
        if matches!(
            request.command,
            CommandName::Pause | CommandName::Resume | CommandName::Stop | CommandName::Start
        ) && request.expected_control_generation.is_none()
        {
            let status = CommandRequest {
                api_version: 1,
                runtime_id: self.target.runtime_id,
                request_id: Uuid::new_v4(),
                command: CommandName::Status,
                args: json!({}),
                expected_config_revision: None,
                expected_control_generation: None,
            };
            let response = self.read_complete(&status).await;
            if !response.ok {
                let error = response.error.unwrap_or_else(|| {
                    Error::new("target_unavailable", "current generation is unavailable")
                });
                return self.failure(request, error, false);
            }
            let generation = response
                .data
                .get("control_generation")
                .and_then(Value::as_u64)
                .or_else(|| {
                    response
                        .data
                        .pointer("/intent/generation")
                        .and_then(Value::as_u64)
                });
            let Some(generation) = generation else {
                return self.failure(
                    request,
                    Error::new(
                        "incomplete_snapshot",
                        "status lacks current control generation; lifecycle decision was not sent",
                    ),
                    false,
                );
            };
            request.expected_control_generation = Some(generation);
        }
        if kegari_control_api::api::command_mutates(&request.command, &request.args) {
            self.send(request, secret).await
        } else {
            self.read_complete(request).await
        }
    }
    pub async fn wait_operation(&self, mut response: Envelope, seconds: u64) -> Envelope {
        let Some(operation) = response.operation_id else {
            return response;
        };
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let original_id = response.request_id;
        loop {
            if !response.ok
                || response.state.as_deref().is_some_and(|state| {
                    [
                        "completed",
                        "stopped",
                        "failed",
                        "partial",
                        "unknown",
                        "cancelled",
                        "blocked",
                    ]
                    .contains(&state)
                })
            {
                return response;
            }
            if Instant::now() >= deadline {
                response.data = json!({"observation":response.data,"wait_timeout":true,"operation_continues":true});
                return response;
            }
            let request = CommandRequest {
                api_version: 1,
                runtime_id: self.target.runtime_id,
                request_id: Uuid::new_v4(),
                command: CommandName::OperationShow,
                args: json!({"operation":operation}),
                expected_config_revision: None,
                expected_control_generation: None,
            };
            match timeout_at(deadline, self.read_complete(&request)).await {
                Ok(mut observed) => {
                    if observed.ok && observed.operation_id != Some(operation) {
                        observed = self.failure(
                            &request,
                            Error::new(
                                "identity_mismatch",
                                "operation observation does not identify the requested operation",
                            ),
                            false,
                        );
                    }
                    observed.request_id = original_id;
                    observed.operation_id = Some(operation);
                    response = observed;
                }
                Err(_) => {
                    response.data = json!({"observation":response.data,"wait_timeout":true,"operation_continues":true});
                    return response;
                }
            }
            tokio::time::sleep(
                Duration::from_millis(200).min(deadline.saturating_duration_since(Instant::now())),
            )
            .await;
        }
    }
}
