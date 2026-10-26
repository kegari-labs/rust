use kegari_v0::{
    control::Lease,
    digest::bytes_digest,
    error::Result,
    platform::{BinaryIdentity, ProcessIdentity, ProcessObservation, ProcessProbe},
    process_group::{OwnedGroup, Termination},
    process_records::{ProcessJournal, RecoveryState, SpawnBinding},
    storage::PrivateDir,
};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;
struct Probe {
    host: ProcessIdentity,
    children: Mutex<Vec<ProcessIdentity>>,
    gone: AtomicBool,
}
impl ProcessProbe for Probe {
    fn current(&self) -> Result<ProcessIdentity> {
        Ok(self.host.clone())
    }
    fn observe(&self, p: &ProcessIdentity) -> Result<ProcessObservation> {
        Ok(if self.gone.load(Ordering::SeqCst) {
            ProcessObservation::Gone
        } else if self.children.lock().unwrap().contains(p) {
            ProcessObservation::Running
        } else {
            ProcessObservation::DifferentIdentity
        })
    }
    fn matching_executable(&self, path: &Path) -> Result<Vec<ProcessIdentity>> {
        Ok(self
            .children
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.executable == path)
            .cloned()
            .collect())
    }
}
fn setup() -> (
    tempfile::TempDir,
    Arc<ProcessJournal>,
    Arc<Probe>,
    SpawnBinding,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let runtime = Uuid::new_v4();
    let probe = Arc::new(Probe {
        host: ProcessIdentity {
            pid: std::process::id(),
            parent_pid: 1,
            process_group: std::process::id(),
            uid: unsafe { libc::geteuid() },
            boot_id: Uuid::new_v4(),
            started_seconds: 1,
            started_microseconds: 1,
            executable: "/fixture/host".into(),
        },
        children: Mutex::new(vec![]),
        gone: AtomicBool::new(false),
    });
    let journal = Arc::new(
        ProcessJournal::with_probe(
            PrivateDir::create(&root.join("processes")).unwrap(),
            runtime,
            probe.clone(),
        )
        .unwrap(),
    );
    let binding = SpawnBinding {
        operation_id: Uuid::new_v4(),
        effect_id: Uuid::new_v4(),
        lease: Some(Lease {
            runtime_id: runtime,
            owner_id: Uuid::new_v4(),
            owner_generation: 1,
        }),
        control_owner: None,
        work_key: bytes_digest(b"canonical work"),
        scope_digest: bytes_digest(b"exact scope"),
        cwd: root,
        executable: BinaryIdentity::inspect(Path::new("/bin/sleep"), false).unwrap(),
        observed_executables: vec![],
        argv_digest: bytes_digest(b"fixture sleep"),
        kind: "fixture_only".into(),
    };
    (temp, journal, probe, binding)
}
fn reap(child: &mut std::process::Child, group: &mut OwnedGroup) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if group.observe_exit_before_reap().unwrap() {
            return child.try_wait().unwrap().unwrap();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("owned fixture failed to exit within bound")
}
fn reap_evidence(
    child: &mut std::process::Child,
    group: &mut OwnedGroup,
) -> kegari_v0::process_group::OwnedGroupExit {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(exit) = group.try_reap_std(child).unwrap() {
            return exit;
        }
        assert!(Instant::now() < deadline, "owned fixture did not exit");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn foreign_exit_cannot_retire_another_spawn_and_original_evidence_survives_write_failure() {
    let (_temp, journal, _probe, binding) = setup();
    let mut guard = journal.begin(binding.clone()).unwrap();
    let mut command = std::process::Command::new("/bin/sleep");
    command.arg("0.02");
    guard.configure_std(&mut command).unwrap();
    let (mut child, mut group) = OwnedGroup::spawn_std(&mut command).unwrap();
    assert!(guard.spawned_std(&child, &group).is_err());
    let mut other_command = std::process::Command::new("/bin/sleep");
    other_command.arg("0.02");
    let (mut other_child, mut other_group) = OwnedGroup::spawn_std(&mut other_command).unwrap();
    let foreign = reap_evidence(&mut other_child, &mut other_group);
    let original = reap_evidence(&mut child, &mut group);
    let directory = binding
        .cwd
        .join("processes")
        .join(binding.operation_id.to_string());
    let path = directory.join(format!("{}.json", binding.effect_id));
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        guard.group_exited(&foreign).unwrap_err().code,
        "ownership_conflict"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        journal.observe(binding.operation_id).unwrap()[0].state,
        RecoveryState::GuardHeld
    );

    // A real pre-write storage failure must retain the original proof and lock.
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(guard.group_exited(&original).is_err());
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, before).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        journal.observe(binding.operation_id).unwrap()[0].state,
        RecoveryState::GuardHeld
    );
    guard.group_exited(&original).unwrap();
    let rows = journal.observe(binding.operation_id).unwrap();
    assert!(rows[0].owned_group_exited);
    assert!(!rows[0].all_descendants_exited);
    // Retrying this same observation is harmless; it does not manufacture a wait.
    guard.group_exited(&original).unwrap();
    let recorded = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        guard.group_exited(&original).unwrap_err().code,
        "record_missing"
    );
    std::fs::write(&path, &recorded).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let records = PrivateDir::open(&directory).unwrap();
    let name = format!("{}.json", binding.effect_id);
    let mut replaced: kegari_v0::process_records::ProcessRecord = records.read(&name).unwrap();
    replaced.binding.kind = "substituted".into();
    records.write(&name, &replaced).unwrap();
    assert_eq!(
        guard.group_exited(&original).unwrap_err().code,
        "record_corrupt"
    );
    std::fs::write(&path, recorded).unwrap();
    guard.group_exited(&original).unwrap();
    assert!(group.try_reap_std(&mut child).is_err());
}

#[test]
fn legacy_group_rows_are_read_without_promoting_their_exit_provenance() {
    let (_temp, journal, _probe, binding) = setup();
    let mut guard = journal.begin(binding.clone()).unwrap();
    let mut command = std::process::Command::new("/bin/sleep");
    command.arg("0.02");
    guard.configure_std(&mut command).unwrap();
    let (mut child, mut group) = OwnedGroup::spawn_std(&mut command).unwrap();
    assert!(guard.spawned_std(&child, &group).is_err());
    let exit = reap_evidence(&mut child, &mut group);
    guard.group_exited(&exit).unwrap();
    let directory = PrivateDir::open(
        &binding
            .cwd
            .join("processes")
            .join(binding.operation_id.to_string()),
    )
    .unwrap();
    let name = format!("{}.json", binding.effect_id);
    let mut row: serde_json::Value = directory.read(&name).unwrap();
    row["schema_version"] = 1.into();
    row["state"].as_object_mut().unwrap().remove("owned_group");
    row["state"]
        .as_object_mut()
        .unwrap()
        .remove("guard_release_verified");
    let legacy: kegari_v0::process_records::ProcessRecord =
        serde_json::from_value(row.clone()).unwrap();
    assert_eq!(serde_json::to_value(legacy).unwrap(), row);
    directory.write(&name, &row).unwrap();
    let before = std::fs::read(directory.path().join(&name)).unwrap();
    let rows = journal.observe(binding.operation_id).unwrap();
    assert_eq!(
        rows[0].state,
        RecoveryState::GroupExitRecordedTreeUnverified
    );
    assert!(!rows[0].owned_group_exited);
    assert!(!rows[0].all_descendants_exited);
    assert_eq!(std::fs::read(directory.path().join(&name)).unwrap(), before);
    row["schema_version"] = 2.into();
    directory.write(&name, &row).unwrap();
    assert_eq!(
        journal.observe(binding.operation_id).unwrap_err().code,
        "record_corrupt"
    );
}

#[test]
fn observed_inherited_guard_holder_is_not_cleared_by_descriptor_release() {
    if let Some(release) = std::env::var_os("KEGARI_FINITE_GUARD_HOLDER") {
        use std::os::unix::ffi::OsStrExt;
        let release = std::ffi::CString::new(release.as_bytes()).unwrap();
        // A finite isolated helper holds only its inherited fixture descriptors.
        // The pipe ensures detachment precedes the original leader's exit.
        let mut ready = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(ready.as_mut_ptr()) }, 0);
        let child = unsafe { libc::fork() };
        assert!(child >= 0);
        if child == 0 {
            unsafe {
                libc::close(ready[0]);
                if libc::setsid() < 0 {
                    libc::_exit(10);
                }
                libc::write(ready[1], b"r".as_ptr().cast(), 1);
                libc::close(ready[1]);
                let delay = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 5_000_000,
                };
                // Parent releases only after asserting the held-guard state.
                // Finite fallback guarantees cleanup even if it panics.
                for _ in 0..1000 {
                    if libc::access(release.as_ptr(), libc::F_OK) == 0 {
                        break;
                    }
                    libc::nanosleep(&delay, std::ptr::null_mut());
                }
                libc::_exit(0);
            }
        }
        unsafe {
            libc::close(ready[1]);
        }
        let mut byte = [0u8];
        assert_eq!(
            unsafe { libc::read(ready[0], byte.as_mut_ptr().cast(), 1) },
            1
        );
        assert_eq!(byte, [b'r']);
        std::process::exit(0);
    }
    let (_temp, journal, _probe, mut binding) = setup();
    let release = binding.cwd.join("release-guard-holder");
    let executable = std::env::current_exe().unwrap();
    binding.executable = BinaryIdentity::inspect(&executable, false).unwrap();
    let mut guard = journal.begin(binding.clone()).unwrap();
    let mut command = std::process::Command::new(&executable);
    command
        .args([
            "--exact",
            "observed_inherited_guard_holder_is_not_cleared_by_descriptor_release",
        ])
        .env_clear()
        .env("KEGARI_FINITE_GUARD_HOLDER", &release)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    guard.configure_std(&mut command).unwrap();
    let (mut child, mut group) = OwnedGroup::spawn_std(&mut command).unwrap();
    assert!(guard.spawned_std(&child, &group).is_err());
    let exit = reap_evidence(&mut child, &mut group);
    assert!(exit.status().success());
    assert_eq!(
        guard.group_exited(&exit).unwrap_err().code,
        "operation_uncertain"
    );
    let observed = journal.observe(binding.operation_id).unwrap();
    assert_eq!(observed[0].state, RecoveryState::GuardHeld);
    assert!(!observed[0].owned_group_exited);
    std::fs::write(release, b"release").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let state = &journal.observe(binding.operation_id).unwrap()[0].state;
        if *state != RecoveryState::GuardHeld {
            assert_eq!(*state, RecoveryState::GroupExitGuardUnverified);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "finite guard holder failed to release"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        guard.group_exited(&exit).unwrap_err().code,
        "operation_uncertain"
    );
    drop(guard);
    let observed = journal.observe(binding.operation_id).unwrap();
    assert_eq!(observed[0].state, RecoveryState::GroupExitGuardUnverified);
    assert!(!observed[0].owned_group_exited);
    assert!(!observed[0].all_descendants_exited);
}
#[test]
fn incomplete_spawn_record_survives_without_manufacturing_exit_or_replay() {
    let (_temp, journal, _probe, binding) = setup();
    let guard = journal.begin(binding.clone()).unwrap();
    assert_eq!(
        journal.observe(binding.operation_id).unwrap()[0].state,
        RecoveryState::GuardHeld
    );
    drop(guard);
    let observed = journal.observe(binding.operation_id).unwrap();
    assert_eq!(observed[0].state, RecoveryState::StartingUnknown);
    assert!(!observed[0].all_descendants_exited);
    assert_eq!(
        journal.begin(binding).err().unwrap().code,
        "operation_uncertain"
    );
}
#[test]
fn inherited_guard_survives_parent_handle_loss_and_never_adopts_recovered_pid() {
    let (_temp, journal, probe, binding) = setup();
    let mut guard = journal.begin(binding.clone()).unwrap();
    let mut command = std::process::Command::new("/bin/sleep");
    command.arg("0.25");
    guard.configure_std(&mut command).unwrap();
    let (mut child, mut group) = OwnedGroup::spawn_std(&mut command).unwrap();
    let mut identity = probe.host.clone();
    identity.pid = child.id();
    identity.parent_pid = probe.host.pid;
    identity.process_group = child.id();
    identity.executable = "/bin/sleep".into();
    identity.started_seconds = 2;
    probe.children.lock().unwrap().push(identity.clone());
    assert_eq!(guard.spawned_std(&child, &group).unwrap(), identity);
    drop(guard);
    let observed = journal.observe(binding.operation_id).unwrap();
    assert_eq!(observed[0].state, RecoveryState::GuardHeld);
    reap(&mut child, &mut group);
    probe.gone.store(true, Ordering::SeqCst);
    let observed = journal.observe(binding.operation_id).unwrap();
    assert_eq!(observed[0].state, RecoveryState::LeaderGoneTreeUnverified);
    assert!(!observed[0].all_descendants_exited);
}
#[test]
fn quick_exit_can_record_actual_owned_group_cleanup_without_live_identity_guess() {
    let (_temp, journal, _probe, binding) = setup();
    let mut guard = journal.begin(binding.clone()).unwrap();
    let mut command = std::process::Command::new("/bin/sleep");
    command.arg("0.02");
    guard.configure_std(&mut command).unwrap();
    let (mut child, mut group) = OwnedGroup::spawn_std(&mut command).unwrap();
    assert_eq!(
        guard.spawned_std(&child, &group).unwrap_err().code,
        "operation_uncertain"
    );
    group.terminate(Termination::Kill).unwrap();
    let exit = reap_evidence(&mut child, &mut group);
    guard.group_exited(&exit).unwrap();
    let observed = journal.observe(binding.operation_id).unwrap();
    assert_eq!(observed[0].state, RecoveryState::OwnedGroupExitRecorded);
    assert!(observed[0].process.is_none());
    assert!(observed[0].owned_group_exited);
    assert!(!observed[0].all_descendants_exited);
}

#[test]
fn replaced_or_missing_guard_cannot_erase_retained_process_identity() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, journal, _probe, binding) = setup();
    let guard = journal.begin(binding.clone()).unwrap();
    let path = temp
        .path()
        .join("processes")
        .join(binding.operation_id.to_string())
        .join(format!("{}.guard", binding.effect_id));
    // Retain the old inode to prevent allocator reuse while replacing its name.
    std::fs::rename(&path, path.with_extension("old")).unwrap();
    std::fs::write(&path, b"").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::remove_file(path.with_extension("old")).unwrap();
    assert_eq!(
        journal.observe(binding.operation_id).unwrap_err().code,
        "unsafe_storage"
    );
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        journal.observe(binding.operation_id).unwrap_err().code,
        "record_missing"
    );
    assert!(!path.exists(), "recovery must not create a new guard");
    drop(guard);
}

#[test]
fn oversized_or_foreign_recovery_entries_fail_before_unbounded_record_parsing() {
    let (temp, journal, _probe, binding) = setup();
    drop(journal.begin(binding.clone()).unwrap());
    let directory = PrivateDir::open(
        &temp
            .path()
            .canonicalize()
            .unwrap()
            .join("processes")
            .join(binding.operation_id.to_string()),
    )
    .unwrap();
    directory
        .write(&format!("{}.json", binding.effect_id), &"x".repeat(65_537))
        .unwrap();
    assert_eq!(
        journal.observe(binding.operation_id).unwrap_err().code,
        "record_corrupt"
    );
    std::fs::write(directory.path().join("unexpected-entry"), b"unknown").unwrap();
    assert_eq!(
        journal.observe(binding.operation_id).unwrap_err().code,
        "record_corrupt"
    );
}

#[test]
fn concurrent_claims_cannot_rewrite_a_prior_effect_after_guard_release() {
    let (_temp, journal, _probe, binding) = setup();
    let barrier = Arc::new(std::sync::Barrier::new(16));
    let mut threads = Vec::new();
    for _ in 0..16 {
        let journal = journal.clone();
        let binding = binding.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            journal.begin(binding).map(drop)
        }));
    }
    let results: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        1,
        "{results:?}"
    );
    // Competing OS opens/locks may fail conservatively before duplicate lookup.
    // The observable invariant is one durable claimant, never an accepted replay.
    assert_eq!(journal.observe(binding.operation_id).unwrap().len(), 1);
    assert_eq!(
        journal.begin(binding).err().unwrap().code,
        "operation_uncertain"
    );
}
