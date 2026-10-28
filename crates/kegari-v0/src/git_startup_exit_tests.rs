//! Owned local subprocess fixtures, not native host-identity or sandbox qualification.
use super::*;
use crate::{
    control::Lease,
    platform::{BinaryIdentity, ProcessIdentity, ProcessObservation, ProcessProbe},
    process_group::{OwnedGroup, Termination},
    process_records::{ProcessJournal, RecoveryState, SpawnBinding, SpawnGuard},
};
use kegari_storage::PrivateDir;
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
            check_bundle: None,
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
        assert_eq!(rows[0].state, RecoveryState::OwnedGroupExitRecorded);
        assert!(rows[0].process.is_none());
        assert!(rows[0].owned_group_exited);
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
                .spawned_std(owned.child.as_ref().unwrap(), &owned.group)
                .unwrap_err()
                .code,
            "operation_uncertain"
        );
        assert!(!owned.group.observe_exit_before_reap().unwrap());
        owned
    }
    fn reap(&mut self) -> std::process::ExitStatus {
        let exit = self
            .group
            .try_reap_std(self.child.as_mut().unwrap())
            .unwrap()
            .unwrap();
        let status = exit.status();
        self.guard.as_mut().unwrap().group_exited(&exit).unwrap();
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
                && let Ok(Some(exit)) = self.group.try_reap_std(child)
                && let Some(guard) = self.guard.as_mut()
            {
                let _ = guard.group_exited(&exit);
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

#[test]
fn failed_git_output_is_drained_before_owned_cleanup_completes() {
    let state = Arc::new(
        GitOperationState::new(
            GitLimits {
                output_bytes: 1,
                ..GitLimits::default()
            },
            None,
        )
        .unwrap(),
    );
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "printf oversized"]).env_clear();
    assert_eq!(state.run(command, None).unwrap_err().code, "output_limit");
    assert!(state.active.lock().unwrap().is_none());
    assert!(!state.uncertain_streams.load(Ordering::SeqCst));
    // This unit invokes run directly, so supply only the outer thread's local
    // closed marker. No worker/native completion capability is constructed.
    state.closed.store(true, Ordering::SeqCst);
    assert!(GitCancellation { state }.exited().unwrap());
}

#[test]
fn failed_git_stream_uncertainty_survives_local_thread_close() {
    let state = Arc::new(GitOperationState::new(GitLimits::default(), None).unwrap());
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    writer.write_all(b"retained output").unwrap();
    {
        let mut closure = GitPipeClosure {
            uncertain: &state.uncertain_streams,
            closed: false,
        };
        closure.closed = drain_failed_git_output(&mut reader).unwrap();
        assert!(!closure.closed); // Real writer remains open; WouldBlock is not EOF.
    }
    assert_eq!(state.current().unwrap_err().code, "operation_uncertain");
    state.closed.store(true, Ordering::SeqCst);
    let cancellation = GitCancellation {
        state: state.clone(),
    };
    assert!(!cancellation.exited().unwrap());
    drop(writer);
    assert!(drain_failed_git_output(&mut reader).unwrap());
    assert!(!cancellation.exited().unwrap()); // No retroactive invented closure.

    struct ContinuousOutput(usize);
    impl std::io::Read for ContinuousOutput {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            self.0 += 1;
            bytes.fill(b'x');
            Ok(bytes.len())
        }
    }
    let mut continuous = ContinuousOutput(0);
    assert!(!drain_failed_git_output(&mut continuous).unwrap());
    assert_eq!(continuous.0, 8); // Cleanup yields to deadline/cancellation checks.
}

#[test]
fn git_exit_rechecks_stream_uncertainty_after_observing_thread_close() {
    let state = Arc::new(GitOperationState::new(GitLimits::default(), None).unwrap());
    let (initial_read, observed_initial_read) = std::sync::mpsc::sync_channel(1);
    let (resume, resumed) = std::sync::mpsc::sync_channel(1);
    *state.before_exit_lock.lock().unwrap() = Some(Box::new(move || {
        initial_read.send(()).unwrap();
        resumed.recv_timeout(Duration::from_secs(5)).unwrap();
    }));
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    writer.write_all(b"unresolved output").unwrap();
    let cancellation = GitCancellation {
        state: state.clone(),
    };
    std::thread::scope(|scope| {
        let active = state.active.lock().unwrap();
        let observer = scope.spawn(move || cancellation.exited());
        observed_initial_read
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        // The actual observer has already read false. The original pipe guard
        // now discovers WouldBlock, with a real writer still open, and latches
        // uncertainty before the component's outer-thread closed publication.
        {
            let mut closure = GitPipeClosure {
                uncertain: &state.uncertain_streams,
                closed: false,
            };
            closure.closed = drain_failed_git_output(&mut reader).unwrap();
            assert!(!closure.closed);
        }
        assert!(state.uncertain_streams.load(Ordering::SeqCst));
        // As in the neighboring stream test, this is only the local thread's
        // marker, never a child/group/native completion capability.
        state.closed.store(true, Ordering::SeqCst);
        resume.send(()).unwrap();
        drop(active);
        assert!(!observer.join().unwrap().unwrap());
    });
    drop(writer);
}

#[test]
fn git_journal_retry_retains_the_actual_owned_exit_token() {
    let fixture = JournalFixture::new();
    let mut guard = fixture.journal.begin(fixture.binding.clone()).unwrap();
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "exit 0"])
        .env_clear()
        .current_dir(&fixture.binding.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    guard.configure_std(&mut command).unwrap();
    let (child, mut group) = OwnedGroup::spawn_std(&mut command).unwrap();
    assert_eq!(
        guard.spawned_std(&child, &group).unwrap_err().code,
        "operation_uncertain"
    );
    assert!(
        group
            .await_exit_before_reap(Instant::now() + Duration::from_secs(2), || Ok(()))
            .unwrap()
    );
    let mut active = ActiveGitChild {
        child,
        group,
        terminal: None,
        guard: Some(guard),
    };
    let record = fixture
        .binding
        .cwd
        .join("processes")
        .join(fixture.binding.operation_id.to_string())
        .join(format!("{}.json", fixture.binding.effect_id));
    let backup = record.with_extension("saved");
    std::fs::rename(&record, &backup).unwrap();
    std::fs::create_dir(&record).unwrap();
    struct RestoreRecord {
        record: PathBuf,
        backup: PathBuf,
    }
    impl Drop for RestoreRecord {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir(&self.record);
            let _ = std::fs::rename(&self.backup, &self.record);
        }
    }
    let restore = RestoreRecord { record, backup };
    assert_eq!(active.observe().unwrap_err().code, "unsafe_storage");
    assert!(active.terminal.as_ref().unwrap().status().success());
    assert!(active.guard.is_some());
    drop(restore);
    assert!(active.observe().unwrap().unwrap().success());
    assert!(active.guard.is_none());
    assert!(active.observe().unwrap().unwrap().success());
    fixture.assert_closed_without_running_identity();
}

#[test]
fn guard_retirement_waits_for_unrelated_pre_exec_but_not_its_process_exit() {
    use std::io::Read;
    use std::os::{fd::AsRawFd, unix::process::CommandExt};
    struct ReapOnDrop(std::process::Child);
    impl Drop for ReapOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let fixture = JournalFixture::new();
    let mut original = GatedChild::start(&fixture);
    let (mut pre_parent, pre_child) = UnixStream::pair().unwrap();
    pre_parent
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    pre_parent
        .set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let (mut after_parent, after_child) = UnixStream::pair().unwrap();
    let (retiring, entered_retirement) = std::sync::mpsc::sync_channel(1);
    let (retired, retirement_result) = std::sync::mpsc::sync_channel(1);

    // This actual unrelated host child inherits the guard only until exec. Its
    // pre-exec rendezvous uses only async-signal-safe syscalls and has a finite
    // fallback. No post-fork Rust lock, allocation or process capability is used.
    let launcher = std::thread::spawn(move || {
        let fd = pre_child.as_raw_fd();
        let mut command = Command::new("/bin/bash");
        command
            .args([
                "--noprofile",
                "--norc",
                "-c",
                "IFS= read -r -t 10 release; test \"$release\" = done",
            ])
            .env_clear()
            .stdin(Stdio::from(OwnedFd::from(after_child)))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                if libc::write(fd, b"R".as_ptr().cast(), 1) != 1 {
                    libc::_exit(111);
                }
                let mut poll = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                if libc::poll(&mut poll, 1, 10_000) != 1 {
                    libc::_exit(112);
                }
                let mut byte = 0_u8;
                if libc::read(fd, (&mut byte as *mut u8).cast(), 1) != 1 || byte != b'G' {
                    libc::_exit(113);
                }
                Ok(())
            });
        }
        let (child, _) = OwnedGroup::spawn_std(&mut command).unwrap();
        drop(pre_child);
        ReapOnDrop(child)
    });
    let mut byte = [0];
    pre_parent.read_exact(&mut byte).unwrap();
    assert_eq!(byte, [b'R']);

    original.release.write_all(b"released\n").unwrap();
    assert!(
        original
            .group
            .await_exit_before_reap(Instant::now() + Duration::from_secs(2), || Ok(()))
            .unwrap()
    );
    let exit = original
        .group
        .try_reap_std(original.child.as_mut().unwrap())
        .unwrap()
        .unwrap();
    assert!(exit.status().success());
    original.child.take();
    let mut guard = original.guard.take().unwrap();
    let retirement = std::thread::spawn(move || {
        retiring.send(()).unwrap();
        let result = guard.group_exited(&exit);
        retired.send(result.clone()).unwrap();
        result
    });
    entered_retirement
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    assert!(matches!(
        retirement_result.recv_timeout(Duration::from_millis(50)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    pre_parent.write_all(b"G").unwrap();
    let mut unrelated = launcher.join().unwrap();
    retirement_result
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    retirement.join().unwrap().unwrap();
    assert!(unrelated.0.try_wait().unwrap().is_none());
    fixture.assert_closed_without_running_identity();
    after_parent.write_all(b"done\n").unwrap();
    assert!(unrelated.0.wait().unwrap().success());
}

#[test]
fn reused_guarded_command_does_not_inherit_retained_or_released_original_guard() {
    struct ReapOnDrop(std::process::Child);
    impl Drop for ReapOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut fixture = JournalFixture::new();
    fixture.binding.executable = BinaryIdentity::inspect(Path::new("/bin/bash"), false).unwrap();
    let (mut release, input) = UnixStream::pair().unwrap();
    let mut command = Command::new("/bin/bash");
    command
        .args([
            "--noprofile",
            "--norc",
            "-c",
            "IFS= read -r -t 10 release; test \"$release\" = done",
        ])
        .env_clear()
        .current_dir(&fixture.binding.cwd)
        .stdin(Stdio::from(OwnedFd::from(input)))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut guard = fixture.journal.begin(fixture.binding.clone()).unwrap();
    guard.configure_std(&mut command).unwrap();
    let (child, mut group) = OwnedGroup::spawn_std(&mut command).unwrap();
    let mut original = ReapOnDrop(child);
    // Actual child/group binding disarms inheritance before this deliberately
    // missing process metadata result. No native Running identity is invented.
    assert_eq!(
        guard.spawned_std(&original.0, &group).unwrap_err().code,
        "operation_uncertain"
    );
    release.write_all(b"done\n").unwrap();
    assert!(
        group
            .await_exit_before_reap(Instant::now() + Duration::from_secs(2), || Ok(()))
            .unwrap()
    );
    let exit = group.try_reap_std(&mut original.0).unwrap().unwrap();
    assert!(exit.status().success());

    // Reusing the configured command while the original guard is still held
    // must not extend that guard into this unrelated actual child. Without the
    // intended-hook disarm, group_exited below deterministically sees a holder.
    let (child, mut second_group) = OwnedGroup::spawn_std(&mut command).unwrap();
    let mut second = ReapOnDrop(child);
    assert!(second.0.try_wait().unwrap().is_none());
    guard.group_exited(&exit).unwrap();
    fixture.assert_closed_without_running_identity();
    drop(guard);
    release.write_all(b"done\n").unwrap();
    assert!(
        second_group
            .await_exit_before_reap(Instant::now() + Duration::from_secs(2), || Ok(()))
            .unwrap()
    );
    assert!(
        second_group
            .try_reap_std(&mut second.0)
            .unwrap()
            .unwrap()
            .status()
            .success()
    );

    // The same earlier intended hook is also inert after its descriptors close
    // (or are recycled). A fresh invocation cannot fail on its stale raw FD.
    let (child, mut third_group) = OwnedGroup::spawn_std(&mut command).unwrap();
    let mut third = ReapOnDrop(child);
    release.write_all(b"done\n").unwrap();
    assert!(
        third_group
            .await_exit_before_reap(Instant::now() + Duration::from_secs(2), || Ok(()))
            .unwrap()
    );
    assert!(
        third_group
            .try_reap_std(&mut third.0)
            .unwrap()
            .unwrap()
            .status()
            .success()
    );
}
