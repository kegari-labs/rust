use kegari_cli::{
    cli::GuidanceRequest,
    client::{Endpoint, Target},
    local_cli::*,
};
use kegari_v0::{
    config::{Config, ProviderKind},
    error::Result,
    platform::*,
    storage::PrivateDir,
};
use serde_json::json;
use std::{
    io::Cursor,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;

fn fixture() -> (tempfile::TempDir, HostIdentity, AccountHome) {
    let temporary = tempfile::tempdir().unwrap();
    let home = temporary.path().canonicalize().unwrap();
    let account = AccountHome {
        uid: unsafe { libc::geteuid() },
        user: "fixture".into(),
        home: home.clone(),
    };
    let host = HostIdentity {
        host_id: "fixture-host".into(),
        boot_id: Uuid::from_u128(1),
        uid: account.uid,
        user: account.user.clone(),
        home,
    };
    (temporary, host, account)
}
struct Probe {
    alive: AtomicBool,
}
impl Probe {
    fn new() -> Self {
        Self {
            alive: AtomicBool::new(true),
        }
    }
}
impl ProcessProbe for Probe {
    fn current(&self) -> Result<ProcessIdentity> {
        Ok(ProcessIdentity {
            pid: std::process::id(),
            parent_pid: 1,
            process_group: 1,
            uid: unsafe { libc::geteuid() },
            boot_id: Uuid::from_u128(1),
            started_seconds: 1,
            started_microseconds: 1,
            executable: std::env::current_exe().unwrap(),
        })
    }
    fn observe(&self, _: &ProcessIdentity) -> Result<ProcessObservation> {
        Ok(if self.alive.load(Ordering::SeqCst) {
            ProcessObservation::Running
        } else {
            ProcessObservation::Gone
        })
    }
    fn matching_executable(&self, _: &Path) -> Result<Vec<ProcessIdentity>> {
        Ok(vec![])
    }
}
fn config(host: &HostIdentity, runtime: Uuid) -> Config {
    serde_json::from_value(json!({"schema_version":1,"runtime":{"id":runtime,"host_id":host.host_id,"max_agents":1},"connections":{},"workspaces":{},"control":{"local_users":{host.uid.to_string():{"id":"operator","linear_user_id":Uuid::new_v4(),"roles":["inspect","configure","lifecycle"]}}}})).unwrap()
}
fn registration(
    host: &HostIdentity,
    probe: Arc<Probe>,
) -> (Registration, kegari_v0::control::ControlHost, Config) {
    let id = Uuid::new_v4();
    let binary = BinaryIdentity::inspect(&std::env::current_exe().unwrap(), false).unwrap();
    let (registration, control) =
        Registration::bootstrap_with_probe(host, id, binary, Uuid::new_v4(), probe).unwrap();
    let config = config(host, id);
    registration.complete_setup(&config).unwrap();
    (registration, control, config)
}
fn fake_binary(host: &HostIdentity, info: serde_json::Value) -> std::path::PathBuf {
    let path = host.home.join("fixture-binary");
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = \"__build-info\" ] || exit 91\nprintf '%s\\n' '{}'\n",
        serde_json::to_string(&info).unwrap()
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o555)).unwrap();
    path
}
#[test]
fn remote_client_preferences_create_no_control_state_and_ignore_partial_local_setup() {
    let (_temp, _host, account) = fixture();
    let targets = target_store(&account).unwrap();
    let target = Target {
        name: "studio".into(),
        runtime_id: Uuid::new_v4(),
        api_version: 1,
        endpoint: Endpoint::Https {
            origin: "https://fixture.invalid".into(),
        },
    };
    targets.add(target.clone()).unwrap();
    let root = PrivateDir::open(&state_root(&account)).unwrap();
    root.write_bytes("config.toml", b"partial = true").unwrap();
    assert_eq!(targets.select(Some("studio")).unwrap(), target);
    assert_eq!(
        local_registration(&account).unwrap_err().code,
        "setup_incomplete"
    );
    assert!(root.read_bytes("owner.json", 100).is_err());
    assert!(root.read_bytes("intent.json", 100).is_err());
    let text = String::from_utf8(root.read_bytes("clients.toml", 4096).unwrap()).unwrap();
    assert!(text.contains("studio"));
    assert!(!root.path().join("targets.json").exists());
}
#[test]
fn local_registration_pins_only_complete_owner_and_preserves_conflicting_pin() {
    let (_temp, host, account) = fixture();
    let targets = target_store(&account).unwrap();
    assert!(registered_target(&account, &targets).unwrap().is_none());
    let probe = Arc::new(Probe::new());
    let (registration, _control, config) = registration(&host, probe);
    let target = registered_target(&account, &targets).unwrap().unwrap();
    assert_eq!(target.runtime_id, config.runtime.id);
    assert_eq!(
        target.endpoint,
        Endpoint::Unix {
            socket: host.state_root().join("control.sock"),
            authority: "kegari.local".into()
        }
    );
    let mut changed = registration.snapshot().unwrap();
    changed.runtime_id = Uuid::new_v4();
    registration.root().write("owner.json", &changed).unwrap();
    assert_eq!(
        registered_target(&account, &targets).unwrap_err().code,
        "identity_mismatch"
    );
    assert_eq!(targets.select(None).unwrap(), target);
}
#[test]
fn shutdown_marker_blocks_recovery_and_explicit_start_requires_gone_owner_and_lock() {
    let (_temp, host, _account) = fixture();
    let probe = Arc::new(Probe::new());
    let (registration, control, _config) = registration(&host, probe.clone());
    let before = registration.root().read_bytes("intent.json", 4096).unwrap();
    registration.reserve_service_shutdown().unwrap();
    assert_eq!(
        Registration::prepare_service_start(
            &host,
            probe.as_ref(),
            &registration.snapshot().unwrap().binary
        )
        .unwrap_err()
        .code,
        "stop_in_progress"
    );
    probe.alive.store(false, Ordering::SeqCst);
    assert_eq!(
        Registration::prepare_service_start(
            &host,
            probe.as_ref(),
            &registration.snapshot().unwrap().binary
        )
        .unwrap_err()
        .code,
        "ownership_conflict"
    );
    let binary = registration.snapshot().unwrap().binary;
    drop(control);
    drop(registration);
    assert_eq!(
        Registration::recover_with_probe(&host, &binary, probe.clone())
            .err()
            .unwrap()
            .code,
        "service_stopped"
    );
    assert!(matches!(
        Registration::prepare_service_start(&host, probe.as_ref(), &binary).unwrap(),
        ServiceStartState::Ready(_)
    ));
    let root = PrivateDir::open(&host.state_root()).unwrap();
    assert!(
        !root
            .read::<ServiceShutdown>("service-shutdown.json")
            .unwrap()
            .stopped
    );
    assert_eq!(root.read_bytes("intent.json", 4096).unwrap(), before);
    assert!(Registration::recover_with_probe(&host, &binary, probe).is_ok());
}
#[test]
fn service_shutdown_cannot_hide_pending_orchestration_ownership() {
    let (_temp, host, _account) = fixture();
    let (registration, _control, _) = registration(&host, Arc::new(Probe::new()));
    registration.prepare_owner().unwrap();
    assert_eq!(
        registration.reserve_service_shutdown().unwrap_err().code,
        "ownership_conflict"
    );
    assert!(
        registration
            .root()
            .read::<ServiceShutdown>("service-shutdown.json")
            .is_err()
    );
}
#[tokio::test]
async fn installation_in_owned_fixture_is_idempotent_preserves_intent_and_verifies_build() {
    let (_temp, host, _account) = fixture();
    let path = fake_binary(&host, serde_json::to_value(build_info()).unwrap());
    let root = PrivateDir::create(&host.state_root()).unwrap();
    root.write_bytes("intent.json", b"retained intent fixture")
        .unwrap();
    let before = root.read_bytes("intent.json", 4096).unwrap();
    let installed = install_service(&host, &path).await.unwrap();
    assert_eq!(
        install_service(&host, &path).await.unwrap().plan.digest,
        installed.plan.digest
    );
    assert_eq!(
        installed_service(&host).unwrap().plan.digest,
        installed.plan.digest
    );
    assert_eq!(root.read_bytes("intent.json", 4096).unwrap(), before);
    assert!(root.read::<OwnerRecord>("owner.json").is_err());
    assert!(installed.plan.plist.contains("__control-host"));
    std::fs::set_permissions(
        &installed.plan.plist_path,
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(installed_service(&host).unwrap_err().code, "unsafe_storage");
}
#[tokio::test]
async fn installation_rejects_another_build_and_existing_unrelated_service_file() {
    let (_temp, host, _) = fixture();
    let mut info = serde_json::to_value(build_info()).unwrap();
    info["runtime_build"] = json!("another-build");
    let path = fake_binary(&host, info);
    assert_eq!(
        install_service(&host, &path).await.unwrap_err().code,
        "handover_required"
    );
    assert!(!host.state_root().exists());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = fake_binary(&host, serde_json::to_value(build_info()).unwrap());
    let plan = service_plan(&host, BinaryIdentity::inspect(&path, true).unwrap()).unwrap();
    std::fs::create_dir_all(plan.plist_path.parent().unwrap()).unwrap();
    std::fs::write(&plan.plist_path, b"unrelated").unwrap();
    std::fs::set_permissions(&plan.plist_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        install_service(&host, &path).await.unwrap_err().code,
        "ownership_conflict"
    );
    assert_eq!(std::fs::read(&plan.plist_path).unwrap(), b"unrelated");
}
#[test]
fn guided_toml_uses_exact_typed_schema_and_never_echoes_document() {
    let request = GuidanceRequest::ConnectionAdd {
        name: "agent".into(),
        kind: ProviderKind::Codex,
    };
    let input = "kind = 'codex'\naccount_id = 'PRIVATE-ACCOUNT'\nauth_home = '/fixture/auth'\nmax_agents = 1\n.\n";
    let mut output = Vec::new();
    let document = guided_document(&request, Cursor::new(input), &mut output).unwrap();
    assert!(document.contains("PRIVATE-ACCOUNT"));
    assert!(
        !String::from_utf8(output)
            .unwrap()
            .contains("PRIVATE-ACCOUNT")
    );
    assert!(
        guided_document(
            &request,
            Cursor::new(input.replace("codex", "github")),
            Vec::new()
        )
        .is_err()
    );
    assert!(
        guided_document(
            &request,
            Cursor::new(input.trim_end_matches(".\n")),
            Vec::new()
        )
        .is_err()
    );
    assert!(
        guided_document(
            &request,
            Cursor::new(format!("{input}unknown = 1\n")),
            Vec::new()
        )
        .is_ok()
    ); // bytes after terminator are never submitted
}
#[test]
fn guided_configuration_bound_and_supplied_error_request_identity_are_preserved() {
    let huge = format!("{}\n.\n", "x".repeat(kegari_cli::cli::MAX_CONFIG + 1));
    assert_eq!(
        guided_document(&GuidanceRequest::Setup, Cursor::new(huge), Vec::new())
            .unwrap_err()
            .code,
        "invalid_input"
    );
    let id = Uuid::new_v4();
    let arguments = [
        "kegari-v0",
        "--json",
        "--target",
        "studio",
        "correct",
        "A-1",
        "--request-id",
        &id.to_string(),
        "--bad",
        "PRIVATE",
    ]
    .map(Into::into);
    let (json, target, request) = output_hints(&arguments);
    assert!(json);
    assert_eq!(target.as_deref(), Some("studio"));
    assert_eq!(request, id);
}

#[test]
fn initial_setup_collects_fields_and_reviews_exact_empty_scope_before_submission() {
    let (_temporary, host, _) = fixture();
    let linear = Uuid::from_u128(42);
    let input = format!("fixture-operator\n{linear}\n2\n\n\ncreate\n");
    let mut output = Vec::new();
    let document = guided_initial_setup(&host, Cursor::new(&input), &mut output).unwrap();
    let config = Config::parse(&document).unwrap();
    assert!(!config.runtime.id.is_nil());
    assert_eq!(config.runtime.host_id, host.host_id);
    assert_eq!(config.runtime.max_agents, 2);
    assert_eq!(
        config.runtime.execution_root,
        host.home.join(".kegari/runs").to_str().unwrap()
    );
    assert!(config.connections.is_empty());
    assert!(config.workspaces.is_empty());
    assert!(config.control.https.is_none());
    assert!(config.control.tailscale_users.is_empty());
    assert_eq!(config.control.local_users.len(), 1);
    let operator = &config.control.local_users[&host.uid.to_string()];
    assert_eq!(operator.id, "fixture-operator");
    assert_eq!(operator.linear_user_id, linear);
    assert_eq!(operator.roles.len(), 7);
    assert!(
        operator
            .roles
            .contains(&kegari_v0::config::ControlRole::Configure)
    );
    assert!(
        !operator
            .roles
            .contains(&kegari_v0::config::ControlRole::Evaluate)
    );
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains(&document));
    assert!(output.contains("not yet provider-verified"));
    assert!(output.contains("a macOS Keychain attestation key, with STOPPED intent"));
    assert!(output.contains("connection add"));
    assert!(output.contains("workspace add"));
    assert!(output.find(&document).unwrap() < output.find("Type create").unwrap());
    assert!(std::fs::read_dir(&host.home).unwrap().next().is_none());
    let second = guided_initial_setup(&host, Cursor::new(&input), Vec::new()).unwrap();
    assert_ne!(
        config.runtime.id,
        Config::parse(&second).unwrap().runtime.id
    );
}

#[test]
fn initial_setup_requires_real_values_and_explicit_review_confirmation() {
    let (_temporary, host, _) = fixture();
    let linear = Uuid::from_u128(42);
    let complete =
        format!("operator\n{linear}\n3\n~/private-runs\ninspect,configure,evaluate\ncreate\n");
    let document = guided_initial_setup(&host, Cursor::new(&complete), Vec::new()).unwrap();
    let config = Config::parse(&document).unwrap();
    assert_eq!(
        config.runtime.execution_root,
        host.home.join("private-runs").to_str().unwrap()
    );
    assert_eq!(
        config.control.local_users[&host.uid.to_string()]
            .roles
            .len(),
        3
    );
    for input in [
        complete.replace(&linear.to_string(), "not-a-uuid"),
        complete.replace(&linear.to_string(), &Uuid::nil().to_string()),
        complete.replace("\n3\n", "\n0\n"),
        complete.replace("\n3\n", "\n\n"),
        complete.replace("inspect,configure,evaluate", "inspect"),
        complete.replace("inspect,configure,evaluate", "configure,configure"),
        complete.replace("inspect,configure,evaluate", "configure,all"),
        complete.replace("~/private-runs", "$UNREAD_SETUP_ENV"),
        complete.replace("operator\n", "operator\u{1b}[0m\n"),
    ] {
        assert!(guided_initial_setup(&host, Cursor::new(input), Vec::new()).is_err());
    }
    for answers in 0..6 {
        let input = complete
            .lines()
            .take(answers)
            .collect::<Vec<_>>()
            .join("\n");
        let error = guided_initial_setup(&host, Cursor::new(input), Vec::new()).unwrap_err();
        assert_eq!(error.code, "cancelled");
    }
    for confirmation in ["/cancel", "yes", ""] {
        let input = complete.replace("create\n", &format!("{confirmation}\n"));
        assert_eq!(
            guided_initial_setup(&host, Cursor::new(input), Vec::new())
                .unwrap_err()
                .code,
            "cancelled"
        );
    }
    assert_eq!(
        guided_initial_setup(
            &host,
            Cursor::new(complete.trim_end_matches('\n')),
            Vec::new()
        )
        .unwrap_err()
        .code,
        "cancelled"
    );
    assert!(std::fs::read_dir(&host.home).unwrap().next().is_none());
}

#[test]
fn initial_setup_bounds_input_and_requires_successful_review_output() {
    let (_temporary, host, _) = fixture();
    assert_eq!(
        guided_initial_setup(
            &host,
            Cursor::new(format!("{}\n", "x".repeat(4097))),
            Vec::new()
        )
        .unwrap_err()
        .code,
        "invalid_input"
    );
    struct BrokenReview;
    impl std::io::Write for BrokenReview {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes
                .windows(b"Review the complete".len())
                .any(|chunk| chunk == b"Review the complete")
            {
                return Err(std::io::Error::other("fixture review output unavailable"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let answers = format!("operator\n{}\n2\n\n\ncreate\n", Uuid::from_u128(42));
    let mut input = Cursor::new(answers.as_bytes());
    assert_eq!(
        guided_initial_setup(&host, &mut input, BrokenReview)
            .unwrap_err()
            .code,
        "io_error"
    );
    assert_eq!(
        &answers.as_bytes()[input.position() as usize..],
        b"create\n"
    );
    assert!(std::fs::read_dir(&host.home).unwrap().next().is_none());
}

#[test]
fn service_binary_mismatch_never_clears_shutdown_marker() {
    let (_temp, host, _account) = fixture();
    let probe = Arc::new(Probe::new());
    let (registration, _control, _) = registration(&host, probe.clone());
    registration.reserve_service_shutdown().unwrap();
    probe.alive.store(false, Ordering::SeqCst);
    let mut binary = registration.snapshot().unwrap().binary;
    binary.digest = "different".into();
    assert_eq!(
        Registration::prepare_service_start(&host, probe.as_ref(), &binary)
            .unwrap_err()
            .code,
        "handover_required"
    );
    assert!(
        registration
            .root()
            .read::<ServiceShutdown>("service-shutdown.json")
            .unwrap()
            .stopped
    );
}
#[tokio::test]
async fn replaced_plist_fifo_is_rejected_without_waiting_for_writer() {
    let (_temp, host, _) = fixture();
    let path = fake_binary(&host, serde_json::to_value(build_info()).unwrap());
    let installed = install_service(&host, &path).await.unwrap();
    std::fs::remove_file(&installed.plan.plist_path).unwrap();
    use std::os::unix::ffi::OsStrExt;
    let fifo = std::ffi::CString::new(installed.plan.plist_path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let observed = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::task::spawn_blocking(move || installed_service(&host)),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(observed.unwrap_err().code, "unsafe_storage");
}
