#![cfg(target_os = "macos")]
use kegari_platform::{
    BinaryIdentity, HostIdentity, ProcessIdentity, ProcessObservation, ProcessProbe,
};
use kegari_v0::platform::Registration;
use std::{path::Path, sync::Arc};
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
fn binary() -> BinaryIdentity {
    BinaryIdentity::inspect(&std::env::current_exe().unwrap(), false).unwrap()
}

#[tokio::test]
async fn control_socket_recovery_requires_owned_dead_socket_and_preserves_other_entries() {
    use std::os::unix::fs::PermissionsExt;
    let (_temporary, host) = host_fixture();
    let (registration, _control) =
        bootstrap(&host, Uuid::new_v4(), binary(), Uuid::new_v4()).unwrap();
    let socket = registration.root().path().join("control.sock");
    kegari_controller::host::reconcile_control_socket(&registration)
        .await
        .unwrap();
    std::fs::write(&socket, b"operator data").unwrap();
    assert_eq!(
        kegari_controller::host::reconcile_control_socket(&registration)
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
        kegari_controller::host::reconcile_control_socket(&registration)
            .await
            .unwrap_err()
            .code,
        "ownership_conflict"
    );
    assert!(socket.exists());
    drop(listener);
    kegari_controller::host::reconcile_control_socket(&registration)
        .await
        .unwrap();
    assert!(!socket.exists());
    std::os::unix::fs::symlink(registration.root().path().join("owner.json"), &socket).unwrap();
    assert_eq!(
        kegari_controller::host::reconcile_control_socket(&registration)
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
