//! Production HTTPS adapters. Provider input is untrusted; collection reads are
//! atomic. Typed writes are trusted backend integration APIs; the backend owns
//! authorization, durable reservation and effect admission.
use chrono::{DateTime, Utc};
use kegari_config::{GithubSelection, LinearSelection, Selector};
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_types::error::{Error, Result};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use url::Url;
use uuid::Uuid;

pub mod validation;
use validation::*;

/// Same fixed byte bound as the backend instruction context.
pub const MAX_PULL_REQUEST_TEXT_BYTES: usize = 1_048_576;

pub const GITHUB_API_VERSION: &str = "2026-03-10";
pub const LINEAR_SCHEMA_REVISION: &str = "618a737a4e6aa279b7853c650f79f209a5142bbc";
pub const MAX_BODY: usize = 8 * 1024 * 1024;
const PAGE_SIZE: usize = 100;
pub const MAX_VIDEO_BYTES: usize = 256 * 1024 * 1024;
mod assets;
pub use assets::{LinearAssetUrl, linear_video_markdown};
mod writes;
pub use writes::{GithubCommitWrite, GithubDispatchWrite, GithubPullRequestWrite};

/// Never serializable; Debug cannot expose a credential.
pub struct Credential(String);
impl Credential {
    pub fn new(value: String) -> Result<Self> {
        if value.is_empty() || value.contains(['\r', '\n', '\0']) {
            return Err(Error::new(
                "missing_secret",
                "provider credential is absent or malformed",
            ));
        }
        Ok(Self(value))
    }
}
impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credential([REDACTED])")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
}
impl Method {
    fn reqwest(self) -> reqwest::Method {
        match self {
            Self::Get => reqwest::Method::GET,
            Self::Post => reqwest::Method::POST,
            Self::Put => reqwest::Method::PUT,
            Self::Patch => reqwest::Method::PATCH,
        }
    }
}
pub struct HttpRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}
impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("body_bytes", &self.body.len())
            .finish_non_exhaustive()
    }
}
pub struct HttpResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}
impl std::fmt::Debug for HttpResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpResponse")
            .field("status", &self.status)
            .field("body_bytes", &self.body.len())
            .finish_non_exhaustive()
    }
}
/// Injectable wire boundary; production uses ReqwestTransport, not fixture data.
pub trait HttpTransport: Send + Sync {
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>>;
    fn send_bounded(
        &self,
        request: HttpRequest,
        max_body: usize,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            let response = self.send(request).await?;
            if response.body.len() > max_body {
                return Err(Error::new(
                    "source_limit_exceeded",
                    "HTTP response exceeds its exact bound",
                ));
            }
            Ok(response)
        })
    }
}
#[derive(Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
    max_body: usize,
}
impl ReqwestTransport {
    pub fn new(timeout: Duration, max_body: usize) -> Result<Self> {
        if timeout.is_zero() || max_body == 0 || max_body > MAX_BODY {
            return Err(Error::invalid(
                "HTTP bounds must be positive and at most 8 MiB per response",
            ));
        }
        Self::bounded(timeout, max_body)
    }
    fn bounded(timeout: Duration, max_body: usize) -> Result<Self> {
        if timeout.is_zero() || max_body == 0 || max_body > MAX_VIDEO_BYTES {
            return Err(Error::invalid(
                "binary HTTP bounds must be positive and at most 256 MiB",
            ));
        }
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(timeout)
            .connect_timeout(timeout)
            .user_agent(concat!("kegari-v0/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| {
                Error::new(
                    "provider_transport",
                    "cannot initialize HTTPS provider client",
                )
            })?;
        Ok(Self { client, max_body })
    }
}
impl HttpTransport for ReqwestTransport {
    fn send_bounded(
        &self,
        request: HttpRequest,
        max_body: usize,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        let bounded = Self {
            client: self.client.clone(),
            max_body: self.max_body.min(max_body),
        };
        Box::pin(async move { bounded.send(request).await })
    }
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            let mut response = self
                .client
                .request(request.method.reqwest(), request.url)
                .headers(request.headers)
                .body(request.body)
                .send()
                .await
                .map_err(|error| {
                    let mut e = Error::new(
                        "provider_transport",
                        if error.is_timeout() {
                            "provider request timed out; mutations require reconciliation"
                        } else {
                            "provider HTTPS request failed"
                        },
                    );
                    e.retryable = true;
                    e
                })?;
            if response
                .content_length()
                .is_some_and(|length| length > self.max_body as u64)
            {
                return Err(Error::new(
                    "provider_response",
                    "provider response exceeds configured body limit",
                ));
            }
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| {
                Error::new(
                    "provider_transport",
                    "provider response body was interrupted",
                )
            })? {
                if body.len().saturating_add(chunk.len()) > self.max_body {
                    return Err(Error::new(
                        "provider_response",
                        "provider response exceeds configured body limit",
                    ));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        })
    }
}

mod object_transfer;
pub use object_transfer::GithubObjectSession;
#[derive(Clone, Debug)]
pub struct HttpPolicy {
    pub endpoint: Url,
    pub account_id: String,
    pub timeout: Duration,
    pub max_pages: usize,
    pub max_items: usize,
    pub max_body: usize,
}
impl HttpPolicy {
    pub fn new(endpoint: &str, account_id: String, timeout: Duration) -> Result<Self> {
        let endpoint = Url::parse(endpoint)
            .map_err(|_| Error::invalid("provider endpoint must be canonical HTTPS"))?;
        validate_https(&endpoint)?;
        if account_id.trim().is_empty() || timeout.is_zero() {
            return Err(Error::invalid(
                "provider account and positive timeout are required",
            ));
        }
        Ok(Self {
            endpoint,
            account_id,
            timeout,
            max_pages: 1000,
            max_items: 100_000,
            max_body: MAX_BODY,
        })
    }
}
#[derive(Clone)]
struct HttpCore {
    policy: HttpPolicy,
    transport: Arc<dyn HttpTransport>,
    authorization: HeaderValue,
    github: bool,
    rate_limit: Arc<Mutex<RateLimit>>,
}
impl HttpCore {
    fn new(
        policy: HttpPolicy,
        credential: Credential,
        github: bool,
        transport: Option<Arc<dyn HttpTransport>>,
    ) -> Result<Self> {
        validate_https(&policy.endpoint)?;
        if policy.max_pages == 0
            || policy.max_items == 0
            || policy.max_body == 0
            || policy.max_body > MAX_BODY
            || policy.timeout.is_zero()
        {
            return Err(Error::invalid(
                "invalid finite provider request/collection bounds",
            ));
        }
        let credential = if github {
            format!("Bearer {}", credential.0)
        } else {
            credential.0
        };
        let mut authorization = HeaderValue::from_str(&credential).map_err(|_| {
            Error::new(
                "missing_secret",
                "provider credential cannot be transmitted safely",
            )
        })?;
        authorization.set_sensitive(true);
        let transport = match transport {
            Some(t) => t,
            None => Arc::new(ReqwestTransport::new(policy.timeout, policy.max_body)?),
        };
        Ok(Self {
            policy,
            transport,
            authorization,
            github,
            rate_limit: Arc::new(Mutex::new(RateLimit::default())),
        })
    }
    fn url(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.policy.endpoint.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| Error::invalid("provider endpoint has no hierarchical path"))?;
            path.pop_if_empty();
            for segment in segments {
                if segment.is_empty()
                    || segment.contains(['\0', '/'])
                    || *segment == "."
                    || *segment == ".."
                {
                    return Err(Error::invalid("invalid provider path component"));
                }
                path.push(segment);
            }
        }
        Ok(url)
    }
    async fn send(&self, method: Method, url: Url, body: Option<Value>) -> Result<HttpResponse> {
        let response = self.raw(method, url, body).await?;
        if !(200..300).contains(&response.status) {
            return Err(status_error(&response, self.github));
        }
        Ok(response)
    }
    async fn raw(&self, method: Method, url: Url, body: Option<Value>) -> Result<HttpResponse> {
        if url.origin() != self.policy.endpoint.origin() {
            return Err(Error::new(
                "access_denied",
                "provider request escaped the configured origin",
            ));
        }
        let mut headers = HeaderMap::new();
        headers.insert("authorization", self.authorization.clone());
        headers.insert(
            "accept",
            HeaderValue::from_static(if self.github {
                "application/vnd.github+json"
            } else {
                "application/json"
            }),
        );
        if self.github {
            headers.insert(
                "x-github-api-version",
                HeaderValue::from_static(GITHUB_API_VERSION),
            );
        }
        let body = match body {
            Some(value) => {
                headers.insert("content-type", HeaderValue::from_static("application/json"));
                serde_json::to_vec(&value)
                    .map_err(|_| Error::invalid("provider payload cannot be encoded"))?
            }
            None => Vec::new(),
        };
        if body.len() > self.policy.max_body {
            return Err(Error::invalid("provider request exceeds configured bound"));
        }
        let response = tokio::time::timeout(
            self.policy.timeout,
            self.transport.send(HttpRequest {
                method,
                url,
                headers,
                body,
            }),
        )
        .await
        .map_err(|_| Error::new("provider_transport", "bounded provider request timed out"))??;
        if response.body.len() > self.policy.max_body {
            return Err(Error::new(
                "provider_response",
                "provider response exceeds configured bound",
            ));
        }
        *self
            .rate_limit
            .lock()
            .map_err(|_| Error::new("provider_transport", "rate metadata unavailable"))? =
            RateLimit::from_headers(&response.headers, self.github);
        Ok(response)
    }
    async fn json(
        &self,
        method: Method,
        url: Url,
        body: Option<Value>,
    ) -> Result<(Value, HeaderMap)> {
        let response = self.send(method, url, body).await?;
        let value = if response.body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&response.body).map_err(|_| {
                Error::new("provider_response", "provider response is not valid JSON")
            })?
        };
        Ok((value, response.headers))
    }
}
fn validate_https(url: &Url) -> Result<()> {
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::invalid(
            "provider endpoint must be HTTPS without credentials, query or fragment",
        ));
    }
    Ok(())
}
fn status_error(response: &HttpResponse, github: bool) -> Error {
    let rate = RateLimit::from_headers(&response.headers, github);
    let graphql_limited = !github
        && serde_json::from_slice::<Value>(&response.body)
            .ok()
            .and_then(|v| v.get("errors").and_then(Value::as_array).cloned())
            .is_some_and(|errors| {
                errors.iter().any(|e| {
                    e.pointer("/extensions/code")
                        .and_then(Value::as_str)
                        .is_some_and(|code| matches!(code, "RATELIMITED" | "RATE_LIMITED"))
                })
            });
    let limited = graphql_limited
        || response.status == 429
        || (response.status == 403 && (rate.remaining == Some(0) || rate.retry_after_ms.is_some()));
    let code = if limited {
        "rate_limited"
    } else {
        match response.status {
            401 => "authentication_required",
            403 => "access_denied",
            _ => "provider_status",
        }
    };
    let mut error = Error::new(
        code,
        if limited {
            "provider rate limit blocks further requests until the stated reset"
        } else {
            "provider rejected the request; response body omitted"
        },
    )
    .detail(json!({"http_status":response.status,"rate_limit":rate}));
    error.retryable = limited || response.status >= 500;
    error
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RateLimit {
    pub remaining: Option<u64>,
    pub reset_at: Option<DateTime<Utc>>,
    pub retry_after_ms: Option<u64>,
}
impl RateLimit {
    fn from_headers(headers: &HeaderMap, github: bool) -> Self {
        let value = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        let remaining = if github {
            value("x-ratelimit-remaining").and_then(|v| v.parse().ok())
        } else {
            [
                "x-ratelimit-requests-remaining",
                "x-ratelimit-endpoint-requests-remaining",
                "x-ratelimit-complexity-remaining",
            ]
            .into_iter()
            .filter_map(|name| value(name).and_then(|v| v.parse::<u64>().ok()))
            .min()
        };
        let reset_at = if github {
            value("x-ratelimit-reset")
                .and_then(|v| v.parse::<i64>().ok())
                .and_then(|s| DateTime::from_timestamp(s, 0))
        } else {
            [
                "x-ratelimit-requests-reset",
                "x-ratelimit-endpoint-requests-reset",
                "x-ratelimit-complexity-reset",
            ]
            .into_iter()
            .filter_map(|name| {
                value(name)
                    .and_then(|v| v.parse::<i64>().ok())
                    .and_then(DateTime::from_timestamp_millis)
            })
            .max()
        };
        let retry_after_ms = value("retry-after").and_then(|v| {
            v.parse::<u64>()
                .ok()
                .and_then(|s| s.checked_mul(1000))
                .or_else(|| {
                    DateTime::parse_from_rfc2822(v).ok().map(|date| {
                        (date.with_timezone(&Utc) - Utc::now())
                            .num_milliseconds()
                            .max(0) as u64
                    })
                })
        });
        Self {
            remaining,
            reset_at,
            retry_after_ms,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountIdentity {
    pub account_id: String,
    pub workspace_id: Option<Uuid>,
    pub display_name: String,
    pub permission_scopes: Option<Vec<String>>,
    pub observed_at: DateTime<Utc>,
}

/// Read-only credential preflight before a workspace exists. Endpoints and
/// expected account identity come exclusively from the explicit connection.
pub async fn verify_connection_identity(
    policy: HttpPolicy,
    credential: Credential,
    kind: kegari_config::ProviderKind,
    transport: Option<Arc<dyn HttpTransport>>,
) -> Result<AccountIdentity> {
    use kegari_config::ProviderKind;
    if kind == ProviderKind::Codex {
        return Err(Error::new(
            "unsupported_provider",
            "Codex authentication must be verified through the pinned app-server handshake",
        ));
    }
    let expected = policy.account_id.clone();
    let http = HttpCore::new(policy, credential, kind == ProviderKind::Github, transport)?;
    match kind {
        ProviderKind::Linear => {
            let configured = Uuid::parse_str(&expected)
                .map_err(|_| Error::invalid("Linear expected account must be a canonical UUID"))?;
            if configured.is_nil() || configured.to_string() != expected {
                return Err(Error::invalid(
                    "Linear expected account must be a canonical nonnil UUID",
                ));
            }
            let (value, headers) = http.json(Method::Post, http.policy.endpoint.clone(), Some(json!({"query":"query KegariIdentity { viewer { id name } organization { id } }","variables":{}}))).await?;
            let data = graph_data(value, &headers)?;
            let account = uuid_at(&data, "/viewer/id")?;
            let workspace = uuid_at(&data, "/organization/id")?;
            if account != configured {
                return Err(Error::new(
                    "identity_mismatch",
                    "Linear credential differs from expected connection account",
                ));
            }
            Ok(AccountIdentity {
                account_id: account.to_string(),
                workspace_id: Some(workspace),
                display_name: text_at(&data, "/viewer/name")?.into(),
                permission_scopes: None,
                observed_at: Utc::now(),
            })
        }
        ProviderKind::Github => {
            let (value, headers) = http.json(Method::Get, http.url(&["user"])?, None).await?;
            require_github_user(&value)?;
            let account = positive_id(&value, "/id")?.to_string();
            if account != expected {
                return Err(Error::new(
                    "identity_mismatch",
                    "GitHub credential differs from expected connection user",
                ));
            }
            let permission_scopes = headers
                .get("x-oauth-scopes")
                .and_then(|s| s.to_str().ok())
                .map(|s| {
                    s.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                });
            Ok(AccountIdentity {
                account_id: account,
                workspace_id: None,
                display_name: text_at(&value, "/login")?.into(),
                permission_scopes,
                observed_at: Utc::now(),
            })
        }
        ProviderKind::Codex => unreachable!("checked before transport creation"),
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Label {
    pub id: Uuid,
    pub name: String,
    pub comparison_name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Issue {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub endpoint: String,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub team_id: Uuid,
    pub project_id: Option<Uuid>,
    pub creator_id: Option<Uuid>,
    pub assignee_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub state_id: Uuid,
    pub state_name: String,
    pub state_type: String,
    pub labels: Vec<Label>,
    pub priority: Option<i64>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub url: String,
    pub diagnostics: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Comment {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub body: String,
    pub body_digest: String,
    pub user_id: Option<Uuid>,
    pub delegated: bool,
    pub bot: bool,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub edited_at: Option<DateTime<Utc>>,
    pub url: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: Uuid,
    pub actor_id: Option<Uuid>,
    pub actor_ids: Vec<Uuid>,
    pub bot: bool,
    pub added_labels: Vec<Uuid>,
    pub removed_labels: Vec<Uuid>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub from_parent: Option<Uuid>,
    pub to_parent: Option<Uuid>,
    pub description_updated: Option<bool>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dependency {
    pub id: Uuid,
    pub issue_id: Uuid,
    pub related_issue_id: Uuid,
    pub relation_type: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IssueRefresh {
    pub issues: BTreeMap<Uuid, Issue>,
    pub missing: BTreeSet<Uuid>,
}
#[derive(Clone)]
pub struct LinearClient {
    http: HttpCore,
    workspace_id: Uuid,
    selection: LinearSelection,
    uploads: Arc<Mutex<BTreeMap<Uuid, UploadGrant>>>,
    asset_transport: Arc<dyn HttpTransport>,
}
fn graph_data(value: Value, headers: &HeaderMap) -> Result<Value> {
    if let Some(errors) = value.get("errors") {
        if !errors.is_array() {
            return Err(Error::new(
                "provider_response",
                "malformed GraphQL error collection",
            ));
        }
        if !errors.as_array().unwrap().is_empty() {
            let rate_limited = errors.as_array().unwrap().iter().any(|e| {
                e.pointer("/extensions/code")
                    .and_then(Value::as_str)
                    .is_some_and(|c| matches!(c, "RATELIMITED" | "RATE_LIMITED"))
            });
            let mut error = Error::new(if rate_limited{"rate_limited"}else{"provider_response"},"GraphQL returned errors; all partial data discarded").detail(json!({"error_count":errors.as_array().unwrap().len(),"rate_limit":RateLimit::from_headers(headers,false)}));
            error.retryable = rate_limited;
            return Err(error);
        }
    }
    value
        .get("data")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or_else(|| {
            Error::new(
                "provider_response",
                "GraphQL response lacks a complete data object",
            )
        })
}
const ISSUE_FIELDS: &str = "id identifier title description priority createdAt updatedAt url team { id } project { id } creator { id } assignee { id } parent { id } state { id name type } labelIds";
const COMMENT_FIELDS: &str = "id body createdAt updatedAt editedAt url user { id } onBehalfOf { id } botActor { __typename } issue { id }";
const HISTORY_FIELDS: &str = "id actor { id } actorId actors { id } botActor { __typename } addedLabelIds removedLabelIds createdAt updatedAt fromParentId toParentId updatedDescription";
impl LinearClient {
    pub fn new(
        policy: HttpPolicy,
        workspace_id: Uuid,
        selection: LinearSelection,
        credential: Credential,
    ) -> Result<Self> {
        Self::with_transport(policy, workspace_id, selection, credential, None)
    }
    pub fn with_transport(
        policy: HttpPolicy,
        workspace_id: Uuid,
        selection: LinearSelection,
        credential: Credential,
        transport: Option<Arc<dyn HttpTransport>>,
    ) -> Result<Self> {
        if workspace_id.is_nil() || Uuid::parse_str(&policy.account_id).is_err() {
            return Err(Error::invalid(
                "Linear connection needs canonical workspace and account UUIDs",
            ));
        }
        selection.teams.validate()?;
        selection.projects.validate()?;
        selection.users.validate()?;
        let asset_transport = match &transport {
            Some(transport) => transport.clone(),
            None => Arc::new(ReqwestTransport::bounded(policy.timeout, MAX_VIDEO_BYTES)?),
        };
        Ok(Self {
            asset_transport,
            http: HttpCore::new(policy, credential, false, transport)?,
            workspace_id,
            selection,
            uploads: Arc::new(Mutex::new(BTreeMap::new())),
        })
    }
    async fn graph(&self, query: &str, variables: Value) -> Result<Value> {
        let (value, headers) = self
            .http
            .json(
                Method::Post,
                self.http.policy.endpoint.clone(),
                Some(json!({"query":query,"variables":variables})),
            )
            .await?;
        graph_data(value, &headers)
    }
    pub async fn identity(&self) -> Result<AccountIdentity> {
        let data = self
            .graph(
                "query KegariIdentity { viewer { id name } organization { id } }",
                json!({}),
            )
            .await?;
        let account = uuid_at(&data, "/viewer/id")?;
        let workspace = uuid_at(&data, "/organization/id")?;
        if account.to_string() != self.http.policy.account_id || workspace != self.workspace_id {
            return Err(Error::new(
                "identity_mismatch",
                "Linear credential identity differs from configured account/workspace",
            ));
        }
        Ok(AccountIdentity {
            account_id: account.to_string(),
            workspace_id: Some(workspace),
            display_name: text_at(&data, "/viewer/name")?.into(),
            permission_scopes: None,
            observed_at: Utc::now(),
        })
    }
    async fn collect(
        &self,
        query: &str,
        mut variables: Value,
        pointer: &str,
    ) -> Result<Vec<Value>> {
        let variables = variables
            .as_object_mut()
            .ok_or_else(|| Error::invalid("GraphQL variables must be an object"))?;
        let mut cursor: Option<String> = None;
        let mut cursors = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut result = Vec::new();
        for _ in 0..self.http.policy.max_pages {
            variables.insert("after".into(), json!(cursor));
            let data = self.graph(query, Value::Object(variables.clone())).await?;
            let connection = data
                .pointer(pointer)
                .filter(|v| v.is_object())
                .ok_or_else(|| {
                    Error::new(
                        "provider_response",
                        "requested GraphQL connection is missing",
                    )
                })?;
            let nodes = connection
                .get("nodes")
                .and_then(Value::as_array)
                .ok_or_else(|| Error::new("provider_response", "GraphQL connection lacks nodes"))?;
            for node in nodes {
                let id = text_at(node, "/id")?.to_owned();
                if !ids.insert(id) {
                    return Err(Error::new(
                        "provider_pagination",
                        "duplicate provider ID across GraphQL pages",
                    ));
                }
                result.push(node.clone());
                if result.len() > self.http.policy.max_items {
                    return Err(Error::new(
                        "provider_pagination",
                        "complete provider collection exceeds declared bound",
                    ));
                }
            }
            let more = connection
                .pointer("/pageInfo/hasNextPage")
                .and_then(Value::as_bool)
                .ok_or_else(|| {
                    Error::new("provider_pagination", "GraphQL page lacks hasNextPage")
                })?;
            if !more {
                return Ok(result);
            }
            let next = text_at(connection, "/pageInfo/endCursor")?.to_owned();
            if nodes.is_empty() || !cursors.insert(next.clone()) {
                return Err(Error::new(
                    "provider_pagination",
                    "GraphQL page loop or empty unfinished page",
                ));
            }
            cursor = Some(next);
        }
        Err(Error::new(
            "provider_pagination",
            "GraphQL collection did not complete within declared page bound",
        ))
    }
    fn discovery_filter(&self, states: Option<&BTreeSet<Uuid>>) -> Value {
        let mut filters = vec![json!({"parent":{"null":true}})];
        if let Selector::Ids(ids) = &self.selection.teams {
            filters.push(json!({"team":{"id":{"in":ids}}}));
        }
        if let Selector::Ids(ids) = &self.selection.projects {
            filters.push(json!({"project":{"id":{"in":ids}}}));
        }
        if let Selector::Ids(ids) = &self.selection.users {
            filters
                .push(json!({"or":[{"creator":{"id":{"in":ids}}},{"assignee":{"id":{"in":ids}}}]}));
        }
        if let Some(states) = states {
            filters.push(json!({"state":{"id":{"in":states}}}));
        }
        json!({"and":filters})
    }
    pub async fn discover_parents(&self) -> Result<Vec<Issue>> {
        self.discover_in_states(None).await
    }
    pub async fn discover_in_states(&self, states: Option<&BTreeSet<Uuid>>) -> Result<Vec<Issue>> {
        if states.is_some_and(BTreeSet::is_empty) {
            return Ok(Vec::new());
        }
        self.identity().await?;
        let query = format!(
            "query KegariParents($after:String,$filter:IssueFilter!) {{ issues(first:100,after:$after,filter:$filter,includeArchived:true) {{ nodes {{ {ISSUE_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }} }}"
        );
        let nodes = self
            .collect(
                &query,
                json!({"filter":self.discovery_filter(states)}),
                "/issues",
            )
            .await?;
        let mut issues = Vec::new();
        for node in nodes {
            let issue = self.normalize_issue(node).await?;
            if issue.parent_id.is_some()
                || !self.selection.includes(
                    &issue.team_id.to_string(),
                    issue
                        .project_id
                        .as_ref()
                        .map(|id| id.to_string())
                        .as_deref(),
                    issue
                        .creator_id
                        .as_ref()
                        .map(|id| id.to_string())
                        .as_deref()
                        .unwrap_or(""),
                    issue
                        .assignee_id
                        .as_ref()
                        .map(|id| id.to_string())
                        .as_deref(),
                )
            {
                return Err(Error::new(
                    "provider_response",
                    "provider returned a parent outside the requested selection",
                ));
            }
            if states.is_some_and(|states| !states.contains(&issue.state_id)) {
                return Err(Error::new(
                    "provider_response",
                    "provider returned an issue outside configured-state query",
                ));
            }
            issues.push(issue);
        }
        Ok(issues)
    }
    pub async fn refresh(&self, ids: &BTreeSet<Uuid>) -> Result<IssueRefresh> {
        if ids.is_empty() {
            return Ok(IssueRefresh {
                issues: BTreeMap::new(),
                missing: BTreeSet::new(),
            });
        }
        if ids.iter().any(Uuid::is_nil) {
            return Err(Error::invalid(
                "issue refresh requires canonical nonnil IDs",
            ));
        }
        self.identity().await?;
        let mut issues = BTreeMap::new();
        let query = format!(
            "query KegariRefresh($after:String,$filter:IssueFilter!) {{ issues(first:100,after:$after,filter:$filter,includeArchived:true) {{ nodes {{ {ISSUE_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }} }}"
        );
        for chunk in ids.iter().collect::<Vec<_>>().chunks(PAGE_SIZE) {
            let nodes = self
                .collect(&query, json!({"filter":{"id":{"in":chunk}}}), "/issues")
                .await?;
            for node in nodes {
                let issue = self.normalize_issue(node).await?;
                if !ids.contains(&issue.id) || issues.insert(issue.id, issue).is_some() {
                    return Err(Error::new(
                        "provider_response",
                        "ID refresh returned duplicate or unrequested work",
                    ));
                }
            }
        }
        let missing = ids
            .iter()
            .filter(|id| !issues.contains_key(id))
            .copied()
            .collect();
        Ok(IssueRefresh { issues, missing })
    }
    pub async fn issue(&self, id: Uuid) -> Result<Issue> {
        self.identity().await?;
        let query =
            format!("query KegariIssue($id:String!) {{ issue(id:$id) {{ {ISSUE_FIELDS} }} }}");
        let value = self.graph(&query, json!({"id":id})).await?;
        let issue = self
            .normalize_issue(
                value
                    .get("issue")
                    .cloned()
                    .ok_or_else(|| Error::new("provider_response", "requested issue missing"))?,
            )
            .await?;
        if issue.id != id {
            return Err(Error::new(
                "identity_mismatch",
                "Linear issue identity differs from requested UUID",
            ));
        }
        Ok(issue)
    }
    pub async fn children(&self, parent: Uuid) -> Result<Vec<Issue>> {
        self.identity().await?;
        let query = format!(
            "query KegariChildren($id:String!,$after:String) {{ issue(id:$id) {{ children(first:100,after:$after,includeArchived:true) {{ nodes {{ {ISSUE_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }} }} }}"
        );
        let nodes = self
            .collect(&query, json!({"id":parent}), "/issue/children")
            .await?;
        let mut children = Vec::new();
        for node in nodes {
            let child = self.normalize_issue(node).await?;
            if child.parent_id != Some(parent) {
                return Err(Error::new(
                    "provider_response",
                    "child hierarchy changed during enumeration",
                ));
            }
            children.push(child);
        }
        Ok(children)
    }
    async fn normalize_issue(&self, node: Value) -> Result<Issue> {
        let id = uuid_at(&node, "/id")?;
        let mut diagnostics = Vec::new();
        let query = "query KegariLabels($id:String!,$after:String) { issue(id:$id) { labels(first:100,after:$after,includeArchived:true) { nodes { id name } pageInfo { hasNextPage endCursor } } } }";
        let labels = self
            .collect(query, json!({"id":id}), "/issue/labels")
            .await?;
        let label_ids = uuid_array(
            node.get("labelIds")
                .ok_or_else(|| Error::new("provider_response", "issue labels are missing"))?,
        )?;
        let mut normalized = BTreeMap::new();
        let mut returned_ids = BTreeSet::new();
        for value in labels {
            let id = uuid_at(&value, "/id")?;
            returned_ids.insert(id);
            let name = value
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| Error::new("provider_response", "label name is malformed"))?
                .to_owned();
            let comparison_name = name.trim().to_lowercase();
            if comparison_name.is_empty() {
                diagnostics.push("blank_label_name".into());
                continue;
            }
            normalized.insert(
                id,
                Label {
                    id,
                    name,
                    comparison_name,
                },
            );
        }
        // Blank names are removed from comparison but cannot hide a required label ID.
        let required_ids = label_ids.into_iter().collect::<BTreeSet<_>>();
        if returned_ids != required_ids {
            return Err(Error::new(
                "provider_response",
                "issue labels changed during complete collection",
            ));
        }
        let priority = node
            .get("priority")
            .and_then(Value::as_f64)
            .filter(|v| {
                v.is_finite() && v.fract() == 0.0 && *v >= i64::MIN as f64 && *v <= i64::MAX as f64
            })
            .map(|v| v as i64);
        if priority.is_none() && !node.get("priority").is_none_or(Value::is_null) {
            diagnostics.push("malformed_optional_priority".into());
        }
        let created_at = optional_time(&node, "/createdAt", &mut diagnostics);
        let updated_at = optional_time(&node, "/updatedAt", &mut diagnostics);
        let description = match node.get("description") {
            Some(Value::String(s)) => Some(s.clone()),
            None | Some(Value::Null) => None,
            _ => {
                diagnostics.push("malformed_optional_description".into());
                None
            }
        };
        Ok(Issue {
            id,
            workspace_id: self.workspace_id,
            endpoint: self.http.policy.endpoint.to_string(),
            identifier: text_at(&node, "/identifier")?.into(),
            title: text_at(&node, "/title")?.into(),
            description,
            team_id: uuid_at(&node, "/team/id")?,
            project_id: optional_uuid(&node, "/project/id")?,
            creator_id: optional_uuid(&node, "/creator/id")?,
            assignee_id: optional_uuid(&node, "/assignee/id")?,
            parent_id: optional_uuid(&node, "/parent/id")?,
            state_id: uuid_at(&node, "/state/id")?,
            state_name: text_at(&node, "/state/name")?.into(),
            state_type: text_at(&node, "/state/type")?.trim().to_lowercase(),
            labels: normalized.into_values().collect(),
            priority,
            created_at,
            updated_at,
            url: validated_url(text_at(&node, "/url")?)?,
            diagnostics,
        })
    }
    pub async fn comments(&self, issue: Uuid) -> Result<Vec<Comment>> {
        self.identity().await?;
        let query = format!(
            "query KegariComments($id:String!,$after:String) {{ issue(id:$id) {{ comments(first:100,after:$after,includeArchived:true) {{ nodes {{ {COMMENT_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }} }} }}"
        );
        let nodes = self
            .collect(&query, json!({"id":issue}), "/issue/comments")
            .await?;
        nodes
            .into_iter()
            .map(|v| normalize_comment(v, issue))
            .collect()
    }
    pub async fn history(&self, issue: Uuid) -> Result<Vec<HistoryEntry>> {
        self.identity().await?;
        let query = format!(
            "query KegariHistory($id:String!,$after:String) {{ issue(id:$id) {{ history(first:100,after:$after,includeArchived:true) {{ nodes {{ {HISTORY_FIELDS} }} pageInfo {{ hasNextPage endCursor }} }} }} }}"
        );
        let nodes = self
            .collect(&query, json!({"id":issue}), "/issue/history")
            .await?;
        nodes
            .into_iter()
            .map(|v| {
                let mut diagnostics = Vec::new();
                let actor_id = optional_uuid(&v, "/actor/id")?;
                let direct = optional_uuid(&v, "/actorId")?;
                if direct.is_some() && actor_id != direct {
                    return Err(Error::new(
                        "provider_response",
                        "history actor fields disagree",
                    ));
                }
                let actors = v
                    .get("actors")
                    .filter(|v| !v.is_null())
                    .map(|v| {
                        v.as_array().ok_or_else(|| {
                            Error::new("provider_response", "history actors malformed")
                        })
                    })
                    .transpose()?
                    .map(|a| {
                        a.iter()
                            .map(|a| uuid_at(a, "/id"))
                            .collect::<Result<Vec<_>>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                Ok(HistoryEntry {
                    id: uuid_at(&v, "/id")?,
                    actor_id,
                    actor_ids: actors,
                    bot: v.get("botActor").is_some_and(|v| !v.is_null()),
                    added_labels: nullable_uuid_array(v.get("addedLabelIds"))?,
                    removed_labels: nullable_uuid_array(v.get("removedLabelIds"))?,
                    created_at: optional_time(&v, "/createdAt", &mut diagnostics),
                    updated_at: optional_time(&v, "/updatedAt", &mut diagnostics),
                    from_parent: optional_uuid(&v, "/fromParentId")?,
                    to_parent: optional_uuid(&v, "/toParentId")?,
                    description_updated: v.get("updatedDescription").and_then(Value::as_bool),
                })
            })
            .collect()
    }
    pub async fn dependencies(&self, issue: Uuid) -> Result<Vec<Dependency>> {
        self.identity().await?;
        let mut dependencies = BTreeMap::new();
        for field in ["relations", "inverseRelations"] {
            let query = format!(
                "query KegariDependencies($id:String!,$after:String) {{ issue(id:$id) {{ {field}(first:100,after:$after,includeArchived:true) {{ nodes {{ id type issue {{ id }} relatedIssue {{ id }} }} pageInfo {{ hasNextPage endCursor }} }} }} }}"
            );
            for v in self
                .collect(&query, json!({"id":issue}), &format!("/issue/{field}"))
                .await?
            {
                let relation = Dependency {
                    id: uuid_at(&v, "/id")?,
                    issue_id: uuid_at(&v, "/issue/id")?,
                    related_issue_id: uuid_at(&v, "/relatedIssue/id")?,
                    relation_type: text_at(&v, "/type")?.into(),
                };
                if relation.issue_id != issue && relation.related_issue_id != issue {
                    return Err(Error::new(
                        "provider_response",
                        "dependency does not reference requested issue",
                    ));
                }
                if let Some(prior) = dependencies.insert(relation.id, relation.clone())
                    && canonical_digest(1, &prior)? != canonical_digest(1, &relation)?
                {
                    return Err(Error::new(
                        "provider_response",
                        "dependency changed during collection",
                    ));
                }
            }
        }
        Ok(dependencies.into_values().collect())
    }
    pub async fn actor(&self, user: Uuid) -> Result<AccountIdentity> {
        self.identity().await?;
        let v = self
            .graph(
                "query KegariActor($id:String!) { user(id:$id) { id name } }",
                json!({"id":user}),
            )
            .await?;
        if uuid_at(&v, "/user/id")? != user {
            return Err(Error::new(
                "identity_mismatch",
                "actor lookup returned another user",
            ));
        }
        Ok(AccountIdentity {
            account_id: user.to_string(),
            workspace_id: Some(self.workspace_id),
            display_name: text_at(&v, "/user/name")?.into(),
            permission_scopes: None,
            observed_at: Utc::now(),
        })
    }
    pub async fn label(&self, label: Uuid) -> Result<Label> {
        self.identity().await?;
        let v = self
            .graph(
                "query KegariLabel($id:String!) { issueLabel(id:$id) { id name } }",
                json!({"id":label}),
            )
            .await?;
        if uuid_at(&v, "/issueLabel/id")? != label {
            return Err(Error::new(
                "identity_mismatch",
                "label lookup returned another ID",
            ));
        }
        let name = text_at(&v, "/issueLabel/name")?.to_owned();
        Ok(Label {
            id: label,
            comparison_name: name.trim().to_lowercase(),
            name,
        })
    }
}
fn normalize_comment(v: Value, issue: Uuid) -> Result<Comment> {
    if uuid_at(&v, "/issue/id")? != issue {
        return Err(Error::new(
            "provider_response",
            "comment belongs to another issue",
        ));
    }
    let body = v
        .get("body")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::new("provider_response", "comment body is malformed"))?
        .to_owned();
    let mut diagnostics = Vec::new();
    Ok(Comment {
        id: uuid_at(&v, "/id")?,
        issue_id: issue,
        body_digest: bytes_digest(body.as_bytes()),
        body,
        user_id: optional_uuid(&v, "/user/id")?,
        delegated: v.get("onBehalfOf").is_some_and(|v| !v.is_null()),
        bot: v.get("botActor").is_some_and(|v| !v.is_null()),
        created_at: optional_time(&v, "/createdAt", &mut diagnostics),
        updated_at: optional_time(&v, "/updatedAt", &mut diagnostics),
        edited_at: optional_time(&v, "/editedAt", &mut diagnostics),
        url: validated_url(text_at(&v, "/url")?)?,
    })
}

fn uuid_at(value: &Value, path: &str) -> Result<Uuid> {
    let id = Uuid::parse_str(text_at(value, path)?).map_err(|_| {
        Error::new(
            "provider_response",
            format!("required provider UUID {path} is malformed"),
        )
    })?;
    if id.is_nil() {
        return Err(Error::new(
            "provider_response",
            "provider returned a nil identity",
        ));
    }
    Ok(id)
}
fn optional_uuid(value: &Value, path: &str) -> Result<Option<Uuid>> {
    match value.pointer(path) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => uuid_at(value, path).map(Some),
    }
}
fn uuid_array(value: &Value) -> Result<Vec<Uuid>> {
    value
        .as_array()
        .ok_or_else(|| Error::new("provider_response", "provider ID collection is malformed"))?
        .iter()
        .map(|v| {
            let s = v
                .as_str()
                .ok_or_else(|| Error::new("provider_response", "provider ID is malformed"))?;
            let id = Uuid::parse_str(s)
                .map_err(|_| Error::new("provider_response", "provider UUID is malformed"))?;
            if id.is_nil() {
                return Err(Error::new("provider_response", "provider UUID is nil"));
            }
            Ok(id)
        })
        .collect()
}
fn nullable_uuid_array(value: Option<&Value>) -> Result<Vec<Uuid>> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(v) => uuid_array(v),
    }
}
fn optional_time(
    value: &Value,
    path: &str,
    diagnostics: &mut Vec<String>,
) -> Option<DateTime<Utc>> {
    match value.pointer(path) {
        None | Some(Value::Null) => None,
        Some(v) => match v
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        {
            Some(t) => Some(t.with_timezone(&Utc)),
            None => {
                diagnostics.push(format!("malformed_optional_timestamp:{path}"));
                None
            }
        },
    }
}

fn positive_id(value: &Value, path: &str) -> Result<u64> {
    value
        .pointer(path)
        .and_then(Value::as_u64)
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            Error::new(
                "provider_response",
                format!("positive provider ID {path} is missing"),
            )
        })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryIdentity {
    pub id: u64,
    pub owner: String,
    pub name: String,
    pub owner_id: u64,
    pub organization_id: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Repository {
    pub identity: RepositoryIdentity,
    pub default_branch: String,
    pub private: bool,
    pub archived: bool,
    pub disabled: bool,
    pub squash_allowed: Option<bool>,
    pub permissions: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullRequest {
    pub repository_id: u64,
    pub id: u64,
    pub number: u64,
    pub author_id: u64,
    pub state: String,
    pub draft: bool,
    pub head_sha: String,
    pub base_sha: String,
    pub head_ref: String,
    pub base_ref: String,
    pub head_repository_id: Option<u64>,
    pub merged: Option<bool>,
    pub merge_commit_sha: Option<String>,
    pub url: String,
}
/// Literal, bounded task input from the same scoped wire object as PR metadata.
/// Neither this data nor its text grants authority or enables repository code.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequestText {
    pub title: String,
    pub body: Option<String>,
}
impl std::fmt::Debug for PullRequestText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PullRequestText")
            .field("title_bytes", &self.title.len())
            .field("body_bytes", &self.body.as_ref().map(String::len))
            .finish()
    }
}
impl PullRequestText {
    pub fn validate(&self) -> Result<()> {
        if self.title.trim().is_empty() {
            return Err(Error::new(
                "provider_response",
                "independent PR title is missing or blank",
            ));
        }
        let bytes =
            serde_json::to_vec(self).map_err(|_| Error::invalid("PR text encoding failed"))?;
        if bytes.len() > MAX_PULL_REQUEST_TEXT_BYTES {
            return Err(Error::new(
                "bounded_input",
                "independent PR text exceeds the production context bound",
            ));
        }
        Ok(())
    }
    pub fn from_value(value: &Value) -> Result<Self> {
        let title = value.get("title").and_then(Value::as_str).ok_or_else(|| {
            Error::new(
                "provider_response",
                "requested independent PR title is absent or malformed",
            )
        })?;
        let body = match value.get("body") {
            Some(Value::Null) => None,
            Some(Value::String(body)) => Some(body.clone()),
            _ => {
                return Err(Error::new(
                    "provider_response",
                    "requested independent PR body is absent or malformed",
                ));
            }
        };
        let text = Self {
            title: title.into(),
            body,
        };
        text.validate()?;
        Ok(text)
    }
}
/// An observation only. Generic PullRequest serialization remains unchanged.
#[derive(Clone, Debug, Serialize)]
pub struct IndependentPullRequest {
    #[serde(flatten)]
    pub pull_request: PullRequest,
    #[serde(flatten)]
    pub text: PullRequestText,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GithubRecord {
    pub repository_id: u64,
    pub id: u64,
    pub observed_at: DateTime<Utc>,
    pub data: Value,
}
/// Jobs' mutable labels/names are never evidence of hosted execution. Only a
/// separately verified signed OIDC proof may establish Hosted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunnerEnvironment {
    Unknown,
    Hosted,
    SelfHosted,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub repository_id: u64,
    pub id: u64,
    pub attempt: u64,
    pub workflow_id: u64,
    pub actor_id: u64,
    pub triggering_actor_id: u64,
    pub event: String,
    pub head_sha: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub workflow_path: String,
    pub url: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkflowJob {
    pub repository_id: u64,
    pub id: u64,
    pub run_id: u64,
    pub attempt: u64,
    pub name: String,
    pub head_sha: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub check_run_id: u64,
    pub environment: RunnerEnvironment,
    pub steps: Vec<Value>,
}
#[derive(Clone)]
pub struct GithubClient {
    http: HttpCore,
    selection: GithubSelection,
    object_transport: Arc<dyn HttpTransport>,
}
impl GithubClient {
    pub fn new(
        policy: HttpPolicy,
        selection: GithubSelection,
        credential: Credential,
    ) -> Result<Self> {
        Self::with_transport(policy, selection, credential, None)
    }
    pub fn with_transport(
        policy: HttpPolicy,
        selection: GithubSelection,
        credential: Credential,
        transport: Option<Arc<dyn HttpTransport>>,
    ) -> Result<Self> {
        numeric_id(&policy.account_id)?;
        for id in selection
            .repositories
            .iter()
            .chain(&selection.organizations)
            .chain(&selection.pr_authors)
        {
            numeric_id(id)?;
        }
        if selection.repositories.is_empty() && selection.organizations.is_empty() {
            return Err(Error::invalid(
                "GitHub needs a nonempty repository/organization selection",
            ));
        }
        let object_transport = match &transport {
            Some(transport) => transport.clone(),
            None => Arc::new(ReqwestTransport::bounded(policy.timeout, MAX_VIDEO_BYTES)?),
        };
        Ok(Self {
            object_transport,
            http: HttpCore::new(policy, credential, true, transport)?,
            selection,
        })
    }
    pub async fn identity(&self) -> Result<AccountIdentity> {
        let (v, headers) = self
            .http
            .json(Method::Get, self.http.url(&["user"])?, None)
            .await?;
        let id = positive_id(&v, "/id")?;
        require_github_user(&v)?;
        if id.to_string() != self.http.policy.account_id {
            return Err(Error::new(
                "identity_mismatch",
                "GitHub credential is not the configured user",
            ));
        }
        let scopes = headers
            .get("x-oauth-scopes")
            .and_then(|s| s.to_str().ok())
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            });
        Ok(AccountIdentity {
            account_id: id.to_string(),
            workspace_id: None,
            display_name: text_at(&v, "/login")?.into(),
            permission_scopes: scopes,
            observed_at: Utc::now(),
        })
    }
    fn repo_url(&self, repo: &RepositoryIdentity, tail: &[&str]) -> Result<Url> {
        self.check_scope(repo)?;
        let mut parts = vec!["repos", repo.owner.as_str(), repo.name.as_str()];
        parts.extend_from_slice(tail);
        self.http.url(&parts)
    }
    pub fn check_scope(&self, repo: &RepositoryIdentity) -> Result<()> {
        if repo.id == 0
            || repo.owner_id == 0
            || !self.selection.permits_repository(
                &repo.id.to_string(),
                repo.organization_id.map(|id| id.to_string()).as_deref(),
            )
        {
            return Err(Error::new(
                "scope_conflict",
                "repository is outside configured GitHub selection",
            ));
        }
        Ok(())
    }
    /// Resolve documented owner/name endpoints, then require the stable ID. A
    /// rename/transfer redirect is not followed; rediscovery is explicit.
    pub async fn repository(&self, expected: &RepositoryIdentity) -> Result<Repository> {
        self.identity().await?;
        let (v, _) = self
            .http
            .json(Method::Get, self.repo_url(expected, &[])?, None)
            .await?;
        let repo = normalize_repository(v)?;
        if repo.identity != *expected {
            return Err(Error::new(
                "identity_mismatch",
                "repository identity/ownership changed",
            ));
        }
        self.check_scope(&repo.identity)?;
        Ok(repo)
    }
    /// /user/repos enumerates accessible repositories. Organization selection
    /// additionally uses /user/orgs and /orgs/{login}/repos to include accessible
    /// public repositories not returned as user affiliations.
    pub async fn discover_repositories(&self) -> Result<Vec<Repository>> {
        self.identity().await?;
        let mut values = self
            .pages(
                self.http.url(&["user", "repos"])?,
                &[
                    ("affiliation", "owner,collaborator,organization_member"),
                    ("visibility", "all"),
                ],
                None,
                None,
            )
            .await?;
        if !self.selection.organizations.is_empty() {
            let mut organizations = BTreeMap::new();
            for value in &values {
                if value.pointer("/owner/type").and_then(Value::as_str) == Some("Organization") {
                    organizations.insert(
                        positive_id(value, "/owner/id")?.to_string(),
                        text_at(value, "/owner/login")?.to_owned(),
                    );
                }
            }
            for org in self
                .pages(self.http.url(&["user", "orgs"])?, &[], None, None)
                .await?
            {
                organizations.insert(
                    positive_id(&org, "/id")?.to_string(),
                    text_at(&org, "/login")?.to_owned(),
                );
            }
            for id in &self.selection.organizations {
                let login = organizations.get(id).ok_or_else(||Error::new("identity_resolution_required","configured organization ID could not be resolved from verified accessible identities"))?;
                values.extend(
                    self.pages(
                        self.http.url(&["orgs", login, "repos"])?,
                        &[("type", "all")],
                        None,
                        None,
                    )
                    .await?,
                );
            }
        }
        let mut result = BTreeMap::new();
        for value in values {
            let repo = normalize_repository(value)?;
            if self.check_scope(&repo.identity).is_ok()
                && let Some(old) = result.insert(repo.identity.id, repo.clone())
                && canonical_digest(1, &old)? != canonical_digest(1, &repo)?
            {
                return Err(Error::new(
                    "provider_response",
                    "repository changed during complete discovery",
                ));
            }
        }
        Ok(result.into_values().collect())
    }
    async fn pages(
        &self,
        mut url: Url,
        filters: &[(&str, &str)],
        field: Option<&str>,
        repo: Option<&RepositoryIdentity>,
    ) -> Result<Vec<Value>> {
        url.query_pairs_mut()
            .extend_pairs(filters.iter().copied())
            .append_pair("per_page", "100")
            .append_pair("page", "1");
        let first = url.clone();
        let mut seen = BTreeSet::new();
        let mut ids = BTreeSet::new();
        let mut output = Vec::new();
        let mut total = None;
        for page in 1..=self.http.policy.max_pages {
            if !seen.insert(url.to_string()) {
                return Err(Error::new("provider_pagination", "GitHub pagination loop"));
            }
            let (value, headers) = self.http.json(Method::Get, url.clone(), None).await?;
            let nodes = match field {
                Some(f) => value.get(f),
                None => Some(&value),
            }
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Error::new("provider_response", "GitHub list response is not an array")
            })?;
            if let Some(count) = value.get("total_count") {
                let count = count.as_u64().ok_or_else(|| {
                    Error::new("provider_pagination", "GitHub total_count is malformed")
                })?;
                if total.is_some_and(|old| old != count) {
                    return Err(Error::new(
                        "provider_pagination",
                        "GitHub collection changed during enumeration",
                    ));
                }
                total = Some(count);
            }
            for node in nodes {
                let id = positive_id(node, "/id")?;
                if !ids.insert(id) {
                    return Err(Error::new(
                        "provider_pagination",
                        "GitHub list returned duplicate IDs",
                    ));
                }
                output.push(node.clone());
                if output.len() > self.http.policy.max_items {
                    return Err(Error::new(
                        "provider_pagination",
                        "GitHub collection exceeds configured bound",
                    ));
                }
            }
            let next = next_link(&headers)?;
            match next {
                None => {
                    if total.is_some_and(|count| count != output.len() as u64) {
                        return Err(Error::new(
                            "provider_pagination",
                            "GitHub collection ended before total_count",
                        ));
                    }
                    return Ok(output);
                }
                Some(next) => {
                    if nodes.is_empty() {
                        return Err(Error::new(
                            "provider_pagination",
                            "empty GitHub page has a continuation",
                        ));
                    }
                    validate_next_page(&first, &next, page + 1, repo)?;
                    url = next;
                }
            }
        }
        Err(Error::new(
            "provider_pagination",
            "GitHub collection exceeded configured page bound",
        ))
    }
    async fn pull_request_values(&self, repo: &RepositoryIdentity) -> Result<Vec<Value>> {
        self.repository(repo).await?;
        self.pages(
            self.repo_url(repo, &["pulls"])?,
            &[("state", "all")],
            None,
            Some(repo),
        )
        .await
    }
    async fn pull_request_value(&self, repo: &RepositoryIdentity, number: u64) -> Result<Value> {
        self.repository(repo).await?;
        let (value, _) = self
            .http
            .json(
                Method::Get,
                self.repo_url(repo, &["pulls", &nonzero(number)?])?,
                None,
            )
            .await?;
        Ok(value)
    }
    pub async fn pull_requests(&self, repo: &RepositoryIdentity) -> Result<Vec<PullRequest>> {
        self.pull_request_values(repo)
            .await?
            .into_iter()
            .map(|value| normalize_pr(value, repo.id))
            .collect()
    }
    pub async fn pull_request(
        &self,
        repo: &RepositoryIdentity,
        number: u64,
    ) -> Result<PullRequest> {
        let pr = normalize_pr(self.pull_request_value(repo, number).await?, repo.id)?;
        if pr.number != number {
            return Err(Error::new("identity_mismatch", "PR number changed"));
        }
        Ok(pr)
    }
    pub async fn independent_review_pull_request(
        &self,
        repo: &RepositoryIdentity,
        number: u64,
    ) -> Result<IndependentPullRequest> {
        let pr = normalize_independent_pr(self.pull_request_value(repo, number).await?, repo.id)?;
        if pr.pull_request.number != number {
            return Err(Error::new(
                "identity_mismatch",
                "independent PR number changed",
            ));
        }
        Ok(pr)
    }
    async fn records(
        &self,
        repo: &RepositoryIdentity,
        tail: &[&str],
        filters: &[(&str, &str)],
        field: Option<&str>,
    ) -> Result<Vec<GithubRecord>> {
        self.repository(repo).await?;
        self.pages(self.repo_url(repo, tail)?, filters, field, Some(repo))
            .await?
            .into_iter()
            .map(|data| {
                Ok(GithubRecord {
                    repository_id: repo.id,
                    id: positive_id(&data, "/id")?,
                    observed_at: Utc::now(),
                    data,
                })
            })
            .collect()
    }
    pub async fn checks(&self, repo: &RepositoryIdentity, head: &str) -> Result<Vec<GithubRecord>> {
        self.records(
            repo,
            &["commits", sha(head)?, "check-runs"],
            &[("filter", "all")],
            Some("check_runs"),
        )
        .await
    }
    pub async fn statuses(
        &self,
        repo: &RepositoryIdentity,
        head: &str,
    ) -> Result<Vec<GithubRecord>> {
        self.records(repo, &["commits", sha(head)?, "statuses"], &[], None)
            .await
    }
    pub async fn reviews(&self, repo: &RepositoryIdentity, pr: u64) -> Result<Vec<GithubRecord>> {
        self.records(repo, &["pulls", &nonzero(pr)?, "reviews"], &[], None)
            .await
    }
    pub async fn comments(&self, repo: &RepositoryIdentity, pr: u64) -> Result<Vec<GithubRecord>> {
        self.records(repo, &["issues", &nonzero(pr)?, "comments"], &[], None)
            .await
    }
    pub async fn workflows(&self, repo: &RepositoryIdentity) -> Result<Vec<GithubRecord>> {
        self.records(repo, &["actions", "workflows"], &[], Some("workflows"))
            .await
    }
    pub async fn deployments(
        &self,
        repo: &RepositoryIdentity,
        head: &str,
    ) -> Result<Vec<GithubRecord>> {
        self.records(repo, &["deployments"], &[("sha", sha(head)?)], None)
            .await
    }
    pub async fn deployment_statuses(
        &self,
        repo: &RepositoryIdentity,
        deployment: u64,
    ) -> Result<Vec<GithubRecord>> {
        self.records(
            repo,
            &["deployments", &nonzero(deployment)?, "statuses"],
            &[],
            None,
        )
        .await
    }
    pub async fn workflow_runs(
        &self,
        repo: &RepositoryIdentity,
        workflow: u64,
        head: Option<&str>,
    ) -> Result<Vec<WorkflowRun>> {
        self.repository(repo).await?;
        let filters = match head {
            Some(s) => vec![("head_sha", sha(s)?)],
            None => Vec::new(),
        };
        self.pages(
            self.repo_url(repo, &["actions", "workflows", &nonzero(workflow)?, "runs"])?,
            &filters,
            Some("workflow_runs"),
            Some(repo),
        )
        .await?
        .into_iter()
        .map(|v| {
            let run = normalize_run(v, repo.id)?;
            if run.workflow_id != workflow || head.is_some_and(|s| s != run.head_sha) {
                return Err(Error::new(
                    "provider_response",
                    "run filter identity mismatch",
                ));
            }
            Ok(run)
        })
        .collect()
    }
    pub async fn workflow_run(
        &self,
        repo: &RepositoryIdentity,
        run: u64,
        attempt: Option<u64>,
    ) -> Result<WorkflowRun> {
        self.repository(repo).await?;
        let run_id = nonzero(run)?;
        let mut parts = vec!["actions", "runs", run_id.as_str()];
        let attempt_id = attempt.map(nonzero).transpose()?;
        if let Some(a) = &attempt_id {
            parts.extend(["attempts", a]);
        }
        let (v, _) = self
            .http
            .json(Method::Get, self.repo_url(repo, &parts)?, None)
            .await?;
        let value = normalize_run(v, repo.id)?;
        if value.id != run || attempt.is_some_and(|a| a != value.attempt) {
            return Err(Error::new("identity_mismatch", "run or attempt changed"));
        }
        Ok(value)
    }
    pub async fn jobs(
        &self,
        repo: &RepositoryIdentity,
        run: u64,
        attempt: u64,
    ) -> Result<Vec<WorkflowJob>> {
        self.workflow_run(repo, run, Some(attempt)).await?;
        self.pages(
            self.repo_url(
                repo,
                &[
                    "actions",
                    "runs",
                    &nonzero(run)?,
                    "attempts",
                    &nonzero(attempt)?,
                    "jobs",
                ],
            )?,
            &[],
            Some("jobs"),
            Some(repo),
        )
        .await?
        .into_iter()
        .map(|v| {
            if positive_id(&v, "/run_id")? != run || positive_id(&v, "/run_attempt")? != attempt {
                return Err(Error::new(
                    "identity_mismatch",
                    "job belongs to another attempt",
                ));
            }
            let url = Url::parse(text_at(&v, "/check_run_url")?)
                .map_err(|_| Error::new("provider_response", "job check URL malformed"))?;
            let check_id = url
                .path_segments()
                .and_then(|mut s| s.next_back())
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|id| *id > 0)
                .ok_or_else(|| {
                    Error::new("provider_response", "job lacks exact check run identity")
                })?;
            let expected = self.repo_url(repo, &["check-runs", &check_id.to_string()])?;
            if url != expected
                && url
                    != self.http.url(&[
                        "repositories",
                        &repo.id.to_string(),
                        "check-runs",
                        &check_id.to_string(),
                    ])?
            {
                return Err(Error::new(
                    "identity_mismatch",
                    "job check URL escaped repository",
                ));
            }
            Ok(WorkflowJob {
                repository_id: repo.id,
                id: positive_id(&v, "/id")?,
                run_id: run,
                attempt,
                name: text_at(&v, "/name")?.into(),
                head_sha: sha(text_at(&v, "/head_sha")?)?.into(),
                status: text_at(&v, "/status")?.into(),
                conclusion: optional_string(&v, "/conclusion")?,
                check_run_id: check_id,
                environment: RunnerEnvironment::Unknown,
                steps: v
                    .get("steps")
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(|| Error::new("provider_response", "job steps missing"))?,
            })
        })
        .collect()
    }
    pub async fn reference(&self, repo: &RepositoryIdentity, reference: &str) -> Result<String> {
        self.repository(repo).await?;
        let components = reference_components(reference)?;
        let mut parts = vec!["git", "ref"];
        parts.extend(components);
        let (v, _) = self
            .http
            .json(Method::Get, self.repo_url(repo, &parts)?, None)
            .await?;
        if text_at(&v, "/ref")? != format!("refs/{reference}")
            || text_at(&v, "/object/type")? != "commit"
        {
            return Err(Error::new(
                "provider_response",
                "reference is not the expected direct commit",
            ));
        }
        Ok(sha(text_at(&v, "/object/sha")?)?.into())
    }
    pub async fn commit(&self, repo: &RepositoryIdentity, commit: &str) -> Result<Value> {
        self.repository(repo).await?;
        let (v, _) = self
            .http
            .json(
                Method::Get,
                self.repo_url(repo, &["git", "commits", sha(commit)?])?,
                None,
            )
            .await?;
        if text_at(&v, "/sha")? != commit {
            return Err(Error::new("identity_mismatch", "commit identity changed"));
        }
        sha(text_at(&v, "/tree/sha")?)?;
        for parent in v
            .get("parents")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new("provider_response", "commit parents missing"))?
        {
            sha(text_at(parent, "/sha")?)?;
        }
        Ok(v)
    }
    pub async fn tree(&self, repo: &RepositoryIdentity, tree: &str) -> Result<Value> {
        self.repository(repo).await?;
        let (v, _) = self
            .http
            .json(
                Method::Get,
                self.repo_url(repo, &["git", "trees", sha(tree)?])?,
                None,
            )
            .await?;
        if text_at(&v, "/sha")? != tree
            || v.get("truncated").and_then(Value::as_bool) != Some(false)
        {
            return Err(Error::new(
                "provider_response",
                "Git tree identity differs or enumeration was truncated",
            ));
        }
        Ok(v)
    }
    pub async fn protection(&self, repo: &RepositoryIdentity, branch: &str) -> Result<Value> {
        self.repository(repo).await?;
        let (v, _) = self
            .http
            .json(
                Method::Get,
                self.repo_url(repo, &["branches", branch, "protection"])?,
                None,
            )
            .await?;
        Ok(v)
    }
    pub async fn branch_rules(
        &self,
        repo: &RepositoryIdentity,
        branch: &str,
    ) -> Result<Vec<Value>> {
        self.repository(repo).await?;
        let (v, _) = self
            .http
            .json(
                Method::Get,
                self.repo_url(repo, &["rules", "branches", branch])?,
                None,
            )
            .await?;
        v.as_array()
            .cloned()
            .ok_or_else(|| Error::new("provider_response", "branch rules are not an array"))
    }
}

fn nonzero(id: u64) -> Result<String> {
    if id == 0 {
        return Err(Error::invalid("positive provider ID required"));
    }
    Ok(id.to_string())
}
fn optional_string(v: &Value, path: &str) -> Result<Option<String>> {
    match v.pointer(path) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        _ => Err(Error::new(
            "provider_response",
            "optional provider string is malformed",
        )),
    }
}
fn normalize_repository(v: Value) -> Result<Repository> {
    let owner_type = text_at(&v, "/owner/type")?;
    let owner_id = positive_id(&v, "/owner/id")?;
    Ok(Repository {
        identity: RepositoryIdentity {
            id: positive_id(&v, "/id")?,
            owner: text_at(&v, "/owner/login")?.into(),
            name: text_at(&v, "/name")?.into(),
            owner_id,
            organization_id: (owner_type == "Organization").then_some(owner_id),
        },
        default_branch: text_at(&v, "/default_branch")?.into(),
        private: bool_at(&v, "/private")?,
        archived: bool_at(&v, "/archived")?,
        disabled: bool_at(&v, "/disabled")?,
        squash_allowed: v.get("allow_squash_merge").and_then(Value::as_bool),
        permissions: v.get("permissions").cloned().unwrap_or(Value::Null),
    })
}
fn normalize_pr(v: Value, repository: u64) -> Result<PullRequest> {
    if positive_id(&v, "/base/repo/id")? != repository {
        return Err(Error::new(
            "identity_mismatch",
            "PR belongs to another repository",
        ));
    }
    let merge = optional_string(&v, "/merge_commit_sha")?;
    if let Some(s) = &merge {
        sha(s)?;
    }
    Ok(PullRequest {
        repository_id: repository,
        id: positive_id(&v, "/id")?,
        number: positive_id(&v, "/number")?,
        author_id: positive_id(&v, "/user/id")?,
        state: text_at(&v, "/state")?.into(),
        draft: bool_at(&v, "/draft")?,
        head_sha: sha(text_at(&v, "/head/sha")?)?.into(),
        base_sha: sha(text_at(&v, "/base/sha")?)?.into(),
        head_ref: text_at(&v, "/head/ref")?.into(),
        base_ref: text_at(&v, "/base/ref")?.into(),
        head_repository_id: v
            .pointer("/head/repo/id")
            .filter(|v| !v.is_null())
            .map(|_| positive_id(&v, "/head/repo/id"))
            .transpose()?,
        merged: v.get("merged").and_then(Value::as_bool),
        merge_commit_sha: merge,
        url: validated_url(text_at(&v, "/html_url")?)?,
    })
}
fn normalize_independent_pr(value: Value, repository: u64) -> Result<IndependentPullRequest> {
    // Both parts are derived from one response. No second read can mix an old
    // source identity with a new title/body.
    let text = PullRequestText::from_value(&value)?;
    let pull_request = normalize_pr(value, repository)?;
    Ok(IndependentPullRequest { pull_request, text })
}
fn normalize_run(v: Value, repository: u64) -> Result<WorkflowRun> {
    if positive_id(&v, "/repository/id")? != repository {
        return Err(Error::new(
            "identity_mismatch",
            "workflow run repository differs",
        ));
    }
    Ok(WorkflowRun {
        repository_id: repository,
        id: positive_id(&v, "/id")?,
        attempt: positive_id(&v, "/run_attempt")?,
        workflow_id: positive_id(&v, "/workflow_id")?,
        actor_id: positive_id(&v, "/actor/id")?,
        triggering_actor_id: positive_id(&v, "/triggering_actor/id")?,
        event: text_at(&v, "/event")?.into(),
        head_sha: sha(text_at(&v, "/head_sha")?)?.into(),
        status: text_at(&v, "/status")?.into(),
        conclusion: optional_string(&v, "/conclusion")?,
        workflow_path: text_at(&v, "/path")?.into(),
        url: validated_url(text_at(&v, "/html_url")?)?,
    })
}
fn bool_at(v: &Value, path: &str) -> Result<bool> {
    v.pointer(path).and_then(Value::as_bool).ok_or_else(|| {
        Error::new(
            "provider_response",
            format!("required boolean {path} missing"),
        )
    })
}
fn next_link(headers: &HeaderMap) -> Result<Option<Url>> {
    let mut next = None;
    for header in headers.get_all("link") {
        let text = header
            .to_str()
            .map_err(|_| Error::new("provider_pagination", "nontext GitHub Link header"))?;
        for entry in text.split(',') {
            let entry = entry.trim();
            let Some(end) = entry.find('>') else {
                return Err(Error::new(
                    "provider_pagination",
                    "malformed GitHub Link header",
                ));
            };
            if !entry.starts_with('<') {
                return Err(Error::new(
                    "provider_pagination",
                    "malformed GitHub Link URL",
                ));
            }
            let rel = entry[end + 1..]
                .split(';')
                .map(str::trim)
                .find_map(|p| p.strip_prefix("rel="));
            if rel.is_some_and(|r| r.trim_matches('"').split_whitespace().any(|r| r == "next")) {
                if next.is_some() {
                    return Err(Error::new(
                        "provider_pagination",
                        "ambiguous multiple next links",
                    ));
                }
                next =
                    Some(Url::parse(&entry[1..end]).map_err(|_| {
                        Error::new("provider_pagination", "invalid continuation URL")
                    })?);
            }
        }
    }
    Ok(next)
}
fn validate_next_page(
    first: &Url,
    next: &Url,
    page: usize,
    repo: Option<&RepositoryIdentity>,
) -> Result<()> {
    if first.origin() != next.origin()
        || !next.username().is_empty()
        || next.password().is_some()
        || next.fragment().is_some()
    {
        return Err(Error::new(
            "provider_pagination",
            "continuation escaped provider origin",
        ));
    }
    let expected_path = first.path();
    let alias = repo.and_then(|r| {
        expected_path
            .split_once(&format!("/repos/{}/{}/", r.owner, r.name))
            .map(|(prefix, tail)| format!("{prefix}/repositories/{}/{tail}", r.id))
    });
    if next.path() != expected_path && alias.as_deref() != Some(next.path()) {
        return Err(Error::new(
            "provider_pagination",
            "continuation changed repository or resource",
        ));
    }
    let pairs = |u: &Url| -> Result<BTreeMap<String, String>> {
        let mut m = BTreeMap::new();
        for (k, v) in u.query_pairs() {
            if m.insert(k.into_owned(), v.into_owned()).is_some() {
                return Err(Error::new(
                    "provider_pagination",
                    "duplicate continuation parameter",
                ));
            }
        }
        Ok(m)
    };
    let mut expected = pairs(first)?;
    expected.insert("page".into(), page.to_string());
    if pairs(next)? != expected {
        return Err(Error::new(
            "provider_pagination",
            "continuation changed filters or page sequence",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusState {
    Pending,
    Success,
    Failure,
    Error,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitIdentity {
    pub name: String,
    pub email: String,
    pub date: String,
}
impl CommitIdentity {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty()
            || self.email.trim().is_empty()
            || self.name.len() > 1024
            || self.email.len() > 1024
            || self.name.contains(['\r', '\n', '\0', '<', '>'])
            || self.email.contains(['\r', '\n', '\0', '<', '>'])
            || DateTime::parse_from_rfc3339(&self.date).is_err()
        {
            return Err(Error::invalid(
                "Git commit author/committer requires bounded explicit name, email and RFC3339 date",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TreeEntry {
    pub path: String,
    pub mode: TreeMode,
    pub sha: Option<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum TreeMode {
    #[serde(rename = "100644")]
    File,
    #[serde(rename = "100755")]
    Executable,
    #[serde(rename = "120000")]
    Symlink,
    #[serde(rename = "160000")]
    Gitlink,
}
impl TreeMode {
    fn kind(self) -> &'static str {
        match self {
            Self::Gitlink => "commit",
            _ => "blob",
        }
    }
}
struct UploadGrant {
    issue: Uuid,
    asset_url: String,
    upload_url: Url,
    headers: HeaderMap,
    size: usize,
    content_type: String,
}
fn require_github_user(value: &Value) -> Result<()> {
    if value.get("type").and_then(Value::as_str) != Some("User") {
        return Err(Error::new(
            "identity_mismatch",
            "GitHub connection must authenticate the configured user, not a bot, organization or integration identity",
        ));
    }
    Ok(())
}

fn mutation_success(v: &Value, path: &str) -> Result<()> {
    if bool_at(v, path)? {
        Ok(())
    } else {
        Err(Error::new(
            "provider_response",
            "provider mutation did not confirm success",
        ))
    }
}
fn safe_upload_url(url: &Url) -> Result<()> {
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
    {
        return Err(Error::new(
            "provider_response",
            "signed upload URL is not safe HTTPS",
        ));
    }
    // The destination is obtained only from the authenticated Linear grant, not
    // from agent/issue text. Never forward the Linear credential to this origin.
    if url.host_str() == Some("localhost")
        || url
            .host_str()
            .is_some_and(|h| h.parse::<std::net::IpAddr>().is_ok())
    {
        return Err(Error::new(
            "provider_response",
            "signed upload URL cannot target a local/IP origin",
        ));
    }
    Ok(())
}
pub struct ArtifactArchive {
    pub artifact_id: u64,
    pub archive_digest: String,
    pub bytes: Vec<u8>,
}
impl std::fmt::Debug for ArtifactArchive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtifactArchive")
            .field("artifact_id", &self.artifact_id)
            .field("archive_digest", &self.archive_digest)
            .field("size", &self.bytes.len())
            .finish()
    }
}
impl LinearClient {
    pub fn last_rate_limit(&self) -> Result<RateLimit> {
        self.http
            .rate_limit
            .lock()
            .map(|v| v.clone())
            .map_err(|_| Error::new("provider_transport", "rate metadata unavailable"))
    }
}
impl GithubClient {
    pub fn last_rate_limit(&self) -> Result<RateLimit> {
        self.http
            .rate_limit
            .lock()
            .map(|v| v.clone())
            .map_err(|_| Error::new("provider_transport", "rate metadata unavailable"))
    }
    pub async fn artifacts(
        &self,
        repository: &RepositoryIdentity,
        run: u64,
    ) -> Result<Vec<GithubRecord>> {
        self.records(
            repository,
            &["actions", "runs", &nonzero(run)?, "artifacts"],
            &[],
            Some("artifacts"),
        )
        .await
    }
    /// GitHub's archive endpoint documents one 302 to a short-lived signed blob
    /// URL. Only that exact returned grant is fetched, with no auth forwarding,
    /// further redirects, or use of URLs found in artifact contents.
    pub async fn artifact_archive(
        &self,
        repository: &RepositoryIdentity,
        artifact: u64,
        run: u64,
        trigger_sha: &str,
    ) -> Result<ArtifactArchive> {
        self.repository(repository).await?;
        let (metadata, _) = self
            .http
            .json(
                Method::Get,
                self.repo_url(repository, &["actions", "artifacts", &nonzero(artifact)?])?,
                None,
            )
            .await?;
        if positive_id(&metadata, "/id")? != artifact
            || positive_id(&metadata, "/workflow_run/id")? != run
            || positive_id(&metadata, "/workflow_run/repository_id")? != repository.id
            || text_at(&metadata, "/workflow_run/head_sha")? != sha(trigger_sha)?
            || bool_at(&metadata, "/expired")?
        {
            return Err(Error::new(
                "identity_mismatch",
                "artifact is not the expected live run/source evidence",
            ));
        }
        if metadata
            .get("size_in_bytes")
            .and_then(Value::as_u64)
            .is_none_or(|size| size > self.http.policy.max_body as u64)
        {
            return Err(Error::new(
                "provider_response",
                "artifact exceeds configured download bound",
            ));
        }
        let expected_digest = text_at(&metadata, "/digest")?;
        let response = self
            .http
            .raw(
                Method::Get,
                self.repo_url(
                    repository,
                    &["actions", "artifacts", &nonzero(artifact)?, "zip"],
                )?,
                None,
            )
            .await?;
        if response.status != 302 {
            return Err(Error::new(
                "provider_response",
                "artifact archive endpoint did not return documented signed redirect",
            ));
        }
        let location = response
            .headers
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| Error::new("provider_response", "artifact archive grant missing"))?;
        let url = Url::parse(location)
            .map_err(|_| Error::new("provider_response", "artifact archive grant malformed"))?;
        safe_upload_url(&url)?;
        let archive = tokio::time::timeout(
            self.http.policy.timeout,
            self.http.transport.send(HttpRequest {
                method: Method::Get,
                url,
                headers: HeaderMap::new(),
                body: Vec::new(),
            }),
        )
        .await
        .map_err(|_| Error::new("provider_transport", "artifact download timed out"))??;
        if archive.status != 200 || archive.body.len() > self.http.policy.max_body {
            return Err(Error::new(
                "provider_response",
                "artifact archive download failed or exceeds bounds",
            ));
        }
        let digest = bytes_digest(&archive.body);
        if digest != expected_digest {
            return Err(Error::new(
                "provider_response",
                "artifact archive digest does not match GitHub metadata",
            ));
        }
        Ok(ArtifactArchive {
            artifact_id: artifact,
            archive_digest: digest,
            bytes: archive.body,
        })
    }
}
impl GithubClient {
    /// Target preparation resolves the configured source locator once, but keeps
    /// numeric ID checks for every later operation. No repository is trusted on
    /// its owner/name alone.
    pub async fn resolve_repository(
        &self,
        owner: &str,
        name: &str,
        expected_id: u64,
    ) -> Result<Repository> {
        self.identity().await?;
        let (v, _) = self
            .http
            .json(Method::Get, self.http.url(&["repos", owner, name])?, None)
            .await?;
        let repository = normalize_repository(v)?;
        if repository.identity.id != expected_id
            || repository.identity.owner != owner
            || repository.identity.name != name
        {
            return Err(Error::new(
                "identity_mismatch",
                "configured source locator resolves to another repository",
            ));
        }
        self.check_scope(&repository.identity)?;
        Ok(repository)
    }
    pub async fn independent_review_candidates(
        &self,
        repository: &RepositoryIdentity,
    ) -> Result<Vec<IndependentPullRequest>> {
        let mut output = Vec::new();
        let mut text_bytes = 0usize;
        for value in self.pull_request_values(repository).await? {
            let observed = normalize_independent_pr(value, repository.id)?;
            text_bytes = text_bytes
                .checked_add(
                    serde_json::to_vec(&observed.text)
                        .map_err(|_| Error::invalid("PR text encoding failed"))?
                        .len(),
                )
                .filter(|total| *total <= self.http.policy.max_body)
                .ok_or_else(|| {
                    Error::new(
                        "bounded_input",
                        "independent PR text inventory exceeds provider resource bound",
                    )
                })?;
            let pr = &observed.pull_request;
            if pr.state == "open"
                && !pr.draft
                && self.selection.permits_pr_author(&pr.author_id.to_string())
            {
                output.push(observed);
            }
        }
        Ok(output)
    }
}

/// Stable environment identity observed through the selected repository. Names
/// are provider locators only; a recreated same-name environment has another ID.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubEnvironment {
    pub repository_id: u64,
    pub id: u64,
    pub name: String,
    pub protection_rules: Vec<Value>,
    pub deployment_branch_policy: Value,
    pub observed_at: DateTime<Utc>,
}
impl GithubClient {
    pub async fn environments(
        &self,
        repository: &RepositoryIdentity,
    ) -> Result<Vec<GithubEnvironment>> {
        self.repository(repository).await?;
        self.pages(
            self.repo_url(repository, &["environments"])?,
            &[],
            Some("environments"),
            Some(repository),
        )
        .await?
        .into_iter()
        .map(|v| self.normalize_environment(repository, v))
        .collect()
    }
    fn normalize_environment(
        &self,
        repository: &RepositoryIdentity,
        value: Value,
    ) -> Result<GithubEnvironment> {
        let name = text_at(&value, "/name")?;
        if name.is_empty()
            || name.len() > 4096
            || name.chars().any(char::is_control)
            || Url::parse(text_at(&value, "/url")?).ok().as_ref()
                != Some(&self.repo_url(repository, &["environments", name])?)
        {
            return Err(Error::new(
                "identity_mismatch",
                "environment locator escaped selected repository",
            ));
        }
        Ok(GithubEnvironment {
            repository_id: repository.id,
            id: positive_id(&value, "/id")?,
            name: name.into(),
            protection_rules: value
                .get("protection_rules")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| {
                    Error::new(
                        "provider_response",
                        "environment protection inventory missing",
                    )
                })?,
            deployment_branch_policy: value.get("deployment_branch_policy").cloned().ok_or_else(
                || {
                    Error::new(
                        "provider_response",
                        "environment deployment policy observation missing",
                    )
                },
            )?,
            observed_at: Utc::now(),
        })
    }
    pub async fn environment_by_id(
        &self,
        repository: &RepositoryIdentity,
        environment_id: u64,
    ) -> Result<GithubEnvironment> {
        nonzero(environment_id)?;
        let environments = self.environments(repository).await?;
        let selected = environments
            .iter()
            .find(|e| e.id == environment_id)
            .ok_or_else(|| {
                Error::new(
                    "incompletely_prepared",
                    "configured canonical deployment environment is absent",
                )
            })?;
        let (value, _) = self
            .http
            .json(
                Method::Get,
                self.repo_url(repository, &["environments", &selected.name])?,
                None,
            )
            .await?;
        let current = self.normalize_environment(repository, value)?;
        if current.id != selected.id || current.name != selected.name {
            return Err(Error::new(
                "identity_mismatch",
                "deployment environment changed between inventory and read",
            ));
        }
        Ok(current)
    }
}
impl GithubClient {
    /// Complete target inventory without a SHA filter: a newer divergent source
    /// or pending deployment must remain visible to production reconciliation.
    pub async fn environment_deployments(
        &self,
        repository: &RepositoryIdentity,
        environment: &GithubEnvironment,
    ) -> Result<Vec<GithubRecord>> {
        if environment.repository_id != repository.id
            || environment.id == 0
            || environment.name.is_empty()
        {
            return Err(Error::new(
                "identity_mismatch",
                "deployment environment belongs to another repository",
            ));
        }
        let records = self
            .records(
                repository,
                &["deployments"],
                &[("environment", environment.name.as_str())],
                None,
            )
            .await?;
        if records.iter().any(|record| {
            record.data.get("environment").and_then(Value::as_str)
                != Some(environment.name.as_str())
        }) {
            return Err(Error::new(
                "provider_response",
                "deployment inventory escaped its exact environment filter",
            ));
        }
        Ok(records)
    }
    pub async fn deployment(
        &self,
        repository: &RepositoryIdentity,
        deployment: u64,
    ) -> Result<GithubRecord> {
        self.repository(repository).await?;
        let (data, _) = self
            .http
            .json(
                Method::Get,
                self.repo_url(repository, &["deployments", &nonzero(deployment)?])?,
                None,
            )
            .await?;
        if positive_id(&data, "/id")? != deployment {
            return Err(Error::new(
                "identity_mismatch",
                "deployment identity changed",
            ));
        }
        Ok(GithubRecord {
            repository_id: repository.id,
            id: deployment,
            observed_at: Utc::now(),
            data,
        })
    }
}

impl LinearClient {
    pub fn endpoint(&self) -> &Url {
        &self.http.policy.endpoint
    }
    pub fn account_id(&self) -> &str {
        &self.http.policy.account_id
    }
    pub fn workspace_id(&self) -> Uuid {
        self.workspace_id
    }
}
impl GithubClient {
    pub fn endpoint(&self) -> &Url {
        &self.http.policy.endpoint
    }
    pub fn account_id(&self) -> &str {
        &self.http.policy.account_id
    }
}
