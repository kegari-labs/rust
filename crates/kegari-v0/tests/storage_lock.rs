//! Ordinary disposable fork/exec fixtures, not native isolation qualification.
#![cfg(unix)]

use kegari_v0::storage::PrivateDir;
use std::{
    io::{self, Read, Write},
    net::Shutdown,
    os::{fd::AsRawFd, unix::net::UnixStream, unix::process::CommandExt},
    process::{Command, ExitStatus, Stdio},
    sync::Mutex,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

// Each fixture intentionally inherits all open descriptors. Serialize this
// binary's fixtures so a different fixture cannot extend the tested lock's
// lifetime with a second child whose handshake this fixture does not control.
static FIXTURE_SERIAL: Mutex<()> = Mutex::new(());

/// `Command::spawn` waits for exec, so a separate thread owns that call and its
/// resulting Child. This guard releases the pre-exec wait and joins/reaps on
/// assertion unwinding too. The child also has its own finite pre-exec timeout.
struct PausedSpawn {
    channel: UnixStream,
    worker: Option<JoinHandle<io::Result<ExitStatus>>>,
}

impl PausedSpawn {
    fn start() -> io::Result<Self> {
        let (channel, child_channel) = UnixStream::pair()?;
        channel.set_read_timeout(Some(Duration::from_secs(5)))?;
        channel.set_write_timeout(Some(Duration::from_secs(5)))?;
        child_channel.set_nonblocking(true)?;
        let fd = child_channel.as_raw_fd();
        let worker = thread::Builder::new()
            .name("owned-lock-inheritance-fixture".into())
            .spawn(move || {
                let mut command = Command::new("/usr/bin/true");
                command
                    .env_clear()
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                // SAFETY: child_channel stays open throughout spawn. After fork
                // the callback uses only stack values and async-signal-safe
                // write/poll/read/_exit, with a bounded nonblocking socket. No
                // allocation, Rust locks, formatting, or unwinding occurs here.
                unsafe {
                    command.pre_exec(move || {
                        let ready = b'R';
                        if libc::write(fd, (&ready as *const u8).cast(), 1) != 1 {
                            libc::_exit(111);
                        }
                        let mut pending = libc::pollfd {
                            fd,
                            events: libc::POLLIN,
                            revents: 0,
                        };
                        if libc::poll(&mut pending, 1, 10_000) != 1 {
                            libc::_exit(112);
                        }
                        let mut release = 0_u8;
                        if libc::read(fd, (&mut release as *mut u8).cast(), 1) != 1
                            || release != b'G'
                        {
                            libc::_exit(113);
                        }
                        Ok(())
                    });
                }
                let spawned = command.spawn();
                drop(child_channel);
                let mut child = spawned?;
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => return Ok(status),
                        Ok(None) if Instant::now() < deadline => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        result => {
                            // Only this actual owned Child is terminated; no
                            // PID lookup, adoption, descendants, or enumeration.
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(match result {
                                Err(error) => error,
                                _ => io::Error::new(
                                    io::ErrorKind::TimedOut,
                                    "owned lock fixture did not exit",
                                ),
                            });
                        }
                    }
                }
            })?;
        Ok(Self {
            channel,
            worker: Some(worker),
        })
    }

    fn wait_before_exec(&mut self) -> io::Result<()> {
        let mut ready = [0];
        self.channel.read_exact(&mut ready)?;
        if ready != *b"R" {
            return Err(io::Error::other("invalid owned fixture handshake"));
        }
        Ok(())
    }

    fn release_and_join(mut self) -> io::Result<ExitStatus> {
        self.channel.write_all(b"G")?;
        self.worker
            .take()
            .expect("owned fixture has one spawn thread")
            .join()
            .map_err(|_| io::Error::other("owned fixture spawn thread panicked"))?
    }
}

impl Drop for PausedSpawn {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            // Shutdown wakes the child's bounded poll/read even if an earlier
            // assertion failed. Its spawn thread still reaps the actual Child.
            let _ = self.channel.shutdown(Shutdown::Both);
            let _ = worker.join();
        }
    }
}

#[test]
fn exclusive_lock_survives_parent_drop_until_inherited_child_execs() {
    let _serial = FIXTURE_SERIAL.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let records = PrivateDir::create(&root.join("state")).unwrap();
    let parent_lock = records.lock("delivery-effects.lock").unwrap();

    let mut child = PausedSpawn::start().unwrap();
    child.wait_before_exec().unwrap();
    drop(parent_lock);

    // The callback proves fork occurred but exec has not. O_CLOEXEC has not
    // closed the child's duplicate, despite dropping the parent's only guard.
    let error = records
        .lock("delivery-effects.lock")
        .err()
        .expect("the pre-exec child still retains the locked file description");
    assert_eq!(error.code, "ownership_conflict", "{error:?}");
    assert_eq!(error.details["os_error"], libc::EWOULDBLOCK, "{error:?}");
    assert_eq!(error.details["os_error_kind"], "WouldBlock", "{error:?}");

    assert!(child.release_and_join().unwrap().success());
    let _uncontended = records.lock("delivery-effects.lock").unwrap();
}

#[test]
fn section_lock_releases_before_child_exec_without_unlocking_its_new_owner() {
    let _serial = FIXTURE_SERIAL.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let records = PrivateDir::create(&root.join("state")).unwrap();
    let parent_lock = records.section_lock("delivery-effects.lock").unwrap();

    let mut child = PausedSpawn::start().unwrap();
    child.wait_before_exec().unwrap();
    drop(parent_lock);

    // No release byte has been sent: the child still retains the original open
    // file description. Explicit section unlock nevertheless ends that lease.
    let new_owner = records.section_lock("delivery-effects.lock").unwrap();
    assert!(child.release_and_join().unwrap().success());

    // Closing the inherited old description must not unlock the new owner's
    // independently opened description, even though both refer to one inode.
    let error = records
        .section_lock("delivery-effects.lock")
        .err()
        .expect("the new section owner must still exclude another opener");
    assert_eq!(error.code, "ownership_conflict", "{error:?}");
    assert_eq!(error.details["os_error"], libc::EWOULDBLOCK, "{error:?}");
    assert_eq!(error.details["os_error_kind"], "WouldBlock", "{error:?}");

    drop(new_owner);
    let _uncontended = records.section_lock("delivery-effects.lock").unwrap();
}

#[test]
fn exclusive_lock_clone_retains_ownership_until_the_last_guard_drops() {
    let _serial = FIXTURE_SERIAL.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let records = PrivateDir::create(&root.join("state")).unwrap();
    let original = records.lock("owner.lock").unwrap();
    let retained = original.clone();
    drop(original);

    let error = records
        .lock("owner.lock")
        .err()
        .expect("dropping one ownership guard must not unlock its live clone");
    assert_eq!(error.code, "ownership_conflict", "{error:?}");
    assert_eq!(error.details["os_error"], libc::EWOULDBLOCK, "{error:?}");
    assert_eq!(error.details["os_error_kind"], "WouldBlock", "{error:?}");

    drop(retained);
    let _uncontended = records.lock("owner.lock").unwrap();
}
