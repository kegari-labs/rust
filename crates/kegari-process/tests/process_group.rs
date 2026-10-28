//! Ordinary disposable process fixtures, not OS sandbox qualification.
#![cfg(unix)]
use kegari_process::group::{OwnedGroup, Termination};
use std::{
    io::Read,
    os::unix::process::CommandExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn natural_leader_exit_cleans_inherited_group_before_reaping() {
    let mut command = Command::new("/bin/sh");
    command
        .args(["-c", "sleep 30 & printf '%s\\n' \"$!\"; exit 0"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    let (mut child, mut group) = OwnedGroup::spawn_std(&mut command).unwrap();
    let started = Instant::now();
    while !group.observe_exit_before_reap().unwrap() {
        assert!(started.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(child.wait().unwrap().success());
    let mut output = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    assert!(output.trim().parse::<u32>().unwrap() > 1);
    assert!(started.elapsed() < Duration::from_secs(3));
    // A terminal cached capability never signals a possibly recycled number.
    group.terminate(Termination::Kill).unwrap();
}

#[test]
fn group_rejects_unisolated_or_already_reaped_child() {
    let mut child = Command::new("/bin/sleep").arg("1").spawn().unwrap();
    assert!(OwnedGroup::from_spawned_std(&child).is_err());
    child.kill().unwrap();
    child.wait().unwrap();
    let mut command = Command::new("/bin/sleep");
    command.arg("0").process_group(0);
    let (mut child, mut group) = OwnedGroup::spawn_std(&mut command).unwrap();
    child.wait().unwrap();
    assert_eq!(
        group.observe_exit_before_reap().unwrap_err().code,
        "ownership_conflict"
    );
    assert_eq!(
        group.terminate(Termination::Kill).unwrap_err().code,
        "ownership_conflict"
    );
}

#[test]
fn cancellation_signals_only_owned_group_then_observes_exit() {
    let mut command = Command::new("/bin/sleep");
    command.arg("30").process_group(0);
    let (mut child, mut group) = OwnedGroup::spawn_std(&mut command).unwrap();
    assert!(!group.observe_exit_before_reap().unwrap());
    group.terminate(Termination::Kill).unwrap();
    let start = Instant::now();
    while !group.observe_exit_before_reap().unwrap() {
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!child.wait().unwrap().success());
}

#[test]
fn group_cleanup_does_not_claim_containment_of_a_new_session() {
    // Separate fixture process avoids forking this test runner's other tests.
    // Its escaped child is finite (500ms), has no external target and writes
    // only the already-open owned temporary result file before _exit.
    if let Some(path) = std::env::var_os("KEGARI_OWNED_ESCAPE_FIXTURE") {
        use std::os::fd::AsRawFd;
        let result = std::fs::File::create(path).unwrap();
        let mut ready = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(ready.as_mut_ptr()) }, 0);
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0);
        if pid == 0 {
            unsafe {
                libc::close(ready[0]);
                if libc::setsid() == -1 {
                    libc::_exit(10);
                }
                libc::close(libc::STDOUT_FILENO);
                libc::close(libc::STDERR_FILENO);
                libc::write(ready[1], b"r".as_ptr().cast(), 1);
                libc::close(ready[1]);
                let delay = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 500_000_000,
                };
                libc::nanosleep(&delay, std::ptr::null_mut());
                libc::write(result.as_raw_fd(), b"escaped write".as_ptr().cast(), 13);
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
    let temp = tempfile::tempdir().unwrap();
    let result = temp.path().join("owned-result");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "group_cleanup_does_not_claim_containment_of_a_new_session",
            "--test-threads=1",
        ])
        .env("KEGARI_OWNED_ESCAPE_FIXTURE", &result)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let (mut child, mut group) = OwnedGroup::spawn_std(&mut command).unwrap();
    let start = Instant::now();
    while !group.observe_exit_before_reap().unwrap() {
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(child.wait().unwrap().success());
    assert!(std::fs::read(&result).unwrap().is_empty());
    loop {
        if std::fs::read(&result).unwrap() == b"escaped write" {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(5));
    }
    // Actual artifact mutation after group cleanup proves the scope limitation.
}
