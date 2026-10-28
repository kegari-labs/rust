use chrono::Utc;
use kegari_types::api::CommandRequest;
use kegari_types::error::Result;
use kegari_v0::{
    api::{ApiState, Authentication, DispatchResult, Dispatcher, Peer},
    config::{ControlConfig, ControlRole, HttpsConfig, Operator},
    control::Actor,
    secrets::Secret,
    storage::PrivateDir,
    transport::*,
};
use rustls::RootCertStore;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    future::Future,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;
#[derive(Default)]
struct Cli {
    output: Mutex<VecDeque<Value>>,
    args: Mutex<Vec<Vec<String>>>,
}
impl Cli {
    fn new(values: Vec<Value>) -> Arc<Self> {
        Arc::new(Self {
            output: Mutex::new(values.into()),
            args: Mutex::new(Vec::new()),
        })
    }
}
impl TailscaleCommand for Cli {
    fn run(&self, args: Vec<String>) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + '_>> {
        Box::pin(async move {
            self.args.lock().unwrap().push(args);
            Ok(serde_json::to_vec(
                &self
                    .output
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unexpected command"),
            )
            .unwrap())
        })
    }
}
fn version() -> Value {
    json!({"short":TAILSCALE_VERSION,"daemonLong":format!("{TAILSCALE_VERSION}-te26db7a57"),"majorMinorPatch":TAILSCALE_VERSION})
}
fn https() -> HttpsConfig {
    HttpsConfig {
        addresses: vec![
            "100.100.1.1".parse().unwrap(),
            "fd7a:115c:a1e0::1".parse().unwrap(),
        ],
        port: 8443,
        dns_name: "runtime.fixture.ts.net".into(),
        certificate: "/fixture/certificate.pem".into(),
        private_key: "/fixture/key.pem".into(),
        tailscale_executable: "/fixture/tailscale".into(),
    }
}
fn status() -> Value {
    json!({"Version":format!("{TAILSCALE_VERSION}-te26db7a57"),"BackendState":"Running","TailscaleIPs":["100.100.1.1","fd7a:115c:a1e0::1"],"Self":{"Online":true,"DNSName":"runtime.fixture.ts.net.","TailscaleIPs":["100.100.1.1","fd7a:115c:a1e0::1"]}})
}
fn whois(peer: &str) -> Value {
    let peer: SocketAddr = peer.parse().unwrap();
    json!({"Node":{"StableID":"node-fixture","User":42,"Addresses":[format!("{}/{}",peer.ip(),if peer.is_ipv4(){32}else{128})]},"UserProfile":{"ID":42,"LoginName":"display@example.invalid"}})
}
fn operator() -> Operator {
    Operator {
        id: "human-fixture".into(),
        linear_user_id: Uuid::from_u128(42),
        roles: BTreeSet::from([ControlRole::Inspect, ControlRole::Accept]),
    }
}
fn configuration() -> ControlConfig {
    ControlConfig {
        api_version: 1,
        local_users: BTreeMap::from([(unsafe { libc::geteuid() }.to_string(), operator())]),
        tailscale_users: BTreeMap::from([("42".into(), operator())]),
        https: Some(https()),
    }
}
#[tokio::test]
async fn exact_tailscale_bindings_and_peer_ipv6_port_are_verified() {
    let cli = Cli::new(vec![
        version(),
        status(),
        version(),
        whois("[fd7a:115c:a1e0::2]:43210"),
    ]);
    let ts = TailscaleClient::with_command(cli.clone());
    ts.verify_bindings(&https()).await.unwrap();
    let identity = ts
        .whois("[fd7a:115c:a1e0::2]:43210".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(identity.user_id, 42);
    assert_eq!(
        cli.args.lock().unwrap()[3],
        vec![
            "whois",
            "--json",
            "--proto=tcp",
            "[fd7a:115c:a1e0::2]:43210"
        ]
    );
}
#[tokio::test]
async fn non_tailnet_or_unassigned_binds_fail_before_listener_creation() {
    for address in ["0.0.0.0", "::", "127.0.0.1", "192.168.1.1", "8.8.8.8"] {
        let cli = Cli::new(vec![]);
        let mut config = https();
        config.addresses = vec![address.parse().unwrap()];
        assert!(
            TailscaleClient::with_command(cli.clone())
                .verify_bindings(&config)
                .await
                .is_err()
        );
        assert!(cli.args.lock().unwrap().is_empty());
    }
    let mut other = status();
    other["Self"]["DNSName"] = json!("other.fixture.ts.net.");
    let cli = Cli::new(vec![version(), other]);
    assert!(
        TailscaleClient::with_command(cli)
            .verify_bindings(&https())
            .await
            .is_err()
    );
    let mut other = status();
    other["TailscaleIPs"] = json!(["100.100.1.2"]);
    let cli = Cli::new(vec![version(), other]);
    assert!(
        TailscaleClient::with_command(cli)
            .verify_bindings(&https())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn tagged_shared_expired_wrong_node_and_cli_versions_cannot_authenticate() {
    for field in ["tags", "shared", "wrong_user", "wrong_address", "expired"] {
        let mut value = whois("100.100.1.2:5555");
        match field {
            "tags" => value["Node"]["Tags"] = json!(["tag:operator"]),
            "shared" => value["Node"]["Sharer"] = json!(8),
            "wrong_user" => value["UserProfile"]["ID"] = json!(43),
            "wrong_address" => value["Node"]["Addresses"] = json!(["100.100.0.0/16"]),
            _ => value["Node"]["KeyExpiry"] = json!("2000-01-01T00:00:00Z"),
        };
        let cli = Cli::new(vec![version(), value]);
        assert!(
            TailscaleClient::with_command(cli)
                .whois("100.100.1.2:5555".parse().unwrap())
                .await
                .is_err(),
            "{field}"
        );
    }
    let mut value = version();
    value["short"] = json!("1.0.0");
    let cli = Cli::new(vec![value]);
    assert_eq!(
        TailscaleClient::with_command(cli)
            .whois("100.100.1.2:5555".parse().unwrap())
            .await
            .unwrap_err()
            .code,
        "unsupported_tailscale_version"
    );
}
#[tokio::test]
async fn role_mapping_is_refreshed_and_tailnet_login_text_never_authenticates() {
    let peer = "100.100.1.2:5555".parse().unwrap();
    let cli = Cli::new(vec![
        version(),
        whois("100.100.1.2:5555"),
        version(),
        whois("100.100.1.2:5555"),
    ]);
    let auth = TransportAuthentication::new(
        configuration(),
        Some(TailscaleClient::with_command(cli.clone())),
    );
    assert!(
        auth.authenticate(Peer::DirectTls { address: peer })
            .await
            .unwrap()
            .roles
            .contains("accept")
    );
    let mut changed = configuration();
    changed.tailscale_users.clear();
    changed
        .tailscale_users
        .insert("display@example.invalid".into(), operator());
    auth.replace_config(changed).unwrap();
    assert_eq!(
        auth.authenticate(Peer::DirectTls { address: peer })
            .await
            .unwrap_err()
            .code,
        "access_denied"
    );
    assert_eq!(cli.args.lock().unwrap().len(), 4);
}
struct Echo;
impl Dispatcher for Echo {
    fn dispatch(
        &self,
        actor: Actor,
        request: CommandRequest,
        _secret: Option<Secret>,
    ) -> Pin<Box<dyn Future<Output = Result<DispatchResult>> + Send + '_>> {
        Box::pin(async move {
            Ok(DispatchResult {
                config_revision: None,
                operation_id: None,
                state: Some("read".into()),
                data: json!({"actor":actor.identity,"command":request.command}),
            })
        })
    }
}
struct DelayedResponse {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Dispatcher for DelayedResponse {
    fn dispatch(
        &self,
        _: Actor,
        _: CommandRequest,
        _: Option<Secret>,
    ) -> Pin<Box<dyn Future<Output = Result<DispatchResult>> + Send + '_>> {
        Box::pin(async move {
            self.started.notify_one();
            self.release.notified().await;
            Ok(DispatchResult {
                config_revision: None,
                operation_id: None,
                state: Some("service_stopping".into()),
                data: json!({"receipt":"acknowledged","complete_body":"x".repeat(512*1024)}),
            })
        })
    }
}
#[tokio::test]
async fn local_shutdown_drains_the_admitted_http_response_before_closing_socket() {
    let temp = tempfile::tempdir().unwrap();
    let root = PrivateDir::create(&temp.path().canonicalize().unwrap().join("control")).unwrap();
    let runtime = Uuid::new_v4();
    let dispatcher = Arc::new(DelayedResponse {
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let server = start_unix(
        &root,
        "control.sock",
        ApiState {
            runtime_id: runtime,
            expected_authority: "local.kegari".into(),
            authentication: Arc::new(TransportAuthentication::new(configuration(), None)),
            dispatcher: dispatcher.clone(),
        },
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder()
        .unix_socket(root.path().join("control.sock"))
        .build()
        .unwrap();
    let request = tokio::spawn(async move {
        client
            .get("http://local.kegari/v1/status")
            .header("x-kegari-protocol", "1")
            .header("x-kegari-runtime", runtime.to_string())
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()
    });
    dispatcher.started.notified().await;
    let shutdown = server.shutdown();
    tokio::pin!(shutdown);
    tokio::select! {biased;result=&mut shutdown=>panic!("shutdown dropped admitted response: {result:?}"),_=tokio::task::yield_now()=>{}}
    dispatcher.release.notify_one();
    let (response, stopped) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(request, shutdown)
    })
    .await
    .unwrap();
    stopped.unwrap();
    let response = response.unwrap();
    assert_eq!(response["data"]["receipt"], "acknowledged");
    assert_eq!(
        response["data"]["complete_body"].as_str().unwrap().len(),
        512 * 1024
    );
    assert!(!root.path().join("control.sock").exists());
}
#[tokio::test]
async fn actual_unix_socket_uses_os_peer_and_stops_without_unlinking_another_owner() {
    let temp = tempfile::tempdir().unwrap();
    let root = PrivateDir::create(&temp.path().canonicalize().unwrap().join("control")).unwrap();
    let runtime = Uuid::new_v4();
    let state = ApiState {
        runtime_id: runtime,
        expected_authority: "local.kegari".into(),
        authentication: Arc::new(TransportAuthentication::new(configuration(), None)),
        dispatcher: Arc::new(Echo),
    };
    let server = start_unix(&root, "control.sock", state.clone())
        .await
        .unwrap();
    let path = root.path().join("control.sock");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        start_unix(&root, "control.sock", state.clone())
            .await
            .is_err()
    );
    assert!(
        start_direct_tls(
            state.clone(),
            https(),
            TailscaleClient::with_command(Cli::new(vec![version(), status()]))
        )
        .await
        .is_err()
    );
    let client = reqwest::Client::builder()
        .unix_socket(path.as_path())
        .build()
        .unwrap();
    let response = client
        .get("http://local.kegari/v1/status")
        .header("x-kegari-protocol", "1")
        .header("x-kegari-runtime", runtime.to_string())
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["data"]["actor"], "human-fixture");
    server.shutdown().await.unwrap();
    assert!(!path.exists());
    let server = start_unix(&root, "control.sock", state).await.unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, "replacement owned by someone else").unwrap();
    server.shutdown().await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "replacement owned by someone else"
    );
}

#[test]
fn direct_tls_api_authority_normalizes_only_default_https_port() {
    let mut config = https();
    config.port = 443;
    assert_eq!(direct_tls_authority(&config).unwrap(), config.dns_name);
    config.port = 8443;
    assert_eq!(
        direct_tls_authority(&config).unwrap(),
        format!("{}:8443", config.dns_name)
    );
}
fn tls_fixture() -> (tempfile::TempDir, PathBuf, PathBuf, Arc<RootCertStore>) {
    let temp = tempfile::tempdir().unwrap();
    let certificate = temp.path().canonicalize().unwrap().join("cert.pem");
    let key = temp.path().canonicalize().unwrap().join("key.pem");
    std::fs::write(&certificate, LEAF).unwrap();
    std::fs::write(&key, KEY).unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut roots = RootCertStore::empty();
    for cert in rustls_pemfile::certs(&mut CA.as_bytes()) {
        roots.add(cert.unwrap()).unwrap();
    }
    (temp, certificate, key, Arc::new(roots))
}
#[test]
fn certificates_require_matching_key_hostname_chain_and_safe_permissions() {
    let (_temp, certificate, key, roots) = tls_fixture();
    let now = Utc::now();
    let material = load_tls_material(
        &certificate,
        &key,
        "runtime.fixture.ts.net",
        now,
        roots.clone(),
    )
    .unwrap();
    assert!(material.valid_at(now));
    assert!(
        load_tls_material(
            &certificate,
            &key,
            "other.fixture.ts.net",
            now,
            roots.clone()
        )
        .is_err()
    );
    assert!(
        load_tls_material(
            &certificate,
            &key,
            "runtime.fixture.ts.net",
            now,
            public_roots()
        )
        .is_err()
    );
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        load_tls_material(
            &certificate,
            &key,
            "runtime.fixture.ts.net",
            now,
            roots.clone()
        )
        .err()
        .unwrap()
        .code,
        "access_denied"
    );
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&key, OTHER_KEY).unwrap();
    assert_eq!(
        load_tls_material(&certificate, &key, "runtime.fixture.ts.net", now, roots)
            .err()
            .unwrap()
            .code,
        "invalid_tls"
    );
}
#[test]
fn bad_replacement_preserves_valid_pair_but_expiry_disables_remote_material() {
    let (_temp, certificate, key, roots) = tls_fixture();
    let now = Utc::now();
    let mut reload = CertificateReloader::new(
        certificate.clone(),
        key,
        "runtime.fixture.ts.net".into(),
        roots,
    );
    let valid = reload.refresh_at(now);
    assert!(valid.enabled);
    std::fs::write(certificate, "malformed replacement").unwrap();
    let stale = reload.refresh_at(now);
    assert!(stale.enabled);
    assert!(stale.reload_error.is_some());
    assert_eq!(stale.certificate_digest, valid.certificate_digest);
    let expired =
        reload.refresh_at(chrono::DateTime::from_timestamp(valid.not_after.unwrap(), 0).unwrap());
    assert!(!expired.enabled);
    assert!(reload.current().is_none());
}
#[tokio::test]
async fn validated_production_tls_config_performs_real_verified_handshake() {
    let (_temp, certificate, key, roots) = tls_fixture();
    let material = load_tls_material(
        &certificate,
        &key,
        "runtime.fixture.ts.net",
        Utc::now(),
        roots.clone(),
    )
    .unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let client = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates((*roots).clone())
        .with_no_client_auth();
    let (client_io, server_io) = tokio::io::duplex(8192);
    let server = tokio::spawn(async move {
        let mut stream = tokio_rustls::TlsAcceptor::from(material.server_config())
            .accept(server_io)
            .await
            .unwrap();
        let mut text = [0; 4];
        stream.read_exact(&mut text).await.unwrap();
        assert_eq!(&text, b"ping");
        stream.write_all(b"pong").await.unwrap();
    });
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
    let mut stream = connector
        .connect(
            rustls::pki_types::ServerName::try_from("runtime.fixture.ts.net").unwrap(),
            client_io,
        )
        .await
        .unwrap();
    stream.write_all(b"ping").await.unwrap();
    let mut text = [0; 4];
    stream.read_exact(&mut text).await.unwrap();
    assert_eq!(&text, b"pong");
    server.await.unwrap();
}

// Disposable fixture certificates/keys, never production credentials.
const CA: &str = r#"-----BEGIN CERTIFICATE-----
MIIDHjCCAgagAwIBAgIUHpIxvVR1kYwjtAYo7e2g3+MczOwwDQYJKoZIhvcNAQEL
BQAwJzElMCMGA1UEAwwcS2VnYXJpLURpc3Bvc2FibGUtRml4dHVyZS1DQTAeFw0y
NjEwMjUwMTA3MjdaFw0zNjEwMjIwMTA3MjdaMCcxJTAjBgNVBAMMHEtlZ2FyaS1E
aXNwb3NhYmxlLUZpeHR1cmUtQ0EwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEK
AoIBAQC1BYz35jYxiHA7VCzzg/IdcEl2mtxdBqtdVmZ7TdKBwcnpY7ZqaMtWYefX
CFAUoZTwsDciM7/rlEYPFSw6idHLEtEIDHTbHkplxW7F6S8ocJI6RSTGK4StRS8q
ISbM/Zsqm0xombxgScwXRaFORJvo5oFDB26IC0SUNsyPpV6PQ7AddxRsWQknV2R0
/zqALsq1zsIxDW0KU/vRJ7Mu86+pjVM7ilrK40FJkPseGgLOsrx5IS1XM7lQBlFz
J+Qxil13kZJKrLmlq8ixIKE7JzfbhXFTL+83KReipMA5GERau5W51Gx2OupltLDK
sTAii6zwrPLGElhU3NxuAdf+c+NdAgMBAAGjQjBAMB0GA1UdDgQWBBTybIOdeBqF
srUs/LhAvzfe49HUPjAPBgNVHRMBAf8EBTADAQH/MA4GA1UdDwEB/wQEAwIBBjAN
BgkqhkiG9w0BAQsFAAOCAQEAkJVj0YmykQ6nHn1oUR8mbnpEsDpLysjS7R96r+kD
O0aLUjjRzkKeVY9hUGZ6puhJtE5oXJFf49NZkJRyNeCrJjiX8kxsXggIveCE5kPR
AqOvpQ00DCgzGnLd9K1we7hFchnewEHV4bSesvwDfq59wfr6KpFaEhsR9kTcKCz6
ssiv3aBCOlbv1y7eASw4U2/G6yU/UPYdtvzm1JgYgcAEPCk2xaCeqxE4kBY/V84I
wT64RsMdwL9T7hI3lEfav93D2NENojZa4KESU/uNgJRYTQzJNPWYJLcQ4L+4K1PW
FJl6y1P4w2zt60o55gYgyG1h+1hXUuFIDVJuLI87JoU47g==
-----END CERTIFICATE-----
"#;
const LEAF: &str = r#"-----BEGIN CERTIFICATE-----
MIIDLjCCAhagAwIBAgIUI0DEOboznU51NwQXlcyuMxmMWkAwDQYJKoZIhvcNAQEL
BQAwJzElMCMGA1UEAwwcS2VnYXJpLURpc3Bvc2FibGUtRml4dHVyZS1DQTAeFw0y
NjEwMjUwMTA3MjdaFw0zNjEwMjIwMTA3MjdaMCExHzAdBgNVBAMMFnJ1bnRpbWUu
Zml4dHVyZS50cy5uZXQwggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQD5
dKLPXfaaLL5bV8/bWQUvG67ee18qus09BxKGm/ac9qtlxExQLu7kXldMiODzidUh
Me/+RXbnKtXW2fFYpGpJ5G8V4hpcNxLCWk2dO8JE4FhPwW+lY3rhdip1kI8CLJEP
ZeWEpcWilMf5/hlrLK6MmRDBhyc8xP1pV3PXcZUwzNFqV1haLof8MctihywlaTJh
jckS8TXjeu4G7iMGHOD8dp30j5GUWLOEwmsat7nUuAbXLkfJ6a8cPcILEbJl4tP4
g5ujqlYr+dGSbPFuxtrAJJ3WUZwlr7lvkGDQ8Sw4kPHTUsfXYCOC0BOG8cKCs32R
kR7IZ5gr84d5cGECJZnnAgMBAAGjWDBWMAwGA1UdEwEB/wQCMAAwDgYDVR0PAQH/
BAQDAgWgMBMGA1UdJQQMMAoGCCsGAQUFBwMBMCEGA1UdEQQaMBiCFnJ1bnRpbWUu
Zml4dHVyZS50cy5uZXQwDQYJKoZIhvcNAQELBQADggEBADwFwrG5Qj3nac0tEBsS
QzsPU5WTezk4A9ccNEoTH2fXm+900RRPefBzEW0Irz2+zO1nPGA3zaB9v7ONIg60
Hhkc1ViJJg468QSMvfqfN+xqm6FqfLGHMNzOHI4U6ZTiS9NEQiVqaqYbCQbzrW6V
AWjx/8PWFxCWW4eWr1Pe9HDPvaBHQ+/NtROIcxJnjLmItvfBGMQNkGP3VgWge6e/
5DMmWNDE+IbTs+49UnUn41ia3YIMRkmEy+fE+ksUuQZFiWTmIod8X58WLBx4N6iD
qnCQMqZnd7dvTnfC/tTWZwfN5ev7eJ45MBL9Z1rcyL5DCKVbCdTsniz1tfOT3Edp
zKs=
-----END CERTIFICATE-----
"#;
const KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQD5dKLPXfaaLL5b
V8/bWQUvG67ee18qus09BxKGm/ac9qtlxExQLu7kXldMiODzidUhMe/+RXbnKtXW
2fFYpGpJ5G8V4hpcNxLCWk2dO8JE4FhPwW+lY3rhdip1kI8CLJEPZeWEpcWilMf5
/hlrLK6MmRDBhyc8xP1pV3PXcZUwzNFqV1haLof8MctihywlaTJhjckS8TXjeu4G
7iMGHOD8dp30j5GUWLOEwmsat7nUuAbXLkfJ6a8cPcILEbJl4tP4g5ujqlYr+dGS
bPFuxtrAJJ3WUZwlr7lvkGDQ8Sw4kPHTUsfXYCOC0BOG8cKCs32RkR7IZ5gr84d5
cGECJZnnAgMBAAECggEANO3QC8LKq7zZqQTxR//T70Jm8LLppuXa3VuOTTxzb6uR
A3dKn+rNamwwBr+2R0ZUEAaJ/wAbMgJvO/ajVGUSXlpXTaPbZvySgOVrOXTwC/A0
HLwU5dk9PfB2ItrI1TPClatPrz0DObNq2PrWxtxErvVdTkrQgv7jLAJPxINHngI0
IXMIilofoz01yZ71C4U6jjOOHbcVuLZk3LO7b933qQp5bqlWs18rZ7/09xVDLcHI
kEIREnrqja44T3kJULK0qFWVqRd6c3Ev5G4AaXg5BT43uzmyDH9zlIiRUxPU59+z
cRtn9aeZCCOPhnzNks39+Lc/ekoKeIX+g/CynizEsQKBgQD9vg99KG4EH9Qc+jYZ
GguCQdcuxIMmPXjlvjt2iCZAqVYbEbRn5CsKtCr3NCinMxR9voI0wQDfSgTp30+e
mqMJNyOyfUwSD/abvtf1vPu11G+9+02jIid9WwZTlEfjId0aZ1iKpSMeMDkbUsW7
+TRmGEs2hC5oF+iFNtp7ndtOFwKBgQD7rM/CR06c2tmUeI8akLawjHDLo+BL1Z0w
uEBhz2R6ZmZXQIMLxv2faKbBta0alYtNVjk6ECdqEUMSQpSB30KtA2gj3YIXUcRj
pZPOz758z4Q3oqea0yQ68GguEYhznKkoa4d9CB/deqORcpXAwIgpqLFyFTtIITdL
F1iEH/3EsQKBgBNN9Gu3ObeCnGaf+qGqQ5njXHWoabntD1ebQ3bfINnmiDCDBlCp
IUUtM9+ezkD6N+LhLZLJ5EEYposMfEg6x5CZ61NXCQSOxlpPCKsfd577zIyNo/h3
Tr3QwQh2zYlQODwcBLZ3ndZcINwdTgHhEcV6r640HDu7JeuMos6o0dh9AoGACCQm
9FnqKRjLQQpFVs6nmj5wx6Wdc9aNJy5OpttWnL8DQH9NvevJ+IYiHtIJr3p27yGC
6GXtYHj+enC87RaTyZT6oW5g7vAuMisBBQejWdY8qLASS+fYnC96r0LR7yt6LVyN
HXZN0ZROBj52qa2QughC9TXKnSK17y/qJRwD0YECgYBrXImY/yxxVy+MpBo7ffZz
LxdNa7uwRfYK2wQT1LtmuXYJS+K75y2BCB1FDDlItWq9Oo2g5DbGmYhmgTvTl0Aa
AuqaPYvXQz6mT/C0YzoQNTE2CHiCC2Wvw0TE6MDmI1kESljGzvhO6HJ5Lznsc8dm
WLJnuNsI7HzSKo+4/T3BLw==
-----END PRIVATE KEY-----
"#;
const OTHER_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC1BYz35jYxiHA7
VCzzg/IdcEl2mtxdBqtdVmZ7TdKBwcnpY7ZqaMtWYefXCFAUoZTwsDciM7/rlEYP
FSw6idHLEtEIDHTbHkplxW7F6S8ocJI6RSTGK4StRS8qISbM/Zsqm0xombxgScwX
RaFORJvo5oFDB26IC0SUNsyPpV6PQ7AddxRsWQknV2R0/zqALsq1zsIxDW0KU/vR
J7Mu86+pjVM7ilrK40FJkPseGgLOsrx5IS1XM7lQBlFzJ+Qxil13kZJKrLmlq8ix
IKE7JzfbhXFTL+83KReipMA5GERau5W51Gx2OupltLDKsTAii6zwrPLGElhU3Nxu
Adf+c+NdAgMBAAECggEAT6merOWxycCaa3GSyqOxZAUrPzfNbF7oAQ5p0S+hSRso
KT6JY3C0s1IovffZjgrByd4ylSmi9yBsaGzP5v8JxU2Fkf4xWtCMuxbUteH1r91C
bMRK967lF3+PzwGB2spsg6aOTKYDQXiYiVFKOHB3EFqIXiAiI1qaFoO7KCgFYhQO
/6vWfzXenPTr3zdTsWQ1bivOX+TtFXvOdespsVyV0WI1lG9W52mXyPrAOu3xEXRO
yfSmKJ7SrnqRgGYjTlmctzySsPvANITb63k1sZdzXj5ISNJ+nro1qU+TtP8qB9jd
/LW3ZmEEbO7UsXEi2Jb6/pe+PXopf4AXcHEeJMke3QKBgQD6i8GVygSrETIDDYXR
35lsnYGNkv5ytivn07TKzmXkqizPYMwDNFGbwtwFhEtgRbpYOHQHQCg5JOjLCX0r
E/w8/XXgkan90fMPRmA+5aYaWEJnsIN7vdt93UTmsrl3nT0YkZfGZTcu5SyosBmh
LPnw4Bvvo1054DqXsAdHSZc5DwKBgQC49llwA7UqJOuUcQFt4Co5UTbJxIsTsYSY
g9InQsD7EBBqhBOWL4mxE/e5hTHqcmmSD4ZQbWbzM7SQRARbrf/JZLa+yJoyRzpJ
fAhMS8iYmjpTSicZZUnnmivsaJCjwin2OWEsaqStqhi6DXTpb/Ti1MK2PRfQcfmB
Py0y9xpk0wKBgQDm+9mpQuoy3FODRQBOkRpdbIh4AUDceN7Zh/Bx2P9PZSSSu+ba
PW3PcEJwQ5tKUn6Nvt4ZoSh0JncIWOMwx+gQQoxeUWyrONghCPzKf+Y2+4+dELrn
Zg3paAmRtswOXsRo1jFbEw1hCp+1lUTLty9mxvZ6Y94a8YKTPhI3wrzMOQKBgFRY
kwSPzEfBumRg/CPc+6RTssZkCIAcjbj3NBiPSm2zL6lDPaNZXlMQJgHGCbM+JYJj
7f4zRaVWSNsZCVu9h02RzfxGgnq3HI9yLl6MmJdujXZ59EaSeBdTqQAP1xW2b2hH
Vlbo8tv3G3uMs2nYkjX2jCyKw5dcwkj8RGJfaPvdAoGARqcFbz6df8L8RSpGaYuc
Pt92nqOTFf1Bd0cchwAAhaQQ8LOXcUIv7/spySvp6vxQ9OZJX5YPWa4HC+XU+OsH
IDXLkQXEaQrpwe4CcExzxBOrWcx4Ryd7ZCTQNtrQyea22LWePgB1ZASR51AAar0S
1qVDLFBzlpE/Qxs/dunk3q4=
-----END PRIVATE KEY-----
"#;

struct DelayedWhois {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl TailscaleCommand for DelayedWhois {
    fn run(&self, args: Vec<String>) -> Pin<Box<dyn Future<Output = Result<Vec<u8>>> + Send + '_>> {
        Box::pin(async move {
            let value = if args[0] == "version" {
                version()
            } else {
                self.started.notify_one();
                self.release.notified().await;
                whois("100.100.1.2:5555")
            };
            Ok(serde_json::to_vec(&value).unwrap())
        })
    }
}
#[tokio::test]
async fn role_revocation_during_whois_takes_effect_before_dispatch() {
    let cli = Arc::new(DelayedWhois {
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let auth = TransportAuthentication::new(
        configuration(),
        Some(TailscaleClient::with_command(cli.clone())),
    );
    let copy = auth.clone();
    let pending = tokio::spawn(async move {
        copy.authenticate(Peer::DirectTls {
            address: "100.100.1.2:5555".parse().unwrap(),
        })
        .await
    });
    cli.started.notified().await;
    let mut changed = configuration();
    changed.tailscale_users.clear();
    auth.replace_config(changed).unwrap();
    cli.release.notify_one();
    assert_eq!(pending.await.unwrap().unwrap_err().code, "access_denied");
}
#[tokio::test]
async fn installed_cli_runner_executes_only_read_only_identity_vocabulary() {
    let fixture = tempfile::tempdir().unwrap();
    let executable = fixture
        .path()
        .canonicalize()
        .unwrap()
        .join("tailscale-fixture");
    std::fs::write(&executable, "#!/bin/sh\nprintf 'fixture-identity-output'\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let runner = InstalledTailscale::new(executable.clone()).unwrap();
    assert_eq!(
        runner
            .run(vec!["version".into(), "--json".into(), "--daemon".into()])
            .await
            .unwrap(),
        b"fixture-identity-output"
    );
    assert_eq!(
        runner.run(vec!["up".into()]).await.unwrap_err().code,
        "access_denied"
    );
    std::fs::set_permissions(executable.clone(), std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(InstalledTailscale::new(executable).is_err());
}
#[tokio::test]
async fn enabling_and_replacing_remote_verifier_is_atomic_and_old_lookup_cannot_authenticate() {
    let mut disabled = configuration();
    disabled.https = None;
    let auth = TransportAuthentication::new(disabled, None);
    let peer = Peer::DirectTls {
        address: "100.100.1.2:5555".parse().unwrap(),
    };
    assert_eq!(
        auth.authenticate(peer.clone()).await.unwrap_err().code,
        "access_denied"
    );
    auth.replace_config(configuration()).unwrap();
    assert_eq!(
        auth.authenticate(peer.clone()).await.unwrap_err().code,
        "identity_unavailable"
    );
    let old = Arc::new(DelayedWhois {
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    auth.replace_tailscale(Some(TailscaleClient::with_command(old.clone())))
        .unwrap();
    let pending_auth = auth.clone();
    let pending_peer = peer.clone();
    let pending = tokio::spawn(async move { pending_auth.authenticate(pending_peer).await });
    old.started.notified().await;
    let new = Cli::new(vec![version(), whois("100.100.1.2:5555")]);
    auth.replace_configuration(configuration(), Some(TailscaleClient::with_command(new)))
        .unwrap();
    old.release.notify_one();
    assert_eq!(pending.await.unwrap().unwrap_err().code, "access_denied");
    assert!(
        auth.authenticate(peer)
            .await
            .unwrap()
            .roles
            .contains("accept")
    );
}
