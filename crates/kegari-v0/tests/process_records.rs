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
        lease: Lease {
            runtime_id: runtime,
            owner_id: Uuid::new_v4(),
            owner_generation: 1,
        },
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
    assert_eq!(guard.spawned_std(&child).unwrap(), identity);
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
        guard.spawned_std(&child).unwrap_err().code,
        "operation_uncertain"
    );
    group.terminate(Termination::Kill).unwrap();
    let status = reap(&mut child, &mut group);
    guard.group_exited(status).unwrap();
    let observed = journal.observe(binding.operation_id).unwrap();
    assert_eq!(
        observed[0].state,
        RecoveryState::GroupExitRecordedTreeUnverified
    );
    assert!(observed[0].process.is_none());
    assert!(!observed[0].all_descendants_exited);
}
