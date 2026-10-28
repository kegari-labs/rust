#![cfg(target_os = "macos")]
use kegari_platform::*;
use std::path::Path;
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

#[test]
#[ignore = "requires separately authorized native host identity qualification outside the sandbox"]
fn native_host_and_birth_identity_are_observed_without_environment_trust() {
    let host = HostIdentity::discover().unwrap();
    assert!(host.home.is_absolute());
    assert_eq!(host.state_root(), host.home.join(".kegari"));
    assert!(!host.boot_id.is_nil());
    let mut process = ProcessIdentity::current().unwrap();
    assert_eq!(process.pid, std::process::id());
    assert_eq!(process.uid, host.uid);
    assert_eq!(process.observe().unwrap(), ProcessObservation::Running);
    process.started_microseconds += 1;
    assert_eq!(
        process.observe().unwrap(),
        ProcessObservation::DifferentIdentity
    );
    assert!(free_bytes(&host.home).unwrap() > 0);
}

#[test]
#[ignore = "requires separately authorized native owned-child capture and signalling"]
fn child_signal_uses_owned_unreaped_handle_and_never_adopts_pid_metadata() {
    let (_temporary, host) = host_fixture();
    let binary = BinaryIdentity::inspect(Path::new("/bin/sleep"), false).unwrap();
    // The fixture itself expires after five seconds even if native signalling
    // or the test process fails. Its outer qualification wrapper is separately
    // bounded; a failed observation must not leave a long-lived sleep behind.
    let mut child = OwnedChild::spawn(&binary, &["5".into()], &host.home).unwrap();
    assert_ne!(child.identity().pid, std::process::id());
    assert!(child.signal_group(true).unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while child.try_wait().unwrap().is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "owned child did not exit"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!child.signal_group(true).unwrap());
}
