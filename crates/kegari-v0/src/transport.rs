//! Direct, identity-preserving control transport. No enrollment, certificate
//! issuance, proxy, plaintext remote listener or host configuration mutation.
use crate::{
    api::{ApiState, Authentication, MAX_HEADERS, Peer, REQUEST_DEADLINE, connection_router},
    control::{Actor, Principal},
    storage::PrivateDir,
};
use chrono::{DateTime, Utc};
use hyper::server::conn::http1;
use hyper_util::{
    rt::{TokioIo, TokioTimer},
    service::TowerToHyperService,
};
use kegari_config::{ControlConfig, ControlRole, HttpsConfig, Operator, is_tailscale_address};
use kegari_digest::bytes_digest;
use kegari_types::error::{Error, Result};
use rustls::{
    RootCertStore, ServerConfig,
    client::{WebPkiServerVerifier, danger::ServerCertVerifier},
    pki_types::{ServerName, UnixTime},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    future::Future,
    io::Read,
    net::{IpAddr, SocketAddr},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    net::{TcpListener, UnixListener},
    sync::{Semaphore, watch},
    task::{JoinHandle, JoinSet},
};
use tokio_rustls::TlsAcceptor;
use x509_parser::prelude::{FromDer, X509Certificate};

pub const TAILSCALE_VERSION: &str = "1.104.2";
pub const TAILSCALE_SOURCE: &str = "e26db7a5797683ec99ad1516c40b628c7f2dfd71";
const CLI_BOUND: usize = 4 * 1024 * 1024;
const FILE_BOUND: usize = 1024 * 1024;
const CONNECTIONS: usize = 128;
pub trait TailscaleCommand: Send + Sync {
    fn run(&self, args: Vec<String>) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + '_>>;
}
pub struct InstalledTailscale {
    executable: PathBuf,
    timeout: Duration,
}
impl InstalledTailscale {
    pub fn new(executable: PathBuf) -> Result<Self> {
        if !executable.is_absolute() {
            return Err(Error::invalid(
                "Tailscale executable must be an explicit absolute path",
            ));
        }
        let executable = std::fs::canonicalize(executable)
            .map_err(|e| Error::io("resolve configured Tailscale executable", e))?;
        validate_executable_path(&executable)?;
        let metadata = std::fs::metadata(&executable)
            .map_err(|e| Error::io("inspect configured Tailscale executable", e))?;
        if !metadata.is_file()
            || metadata.mode() & 0o022 != 0
            || metadata.mode() & 0o111 == 0
            || !trusted_owner(metadata.uid())
        {
            return Err(Error::new(
                "access_denied",
                "Tailscale executable ownership or permissions are unsafe",
            ));
        }
        Ok(Self {
            executable,
            timeout: Duration::from_secs(3),
        })
    }
}
impl TailscaleCommand for InstalledTailscale {
    fn run(&self, args: Vec<String>) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + '_>> {
        Box::pin(async move {
            let allowed = args == ["version", "--json", "--daemon"]
                || args == ["status", "--json", "--peers=false"]
                || (args.len() == 4
                    && args[..3] == ["whois", "--json", "--proto=tcp"]
                    && args[3].parse::<SocketAddr>().is_ok());
            if !allowed {
                return Err(Error::new(
                    "access_denied",
                    "only exact read-only Tailscale identity commands are permitted",
                ));
            }
            validate_executable_path(&self.executable)?;
            let cutoff = std::time::Instant::now() + self.timeout;
            let mut child = crate::process_spawn::spawn_tokio(
                tokio::process::Command::new(&self.executable)
                    .args(args)
                    .env_clear()
                    .env("PATH", "/usr/bin:/bin")
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true),
                Some(cutoff),
            )
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::TimedOut {
                    Error::new(
                        "identity_unavailable",
                        "Tailscale identity lookup exceeded its deadline",
                    )
                } else {
                    Error::io("start configured Tailscale identity command", error)
                }
            })?;
            let stdout = child.stdout.take().ok_or_else(|| {
                Error::new("identity_unavailable", "Tailscale stdout unavailable")
            })?;
            let stderr = child.stderr.take().ok_or_else(|| {
                Error::new("identity_unavailable", "Tailscale stderr unavailable")
            })?;
            let result = tokio::time::timeout_at(tokio::time::Instant::from_std(cutoff), async {
                let (mut out, mut err) = (Vec::new(), Vec::new());
                let mut stdout = stdout.take((CLI_BOUND + 1) as u64);
                let mut stderr = stderr.take((CLI_BOUND + 1) as u64);
                let (a, b, status) = tokio::join!(
                    stdout.read_to_end(&mut out),
                    stderr.read_to_end(&mut err),
                    child.wait()
                );
                a.map_err(|e| Error::io("read Tailscale identity output", e))?;
                b.map_err(|e| Error::io("read Tailscale identity diagnostics", e))?;
                if out.len() > CLI_BOUND
                    || err.len() > CLI_BOUND
                    || !status
                        .map_err(|e| Error::io("wait for Tailscale identity command", e))?
                        .success()
                {
                    return Err(Error::new(
                        "identity_unavailable",
                        "Tailscale identity command failed or exceeded its output bound",
                    ));
                }
                Ok(out)
            })
            .await;
            match result {
                Ok(result) => result,
                Err(_) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    Err(Error::new(
                        "identity_unavailable",
                        "Tailscale identity lookup exceeded its deadline",
                    ))
                }
            }
        })
    }
}
fn validate_executable_path(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        let metadata = std::fs::symlink_metadata(&current)
            .map_err(|e| Error::io("inspect Tailscale executable ancestry", e))?;
        let protected_temporary = metadata.is_dir()
            && metadata.uid() == 0
            && metadata.mode() & 0o1000 != 0
            && Some(current.as_path()) != path.parent();
        if metadata.file_type().is_symlink()
            || !trusted_owner(metadata.uid())
            || (metadata.mode() & 0o022 != 0 && !protected_temporary)
        {
            return Err(Error::new(
                "access_denied",
                "Tailscale executable path is writable by another OS user",
            ));
        }
    }
    Ok(())
}
#[derive(Clone)]
pub struct TailscaleClient {
    command: Arc<dyn TailscaleCommand>,
    executable: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TailnetIdentity {
    pub user_id: u64,
    pub node_id: String,
    pub peer: SocketAddr,
}
impl TailscaleClient {
    pub fn installed(executable: PathBuf) -> Result<Self> {
        Ok(Self {
            command: Arc::new(InstalledTailscale::new(executable.clone())?),
            executable: Some(executable),
        })
    }
    pub fn with_command(command: Arc<dyn TailscaleCommand>) -> Self {
        Self {
            command,
            executable: None,
        }
    }
    async fn json(&self, args: &[&str]) -> Result<Value> {
        let bytes = tokio::time::timeout(
            Duration::from_secs(3),
            self.command.run(args.iter().map(|v| (*v).into()).collect()),
        )
        .await
        .map_err(|_| {
            Error::new(
                "identity_unavailable",
                "Tailscale identity lookup deadline elapsed",
            )
        })??;
        if bytes.len() > CLI_BOUND {
            return Err(Error::new(
                "identity_unavailable",
                "Tailscale identity output exceeds bound",
            ));
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| Error::new("identity_unavailable", "Tailscale JSON output is malformed"))
    }
    pub async fn verify_version(&self) -> Result<()> {
        let value = self.json(&["version", "--json", "--daemon"]).await?;
        if value.get("short").and_then(Value::as_str) != Some(TAILSCALE_VERSION)
            || value
                .get("isDev")
                .is_some_and(|v| v.as_bool() != Some(false))
            || !value
                .get("daemonLong")
                .and_then(Value::as_str)
                .is_some_and(supported_long_version)
        {
            return Err(Error::new(
                "unsupported_tailscale_version",
                "installed CLI/daemon do not match the supported Tailscale JSON profile",
            ));
        }
        Ok(())
    }
    pub async fn verify_bindings(&self, config: &HttpsConfig) -> Result<()> {
        validate_bind_config(config)?;
        self.verify_version().await?;
        let value = self.json(&["status", "--json", "--peers=false"]).await?;
        if value.get("BackendState").and_then(Value::as_str) != Some("Running")
            || !value
                .get("Version")
                .and_then(Value::as_str)
                .is_some_and(supported_long_version)
            || value.pointer("/Self/Online").and_then(Value::as_bool) != Some(true)
            || value.pointer("/Self/Expired").and_then(Value::as_bool) == Some(true)
            || value
                .pointer("/Self/DNSName")
                .and_then(Value::as_str)
                .map(|s| s.trim_end_matches('.'))
                != Some(config.dns_name.as_str())
        {
            return Err(Error::new(
                "identity_unavailable",
                "Tailscale runtime is not online under the configured DNS identity",
            ));
        }
        for pointer in ["/TailscaleIPs", "/Self/TailscaleIPs"] {
            let ips = value
                .pointer(pointer)
                .and_then(Value::as_array)
                .ok_or_else(|| Error::new("identity_unavailable", "Tailscale address set missing"))?
                .iter()
                .map(|v| {
                    v.as_str()
                        .and_then(|s| s.parse::<IpAddr>().ok())
                        .ok_or_else(|| {
                            Error::new("identity_unavailable", "Tailscale address malformed")
                        })
                })
                .collect::<Result<BTreeSet<_>>>()?;
            if config.addresses.iter().any(|ip| !ips.contains(ip)) {
                return Err(Error::new(
                    "identity_mismatch",
                    "configured listener address is not currently assigned by Tailscale",
                ));
            }
        }
        Ok(())
    }
    pub async fn whois(&self, peer: SocketAddr) -> Result<TailnetIdentity> {
        if !is_tailscale_address(peer.ip()) || peer.port() == 0 {
            return Err(Error::new(
                "access_denied",
                "accepted peer is not an exact tailnet TCP endpoint",
            ));
        }
        self.verify_version().await?;
        let value = self
            .json(&["whois", "--json", "--proto=tcp", &peer.to_string()])
            .await?;
        let id = value
            .pointer("/UserProfile/ID")
            .and_then(Value::as_u64)
            .filter(|v| *v > 0)
            .ok_or_else(|| {
                Error::new(
                    "access_denied",
                    "whois did not establish a direct human user",
                )
            })?;
        if value.pointer("/Node/User").and_then(Value::as_u64) != Some(id)
            || value
                .pointer("/Node/Sharer")
                .is_some_and(|v| v.as_u64() != Some(0))
            || value
                .pointer("/Node/Tags")
                .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
            || value.pointer("/Node/Expired").and_then(Value::as_bool) == Some(true)
        {
            return Err(Error::new(
                "access_denied",
                "tagged, shared, expired or ambiguous nodes cannot establish operator identity",
            ));
        }
        let addresses = value
            .pointer("/Node/Addresses")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Error::new(
                    "access_denied",
                    "whois lacks the node's directly assigned addresses",
                )
            })?;
        let exact = format!("{}/{}", peer.ip(), if peer.is_ipv4() { 32 } else { 128 });
        if !addresses.iter().any(|v| v.as_str() == Some(exact.as_str())) {
            return Err(Error::new(
                "identity_mismatch",
                "whois returned a subnet route or another node",
            ));
        }
        if let Some(expiry) = value.pointer("/Node/KeyExpiry")
            && expiry.as_str() != Some("0001-01-01T00:00:00Z")
            && expiry
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .is_none_or(|v| v.with_timezone(&Utc) <= Utc::now())
        {
            return Err(Error::new(
                "access_denied",
                "tailnet peer key expiry is invalid or elapsed",
            ));
        }
        let node_id = value
            .pointer("/Node/StableID")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                Error::new("identity_unavailable", "whois stable node identity missing")
            })?;
        Ok(TailnetIdentity {
            user_id: id,
            node_id: node_id.into(),
            peer,
        })
    }
}
fn supported_long_version(value: &str) -> bool {
    value == TAILSCALE_VERSION
        || value.strip_prefix(TAILSCALE_VERSION).is_some_and(|rest| {
            rest.starts_with('-')
                && !rest.contains("dev")
                && !rest.contains("dirty")
                && !rest.contains("ERR")
        })
}
fn validate_bind_config(config: &HttpsConfig) -> Result<()> {
    if config.addresses.is_empty()
        || config.port == 0
        || config.addresses.iter().any(|ip| !is_tailscale_address(*ip))
        || config.addresses.iter().collect::<BTreeSet<_>>().len() != config.addresses.len()
        || config.dns_name.ends_with('.')
        || !config.dns_name.ends_with(".ts.net")
        || config.dns_name.split('.').count() < 4
        || config.dns_name.contains(['/', ':', '*', '\0'])
    {
        return Err(Error::invalid(
            "direct TLS requires explicit tailnet addresses, port and full tailnet DNS hostname",
        ));
    }
    Ok(())
}
#[derive(Clone)]
pub struct TransportAuthentication {
    state: Arc<RwLock<AuthenticationState>>,
}
#[derive(Clone)]
struct AuthenticationState {
    config: ControlConfig,
    tailscale: Option<TailscaleClient>,
    revision: u64,
}
impl TransportAuthentication {
    pub fn new(config: ControlConfig, tailscale: Option<TailscaleClient>) -> Self {
        Self {
            state: Arc::new(RwLock::new(AuthenticationState {
                config,
                tailscale,
                revision: 0,
            })),
        }
    }
    pub fn replace_config(&self, config: ControlConfig) -> Result<()> {
        let mut state = self
            .state
            .write()
            .map_err(|_| Error::new("authentication_required", "operator mapping unavailable"))?;
        if state.config.https.as_ref().map(|c| &c.tailscale_executable)
            != config.https.as_ref().map(|c| &c.tailscale_executable)
        {
            state.tailscale = None;
        }
        state.config = config;
        state.revision = state.revision.wrapping_add(1);
        Ok(())
    }
    /// Atomically swaps the configured mapping and its exact installed verifier.
    /// Old in-flight whois observations cannot authenticate the replacement.
    pub fn replace_configuration(
        &self,
        config: ControlConfig,
        tailscale: Option<TailscaleClient>,
    ) -> Result<()> {
        check_verifier(&config, tailscale.as_ref())?;
        let mut state = self
            .state
            .write()
            .map_err(|_| Error::new("authentication_required", "operator mapping unavailable"))?;
        state.config = config;
        state.tailscale = tailscale;
        state.revision = state.revision.wrapping_add(1);
        Ok(())
    }
    pub fn replace_tailscale(&self, tailscale: Option<TailscaleClient>) -> Result<()> {
        let mut state = self
            .state
            .write()
            .map_err(|_| Error::new("authentication_required", "operator mapping unavailable"))?;
        check_verifier(&state.config, tailscale.as_ref())?;
        state.tailscale = tailscale;
        state.revision = state.revision.wrapping_add(1);
        Ok(())
    }
}
fn check_verifier(config: &ControlConfig, client: Option<&TailscaleClient>) -> Result<()> {
    if let Some(path) = client.and_then(|c| c.executable.as_ref())
        && config
            .https
            .as_ref()
            .is_none_or(|c| Path::new(&c.tailscale_executable) != path)
    {
        return Err(Error::new(
            "identity_unavailable",
            "identity verifier differs from configured executable",
        ));
    }
    Ok(())
}
impl Authentication for TransportAuthentication {
    fn authenticate(&self, peer: Peer) -> Pin<Box<dyn Future<Output = Result<Actor>> + Send + '_>> {
        Box::pin(async move {
            let initial = self
                .state
                .read()
                .map_err(|_| Error::new("authentication_required", "operator mapping unavailable"))?
                .clone();
            match peer {
                Peer::Unix { uid } => operator_actor(
                    initial
                        .config
                        .local_users
                        .get(&uid.to_string())
                        .ok_or_else(|| {
                            Error::new("access_denied", "OS user has no configured operator roles")
                        })?,
                    Principal::Unix { uid },
                ),
                Peer::DirectTls { address } => {
                    if initial.config.https.is_none() {
                        return Err(Error::new("access_denied", "remote control is disabled"));
                    }
                    check_verifier(&initial.config, initial.tailscale.as_ref())?;
                    let identity = initial
                        .tailscale
                        .as_ref()
                        .ok_or_else(|| {
                            Error::new("identity_unavailable", "Tailscale verifier is unavailable")
                        })?
                        .whois(address)
                        .await?;
                    let current = self.state.read().map_err(|_| {
                        Error::new("authentication_required", "operator mapping unavailable")
                    })?;
                    if current.revision != initial.revision || current.config.https.is_none() {
                        return Err(Error::new(
                            "access_denied",
                            "remote identity configuration changed during verification",
                        ));
                    }
                    operator_actor(
                        current
                            .config
                            .tailscale_users
                            .get(&identity.user_id.to_string())
                            .ok_or_else(|| {
                                Error::new(
                                    "access_denied",
                                    "verified tailnet user has no configured operator roles",
                                )
                            })?,
                        Principal::Tailscale {
                            user_id: identity.user_id,
                            node_id: identity.node_id,
                        },
                    )
                }
            }
        })
    }
}
fn operator_actor(operator: &Operator, principal: Principal) -> Result<Actor> {
    if operator.id.trim().is_empty()
        || operator.linear_user_id.is_nil()
        || operator.roles.is_empty()
    {
        return Err(Error::new(
            "access_denied",
            "operator mapping is incomplete",
        ));
    }
    Ok(Actor {
        principal: Some(principal),
        identity: operator.id.clone(),
        roles: operator
            .roles
            .iter()
            .map(|role| {
                match role {
                    ControlRole::Inspect => "inspect",
                    ControlRole::Configure => "configure",
                    ControlRole::Lifecycle => "lifecycle",
                    ControlRole::Correct => "correct",
                    ControlRole::Accept => "accept",
                    ControlRole::Release => "release",
                    ControlRole::Recover => "recover",
                    ControlRole::Evaluate => "evaluate",
                }
                .into()
            })
            .collect(),
    })
}
fn trusted_owner(uid: u32) -> bool {
    uid == 0 || uid == unsafe { libc::geteuid() }
}

pub struct TlsMaterial {
    config: Arc<ServerConfig>,
    pub certificate_digest: String,
    pub not_before: i64,
    pub not_after: i64,
}
impl TlsMaterial {
    pub fn server_config(&self) -> Arc<ServerConfig> {
        self.config.clone()
    }
    pub fn valid_at(&self, now: DateTime<Utc>) -> bool {
        now.timestamp() >= self.not_before && now.timestamp() < self.not_after
    }
}
pub fn public_roots() -> Arc<RootCertStore> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Arc::new(roots)
}
fn secure_file(path: &Path, key: bool) -> Result<Vec<u8>> {
    if !path.is_absolute() {
        return Err(Error::invalid("TLS material path must be absolute"));
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| Error::io("open operator-provisioned TLS material", e))?;
    let metadata = file
        .metadata()
        .map_err(|e| Error::io("inspect TLS material", e))?;
    if !metadata.is_file()
        || !trusted_owner(metadata.uid())
        || metadata.mode() & if key { 0o077 } else { 0o022 } != 0
        || metadata.len() > FILE_BOUND as u64
    {
        return Err(Error::new(
            "access_denied",
            "TLS material ownership, permissions, type or size is unsafe",
        ));
    }
    let mut bytes = Vec::new();
    file.take((FILE_BOUND + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read TLS material", e))?;
    if bytes.len() > FILE_BOUND {
        return Err(Error::invalid("TLS material exceeds bound"));
    }
    Ok(bytes)
}
pub fn load_tls_material(
    certificate: &Path,
    key: &Path,
    hostname: &str,
    now: DateTime<Utc>,
    roots: Arc<RootCertStore>,
) -> Result<TlsMaterial> {
    let cert_bytes = secure_file(certificate, false)?;
    let key_bytes = secure_file(key, true)?;
    let chain = rustls_pemfile::certs(&mut cert_bytes.as_slice())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::new("invalid_tls", "certificate PEM is malformed"))?;
    if chain.is_empty() || chain.len() > 10 {
        return Err(Error::new(
            "invalid_tls",
            "certificate chain is missing or exceeds bound",
        ));
    }
    let private = rustls_pemfile::private_key(&mut key_bytes.as_slice())
        .map_err(|_| Error::new("invalid_tls", "private key PEM is malformed"))?
        .ok_or_else(|| Error::new("invalid_tls", "private key is absent"))?;
    let mut not_before = i64::MIN;
    let mut not_after = i64::MAX;
    for cert in &chain {
        let (rest, parsed) = X509Certificate::from_der(cert.as_ref())
            .map_err(|_| Error::new("invalid_tls", "certificate DER is malformed"))?;
        if !rest.is_empty() {
            return Err(Error::new(
                "invalid_tls",
                "certificate DER has trailing data",
            ));
        }
        not_before = not_before.max(parsed.validity().not_before.timestamp());
        not_after = not_after.min(parsed.validity().not_after.timestamp());
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let name = ServerName::try_from(hostname.to_owned())
        .map_err(|_| Error::new("invalid_tls", "configured TLS hostname is malformed"))?;
    let verifier = WebPkiServerVerifier::builder_with_provider(roots, provider.clone())
        .build()
        .map_err(|_| Error::new("invalid_tls", "TLS trust roots unavailable"))?;
    let timestamp = u64::try_from(now.timestamp())
        .map_err(|_| Error::new("invalid_tls", "verification clock predates epoch"))?;
    verifier
        .verify_server_cert(
            &chain[0],
            &chain[1..],
            &name,
            &[],
            UnixTime::since_unix_epoch(Duration::from_secs(timestamp)),
        )
        .map_err(|_| {
            Error::new(
                "invalid_tls",
                "certificate chain, hostname or validity does not verify",
            )
        })?;
    let signed =
        rustls::sign::CertifiedKey::from_der(chain.clone(), private.clone_key(), &provider)
            .map_err(|_| {
                Error::new(
                    "invalid_tls",
                    "private key and certificate do not form a supported matching pair",
                )
            })?;
    signed
        .keys_match()
        .map_err(|_| Error::new("invalid_tls", "private key does not match leaf certificate"))?;
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::new("invalid_tls", "TLS protocol configuration failed"))?
        .with_no_client_auth()
        .with_single_cert(chain, private)
        .map_err(|_| Error::new("invalid_tls", "TLS key/chain configuration failed"))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(TlsMaterial {
        config: Arc::new(config),
        certificate_digest: bytes_digest(&cert_bytes),
        not_before,
        not_after,
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TlsHealth {
    pub enabled: bool,
    pub certificate_digest: Option<String>,
    pub not_after: Option<i64>,
    pub reload_error: Option<Error>,
}
pub struct CertificateReloader {
    certificate: PathBuf,
    key: PathBuf,
    hostname: String,
    roots: Arc<RootCertStore>,
    current: Option<Arc<TlsMaterial>>,
    health: TlsHealth,
}
impl CertificateReloader {
    pub fn new(
        certificate: PathBuf,
        key: PathBuf,
        hostname: String,
        roots: Arc<RootCertStore>,
    ) -> Self {
        Self {
            certificate,
            key,
            hostname,
            roots,
            current: None,
            health: TlsHealth {
                enabled: false,
                certificate_digest: None,
                not_after: None,
                reload_error: None,
            },
        }
    }
    pub fn refresh_at(&mut self, now: DateTime<Utc>) -> TlsHealth {
        match load_tls_material(
            &self.certificate,
            &self.key,
            &self.hostname,
            now,
            self.roots.clone(),
        ) {
            Ok(material) => {
                self.current = Some(Arc::new(material));
                self.health.reload_error = None;
            }
            Err(error) => {
                self.health.reload_error = Some(error);
                if self.current.as_ref().is_some_and(|m| !m.valid_at(now)) {
                    self.current = None;
                }
            }
        }
        self.health.enabled = self.current.is_some();
        self.health.certificate_digest =
            self.current.as_ref().map(|m| m.certificate_digest.clone());
        self.health.not_after = self.current.as_ref().map(|m| m.not_after);
        self.health.clone()
    }
    pub fn current(&self) -> Option<Arc<TlsMaterial>> {
        self.current.clone()
    }
}

pub struct ServerHandle {
    shutdown: watch::Sender<bool>,
    task: JoinHandle<Result<()>>,
    pub health: watch::Receiver<TlsHealth>,
}
impl ServerHandle {
    /// Inspect task termination without consuming the handle or losing the
    /// ability to collect its exact result with shutdown().
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
    pub async fn shutdown(self) -> Result<()> {
        let _ = self.shutdown.send(true);
        self.task
            .await
            .map_err(|_| Error::new("transport_failed", "control transport task failed"))?
    }
}
async fn serve_stream<S>(
    stream: S,
    state: ApiState,
    peer: Peer,
    mut shutdown: watch::Receiver<bool>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(REQUEST_DEADLINE)
        .max_buf_size(MAX_HEADERS + 8192)
        .keep_alive(true);
    let service = TowerToHyperService::new(connection_router(state, peer));
    let connection = builder.serve_connection(TokioIo::new(stream), service);
    tokio::pin!(connection);
    if !*shutdown.borrow() {
        tokio::select! {_ = &mut connection=>return,_ = shutdown.changed()=>{}}
    }
    // Stop accepting keep-alive requests while allowing the already admitted
    // shutdown command response to finish. The API has its own ten-second bound.
    connection.as_mut().graceful_shutdown();
    let _ = tokio::time::timeout(REQUEST_DEADLINE, &mut connection).await;
}
async fn drain_connections<T: 'static>(tasks: &mut JoinSet<T>) {
    if tokio::time::timeout(REQUEST_DEADLINE + Duration::from_secs(1), async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
}
/// The caller owns the already-locked private control directory. Existing socket
/// paths are never unlinked or adopted; stale ownership must be reconciled first.
pub async fn start_unix(
    directory: &PrivateDir,
    leaf: &str,
    state: ApiState,
) -> Result<ServerHandle> {
    if leaf.is_empty() || leaf.contains(['/', '\0']) || leaf == "." || leaf == ".." {
        return Err(Error::invalid(
            "local socket must be a leaf within the private control root",
        ));
    }
    let path = directory.path().join(leaf);
    if std::fs::symlink_metadata(&path).is_ok() {
        return Err(Error::new(
            "ownership_conflict",
            "local control socket already exists; reconcile its owner",
        ));
    }
    let listener =
        UnixListener::bind(&path).map_err(|e| Error::io("bind private local control socket", e))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| Error::io("protect local control socket", e))?;
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|e| Error::io("inspect owned control socket", e))?;
    if !metadata.file_type().is_socket() || metadata.uid() != unsafe { libc::geteuid() } {
        return Err(Error::new(
            "ownership_conflict",
            "bound socket identity is not owned",
        ));
    }
    let identity = (metadata.dev(), metadata.ino());
    let (shutdown, mut stop) = watch::channel(false);
    let (health_tx, health) = watch::channel(TlsHealth {
        enabled: true,
        certificate_digest: None,
        not_after: None,
        reload_error: None,
    });
    let task = tokio::spawn(async move {
        let _health_guard = health_tx;
        let cap = Arc::new(Semaphore::new(CONNECTIONS));
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {_ = stop.changed()=>break,accepted=listener.accept()=>{
                let(stream,_)=accepted.map_err(|e|Error::io("accept local control connection",e))?;let Ok(slot)=cap.clone().try_acquire_owned()else{continue};let credentials=stream.peer_cred().map_err(|e|Error::io("read OS control peer identity",e))?;let peer=Peer::Unix{uid:credentials.uid()};let state=state.clone();let stop=stop.clone();connections.spawn(async move{let _slot=slot;serve_stream(stream,state,peer,stop).await;});
            },Some(_)=connections.join_next()=>{}}
        }
        drop(listener);
        drain_connections(&mut connections).await;
        if let Ok(current) = std::fs::symlink_metadata(&path)
            && current.file_type().is_socket()
            && (current.dev(), current.ino()) == identity
        {
            std::fs::remove_file(&path).map_err(|e| Error::io("remove stopped owned socket", e))?;
        }
        Ok(())
    });
    Ok(ServerHandle {
        shutdown,
        task,
        health,
    })
}
async fn bind_direct(config: &HttpsConfig) -> Result<Vec<TcpListener>> {
    validate_bind_config(config)?;
    let mut listeners = Vec::new();
    for address in &config.addresses {
        let listener = TcpListener::bind(SocketAddr::new(*address, config.port))
            .await
            .map_err(|e| Error::io("bind verified direct tailnet TLS address", e))?;
        if listener
            .local_addr()
            .map_err(|e| Error::io("inspect bound TLS address", e))?
            != SocketAddr::new(*address, config.port)
        {
            return Err(Error::new(
                "identity_mismatch",
                "TLS socket bound an unexpected address",
            ));
        }
        listeners.push(listener);
    }
    Ok(listeners)
}
async fn remote_listener(
    listener: TcpListener,
    state: ApiState,
    mut material: watch::Receiver<Option<Arc<TlsMaterial>>>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let cap = Arc::new(Semaphore::new(CONNECTIONS));
    let mut connections = JoinSet::new();
    let result = loop {
        let current = material
            .borrow()
            .clone()
            .ok_or_else(|| Error::new("invalid_tls", "TLS pair no longer valid"))?;
        let now = Utc::now();
        if !current.valid_at(now) {
            break Err(Error::new(
                "invalid_tls",
                "TLS certificate expired; listener disabled",
            ));
        }
        let remaining = Duration::from_millis(
            ((current.not_after * 1000) - now.timestamp_millis()).max(1) as u64,
        );
        tokio::select! {_ = shutdown.changed()=>break Ok(()),_ = tokio::time::sleep(remaining)=>break Err(Error::new("invalid_tls","TLS certificate expired; listener disabled")),changed=material.changed()=>{if changed.is_err(){break Ok(());}},accepted=listener.accept()=>{
            let(stream,peer)=accepted.map_err(|e|Error::io("accept direct TLS peer",e))?;if !is_tailscale_address(peer.ip()){continue;}let Ok(slot)=cap.clone().try_acquire_owned()else{continue};let state=state.clone();let stop=shutdown.clone();
            connections.spawn(async move{let _slot=slot;let acceptor=TlsAcceptor::from(current.server_config());if let Ok(Ok(stream))=tokio::time::timeout(REQUEST_DEADLINE,acceptor.accept(stream)).await{serve_stream(stream,state,Peer::DirectTls{address:peer},stop).await;}});
        },Some(_)=connections.join_next()=>{}}
    };
    if result.is_ok() {
        drain_connections(&mut connections).await;
    } else {
        connections.abort_all();
        while connections.join_next().await.is_some() {}
    }
    result
}
/// Starts only after exact local Tailscale identity and a public-trust TLS pair
/// validate. Failure does not affect a separately running local socket server.
pub async fn start_direct_tls(
    state: ApiState,
    config: HttpsConfig,
    tailscale: TailscaleClient,
) -> Result<ServerHandle> {
    if state.expected_authority != direct_tls_authority(&config)? {
        return Err(Error::invalid(
            "API authority must equal configured TLS DNS name and nondefault port",
        ));
    }
    tailscale.verify_bindings(&config).await?;
    let mut reloader = CertificateReloader::new(
        config.certificate.clone().into(),
        config.private_key.clone().into(),
        config.dns_name.clone(),
        public_roots(),
    );
    let initial = reloader.refresh_at(Utc::now());
    if !initial.enabled {
        return Err(initial
            .reload_error
            .unwrap_or_else(|| Error::new("invalid_tls", "no valid TLS pair")));
    }
    let listeners = bind_direct(&config).await?;
    let (shutdown, mut stop) = watch::channel(false);
    let (health_tx, health) = watch::channel(initial);
    let (material_tx, material) = watch::channel(reloader.current());
    let task = tokio::spawn(async move {
        let mut workers = JoinSet::new();
        for listener in listeners {
            workers.spawn(remote_listener(
                listener,
                state.clone(),
                material.clone(),
                stop.clone(),
            ));
        }
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut refresh_count = 0u8;
        loop {
            tokio::select! {_ = stop.changed()=>break,_ = interval.tick()=>{
                let mut observation=reloader.refresh_at(Utc::now());refresh_count=refresh_count.wrapping_add(1);
                if observation.enabled&&(workers.is_empty()||refresh_count>=30){refresh_count=0;if let Err(error)=tailscale.verify_bindings(&config).await{observation.enabled=false;observation.reload_error=Some(error);}}
                if !observation.enabled {let _=material_tx.send(None);workers.abort_all();while workers.join_next().await.is_some(){}}
                else{let _=material_tx.send(reloader.current());if workers.is_empty(){match bind_direct(&config).await{Ok(listeners)=>{for listener in listeners{workers.spawn(remote_listener(listener,state.clone(),material.clone(),stop.clone()));}},Err(error)=>{observation.enabled=false;observation.reload_error=Some(error);}}}}
                let _=health_tx.send(observation);
            },Some(result)=workers.join_next()=>{if *stop.borrow(){break;}let error=match result{Ok(Err(error))=>error,Ok(Ok(()))=>Error::new("transport_unavailable","TLS listener stopped"),Err(_)=>Error::new("transport_failed","TLS listener task failed")};workers.abort_all();while workers.join_next().await.is_some(){}let _=material_tx.send(None);let _=health_tx.send(TlsHealth{enabled:false,certificate_digest:None,not_after:None,reload_error:Some(error)});}}
        }
        drain_connections(&mut workers).await;
        Ok(())
    });
    Ok(ServerHandle {
        shutdown,
        task,
        health,
    })
}

/// Same canonical authority used by the TLS client: default HTTPS port omitted.
pub fn direct_tls_authority(config: &HttpsConfig) -> Result<String> {
    validate_bind_config(config)?;
    Ok(if config.port == 443 {
        config.dns_name.clone()
    } else {
        format!("{}:{}", config.dns_name, config.port)
    })
}
