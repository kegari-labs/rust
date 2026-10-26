//! Owned local subprocess fixtures, not native host-identity or sandbox qualification.
use super::*;
use crate::{
    control::Lease,
    platform::{BinaryIdentity, ProcessIdentity, ProcessObservation, ProcessProbe},
    process_group::{OwnedGroup, Termination},
    process_records::{ProcessJournal, RecoveryState, SpawnBinding, SpawnGuard},
    storage::PrivateDir,
};
use std::{
    os::{fd::OwnedFd, unix::net::UnixStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

struct MissingChildProbe {
    host: ProcessIdentity,
    snapshots: AtomicUsize,
    snapshot_delay: Duration,
}
impl ProcessProbe for MissingChildProbe {
    fn current(&self) -> Result<ProcessIdentity> {
        Ok(self.host.clone())
    }
    fn observe(&self, _: &ProcessIdentity) -> Result<ProcessObservation> {
        Err(Error::invalid(
            "fixture never manufactures a running child identity",
        ))
    }
    fn matching_executable(&self, _: &Path) -> Result<Vec<ProcessIdentity>> {
        self.snapshots.fetch_add(1, Ordering::SeqCst);
        // A single deliberately delayed snapshot models expiry after actual spawn.
        // It never retries native observation or invents child identity.
        std::thread::sleep(self.snapshot_delay);
        Ok(Vec::new())
    }
}
struct JournalFixture {
    _root: tempfile::TempDir,
    journal: Arc<ProcessJournal>,
    probe: Arc<MissingChildProbe>,
    binding: SpawnBinding,
}
impl JournalFixture {
    fn new() -> Self {
        Self::with_snapshot_delay(Duration::ZERO)
    }
    fn with_snapshot_delay(snapshot_delay: Duration) -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap();
        let runtime = Uuid::new_v4();
        // Synthetic host metadata and deliberately empty child inventory model one
        // missed snapshot. Terminal evidence below comes only from actual Child ownership.
        let probe = Arc::new(MissingChildProbe {
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
            snapshots: AtomicUsize::new(0),
            snapshot_delay,
        });
        let journal = Arc::new(
            ProcessJournal::with_probe(
                PrivateDir::create(&path.join("processes")).unwrap(),
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
            work_key: bytes_digest(b"owned startup fixture"),
            scope_digest: bytes_digest(b"owned startup fixture scope"),
            cwd: path,
            executable: BinaryIdentity::inspect(Path::new("/bin/sh"), false).unwrap(),
            observed_executables: Vec::new(),
            argv_digest: bytes_digest(b"private release pipe fixture"),
            kind: "fixture_only".into(),
        };
        Self {
            _root: root,
            journal,
            probe,
            binding,
        }
    }
    fn assert_closed_without_running_identity(&self) {
        let rows = self.journal.observe(self.binding.operation_id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].state,
            RecoveryState::GroupExitRecordedTreeUnverified
        );
        assert!(rows[0].process.is_none());
        assert!(!rows[0].all_descendants_exited);
        assert_eq!(self.probe.snapshots.load(Ordering::SeqCst), 1);
    }
}

struct GatedChild {
    child: Option<std::process::Child>,
    group: OwnedGroup,
    guard: Option<SpawnGuard>,
    release: UnixStream,
}
impl GatedChild {
    fn start(fixture: &JournalFixture) -> Self {
        let (release, child_input) = UnixStream::pair().unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "IFS= read -r release; test \"$release\" = released"])
            .env_clear()
            .current_dir(&fixture.binding.cwd)
            .stdin(Stdio::from(OwnedFd::from(child_input)))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let guard = fixture.journal.begin(fixture.binding.clone()).unwrap();
        guard.configure_std(&mut command).unwrap();
        let (child, group) = OwnedGroup::spawn_std(&mut command).unwrap();
        let mut owned = Self {
            child: Some(child),
            group,
            guard: Some(guard),
            release,
        };
        assert_eq!(
            owned
                .guard
                .as_mut()
                .unwrap()
                .spawned_std(owned.child.as_ref().unwrap())
                .unwrap_err()
                .code,
            "operation_uncertain"
        );
        assert!(!owned.group.observe_exit_before_reap().unwrap());
        owned
    }
    fn reap(&mut self) -> std::process::ExitStatus {
        let status = self.child.as_mut().unwrap().try_wait().unwrap().unwrap();
        self.guard.as_mut().unwrap().group_exited(status).unwrap();
        self.guard.take();
        self.child.take();
        status
    }
    fn release_and_close(&mut self) {
        self.release.write_all(b"released\n").unwrap();
        assert!(
            self.group
                .await_exit_before_reap(Instant::now() + Duration::from_secs(2), || Ok(()))
                .unwrap()
        );
        assert!(self.reap().success());
    }
}
impl Drop for GatedChild {
    fn drop(&mut self) {
        // Keep actual ownership during assertion unwinding, without signalling a
        // stored PID or claiming closure if the actual group cannot be observed.
        if let Some(child) = self.child.as_mut() {
            let _ = self.release.write_all(b"released\n");
            let _ = self.group.terminate(Termination::Kill);
            if self
                .group
                .await_exit_before_reap(Instant::now() + Duration::from_secs(2), || Ok(()))
                .unwrap_or(false)
                && let Ok(Some(status)) = child.try_wait()
                && let Some(guard) = self.guard.as_mut()
            {
                let _ = guard.group_exited(status);
            }
        }
    }
}

#[test]
fn missed_identity_then_owned_terminal_transition_records_no_running_identity() {
    let fixture = JournalFixture::new();
    let mut owned = GatedChild::start(&fixture);
    let mut checks = 0;
    let mut release = &owned.release;
    assert!(
        owned
            .group
            .await_exit_before_reap(Instant::now() + Duration::from_secs(2), || {
                checks += 1;
                // The first actual group observation is deterministically nonterminal:
                // its release pipe is still open and receives nothing until this check.
                if checks == 2 {
                    release.write_all(b"released\n").unwrap();
                }
                Ok(())
            })
            .unwrap()
    );
    assert!(checks >= 3);
    assert!(owned.reap().success());
    fixture.assert_closed_without_running_identity();
}

#[test]
fn startup_transition_timeout_keeps_original_child_and_starting_guard() {
    let fixture = JournalFixture::new();
    let mut owned = GatedChild::start(&fixture);
    assert!(
        !owned
            .group
            .await_exit_before_reap(Instant::now() + Duration::from_millis(5), || Ok(()))
            .unwrap()
    );
    assert!(!owned.group.observe_exit_before_reap().unwrap());
    let rows = fixture
        .journal
        .observe(fixture.binding.operation_id)
        .unwrap();
    assert_eq!(rows[0].state, RecoveryState::GuardHeld);
    assert!(rows[0].process.is_none());
    assert_eq!(fixture.probe.snapshots.load(Ordering::SeqCst), 1);
    owned.release_and_close();
    fixture.assert_closed_without_running_identity();
}

#[test]
fn startup_transition_current_refusal_keeps_actual_owner_for_cleanup() {
    let fixture = JournalFixture::new();
    let mut owned = GatedChild::start(&fixture);
    let error = owned
        .group
        .await_exit_before_reap(Instant::now() + Duration::from_secs(2), || {
            Err(Error::new(
                "cancelled",
                "fixture cancellation after actual spawn",
            ))
        })
        .unwrap_err();
    assert_eq!(error.code, "cancelled");
    assert!(!owned.group.observe_exit_before_reap().unwrap());
    assert_eq!(
        fixture
            .journal
            .observe(fixture.binding.operation_id)
            .unwrap()[0]
            .state,
        RecoveryState::GuardHeld
    );
    owned.release_and_close();
    fixture.assert_closed_without_running_identity();
}

#[test]
fn git_identity_uncertainty_never_delivers_input_and_preserves_original_failure() {
    let fixture = JournalFixture::new();
    let state = GitOperationState::new(GitLimits::default(), None).unwrap();
    *state.provided_guard.lock().unwrap() =
        Some(fixture.journal.begin(fixture.binding.clone()).unwrap());
    let marker = fixture.binding.cwd.join("input-was-delivered");
    let mut command = Command::new("/bin/sh");
    command
        .args([
            "-c",
            "IFS= read -r line && printf '%s' \"$line\" > input-was-delivered",
        ])
        .env_clear()
        .current_dir(&fixture.binding.cwd);
    let error = state
        .run(command, Some(b"must remain withheld\n"))
        .unwrap_err();
    assert_eq!(error.code, "operation_uncertain");
    assert_eq!(
        error.message,
        "owned child OS identity cannot be established; retain starting guard"
    );
    assert!(!marker.exists());
    assert!(state.active.lock().unwrap().is_none());
    fixture.assert_closed_without_running_identity();
}

#[test]
fn expired_git_command_refuses_before_start_gate_or_spawn() {
    let fixture = JournalFixture::new();
    let gate_calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = gate_calls.clone();
    let state = GitOperationState::new(
        GitLimits {
            command_timeout: Duration::from_nanos(1),
            ..GitLimits::default()
        },
        Some(Arc::new(move |start| {
            observed_calls.fetch_add(1, Ordering::SeqCst);
            start()
        })),
    )
    .unwrap();
    *state.provided_guard.lock().unwrap() =
        Some(fixture.journal.begin(fixture.binding.clone()).unwrap());
    let marker = fixture.binding.cwd.join("forbidden-spawn");
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "printf spawned > forbidden-spawn"])
        .env_clear()
        .current_dir(&fixture.binding.cwd);
    assert_eq!(state.run(command, None).unwrap_err().code, "timeout");
    assert_eq!(gate_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.probe.snapshots.load(Ordering::SeqCst), 0);
    assert!(!marker.exists());
    assert!(state.active.lock().unwrap().is_none());
    assert!(state.provided_guard.lock().unwrap().is_some());
    let rows = fixture
        .journal
        .observe(fixture.binding.operation_id)
        .unwrap();
    assert_eq!(rows[0].state, RecoveryState::GuardHeld);
    assert!(rows[0].process.is_none());
}

#[test]
fn git_start_gate_consuming_command_budget_refuses_without_spawn() {
    let fixture = JournalFixture::new();
    let state = GitOperationState::new(
        GitLimits {
            command_timeout: Duration::from_millis(10),
            ..GitLimits::default()
        },
        Some(Arc::new(|start| {
            std::thread::sleep(Duration::from_millis(20));
            start()
        })),
    )
    .unwrap();
    *state.provided_guard.lock().unwrap() =
        Some(fixture.journal.begin(fixture.binding.clone()).unwrap());
    let marker = fixture.binding.cwd.join("forbidden-spawn");
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "printf spawned > forbidden-spawn"])
        .env_clear()
        .current_dir(&fixture.binding.cwd);
    assert_eq!(state.run(command, None).unwrap_err().code, "timeout");
    assert_eq!(fixture.probe.snapshots.load(Ordering::SeqCst), 0);
    assert!(!marker.exists());
    assert!(state.active.lock().unwrap().is_none());
    assert!(state.provided_guard.lock().unwrap().is_some());
}

#[test]
fn git_deadline_crossed_after_actual_spawn_still_withholds_input_and_closes_owner() {
    // The delay is inside the one post-spawn snapshot. Before that call, the
    // genuine Child already exists and remains blocked on its untouched stdin.
    let fixture = JournalFixture::with_snapshot_delay(Duration::from_millis(1100));
    let state = GitOperationState::new(
        GitLimits {
            command_timeout: Duration::from_secs(1),
            ..GitLimits::default()
        },
        None,
    )
    .unwrap();
    *state.provided_guard.lock().unwrap() =
        Some(fixture.journal.begin(fixture.binding.clone()).unwrap());
    let marker = fixture.binding.cwd.join("input-was-delivered");
    let mut command = Command::new("/bin/sh");
    command
        .args([
            "-c",
            "IFS= read -r line && printf '%s' \"$line\" > input-was-delivered",
        ])
        .env_clear()
        .current_dir(&fixture.binding.cwd);
    assert_eq!(
        state
            .run(command, Some(b"must remain withheld\n"))
            .unwrap_err()
            .code,
        "timeout"
    );
    assert!(!marker.exists());
    assert!(state.active.lock().unwrap().is_none());
    fixture.assert_closed_without_running_identity();
}
