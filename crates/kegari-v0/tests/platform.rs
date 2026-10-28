#![cfg(target_os = "macos")]
use kegari_config::Config;
use kegari_platform::*;
use kegari_storage::PrivateDir;
use kegari_v0::{
    control::{Actor, Lifecycle, MutationRequest},
    platform::*,
};
use serde_json::json;
use std::{collections::BTreeMap, path::Path, sync::Arc};
use uuid::Uuid;

fn host_fixture() -> (tempfile::TempDir, HostIdentity) {
    let temporary = tempfile::tempdir().unwrap();
    let host = HostIdentity {
        host_id: "fixture-host".into(),
        boot_id: Uuid::from_u128(1),
        uid: unsafe { libc::geteuid() },
        user: "fixture-user".into(),
        home: temporary.path().canonicalize().unwrap(),
    };
    (temporary, host)
}
struct FixtureProbe;
impl ProcessProbe for FixtureProbe {
    fn current(&self) -> kegari_types::error::Result<ProcessIdentity> {
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
    fn observe(
        &self,
        process: &ProcessIdentity,
    ) -> kegari_types::error::Result<ProcessObservation> {
        Ok(if *process == self.current()? {
            ProcessObservation::Running
        } else {
            ProcessObservation::DifferentIdentity
        })
    }
    fn matching_executable(&self, _: &Path) -> kegari_types::error::Result<Vec<ProcessIdentity>> {
        Ok(vec![self.current()?])
    }
}
fn bootstrap(
    host: &HostIdentity,
    runtime: Uuid,
    binary: BinaryIdentity,
    request: Uuid,
) -> kegari_types::error::Result<(Registration, kegari_v0::control::ControlHost)> {
    Registration::bootstrap_with_probe(host, runtime, binary, request, Arc::new(FixtureProbe))
}
fn recover(
    host: &HostIdentity,
    binary: &BinaryIdentity,
) -> kegari_types::error::Result<(Registration, kegari_v0::control::ControlHost, Config)> {
    Registration::recover_with_probe(host, binary, Arc::new(FixtureProbe))
}
fn binary() -> BinaryIdentity {
    BinaryIdentity::inspect(&std::env::current_exe().unwrap(), false).unwrap()
}
fn config(host: &HostIdentity, runtime: Uuid) -> Config {
    serde_json::from_value(json!({"schema_version":1,"runtime":{"id":runtime,"host_id":host.host_id,"max_agents":1},"connections":{},"workspaces":{},"control":{"local_users":{host.uid.to_string():{"id":"fixture-operator","linear_user_id":Uuid::new_v4(),"roles":["inspect","configure","lifecycle"]}}}})).unwrap()
}
fn start(control: &kegari_v0::control::ControlHost, runtime: Uuid) {
    control
        .lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: runtime,
                actor: Actor {
                    identity: "fixture-operator".into(),
                    principal: None,
                    roles: ["lifecycle".into()].into(),
                },
                kind: "start".into(),
                canonical_targets: vec![runtime.to_string()],
                expected: BTreeMap::new(),
                payload: json!({}),
            },
            Lifecycle::Start,
            0,
        )
        .unwrap();
}

#[test]
fn setup_stays_stopped_and_registration_is_exclusive_and_revision_bound() {
    let (_temporary, host) = host_fixture();
    let runtime = Uuid::new_v4();
    let binary = binary();
    let (registration, control) =
        bootstrap(&host, runtime, binary.clone(), Uuid::new_v4()).unwrap();
    assert_eq!(
        control.snapshot().unwrap().intent.desired,
        kegari_v0::control::Intent::Stopped
    );
    assert!(!registration.snapshot().unwrap().setup_complete);
    assert!(bootstrap(&host, Uuid::new_v4(), binary.clone(), Uuid::new_v4()).is_err());
    registration
        .complete_setup(&config(&host, runtime))
        .unwrap();
    drop(control);
    drop(registration);
    let (registration, control, config) = recover(&host, &binary).unwrap();
    assert_eq!(config.runtime.id, runtime);
    assert!(!control.snapshot().unwrap().admission_open);
    drop(control);
    drop(registration);
    let registration = PrivateDir::open(&host.state_root()).unwrap();
    registration
        .write_bytes("config.toml", b"invalid = true")
        .unwrap();
    assert!(recover(&host, &binary).is_err());
}

#[tokio::test]
async fn control_socket_recovery_requires_owned_dead_socket_and_preserves_other_entries() {
    use std::os::unix::fs::PermissionsExt;
    let (_temporary, host) = host_fixture();
    let (registration, _control) =
        bootstrap(&host, Uuid::new_v4(), binary(), Uuid::new_v4()).unwrap();
    let socket = registration.root().path().join("control.sock");
    kegari_v0::host::reconcile_control_socket(&registration)
        .await
        .unwrap();
    std::fs::write(&socket, b"operator data").unwrap();
    assert_eq!(
        kegari_v0::host::reconcile_control_socket(&registration)
            .await
            .unwrap_err()
            .code,
        "unsafe_storage"
    );
    assert_eq!(std::fs::read(&socket).unwrap(), b"operator data");
    std::fs::remove_file(&socket).unwrap();
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        kegari_v0::host::reconcile_control_socket(&registration)
            .await
            .unwrap_err()
            .code,
        "ownership_conflict"
    );
    assert!(socket.exists());
    drop(listener);
    kegari_v0::host::reconcile_control_socket(&registration)
        .await
        .unwrap();
    assert!(!socket.exists());
    std::os::unix::fs::symlink(registration.root().path().join("owner.json"), &socket).unwrap();
    assert_eq!(
        kegari_v0::host::reconcile_control_socket(&registration)
            .await
            .unwrap_err()
            .code,
        "unsafe_storage"
    );
    assert!(
        std::fs::symlink_metadata(&socket)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn runtime_lock_requires_running_and_exact_reserved_owner() {
    struct ChildProbe;
    impl ProcessProbe for ChildProbe {
        fn current(&self) -> kegari_types::error::Result<ProcessIdentity> {
            let mut p = FixtureProbe.current()?;
            p.parent_pid = p.pid;
            p.pid += 1;
            p.process_group = p.pid;
            Ok(p)
        }
        fn observe(&self, p: &ProcessIdentity) -> kegari_types::error::Result<ProcessObservation> {
            FixtureProbe.observe(p)
        }
        fn matching_executable(
            &self,
            p: &Path,
        ) -> kegari_types::error::Result<Vec<ProcessIdentity>> {
            FixtureProbe.matching_executable(p)
        }
    }
    let (_temporary, host) = host_fixture();
    let runtime = Uuid::new_v4();
    let (registration, control) = bootstrap(&host, runtime, binary(), Uuid::new_v4()).unwrap();
    registration
        .complete_setup(&config(&host, runtime))
        .unwrap();
    let lease = registration.prepare_owner().unwrap();
    assert!(RuntimeLock::acquire(registration.root(), lease.clone()).is_err());
    start(&control, runtime);
    let mut forged = lease.clone();
    forged.owner_id = Uuid::new_v4();
    assert!(RuntimeLock::acquire(registration.root(), forged).is_err());
    let runtime_lock =
        RuntimeLock::acquire_with_probe(registration.root(), lease.clone(), &ChildProbe).unwrap();
    assert!(RuntimeLock::acquire(registration.root(), lease.clone()).is_err());
    assert!(registration.prepare_owner().is_err());
    assert!(
        registration
            .record_orchestration(lease, FixtureProbe.current().unwrap())
            .is_err()
    );
    drop(runtime_lock);
}

fn fixture_child() -> ProcessIdentity {
    let mut child = FixtureProbe.current().unwrap();
    child.parent_pid = child.pid;
    child.pid += 1;
    child.process_group = child.pid;
    child
}

/// The fixture observes a registered parent and its reserved child without
/// invoking native process discovery or launching a subprocess.
struct RunningChildProbe;
impl ProcessProbe for RunningChildProbe {
    fn current(&self) -> kegari_types::error::Result<ProcessIdentity> {
        FixtureProbe.current()
    }
    fn observe(
        &self,
        process: &ProcessIdentity,
    ) -> kegari_types::error::Result<ProcessObservation> {
        Ok(
            if *process == self.current()? || *process == fixture_child() {
                ProcessObservation::Running
            } else {
                ProcessObservation::DifferentIdentity
            },
        )
    }
    fn matching_executable(&self, _: &Path) -> kegari_types::error::Result<Vec<ProcessIdentity>> {
        Ok(vec![self.current()?, fixture_child()])
    }
}

#[test]
fn orchestration_authority_requires_actual_runtime_lock_contention() {
    let (_temporary, host) = host_fixture();
    let runtime = Uuid::new_v4();
    let (registration, control) = Registration::bootstrap_with_probe(
        &host,
        runtime,
        binary(),
        Uuid::new_v4(),
        Arc::new(RunningChildProbe),
    )
    .unwrap();
    registration
        .complete_setup(&config(&host, runtime))
        .unwrap();
    let lease = registration.prepare_owner().unwrap();
    start(&control, runtime);
    let before = serde_json::to_value(registration.snapshot().unwrap()).unwrap();
    let error = registration
        .record_orchestration(lease.clone(), fixture_child())
        .unwrap_err();
    assert_eq!(
        error.message,
        "orchestration handshake has no held runtime lock"
    );
    assert_eq!(
        serde_json::to_value(registration.snapshot().unwrap()).unwrap(),
        before
    );

    let lock = registration.root().lock("runtime.lock").unwrap();
    registration
        .record_orchestration(lease.clone(), fixture_child())
        .unwrap();
    registration.verify_active_owner(&lease).unwrap();
    let mut wrong_lease = lease.clone();
    wrong_lease.owner_id = Uuid::new_v4();
    assert!(registration.verify_active_owner(&wrong_lease).is_err());
    drop(lock);
    let error = registration.verify_active_owner(&lease).unwrap_err();
    assert_eq!(
        error.message,
        "orchestration process no longer holds runtime.lock"
    );
}

#[test]
fn replaced_directory_cannot_prove_orchestration_lock_ownership() {
    for attached in [false, true] {
        let (_temporary, host) = host_fixture();
        let runtime = Uuid::new_v4();
        let (registration, control) = Registration::bootstrap_with_probe(
            &host,
            runtime,
            binary(),
            Uuid::new_v4(),
            Arc::new(RunningChildProbe),
        )
        .unwrap();
        registration
            .complete_setup(&config(&host, runtime))
            .unwrap();
        let lease = registration.prepare_owner().unwrap();
        start(&control, runtime);
        let _lock = registration.root().lock("runtime.lock").unwrap();
        if attached {
            registration
                .record_orchestration(lease.clone(), fixture_child())
                .unwrap();
            registration.verify_active_owner(&lease).unwrap();
        }
        let before = serde_json::to_value(registration.snapshot().unwrap()).unwrap();
        let original_record = registration
            .root()
            .read_bytes("owner.json", 256 * 1024)
            .unwrap();
        let original = host.home.join("original-control-directory");
        std::fs::rename(registration.root().path(), &original).unwrap();
        let replacement = PrivateDir::create(&host.state_root()).unwrap();

        let error = if attached {
            registration.verify_active_owner(&lease).unwrap_err()
        } else {
            registration
                .record_orchestration(lease, fixture_child())
                .unwrap_err()
        };
        assert_eq!(error.code, "ownership_conflict");
        assert_eq!(error.message, "control directory was replaced");
        assert_eq!(
            serde_json::to_value(registration.snapshot().unwrap()).unwrap(),
            before
        );
        assert_eq!(
            PrivateDir::open(&original)
                .unwrap()
                .read_bytes("owner.json", 256 * 1024)
                .unwrap(),
            original_record
        );
        assert!(!replacement.path().join("owner.json").exists());
        assert!(!replacement.path().join("runtime.lock").exists());
    }
}

#[test]
fn partial_setup_and_corrupt_recovery_preserve_all_records() {
    let (_temporary, host) = host_fixture();
    let runtime = Uuid::new_v4();
    let binary = binary();
    let (registration, control) =
        bootstrap(&host, runtime, binary.clone(), Uuid::new_v4()).unwrap();
    drop(control);
    let before = registration
        .root()
        .read_bytes("owner.json", 256 * 1024)
        .unwrap();
    assert_eq!(
        recover(&host, &binary).err().unwrap().code,
        "ownership_conflict"
    );
    assert_eq!(
        registration
            .root()
            .read_bytes("owner.json", 256 * 1024)
            .unwrap(),
        before
    );
    registration
        .complete_setup(&config(&host, runtime))
        .unwrap();
    let before = registration
        .root()
        .read_bytes("owner.json", 256 * 1024)
        .unwrap();
    registration
        .root()
        .write_bytes("intent.json", b"corrupt")
        .unwrap();
    drop(registration);
    let root = PrivateDir::open(&host.state_root()).unwrap();
    assert!(recover(&host, &binary).is_err());
    assert_eq!(root.read_bytes("owner.json", 256 * 1024).unwrap(), before);
}

#[test]
fn pending_startup_recovery_requires_no_live_process_and_no_runtime_lock() {
    struct Inventory(std::sync::Mutex<Vec<ProcessIdentity>>);
    impl ProcessProbe for Inventory {
        fn current(&self) -> kegari_types::error::Result<ProcessIdentity> {
            FixtureProbe.current()
        }
        fn observe(
            &self,
            process: &ProcessIdentity,
        ) -> kegari_types::error::Result<ProcessObservation> {
            FixtureProbe.observe(process)
        }
        fn matching_executable(
            &self,
            _: &Path,
        ) -> kegari_types::error::Result<Vec<ProcessIdentity>> {
            let mut result = self.0.lock().unwrap().clone();
            result.push(self.current()?);
            Ok(result)
        }
    }
    let (_temporary, host) = host_fixture();
    let runtime = Uuid::new_v4();
    let binary = binary();
    let probe = Arc::new(Inventory(Default::default()));
    let (registration, control) = Registration::bootstrap_with_probe(
        &host,
        runtime,
        binary.clone(),
        Uuid::new_v4(),
        probe.clone(),
    )
    .unwrap();
    registration
        .complete_setup(&config(&host, runtime))
        .unwrap();
    let pending = registration.prepare_owner().unwrap();
    drop(registration);
    drop(control);
    let (registration, _control, _) =
        Registration::recover_with_probe(&host, &binary, probe.clone()).unwrap();
    assert!(registration.prepare_owner().is_err());
    let lock = registration.root().lock("runtime.lock").unwrap();
    assert!(registration.reconcile_pending_start(&pending).is_err());
    drop(lock);
    let mut child = FixtureProbe.current().unwrap();
    child.pid += 1;
    probe.0.lock().unwrap().push(child);
    assert!(registration.reconcile_pending_start(&pending).is_err());
    assert!(registration.snapshot().unwrap().pending_owner.is_some());
    probe.0.lock().unwrap().clear();
    registration.reconcile_pending_start(&pending).unwrap();
    assert!(registration.snapshot().unwrap().pending_owner.is_none());
    assert!(registration.prepare_owner().unwrap().owner_generation > pending.owner_generation);
}

#[test]
fn service_plan_requires_immutable_binary_and_does_not_install_or_start() {
    use std::os::unix::fs::PermissionsExt;
    let (_temporary, host) = host_fixture();
    let root = PrivateDir::create(&host.home.join("installed")).unwrap();
    let path = root.path().join("kegari-v0");
    root.write_bytes("kegari-v0", b"fixture binary bytes")
        .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let binary = BinaryIdentity::inspect(&path, false).unwrap();
    assert!(service_plan(&host, binary).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500)).unwrap();
    let binary = BinaryIdentity::inspect(&path, true).unwrap();
    let plan = service_plan(&host, binary).unwrap();
    assert!(plan.plist.contains("__control-host"));
    assert!(!plan.plist.contains("resume"));
    assert!(!plan.plist_path.exists());
}
