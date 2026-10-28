use clap::Parser;
use kegari_cli::cli::*;
use kegari_cli::client::*;
use kegari_v0::api::{CommandName, CommandRequest};
use kegari_v0::error::{Error, Result};
use kegari_v0::secrets::Secret;
use kegari_v0::snapshot::Snapshots;
use kegari_v0::storage::PrivateDir;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

#[derive(Default)]
struct Inputs {
    stdin: Vec<u8>,
    files: BTreeMap<PathBuf, Vec<u8>>,
    reads: usize,
    tty: bool,
}
impl InputSource for Inputs {
    fn stdin_is_terminal(&self) -> bool {
        self.tty
    }
    fn read_stdin(&mut self, max: usize) -> Result<Vec<u8>> {
        self.reads += 1;
        if self.stdin.len() > max {
            return Err(Error::new("invalid_input", "oversized fixture stdin"));
        }
        Ok(self.stdin.clone())
    }
    fn read_file(&mut self, path: &Path, max: usize) -> Result<Vec<u8>> {
        let bytes = self
            .files
            .get(path)
            .ok_or_else(|| Error::new("io_error", "fixture file unavailable"))?;
        if bytes.len() > max {
            return Err(Error::new("invalid_input", "oversized fixture file"));
        }
        Ok(bytes.clone())
    }
}
fn prepare(args: &[&str]) -> PreparedInvocation {
    Cli::try_parse_from(std::iter::once("kegari-v0").chain(args.iter().copied()))
        .unwrap()
        .prepare(&mut Inputs::default())
        .unwrap()
}
fn server(invocation: &PreparedInvocation) -> &ServerCommand {
    match &invocation.action {
        Action::Server(command) => command,
        _ => panic!("expected server command"),
    }
}
fn runtime() -> Uuid {
    Uuid::from_u128(123)
}
fn request(command: CommandName, args: Value) -> CommandRequest {
    CommandRequest {
        api_version: 1,
        runtime_id: runtime(),
        request_id: Uuid::new_v4(),
        command,
        args,
        expected_config_revision: None,
        expected_control_generation: None,
    }
}

#[test]
fn evaluation_cli_preserves_declared_slot_and_requires_observed_revisions() {
    let workspace = Uuid::new_v4().to_string();
    let request_id = Uuid::new_v4();
    let request_text = request_id.to_string();
    let suite = kegari_v0::digest::bytes_digest(b"declared suite");
    let config = kegari_v0::digest::bytes_digest(b"observed config");
    let args = vec![
        "kegari-v0",
        "--target",
        "local",
        "--json",
        "evaluation",
        "run",
        "--workspace",
        &workspace,
        "--suite-digest",
        &suite,
        "--case",
        "V0-E05",
        "--variant",
        "independent_malicious",
        "--profile",
        "operator_evaluated",
        "--repetition",
        "2",
        "--expected-config",
        &config,
        "--expected-control",
        "7",
        "--request-id",
        &request_text,
    ];
    let mut inputs = Inputs::default();
    let invocation = Cli::try_parse_from(&args)
        .unwrap()
        .prepare(&mut inputs)
        .unwrap();
    assert_eq!(inputs.reads, 0);
    assert_eq!(invocation.request_id, request_id);
    let command = server(&invocation).request(runtime(), invocation.request_id);
    command.validate(runtime()).unwrap();
    assert_eq!(command.command, CommandName::EvaluationRun);
    assert_eq!(
        command.expected_config_revision.as_deref(),
        Some(config.as_str())
    );
    assert_eq!(command.expected_control_generation, Some(7));
    assert_eq!(
        command.args,
        json!({
            "workspace": workspace, "suite_digest": suite, "case": "V0-E05",
            "variant": "independent_malicious", "profile": "operator_evaluated",
            "repetition": 2
        })
    );
    for required in ["--expected-config", "--expected-control"] {
        let mut missing = args.clone();
        let index = missing.iter().position(|arg| *arg == required).unwrap();
        missing.drain(index..=index + 1);
        assert!(Cli::try_parse_from(missing).is_err());
    }
    for forbidden in ["--max-seconds", "--source", "--permit"] {
        let mut injected = args.clone();
        injected.extend([forbidden, "injected"]);
        assert!(Cli::try_parse_from(injected).is_err());
    }
}

#[test]
fn default_overview_and_every_public_command_are_parsed() {
    assert_eq!(server(&prepare(&[])).command, CommandName::Overview);
    let id = "12345678-1234-1234-1234-123456789012";
    let cases = vec![
        vec!["overview", "--workspace", "team"],
        vec!["status"],
        vec!["show", "team/ABC-1"],
        vec!["open", "ABC-1", "--url-only"],
        vec!["approve", "ABC-1", "--result", "token"],
        vec!["correct", "ABC-1", "--message", "change boundary"],
        vec!["release", "ABC-1", "--result", "token"],
        vec!["connection", "list"],
        vec!["connection", "reconnect", "codex", "--noninteractive"],
        vec!["workspace", "list"],
        vec!["config", "show"],
        vec!["config", "check", "--workspace", "team"],
        vec!["pause"],
        vec!["resume"],
        vec!["stop", "--force", "--wait", "--timeout", "1"],
        vec!["start", "--expected-control", "4"],
        vec!["operation", "show", id],
        vec!["operation", "reconcile", id],
        vec!["retry", "team/CHILD-2", "--reason", "blocker resolved"],
        vec![
            "release",
            "recover",
            "ABC-1",
            "--operation",
            id,
            "--action",
            "resume",
            "--reason",
            "reconciled",
        ],
        vec!["cleanup", "--operation", id],
        vec!["cleanup", "--operation", id, "--apply", "--plan", "token"],
        vec!["target", "list"],
        vec![
            "target",
            "add",
            "host",
            "--url",
            "https://node.example.ts.net",
            "--runtime",
            id,
        ],
        vec![
            "service",
            "install",
            "--binary",
            "/private/tmp/versioned-kegari",
        ],
        vec!["service", "start"],
        vec!["service", "stop"],
    ];
    for args in cases {
        let _ = prepare(&args);
    }
    assert_eq!(server(&prepare(&["resume"])).command, CommandName::Resume);
    assert_eq!(server(&prepare(&["pause"])).command, CommandName::Pause);
    assert_eq!(server(&prepare(&["stop", "--wait"])).wait_seconds, Some(30));
}
#[test]
fn lifecycle_workspace_filters_and_unsafe_or_incomplete_grammar_are_rejected() {
    for args in [
        vec!["pause", "--workspace", "x"],
        vec!["resume", "--workspace", "x"],
        vec!["stop", "--workspace", "x"],
        vec!["start", "--workspace", "x"],
        vec!["correct", "ABC-1"],
        vec!["correct", "ABC-1", "--message", "x", "--stdin"],
        vec!["stop", "--timeout", "3"],
        vec!["start", "--wait", "--timeout", "0"],
        vec!["approve", "ABC-1"],
        vec![
            "cleanup",
            "--operation",
            "12345678-1234-1234-1234-123456789012",
            "--apply",
        ],
    ] {
        assert!(Cli::try_parse_from(std::iter::once("kegari-v0").chain(args)).is_err());
    }
    for args in [
        vec!["--target", "remote", "service", "start"],
        vec!["service", "install", "--binary", "relative"],
        vec!["release", "ABC-1"],
        vec![
            "release",
            "recover",
            "ABC-1",
            "--operation",
            "12345678-1234-1234-1234-123456789012",
            "--action",
            "supersede",
            "--reason",
            "fixed",
        ],
    ] {
        assert!(
            Cli::try_parse_from(std::iter::once("kegari-v0").chain(args))
                .unwrap()
                .prepare(&mut Inputs::default())
                .is_err()
        );
    }
}
#[test]
fn mutation_id_is_created_once_or_preserved_exactly_and_nil_is_rejected() {
    let id = Uuid::new_v4();
    let id_text = id.to_string();
    let invocation = prepare(&[
        "correct",
        "ABC-1",
        "--message",
        "x",
        "--request-id",
        &id_text,
    ]);
    assert_eq!(invocation.request_id, id);
    assert_eq!(
        server(&invocation)
            .request(runtime(), invocation.request_id)
            .request_id,
        id
    );
    let default = prepare(&["release", "ABC-1", "--result", "token"]);
    assert!(!default.request_id.is_nil());
    assert_eq!(
        default.request_id,
        server(&default)
            .request(runtime(), default.request_id)
            .request_id
    );
    assert!(
        Cli::try_parse_from([
            "kegari-v0",
            "pause",
            "--request-id",
            "00000000-0000-0000-0000-000000000000"
        ])
        .unwrap()
        .prepare(&mut Inputs::default())
        .is_err()
    );
}
#[test]
fn correction_file_and_stdin_are_client_inputs_bounded_utf8_and_nonempty() {
    let mut input = Inputs {
        stdin: "fix café\n".as_bytes().to_vec(),
        ..Default::default()
    };
    let cli = Cli::try_parse_from([
        "kegari-v0",
        "--target",
        "remote",
        "correct",
        "ABC-1",
        "--stdin",
    ])
    .unwrap();
    let value = cli.prepare(&mut input).unwrap();
    assert_eq!(server(&value).args["message"], "fix café\n");
    assert_eq!(input.reads, 1);
    input
        .files
        .insert(PathBuf::from("client.txt"), b"local file content".to_vec());
    let value = Cli::try_parse_from([
        "kegari-v0",
        "correct",
        "ABC-1",
        "--message-file",
        "client.txt",
    ])
    .unwrap()
    .prepare(&mut input)
    .unwrap();
    assert_eq!(server(&value).args["message"], "local file content");
    for bytes in [
        vec![],
        b"  \n".to_vec(),
        vec![0xff],
        vec![b'a'; MAX_CORRECTION + 1],
    ] {
        let mut input = Inputs {
            stdin: bytes,
            ..Default::default()
        };
        assert!(
            Cli::try_parse_from(["kegari-v0", "correct", "ABC-1", "--stdin"])
                .unwrap()
                .prepare(&mut input)
                .is_err()
        );
    }
}
#[test]
fn secret_input_is_separate_redacted_and_cannot_share_configuration_stdin() {
    let config=b"kind = 'github'\naccount_id = 'verified-user'\nendpoint = 'https://api.github.com/'\ncredential_ref = 'keychain:github'\n";
    let mut input = Inputs {
        stdin: b"fixture-private-value\n".to_vec(),
        files: BTreeMap::from([(PathBuf::from("connection.toml"), config.to_vec())]),
        ..Default::default()
    };
    let value = Cli::try_parse_from([
        "kegari-v0",
        "connection",
        "add",
        "github",
        "--kind",
        "github",
        "--from",
        "connection.toml",
        "--secret-stdin",
        "--noninteractive",
    ])
    .unwrap()
    .prepare(&mut input)
    .unwrap();
    assert_eq!(
        value.secret.as_ref().unwrap().bytes(),
        b"fixture-private-value"
    );
    assert!(!format!("{value:?}").contains("fixture-private-value"));
    assert!(format!("{value:?}").contains("REDACTED"));
    let encoded =
        serde_json::to_string(&server(&value).request(runtime(), value.request_id)).unwrap();
    assert!(!encoded.contains("fixture-private-value"));
    assert!(!encoded.contains("secret_input"));
    let mut shared = Inputs::default();
    assert!(
        Cli::try_parse_from([
            "kegari-v0",
            "connection",
            "add",
            "github",
            "--kind",
            "github",
            "--from",
            "-",
            "--secret-stdin"
        ])
        .unwrap()
        .prepare(&mut shared)
        .is_err()
    );
    assert_eq!(shared.reads, 0);
    input.tty = true;
    input.reads = 0;
    assert!(
        Cli::try_parse_from([
            "kegari-v0",
            "connection",
            "add",
            "github",
            "--kind",
            "github",
            "--from",
            "connection.toml",
            "--secret-stdin",
        ])
        .unwrap()
        .prepare(&mut input)
        .is_err()
    );
    assert_eq!(
        input.reads, 0,
        "terminal secrets must never be read with echo enabled"
    );
}
#[test]
fn noninteractive_missing_input_never_prompts_or_reads_stdin() {
    for args in [
        vec!["setup", "--noninteractive"],
        vec!["workspace", "add", "test", "--noninteractive"],
        vec![
            "connection",
            "add",
            "test",
            "--kind",
            "codex",
            "--noninteractive",
        ],
    ] {
        let mut input = Inputs {
            tty: true,
            ..Default::default()
        };
        let result = Cli::try_parse_from(std::iter::once("kegari-v0").chain(args))
            .unwrap()
            .prepare(&mut input);
        assert!(result.is_err());
        assert_eq!(input.reads, 0);
    }
    let mut input = Inputs {
        tty: true,
        ..Default::default()
    };
    assert!(matches!(
        Cli::try_parse_from(["kegari-v0", "setup"])
            .unwrap()
            .prepare(&mut input)
            .unwrap()
            .action,
        Action::Guided(GuidanceRequest::Setup)
    ));
}
#[test]
fn targets_pin_identity_no_repins_fallback_or_insecure_origins() {
    let temp = TempDir::new().unwrap();
    let store = TargetStore::new(
        PrivateDir::create(&temp.path().canonicalize().unwrap().join("client")).unwrap(),
    );
    let target = Target {
        name: "remote".into(),
        runtime_id: runtime(),
        api_version: 1,
        endpoint: Endpoint::Https {
            origin: "https://node.example.ts.net/".into(),
        },
    };
    store.add(target.clone()).unwrap();
    assert_eq!(store.select(Some("remote")).unwrap(), target);
    assert!(store.select(None).is_err());
    assert!(store.select(Some("unknown")).is_err());
    let mut changed = target.clone();
    changed.runtime_id = Uuid::new_v4();
    assert_eq!(store.add(changed).unwrap_err().code, "identity_mismatch");
    for url in [
        "http://node.example.ts.net",
        "https://user:pass@node.example.ts.net",
        "https://node.example.ts.net/path",
        "https://node.example.ts.net/?token=x",
        "https://node.example.ts.net/#fragment",
    ] {
        let mut bad = target.clone();
        bad.endpoint = Endpoint::Https { origin: url.into() };
        assert!(bad.validate().is_err());
    }
    let mut bad = target;
    bad.api_version = 2;
    assert!(bad.validate().is_err());
}

#[cfg(unix)]
struct Fixture {
    _temp: TempDir,
    client: ControlClient,
    calls: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
#[cfg(unix)]
async fn fixture<F>(count: usize, mut answer: F) -> Fixture
where
    F: FnMut(usize, Value) -> Option<(u16, Value, Vec<(&'static str, String)>)> + Send + 'static,
{
    use std::os::unix::fs::PermissionsExt;
    let temp = TempDir::new().unwrap();
    let dir = PrivateDir::create(&temp.path().canonicalize().unwrap().join("control")).unwrap();
    let socket = dir.path().join("control.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed = calls.clone();
    let task = tokio::spawn(async move {
        for index in 0..count {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            let header_end;
            loop {
                let mut chunk = [0; 4096];
                let length = stream.read(&mut chunk).await.unwrap();
                assert_ne!(length, 0);
                bytes.extend_from_slice(&chunk[..length]);
                if let Some(end) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    header_end = end + 4;
                    break;
                }
                assert!(bytes.len() < 32768);
            }
            let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
            assert!(headers.starts_with("POST /v1/commands HTTP/1.1"));
            assert!(headers.to_lowercase().contains("x-kegari-protocol: 1"));
            assert!(
                headers
                    .to_lowercase()
                    .contains(&format!("x-kegari-runtime: {}", runtime()))
            );
            assert!(!headers.to_lowercase().contains("origin:"));
            let size: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .unwrap()
                .parse()
                .unwrap();
            while bytes.len() - header_end < size {
                let mut chunk = [0; 4096];
                let length = stream.read(&mut chunk).await.unwrap();
                assert_ne!(length, 0);
                bytes.extend_from_slice(&chunk[..length]);
            }
            let request: Value =
                serde_json::from_slice(&bytes[header_end..header_end + size]).unwrap();
            observed.lock().unwrap().push(request.clone());
            if let Some((status, value, extra)) = answer(index, request) {
                let body = serde_json::to_vec(&value).unwrap();
                let mut head = format!(
                    "HTTP/1.1 {status} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for (name, value) in extra {
                    head.push_str(&format!("{name}: {value}\r\n"));
                }
                head.push_str("\r\n");
                stream.write_all(head.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            }
        }
    });
    let client = ControlClient::new(Target {
        name: "fixture".into(),
        runtime_id: runtime(),
        api_version: 1,
        endpoint: Endpoint::Unix {
            socket,
            authority: "localhost".into(),
        },
    })
    .unwrap();
    Fixture {
        _temp: temp,
        client,
        calls,
        task,
    }
}
fn reply(request: &Value, data: Value) -> Value {
    json!({"api_version":1,"request_id":request["request_id"],"runtime_id":runtime(),"config_revision":"revision-one","observed_at":chrono::Utc::now(),"ok":true,"operation_id":null,"state":null,"data":data,"error":null})
}
#[cfg(unix)]
#[tokio::test]
async fn actual_unix_transport_binds_generation_once_and_preserves_mutation_id() {
    let fixture = fixture(2, |index, request| {
        let data = if index == 0 {
            assert_eq!(request["command"], "status");
            json!({"control_generation":7})
        } else {
            assert_eq!(request["command"], "pause");
            assert_eq!(request["expected_control_generation"], 7);
            json!({"control_generation":8})
        };
        Some((200, reply(&request, data), vec![]))
    })
    .await;
    let mut request = request(CommandName::Pause, json!({}));
    let id = request.request_id;
    let result = fixture.client.execute(&mut request, None).await;
    assert!(result.ok);
    assert_eq!(request.expected_control_generation, Some(7));
    assert_eq!(result.request_id, id);
    fixture.task.await.unwrap();
    assert_eq!(fixture.calls.lock().unwrap().len(), 2);
}
#[cfg(unix)]
#[tokio::test]
async fn stale_generation_is_returned_without_refetch_or_replay() {
    let fixture = fixture(1, |_, request| {
        assert_eq!(request["expected_control_generation"], 4);
        let mut reply = reply(&request, Value::Null);
        reply["ok"] = json!(false);
        reply["error"] =
            serde_json::to_value(Error::new("stale_control", "newer intent exists")).unwrap();
        Some((409, reply, vec![]))
    })
    .await;
    let mut request = request(
        CommandName::Stop,
        json!({"force":false,"wait":false,"timeout_seconds":30}),
    );
    request.expected_control_generation = Some(4);
    let result = fixture.client.execute(&mut request, None).await;
    assert_eq!(result.error.unwrap().code, "stale_control");
    fixture.task.await.unwrap();
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
}
#[cfg(unix)]
#[tokio::test]
async fn dropped_mutation_response_remains_unknown_with_original_request_id() {
    let fixture = fixture(1, |_, _| None).await;
    let request = request(
        CommandName::Correct,
        json!({"parent":"ABC-1","message":"a correction"}),
    );
    let id = request.request_id;
    let result = fixture.client.send(&request, None).await;
    assert!(!result.ok);
    assert_eq!(result.request_id, id);
    assert_eq!(result.operation_id, None);
    assert_eq!(result.state.as_deref(), Some("uncertain"));
    assert_eq!(result.error.as_ref().unwrap().exit_code(), 4);
    fixture.task.await.unwrap();
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
}
#[cfg(unix)]
#[tokio::test]
async fn target_identity_redirect_and_response_identity_mismatch_fail_closed() {
    for mutation in [0, 1, 2] {
        let fixture = fixture(1, move |_, request| {
            let mut value = reply(&request, json!({}));
            match mutation {
                0 => value["runtime_id"] = json!(Uuid::new_v4()),
                1 => {
                    return Some((
                        302,
                        value,
                        vec![("Location", "http://127.0.0.1:9/ignored".into())],
                    ));
                }
                _ => value["request_id"] = json!(Uuid::new_v4()),
            };
            Some((200, value, vec![]))
        })
        .await;
        let result = fixture
            .client
            .send(&request(CommandName::Status, json!({})), None)
            .await;
        assert!(!result.ok);
        assert!(matches!(
            result.error.unwrap().code.as_str(),
            "identity_mismatch" | "request_conflict"
        ));
        fixture.task.await.unwrap();
    }
}
#[cfg(unix)]
#[tokio::test]
async fn secret_enters_only_final_transport_and_is_not_in_normal_request() {
    let fixture = fixture(1, |_, request| {
        assert_eq!(request["args"]["secret_input"], "fixture-private-value");
        Some((200, reply(&request, json!({"imported":true})), vec![]))
    })
    .await;
    let request = request(
        CommandName::ConnectionAdd,
        json!({"name":"github","connection":"typed TOML fixture"}),
    );
    let secret = Secret::new(b"fixture-private-value".to_vec()).unwrap();
    let result = fixture.client.send(&request, Some(&secret)).await;
    assert!(result.ok);
    assert!(request.args.get("secret_input").is_none());
    assert!(!format!("{request:?}").contains("fixture-private-value"));
    fixture.task.await.unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn complete_snapshot_pages_and_utf8_chunks_are_assembled_before_output() {
    let mut snapshots = Snapshots::default();
    let now = chrono::Utc::now();
    let data = json!({"children":(0..150).map(|index|json!({"id":index,"status":"required"})).collect::<Vec<_>>(),"description":"🦀".repeat(40000)});
    let first = snapshots
        .capture("actor", runtime(), &json!({}), &data, now)
        .unwrap();
    let mut pages = vec![first];
    while let Some(cursor) = pages.last().unwrap().next_cursor {
        pages.push(
            snapshots
                .page(
                    pages[0].snapshot_id,
                    cursor,
                    "actor",
                    runtime(),
                    &json!({}),
                    now,
                )
                .unwrap(),
        );
    }
    let expected = pages.len();
    let fixture = fixture(expected, move |index, request| {
        if index > 0 {
            assert_eq!(request["args"]["snapshot_id"], json!(pages[0].snapshot_id));
            assert_eq!(
                request["args"]["cursor"],
                json!(pages[index].cursor.unwrap())
            );
        }
        Some((
            200,
            reply(&request, serde_json::to_value(&pages[index]).unwrap()),
            vec![],
        ))
    })
    .await;
    let result = fixture
        .client
        .read_complete(&request(CommandName::Show, json!({"parent":"ABC-1"})))
        .await;
    assert!(result.ok);
    assert_eq!(result.data, data);
    fixture.task.await.unwrap();
    assert_eq!(fixture.calls.lock().unwrap().len(), expected);
}
#[cfg(unix)]
#[tokio::test]
async fn mixed_snapshot_revision_is_not_emitted_as_complete() {
    let mut snapshots = Snapshots::default();
    let now = chrono::Utc::now();
    let data = json!((0..101).collect::<Vec<_>>());
    let first = snapshots
        .capture("actor", runtime(), &json!({}), &data, now)
        .unwrap();
    let mut second = snapshots
        .page(
            first.snapshot_id,
            first.next_cursor.unwrap(),
            "actor",
            runtime(),
            &json!({}),
            now,
        )
        .unwrap();
    second.snapshot_id = Uuid::new_v4();
    let fixture = fixture(2, move |index, request| {
        Some((
            200,
            reply(
                &request,
                serde_json::to_value(if index == 0 { &first } else { &second }).unwrap(),
            ),
            vec![],
        ))
    })
    .await;
    let result = fixture
        .client
        .read_complete(&request(CommandName::Overview, json!({})))
        .await;
    assert!(!result.ok);
    assert!(result.data.is_null());
    assert_eq!(result.error.unwrap().code, "incomplete_snapshot");
    fixture.task.await.unwrap();
}
#[test]
fn json_is_one_complete_envelope_and_human_overview_preserves_all_sections() {
    let output = OutputEnvelope::success(
        Uuid::new_v4(),
        Some(runtime()),
        json!({"running":[],"blocked":[],"ready_for_review":[],"ready_for_release":[],"next_action":"none"}),
    );
    let mut bytes = Vec::new();
    write_output(&output, true, Some("test"), &mut bytes).unwrap();
    assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
    let parsed: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(parsed["operation_id"].is_null());
    assert!(parsed["error"].is_null());
    let text = render_human(&output, Some("test"));
    let positions = [
        "Ready for release",
        "Ready for review",
        "Blocked",
        "Running",
    ]
    .map(|label| text.find(label).unwrap());
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(text.matches("(none)").count(), 4);
    assert!(text.contains("Runtime:"));
    assert!(text.contains("Request:"));
    let client = OutputEnvelope::success(Uuid::new_v4(), None, json!({"targets":[]}));
    assert!(serde_json::to_value(client).unwrap()["runtime_id"].is_null());
}
#[test]
fn error_and_observed_operation_exit_codes_match_contract() {
    for (code, exit) in [
        ("invalid_input", 2),
        ("invalid_config", 3),
        ("timeout", 4),
        ("authentication_required", 5),
        ("identity_mismatch", 6),
        ("operation_uncertain", 7),
    ] {
        assert_eq!(
            OutputEnvelope::failure(Uuid::new_v4(), None, Error::new(code, "safe")).exit_code(),
            exit
        );
    }
    let mut output = OutputEnvelope::success(Uuid::new_v4(), Some(runtime()), json!({}));
    output.state = Some("draining".into());
    assert_eq!(output.exit_code(), 0);
    output.state = Some("partial".into());
    assert_eq!(output.exit_code(), 7);
}

#[derive(Default)]
struct LocalFixture {
    registered: bool,
    bootstraps: usize,
    services: usize,
    browsers: Vec<String>,
    initial_guides: usize,
    document_guides: usize,
    guided_configuration: Option<String>,
    submitted_configuration: Option<String>,
    register_during_guidance: bool,
}
impl LocalActions for LocalFixture {
    fn registered_control(&mut self) -> Result<bool> {
        Ok(self.registered)
    }
    fn bootstrap<'a>(
        &'a mut self,
        configuration: &'a str,
        id: Uuid,
    ) -> LocalFuture<'a, OutputEnvelope> {
        self.bootstraps += 1;
        self.submitted_configuration = Some(configuration.to_owned());
        Box::pin(async move {
            Ok(OutputEnvelope::success(
                id,
                Some(runtime()),
                json!({"desired":"stopped","agents_started":0}),
            ))
        })
    }
    fn service<'a>(&'a mut self, _: LocalCommand, id: Uuid) -> LocalFuture<'a, OutputEnvelope> {
        self.services += 1;
        Box::pin(async move {
            Ok(OutputEnvelope::success(
                id,
                None,
                json!({"service_action":"explicit"}),
            ))
        })
    }
    fn guide<'a>(&'a mut self, _: &'a GuidanceRequest) -> LocalFuture<'a, String> {
        self.document_guides += 1;
        let configuration = self.guided_configuration.clone();
        Box::pin(async move {
            configuration
                .ok_or_else(|| Error::new("invalid_input", "fixture has no interactive input"))
        })
    }
    fn guide_initial_setup(&mut self) -> LocalFuture<'_, String> {
        self.initial_guides += 1;
        let configuration = self.guided_configuration.clone();
        Box::pin(async move {
            if self.register_during_guidance {
                tokio::task::yield_now().await;
                self.registered = true;
            }
            configuration.ok_or_else(|| Error::new("cancelled", "fixture cancelled before review"))
        })
    }
    fn open_browser<'a>(&'a mut self, url: &'a str) -> LocalFuture<'a, ()> {
        self.browsers.push(url.into());
        Box::pin(async { Ok(()) })
    }
}
#[tokio::test]
async fn guided_initial_setup_uses_local_identity_only_for_absent_local_registration() {
    let temporary = TempDir::new().unwrap();
    let home = temporary.path().canonicalize().unwrap();
    let host = kegari_v0::platform::HostIdentity {
        host_id: "explicit-fixture-host".into(),
        boot_id: Uuid::new_v4(),
        uid: 501,
        user: "fixture-operator".into(),
        home: home.clone(),
    };
    // Real pure guidance/typed validation; the LocalActions transaction below
    // is deliberately a fixture, with no native discovery or Keychain action.
    let answers = format!("operator\n{}\n2\n\n\ncreate\n", Uuid::from_u128(42));
    let document = kegari_cli::local_cli::guided_initial_setup(
        &host,
        std::io::Cursor::new(answers),
        Vec::new(),
    )
    .unwrap();
    let store = TargetStore::new(PrivateDir::create(&home.join("client")).unwrap());
    let make = |target: Option<&str>| {
        let mut args = vec!["kegari-v0"];
        if let Some(target) = target {
            args.extend(["--target", target]);
        }
        args.push("setup");
        Cli::try_parse_from(args)
            .unwrap()
            .prepare(&mut Inputs {
                tty: true,
                ..Default::default()
            })
            .unwrap()
    };
    let mut local = LocalFixture {
        guided_configuration: Some(document.clone()),
        ..Default::default()
    };
    let response = execute(make(None), &store, &mut local).await;
    assert!(response.ok);
    assert_eq!(local.initial_guides, 1);
    assert_eq!(local.document_guides, 0);
    assert_eq!(local.bootstraps, 1);
    assert_eq!(
        local.submitted_configuration.as_deref(),
        Some(document.as_str())
    );
    assert_eq!(local.services, 0);
    for (registered, target) in [(true, None), (false, Some("absent-remote"))] {
        let mut local = LocalFixture {
            registered,
            guided_configuration: Some(document.clone()),
            ..Default::default()
        };
        let response = execute(make(target), &store, &mut local).await;
        assert_eq!(response.error.unwrap().code, "target_unavailable");
        assert_eq!(local.initial_guides, 0);
        assert_eq!(local.document_guides, 1);
        assert_eq!(local.bootstraps, 0);
        assert_eq!(local.services, 0);
    }
    let mut cancelled = LocalFixture::default();
    let response = execute(make(None), &store, &mut cancelled).await;
    assert_eq!(response.error.unwrap().code, "cancelled");
    assert_eq!(cancelled.initial_guides, 1);
    assert_eq!(cancelled.bootstraps, 0);
    assert_eq!(cancelled.services, 0);
    assert!(
        std::fs::read_dir(home.join("client"))
            .unwrap()
            .next()
            .is_none()
    );
    let mut inputs = Inputs {
        tty: true,
        ..Default::default()
    };
    assert!(
        Cli::try_parse_from(["kegari-v0", "setup", "--noninteractive"])
            .unwrap()
            .prepare(&mut inputs)
            .is_err()
    );
    assert_eq!(inputs.reads, 0);
}

#[cfg(unix)]
#[tokio::test]
async fn initial_setup_registration_race_never_submits_generated_config_to_existing_host() {
    let server = fixture(2, |_, request| {
        Some((
            200,
            reply(&request, json!({"control_generation": 7})),
            vec![],
        ))
    })
    .await;
    let temporary = TempDir::new().unwrap();
    let home = temporary.path().canonicalize().unwrap();
    let host = kegari_v0::platform::HostIdentity {
        host_id: "initial-setup-race-fixture".into(),
        boot_id: Uuid::new_v4(),
        uid: 501,
        user: "fixture-operator".into(),
        home: home.clone(),
    };
    let document = kegari_cli::local_cli::guided_initial_setup(
        &host,
        std::io::Cursor::new(format!(
            "operator\n{}\n2\n\n\ncreate\n",
            Uuid::from_u128(42)
        )),
        Vec::new(),
    )
    .unwrap();
    let client_root = home.join("client");
    let store = TargetStore::new(PrivateDir::create(&client_root).unwrap());
    // Retained client preferences are not a live registration observation.
    // Leave a real reachable fixture endpoint to detect any accidental routing.
    store
        .register_local(server.client.target().clone())
        .unwrap();
    let before = std::fs::read(client_root.join("clients.toml")).unwrap();
    let invocation = Cli::try_parse_from(["kegari-v0", "setup"])
        .unwrap()
        .prepare(&mut Inputs {
            tty: true,
            ..Default::default()
        })
        .unwrap();
    let id = invocation.request_id;
    let mut local = LocalFixture {
        guided_configuration: Some(document),
        register_during_guidance: true,
        ..Default::default()
    };
    let response = execute(invocation, &store, &mut local).await;
    server.task.abort();
    let _ = server.task.await;
    assert_eq!(response.request_id, id);
    assert_eq!(response.error.unwrap().code, "setup_incomplete");
    assert!(response.runtime_id.is_none());
    assert!(local.registered);
    assert_eq!(local.initial_guides, 1);
    assert_eq!(local.document_guides, 0);
    assert_eq!(local.bootstraps, 0);
    assert_eq!(local.services, 0);
    assert!(local.submitted_configuration.is_none());
    assert!(server.calls.lock().unwrap().is_empty());
    assert_eq!(
        std::fs::read(client_root.join("clients.toml")).unwrap(),
        before
    );
    for leaf in ["owner.json", "config.toml", "intent.json", "service.json"] {
        assert!(!home.join(".kegari").join(leaf).exists());
    }
}

#[tokio::test]
async fn no_argument_first_launch_explains_setup_without_registering_or_starting() {
    let temp = TempDir::new().unwrap();
    let account = kegari_v0::platform::AccountHome {
        uid: 501,
        user: "first-launch-fixture".into(),
        home: temp.path().canonicalize().unwrap(),
    };
    let store = kegari_cli::local_cli::target_store(&account).unwrap();
    assert!(
        kegari_cli::local_cli::registered_target(&account, &store)
            .unwrap()
            .is_none()
    );
    let mut inputs = Inputs::default();
    let invocation = Cli::try_parse_from(["kegari-v0"])
        .unwrap()
        .prepare(&mut inputs)
        .unwrap();
    assert_eq!(server(&invocation).command, CommandName::Overview);
    let mut local = LocalFixture::default();
    let response = execute(invocation, &store, &mut local).await;
    assert!(!response.ok);
    assert_eq!(response.exit_code(), 4);
    assert!(response.runtime_id.is_none());
    let error = response.error.as_ref().unwrap();
    assert_eq!(error.code, "target_unavailable");
    assert!(error.message.contains("kegari-v0 setup --help"));
    assert!(error.message.contains("explicit setup"));
    assert!(
        error
            .next_action
            .as_deref()
            .unwrap()
            .contains("kegari-v0 setup --help")
    );
    let mut human = Vec::new();
    write_output(&response, false, None, &mut human).unwrap();
    let human = String::from_utf8(human).unwrap();
    assert!(human.starts_with("Target: local\nRuntime: none (client-only)\n"));
    assert!(!human.contains("registered local"));
    assert!(human.contains("Result: target_unavailable"));
    assert!(human.contains("Next action: Run `kegari-v0 setup --help`"));
    let mut encoded = Vec::new();
    write_output(&response, true, None, &mut encoded).unwrap();
    let encoded: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(encoded["error"]["code"], "target_unavailable");
    assert_eq!(encoded["error"]["next_action"], json!(error.next_action));
    assert_eq!(inputs.reads, 0);
    assert_eq!(local.bootstraps, 0);
    assert_eq!(local.services, 0);
    assert!(local.browsers.is_empty());
    assert!(
        std::fs::read_dir(kegari_cli::local_cli::state_root(&account))
            .unwrap()
            .next()
            .is_none()
    );
}
#[tokio::test]
async fn setup_bootstraps_only_when_explicitly_local_and_unregistered() {
    let temp = TempDir::new().unwrap();
    let store = TargetStore::new(
        PrivateDir::create(&temp.path().canonicalize().unwrap().join("client")).unwrap(),
    );
    let make = |target| PreparedInvocation {
        request_id: Uuid::new_v4(),
        target,
        json: true,
        action: Action::Setup {
            configuration: "already-validated fixture input".into(),
            noninteractive: true,
        },
        secret: None,
    };
    let mut local = LocalFixture::default();
    let response = execute(make(None), &store, &mut local).await;
    assert!(response.ok);
    assert_eq!(local.bootstraps, 1);
    assert_eq!(response.data["agents_started"], 0);
    let mut local = LocalFixture {
        registered: true,
        ..Default::default()
    };
    let response = execute(make(None), &store, &mut local).await;
    assert!(!response.ok);
    assert_eq!(local.bootstraps, 0);
    let mut local = LocalFixture::default();
    let response = execute(make(Some("missing-remote".into())), &store, &mut local).await;
    assert!(!response.ok);
    assert_eq!(local.bootstraps, 0);
    assert_eq!(local.services, 0);
}
#[cfg(unix)]
#[tokio::test]
async fn open_runs_only_client_browser_and_url_only_does_not_launch() {
    let fixture = fixture(2, |_, request| {
        assert_eq!(request["command"], "open");
        assert!(request["args"].get("browser").is_none());
        Some((
            200,
            reply(
                &request,
                json!({"url":"https://linear.app/example/issue/ABC-1","evidence_token":"current"}),
            ),
            vec![],
        ))
    })
    .await;
    let temp = TempDir::new().unwrap();
    let store = TargetStore::new(
        PrivateDir::create(&temp.path().canonicalize().unwrap().join("client")).unwrap(),
    );
    store
        .register_local(fixture.client.target().clone())
        .unwrap();
    let mut local = LocalFixture {
        registered: true,
        ..Default::default()
    };
    let response = execute(prepare(&["open", "ABC-1"]), &store, &mut local).await;
    assert!(response.ok);
    assert_eq!(local.browsers.len(), 1);
    let response = execute(
        prepare(&["open", "ABC-1", "--url-only"]),
        &store,
        &mut local,
    )
    .await;
    assert!(response.ok);
    assert_eq!(local.browsers.len(), 1);
    fixture.task.await.unwrap();
    assert!(validate_linear_url("javascript:alert(1)").is_err());
    assert!(validate_linear_url("https://attacker.example/issue/ABC-1").is_err());
}
#[cfg(unix)]
#[tokio::test]
async fn expired_snapshot_restarts_the_entire_read_without_splicing() {
    let mut snapshots = Snapshots::default();
    let now = chrono::Utc::now();
    let data = json!((0..101).collect::<Vec<_>>());
    let first = snapshots
        .capture("actor", runtime(), &json!({}), &data, now)
        .unwrap();
    let fixture = fixture(3, move |index, request| {
        if index == 0 {
            return Some((
                200,
                reply(&request, serde_json::to_value(&first).unwrap()),
                vec![],
            ));
        }
        if index == 1 {
            let mut response = reply(&request, Value::Null);
            response["ok"] = json!(false);
            response["error"] =
                serde_json::to_value(Error::new("snapshot_expired", "fixture expiry")).unwrap();
            return Some((409, response, vec![]));
        }
        assert!(request["args"].get("snapshot_id").is_none());
        assert!(request["args"].get("cursor").is_none());
        let mut fresh = Snapshots::default();
        let page = fresh
            .capture(
                "actor",
                runtime(),
                &json!({}),
                &json!({"fresh_complete":true}),
                chrono::Utc::now(),
            )
            .unwrap();
        Some((
            200,
            reply(&request, serde_json::to_value(page).unwrap()),
            vec![],
        ))
    })
    .await;
    let response = fixture
        .client
        .read_complete(&request(CommandName::Overview, json!({})))
        .await;
    assert!(response.ok);
    assert_eq!(response.data, json!({"fresh_complete":true}));
    fixture.task.await.unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn wait_timeout_observes_only_and_never_forces_or_replays_mutation() {
    let fixture = fixture(1, |_, request| {
        assert_eq!(request["command"], "operation_show");
        let mut snapshots = Snapshots::default();
        let page = snapshots
            .capture(
                "actor",
                runtime(),
                &json!({}),
                &json!({"still_draining":true}),
                chrono::Utc::now(),
            )
            .unwrap();
        let mut response = reply(&request, serde_json::to_value(page).unwrap());
        response["state"] = json!("draining");
        response["operation_id"] = request["args"]["operation"].clone();
        Some((200, response, vec![]))
    })
    .await;
    let request_id = Uuid::new_v4();
    let operation = Uuid::new_v4();
    let response = kegari_v0::api::Envelope {
        api_version: 1,
        request_id,
        runtime_id: runtime(),
        config_revision: None,
        observed_at: chrono::Utc::now(),
        ok: true,
        operation_id: Some(operation),
        state: Some("draining".into()),
        data: json!({}),
        error: None,
    };
    // A zero client wait observes nothing and cannot terminate work.
    let immediate = fixture.client.wait_operation(response.clone(), 0).await;
    assert!(immediate.ok);
    assert_eq!(immediate.data["wait_timeout"], true);
    // One observation is enough to see a transport closure; the original IDs remain.
    let result = fixture.client.wait_operation(response, 1).await;
    assert_eq!(result.request_id, request_id);
    assert_eq!(result.operation_id, Some(operation));
    fixture.task.await.unwrap();
    assert_eq!(fixture.calls.lock().unwrap().len(), 1);
}
#[cfg(unix)]
#[tokio::test]
async fn wait_rejects_completion_for_missing_or_different_operation() {
    for returned in [None, Some(Uuid::new_v4())] {
        let fixture = fixture(1, move |_, request| {
            let mut snapshots = Snapshots::default();
            let page = snapshots
                .capture(
                    "actor",
                    runtime(),
                    &json!({}),
                    &json!({"completed":true}),
                    chrono::Utc::now(),
                )
                .unwrap();
            let mut response = reply(&request, serde_json::to_value(page).unwrap());
            response["state"] = json!("completed");
            response["operation_id"] = json!(returned);
            Some((200, response, vec![]))
        })
        .await;
        let original = Uuid::new_v4();
        let operation = Uuid::new_v4();
        let response = kegari_v0::api::Envelope {
            api_version: 1,
            request_id: original,
            runtime_id: runtime(),
            config_revision: None,
            observed_at: chrono::Utc::now(),
            ok: true,
            operation_id: Some(operation),
            state: Some("running".into()),
            data: json!({}),
            error: None,
        };
        let result = fixture.client.wait_operation(response, 1).await;
        assert!(!result.ok);
        assert_eq!(result.error.unwrap().code, "identity_mismatch");
        assert_eq!(result.request_id, original);
        assert_eq!(result.operation_id, Some(operation));
        fixture.task.await.unwrap();
    }
}
#[cfg(unix)]
#[tokio::test]
async fn reflected_secret_is_omitted_from_response_not_partially_redacted() {
    let fixture = fixture(1, |_, request| {
        Some((
            200,
            reply(&request, json!({"bad_echo":"fixture-private-value"})),
            vec![],
        ))
    })
    .await;
    let request = request(
        CommandName::ConnectionAdd,
        json!({"name":"github","connection":"fixture input"}),
    );
    let secret = Secret::new(b"fixture-private-value".to_vec()).unwrap();
    let response = fixture.client.send(&request, Some(&secret)).await;
    assert!(!response.ok);
    assert!(response.data.is_null());
    assert!(
        !serde_json::to_string(&response)
            .unwrap()
            .contains("fixture-private-value")
    );
    assert_eq!(response.error.unwrap().code, "secret_disclosure");
    fixture.task.await.unwrap();
}
#[tokio::test]
async fn https_never_sends_plaintext_control_or_credential_body() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut connection, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        let length = connection.read(&mut bytes).await.unwrap();
        assert_ne!(bytes[0], b'P');
        assert!(!String::from_utf8_lossy(&bytes[..length]).contains("fixture-private-value"));
        connection
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    let client = ControlClient::new(Target {
        name: "tls-only".into(),
        runtime_id: runtime(),
        api_version: 1,
        endpoint: Endpoint::Https {
            origin: format!("https://{address}"),
        },
    })
    .unwrap();
    let response = client
        .send(
            &request(
                CommandName::ConnectionAdd,
                json!({"name":"github","connection":"fixture input"}),
            ),
            Some(&Secret::new(b"fixture-private-value".to_vec()).unwrap()),
        )
        .await;
    assert!(!response.ok);
    assert_eq!(response.state.as_deref(), Some("uncertain"));
    server.await.unwrap();
}

// Public deterministic test-only TLS material; not a live credential.
const CLI_TLS_LEAF: &str = r#"-----BEGIN CERTIFICATE-----
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

// Public deterministic test-only TLS material; not a live credential.
const CLI_TLS_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
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

#[tokio::test]
async fn https_rejects_untrusted_fixture_certificate_without_sending_control_body() {
    let certificates = rustls_pemfile::certs(&mut CLI_TLS_LEAF.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let key = rustls_pemfile::private_key(&mut CLI_TLS_KEY.as_bytes())
        .unwrap()
        .unwrap();
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(certificates, key)
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        assert!(
            acceptor.accept(stream).await.is_err(),
            "untrusted certificate must fail before any control request"
        );
    });
    let client = ControlClient::new(Target {
        name: "untrusted".into(),
        runtime_id: runtime(),
        api_version: 1,
        endpoint: Endpoint::Https {
            origin: format!("https://{address}"),
        },
    })
    .unwrap();
    let result = client
        .send(&request(CommandName::Status, json!({})), None)
        .await;
    assert!(!result.ok);
    assert_eq!(result.error.unwrap().code, "target_unavailable");
    server.await.unwrap();
}
