//! Ordinary owned-child and pipe components, not Seatbelt qualification or an
//! authenticated worker admission. Probe metadata is synthetic; Child/group,
//! output reads, inherited guards and journal writes are real local objects.
use super::*;
use crate::{
    control::Lease,
    process_records::{ProcessJournal, RecoveryState, SpawnBinding},
};
use kegari_platform::{BinaryIdentity, ProcessIdentity, ProcessObservation, ProcessProbe};
use kegari_storage::PrivateDir;
use std::{
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::Instant,
};
use tokio::io::{AsyncRead, AsyncWriteExt, ReadBuf};
use uuid::Uuid;

struct FixtureProbe {
    host: ProcessIdentity,
    child: Mutex<Option<ProcessIdentity>>,
    retired: AtomicBool,
}
impl ProcessProbe for FixtureProbe {
    fn current(&self) -> Result<ProcessIdentity> {
        Ok(self.host.clone())
    }
    fn observe(&self, process: &ProcessIdentity) -> Result<ProcessObservation> {
        if process == &self.host {
            return Ok(ProcessObservation::Running);
        }
        if self.child.lock().unwrap().as_ref() == Some(process) {
            return Ok(if self.retired.load(Ordering::SeqCst) {
                ProcessObservation::Gone
            } else {
                ProcessObservation::Running
            });
        }
        Err(Error::invalid(
            "component probe received an unknown identity",
        ))
    }
    fn matching_executable(&self, path: &Path) -> Result<Vec<ProcessIdentity>> {
        Ok(self
            .child
            .lock()
            .unwrap()
            .iter()
            .filter(|child| child.executable == path)
            .cloned()
            .collect())
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    journal: Arc<ProcessJournal>,
    probe: Arc<FixtureProbe>,
    binding: SpawnBinding,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let runtime = Uuid::new_v4();
        let probe = Arc::new(FixtureProbe {
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
            child: Mutex::new(None),
            retired: AtomicBool::new(false),
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
            check_bundle: None,
            work_key: bytes_digest(b"owned command component"),
            scope_digest: bytes_digest(b"owned command component scope"),
            cwd: root.clone(),
            executable: BinaryIdentity::inspect(Path::new("/bin/sh"), false).unwrap(),
            observed_executables: vec![],
            argv_digest: bytes_digest(b"owned command component argv"),
            kind: "fixture_only".into(),
        };
        Self {
            _temp: temp,
            root,
            journal,
            probe,
            binding,
        }
    }
    fn spawn(
        &self,
        script: &str,
    ) -> (
        CommandProcess,
        tokio::process::ChildStdout,
        tokio::process::ChildStderr,
        tokio::process::ChildStdin,
    ) {
        let mut guard = self.journal.begin(self.binding.clone()).unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", script])
            .current_dir(&self.root)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        guard.configure_tokio(&mut command).unwrap();
        let (mut child, group) =
            kegari_process::group::OwnedGroup::spawn_tokio(&mut command).unwrap();
        let pid = child.id().unwrap();
        // Only metadata is injected. The opaque exit used by the supervisor is
        // produced later by this exact unreaped Child and OwnedGroup.
        *self.probe.child.lock().unwrap() = Some(ProcessIdentity {
            pid,
            parent_pid: self.probe.host.pid,
            process_group: pid,
            uid: self.probe.host.uid,
            boot_id: self.probe.host.boot_id,
            started_seconds: 2,
            started_microseconds: 1,
            executable: self.binding.executable.path.clone(),
        });
        guard.spawned_tokio(&child, &group).unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let stdin = child.stdin.take().unwrap();
        (
            CommandProcess::new(child, group, Some(guard), None),
            stdout,
            stderr,
            stdin,
        )
    }
    fn run(&self, timeout: Duration) -> CommandRun {
        CommandRun {
            argv: vec!["/bin/sh".into(), "component fixture".into()],
            cwd: self.root.clone(),
            isolation_digest: bytes_digest(b"component only, no sandbox qualification"),
            started: Instant::now(),
            timeout,
            secret_values: vec!["fixture-secret".into()],
        }
    }
    fn record_path(&self) -> PathBuf {
        self.root
            .join("processes")
            .join(self.binding.operation_id.to_string())
            .join(format!("{}.json", self.binding.effect_id))
    }
    fn assert_retired(&self, cancellation: &OwnedCommandCancellation) {
        {
            let process = cancellation.process.lock().unwrap();
            assert!(process.reaped.is_some());
            assert!(process.journal_retired);
        }
        // Model the reader's post-exit identity observation only after the real
        // original Child/group token and durable retirement exist. This is
        // synthetic cold-reader metadata, not native process qualification.
        self.probe.retired.store(true, Ordering::SeqCst);
        let rows = self.journal.observe(self.binding.operation_id).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, RecoveryState::OwnedGroupExitRecorded);
        assert!(rows[0].owned_group_exited);
        assert!(!rows[0].all_descendants_exited);
    }
}
async fn wait_closed(cancellation: &OwnedCommandCancellation) {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if cancellation.exited().unwrap() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .expect("owned component did not retire within its cleanup bound");
}

#[tokio::test]
async fn actual_group_journal_and_joined_outputs_return_nonzero_and_redacted_outcome() {
    let fixture = Fixture::new();
    let (process, stdout, stderr, stdin) =
        fixture.spawn("printf 'fixture-secret\\n'; printf 'diagnostic\\n' >&2; exit 7");
    drop(stdin);
    let command = OwnedCommand::supervise(
        fixture.run(Duration::from_secs(3)),
        process,
        stdout,
        stderr,
        1024,
    );
    let cancellation = command.cancellation();
    let outcome = tokio::time::timeout(Duration::from_secs(8), command.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.exit_code, Some(7));
    assert_eq!(outcome.stdout, "[REDACTED]\n");
    assert_eq!(outcome.stderr, "diagnostic\n");
    assert!(!outcome.timed_out);
    assert!(!outcome.output_truncated);
    assert!(cancellation.exited().unwrap());
    fixture.assert_retired(&cancellation);
}

#[tokio::test]
async fn clean_group_and_joined_streams_without_journal_never_release_run_ownership() {
    let mut command = Command::new("/usr/bin/true");
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let (mut child, group) = kegari_process::group::OwnedGroup::spawn_tokio(&mut command).unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let fixture = Fixture::new();
    let command = OwnedCommand::supervise(
        fixture.run(Duration::from_secs(3)),
        CommandProcess::new(child, group, None, None),
        stdout,
        stderr,
        128,
    );
    let cancellation = command.cancellation();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(8), command.wait())
            .await
            .unwrap()
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert!(cancellation.group_exited().unwrap());
    assert!(!cancellation.exited().unwrap());
}

#[tokio::test]
async fn command_timeout_and_dropped_wait_both_retain_bounded_owned_cleanup() {
    for dropped in [false, true] {
        let fixture = Fixture::new();
        let (process, stdout, stderr, stdin) = fixture.spawn("IFS= read -r input");
        let timeout = if dropped {
            Duration::from_secs(30)
        } else {
            Duration::from_millis(20)
        };
        let command = OwnedCommand::supervise(fixture.run(timeout), process, stdout, stderr, 128);
        let cancellation = command.cancellation();
        assert!(!cancellation.exited().unwrap());
        if dropped {
            // Drop the actual wait future after it owns the supervisor handle.
            let mut wait = Box::pin(command.wait());
            std::future::poll_fn(|context| {
                assert!(std::future::Future::poll(wait.as_mut(), context).is_pending());
                Poll::Ready(())
            })
            .await;
            drop(wait);
        } else {
            let outcome = tokio::time::timeout(Duration::from_secs(8), command.wait())
                .await
                .unwrap()
                .unwrap();
            assert!(outcome.timed_out);
            assert!(outcome.exit_code.is_none());
        }
        wait_closed(&cancellation).await;
        drop(stdin);
        fixture.assert_retired(&cancellation);
    }
}

struct FailedRead;
impl AsyncRead for FailedRead {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Err(std::io::Error::other("component read failure")))
    }
}
struct PendingRead(Arc<AtomicBool>);
impl AsyncRead for PendingRead {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Pending
    }
}
impl Drop for PendingRead {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
#[tokio::test]
async fn failed_collector_joins_aborted_sibling_and_latches_uncertainty() {
    let fixture = Fixture::new();
    let (process, stdout, stderr, stdin) = fixture.spawn("exit 0");
    drop((stdout, stderr, stdin));
    let sibling_dropped = Arc::new(AtomicBool::new(false));
    let command = OwnedCommand::supervise(
        fixture.run(Duration::from_secs(3)),
        process,
        FailedRead,
        PendingRead(sibling_dropped.clone()),
        128,
    );
    let cancellation = command.cancellation();
    assert!(
        tokio::time::timeout(Duration::from_secs(8), command.wait())
            .await
            .unwrap()
            .is_err()
    );
    assert!(sibling_dropped.load(Ordering::SeqCst));
    let process = cancellation.process.lock().unwrap();
    assert!(process.reaped.is_some());
    assert!(process.outputs.joined.load(Ordering::SeqCst));
    assert!(process.outputs.uncertain.load(Ordering::SeqCst));
    drop(process);
    assert!(!cancellation.exited().unwrap());
    fixture.assert_retired(&cancellation);
}

#[tokio::test]
async fn output_truncation_drains_to_actual_eof_without_uncertainty() {
    let fixture = Fixture::new();
    let (process, stdout, stderr, stdin) =
        fixture.spawn("i=0; while [ $i -lt 4096 ]; do printf x; i=$((i+1)); done");
    drop(stdin);
    let command = OwnedCommand::supervise(
        fixture.run(Duration::from_secs(5)),
        process,
        stdout,
        stderr,
        32,
    );
    let cancellation = command.cancellation();
    let outcome = tokio::time::timeout(Duration::from_secs(10), command.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.stdout, "x".repeat(32));
    assert!(outcome.output_truncated);
    assert!(cancellation.exited().unwrap());
    fixture.assert_retired(&cancellation);
}

#[tokio::test]
async fn eof_is_not_published_as_closure_before_both_actual_task_joins() {
    let outputs = Arc::new(CommandOutputs::default());
    let tasks = CommandOutputTasks {
        stdout: Some(CommandOutputTasks::spawn(
            tokio::io::empty(),
            16,
            outputs.clone(),
            0,
            None,
        )),
        stderr: Some(CommandOutputTasks::spawn(
            tokio::io::empty(),
            16,
            outputs.clone(),
            1,
            None,
        )),
        outputs: outputs.clone(),
    };
    tokio::time::timeout(Duration::from_secs(2), async {
        while !outputs.eof.iter().all(|eof| eof.load(Ordering::SeqCst)) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!outputs.closed());
    let (stdout, stderr) = tasks.collect().await.unwrap();
    assert!(stdout.0.is_empty());
    assert!(stderr.0.is_empty());
    assert!(outputs.closed());
}

#[tokio::test]
async fn retained_exit_and_joined_collectors_survive_original_journal_write_failure() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let (process, stdout, stderr, mut stdin) = fixture.spawn("IFS= read -r input; exit 0");
    let path = fixture.record_path();
    let original = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let command = OwnedCommand::supervise(
        fixture.run(Duration::from_secs(5)),
        process,
        stdout,
        stderr,
        128,
    );
    let cancellation = command.cancellation();
    stdin.write_all(b"released\n").await.unwrap();
    drop(stdin);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let reaped = cancellation.process.lock().unwrap().reaped.is_some();
            if reaped {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    {
        let process = cancellation.process.lock().unwrap();
        assert!(process.reaped.is_some());
        assert!(process.guard.is_some());
        assert!(!process.journal_retired);
    }
    assert!(cancellation.exited().is_err());
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, &original).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    // The operation retains its original write error, but successful later
    // retirement uses the same opaque exit token, never another wait/reap.
    assert!(
        tokio::time::timeout(Duration::from_secs(8), command.wait())
            .await
            .unwrap()
            .is_err()
    );
    assert!(cancellation.exited().unwrap());
    assert!(cancellation.process.lock().unwrap().guard.is_none());
    fixture.assert_retired(&cancellation);
}

#[tokio::test]
async fn collector_guard_survives_spawn_and_latches_abort_before_first_poll() {
    let outputs = Arc::new(CommandOutputs::default());
    let reader_dropped = Arc::new(AtomicBool::new(false));
    let task = CommandOutputTasks::spawn(
        PendingRead(reader_dropped.clone()),
        16,
        outputs.clone(),
        0,
        None,
    );
    // This current-thread test has not yielded since spawn. The whole collector
    // guard and actual reader remain in the spawned future, not the caller.
    assert!(!reader_dropped.load(Ordering::SeqCst));
    assert!(!outputs.uncertain.load(Ordering::SeqCst));
    task.abort();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap_err()
            .is_cancelled()
    );
    assert!(reader_dropped.load(Ordering::SeqCst));
    assert!(outputs.uncertain.load(Ordering::SeqCst));
    assert!(!outputs.eof[0].load(Ordering::SeqCst));
    assert!(!outputs.closed());
}

async fn wait_fixture_marker(fixture: &Fixture, name: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !fixture.root.join(name).is_file() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("owned fixture did not reach its explicit ready marker");
}

#[tokio::test]
async fn service_term_shutdown_retains_clean_original_result_and_journal() {
    let fixture = Fixture::new();
    let (process, stdout, stderr, stdin) = fixture.spawn(
        "SECONDS=0; trap 'printf stopped; exit 0' TERM; printf ready > ready; while [ \"$SECONDS\" -lt 12 ]; do read -t 1 value || :; done; exit 23",
    );
    let command = OwnedCommand::supervise(
        fixture.run(Duration::from_secs(15)),
        process.with_service(true),
        stdout,
        stderr,
        1024,
    );
    let cancellation = command.cancellation();
    wait_fixture_marker(&fixture, "ready").await;
    assert!(cancellation.service_observation().unwrap().leader_running);
    cancellation.cancel().unwrap();
    let original_stop = cancellation.process.lock().unwrap().cancelled_at;
    let result = tokio::time::timeout(Duration::from_secs(12), command.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert_eq!(result.termination_signal, None);
    assert_eq!(result.stdout, "stopped");
    assert!(!result.timed_out);
    cancellation.cancel().unwrap();
    assert_eq!(
        cancellation.process.lock().unwrap().cancelled_at,
        original_stop
    );
    assert_eq!(
        cancellation.service_observation().unwrap(),
        ServiceObservation {
            leader_running: false,
            stop_requested: true,
            kill_escalated: false,
        }
    );
    assert!(cancellation.exited().unwrap());
    fixture.assert_retired(&cancellation);
    drop(stdin);
}

#[tokio::test]
async fn service_ignoring_term_retains_actual_kill_and_original_stop_clock() {
    let fixture = Fixture::new();
    let (process, stdout, stderr, stdin) =
        fixture.spawn("SECONDS=0; trap '' TERM; printf ready > ready; while [ \"$SECONDS\" -lt 12 ]; do read -t 1 value || :; done; exit 23");
    let command = OwnedCommand::supervise(
        fixture.run(Duration::from_secs(15)),
        process.with_service(true),
        stdout,
        stderr,
        1024,
    );
    let cancellation = command.cancellation();
    wait_fixture_marker(&fixture, "ready").await;
    cancellation.cancel().unwrap();
    let original_stop = cancellation.process.lock().unwrap().cancelled_at.unwrap();
    cancellation.cancel().unwrap();
    assert_eq!(
        cancellation.process.lock().unwrap().cancelled_at,
        Some(original_stop)
    );
    let result = tokio::time::timeout(Duration::from_secs(12), command.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(original_stop.elapsed() >= COMMAND_EXIT_GRACE);
    assert_eq!(result.exit_code, None);
    assert_eq!(result.termination_signal, Some(libc::SIGKILL));
    assert!(cancellation.service_observation().unwrap().kill_escalated);
    assert!(cancellation.exited().unwrap());
    fixture.assert_retired(&cancellation);
    drop(stdin);
}

#[tokio::test]
async fn terminal_service_leader_cannot_hide_forced_group_member_cleanup() {
    let fixture = Fixture::new();
    let (process, stdout, stderr, stdin) = fixture.spawn(
        "(SECONDS=0; trap '' TERM; printf ready > child-ready; while [ \"$SECONDS\" -lt 12 ]; do :; done; exit 23) & SECONDS=0; trap 'exit 0' TERM; printf ready > ready; while [ \"$SECONDS\" -lt 12 ]; do read -t 1 value || :; done; exit 23",
    );
    let command = OwnedCommand::supervise(
        fixture.run(Duration::from_secs(15)),
        process.with_service(true),
        stdout,
        stderr,
        1024,
    );
    let cancellation = command.cancellation();
    wait_fixture_marker(&fixture, "ready").await;
    wait_fixture_marker(&fixture, "child-ready").await;
    cancellation.cancel().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(12), command.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert!(cancellation.service_observation().unwrap().kill_escalated);
    assert!(!cancellation.service_observation().unwrap().leader_running);
    assert!(cancellation.exited().unwrap());
    fixture.assert_retired(&cancellation);
    drop(stdin);
}

#[tokio::test]
async fn bundle_output_allowance_is_shared_across_actual_stdout_and_stderr() {
    let fixture = Fixture::new();
    let (process, stdout, stderr, stdin) =
        fixture.spawn("printf abcdefghij; printf klmnopqrst >&2");
    drop(stdin);
    let budget = Arc::new(CommandOutputBudget::new(7));
    let command = OwnedCommand::supervise(
        fixture.run(Duration::from_secs(3)),
        process.with_output_budget(Some(budget.clone())),
        stdout,
        stderr,
        1024,
    );
    let cancellation = command.cancellation();
    let result = tokio::time::timeout(Duration::from_secs(8), command.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.stdout.len() + result.stderr.len(), 7);
    assert_eq!(budget.remaining.load(Ordering::SeqCst), 0);
    assert!(result.output_truncated);
    assert!(cancellation.exited().unwrap());
    fixture.assert_retired(&cancellation);
}

#[test]
fn original_bundle_cutoff_is_not_renewed_by_preparation() {
    let now = Instant::now();
    let started = now - Duration::from_secs(1);
    let deadline = now + Duration::from_secs(2);
    assert_eq!(
        remaining_bundle_timeout(Duration::from_secs(20), started, Some(deadline)).unwrap(),
        deadline - started
    );
    assert_eq!(
        remaining_bundle_timeout(Duration::from_secs(1), started, Some(deadline)).unwrap(),
        Duration::from_secs(1)
    );
    assert_eq!(
        remaining_bundle_timeout(Duration::from_secs(20), started, Some(started))
            .unwrap_err()
            .code,
        "timeout"
    );
    assert_eq!(
        remaining_bundle_timeout(Duration::from_secs(20), started, None).unwrap(),
        Duration::from_secs(20)
    );
}
