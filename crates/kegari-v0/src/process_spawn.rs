//! Serialize host starts and exact process-guard descriptor lifetimes.
//!
//! Spawn's exec-error-pipe EOF is not a barrier for every CLOEXEC close: Darwin
//! closes descriptors individually. Before exec, close only registered guard
//! descriptors still marked CLOEXEC. The intended child guard was explicitly
//! marked inheritable by its own earlier hook. Never unlock the inherited open
//! file description, wait for a child here, or acquire this mutex after fork.
use crate::{
    error::{Error, Result},
    storage::ExclusiveLock,
};
use std::{
    collections::BTreeMap,
    fs::File,
    io,
    os::{
        fd::{AsRawFd, RawFd},
        unix::process::CommandExt,
    },
    process::{Child, Command},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

const MAX_GUARDS: usize = 65_536;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GuardDescriptors {
    original: RawFd,
    inherited: RawFd,
    device: u64,
    inode: u64,
}
#[derive(Default)]
struct SpawnState {
    guards: BTreeMap<RawFd, GuardDescriptors>,
}
static SPAWN_RETIREMENT: Mutex<SpawnState> = Mutex::new(SpawnState {
    guards: BTreeMap::new(),
});
pub(crate) struct SpawnBoundary(MutexGuard<'static, SpawnState>);

pub(crate) fn retirement() -> Result<SpawnBoundary> {
    SPAWN_RETIREMENT.lock().map(SpawnBoundary).map_err(|_| {
        Error::new(
            "operation_uncertain",
            "host spawn/guard-retirement boundary was interrupted",
        )
    })
}

/// The registration owns both exact descriptors. Creation, removal and closing
/// all occur under the same boundary as fork; no snapshot can contain a reused
/// descriptor or omit a newly created guard. No File/lock is cloned into hooks.
pub(crate) struct RegisteredGuard {
    descriptors: GuardDescriptors,
    lock: Option<ExclusiveLock>,
    inherited: Option<File>,
    inheritance_active: Arc<AtomicBool>,
}
impl RegisteredGuard {
    pub(crate) fn acquire(acquire: impl FnOnce() -> Result<ExclusiveLock>) -> Result<Self> {
        let mut boundary = retirement()?;
        if boundary.0.guards.len() >= MAX_GUARDS {
            return Err(Error::new(
                "process_limit",
                "host process guard inventory exceeds bound",
            ));
        }
        // Failed acquisition/duplication closes its descriptors before the
        // boundary is dropped. The callback only opens/verifies the guard file.
        let lock = acquire()?;
        let inherited = lock.inherited_file()?;
        let (device, inode) = lock.identity()?;
        let descriptors = GuardDescriptors {
            original: lock.raw_fd(),
            inherited: inherited.as_raw_fd(),
            device,
            inode,
        };
        if boundary.0.guards.contains_key(&descriptors.original) {
            return Err(Error::new(
                "ownership_conflict",
                "process guard descriptor is already registered",
            ));
        }
        boundary.0.guards.insert(descriptors.original, descriptors);
        Ok(Self {
            descriptors,
            lock: Some(lock),
            inherited: Some(inherited),
            inheritance_active: Arc::new(AtomicBool::new(true)),
        })
    }
    pub(crate) fn identity(&self) -> (u64, u64) {
        (self.descriptors.device, self.descriptors.inode)
    }
    pub(crate) fn configure(&self, command: &mut Command) -> Result<()> {
        if self.inherited.is_none() {
            return Err(Error::new(
                "ownership_conflict",
                "process guard already closed",
            ));
        }
        let descriptor = self.descriptors;
        let active = self.inheritance_active.clone();
        // A configured Command can outlive its guard or be reused. Never let
        // this earlier hook act on a recycled FD, even before snapshot cleanup.
        unsafe {
            command.pre_exec(move || {
                if !active.load(Ordering::SeqCst) {
                    return Ok(());
                }
                let flags = checked_flags(descriptor.inherited, &descriptor)?;
                if libc::fcntl(
                    descriptor.inherited,
                    libc::F_SETFD,
                    flags & !libc::FD_CLOEXEC,
                ) < 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Ok(())
    }
    pub(crate) fn spawned(&self) {
        // The parent has bound the actual Child/OwnedGroup. The completed
        // invocation's child already ran its hook; future invocations must not
        // inherit this owner's guard, including while retirement is pending.
        self.inheritance_active.store(false, Ordering::SeqCst);
    }
    pub(crate) fn release(&mut self, boundary: &mut SpawnBoundary) -> Result<()> {
        if self.lock.is_none() {
            return Ok(());
        }
        if boundary.0.guards.get(&self.descriptors.original) != Some(&self.descriptors) {
            return Err(Error::new(
                "ownership_conflict",
                "process guard registration changed",
            ));
        }
        self.inheritance_active.store(false, Ordering::SeqCst);
        boundary.0.guards.remove(&self.descriptors.original);
        self.inherited.take();
        self.lock.take();
        Ok(())
    }
}
impl Drop for RegisteredGuard {
    fn drop(&mut self) {
        if self.lock.is_none() {
            return;
        }
        // Poison still forbids subsequent starts/retirement proof. Destruction
        // must nevertheless close this owner's files without reopening them.
        let mut state = SPAWN_RETIREMENT
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.inheritance_active.store(false, Ordering::SeqCst);
        state.guards.remove(&self.descriptors.original);
        self.inherited.take();
        self.lock.take();
    }
}

fn before(deadline: Option<Instant>) -> io::Result<SpawnBoundary> {
    let boundary = SPAWN_RETIREMENT
        .lock()
        .map(SpawnBoundary)
        .map_err(|_| io::Error::other("host spawn/guard-retirement boundary was interrupted"))?;
    check_cutoff(deadline)?;
    Ok(boundary)
}
fn check_cutoff(deadline: Option<Instant>) -> io::Result<()> {
    if deadline.is_some_and(|cutoff| Instant::now() >= cutoff) {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "original process launch deadline expired while awaiting spawn boundary",
        ));
    }
    Ok(())
}

struct ActiveHook(Arc<AtomicBool>);
impl Drop for ActiveHook {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

// Called only after fork. The immutable snapshot was made under the boundary;
// fcntl/fstat/close are async-signal-safe. No allocation, lock, destructor or
// flock unlock occurs here. Inheritable descriptors are the selected guard.
fn close_unrelated_guards(guards: &[GuardDescriptors]) -> io::Result<()> {
    for guard in guards {
        for fd in [guard.original, guard.inherited] {
            let flags = checked_flags(fd, guard)?;
            if flags & libc::FD_CLOEXEC != 0 && unsafe { libc::close(fd) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

fn checked_flags(fd: RawFd, guard: &GuardDescriptors) -> io::Result<i32> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(fd, metadata.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let metadata = unsafe { metadata.assume_init() };
    if metadata.st_dev as u64 != guard.device || metadata.st_ino != guard.inode {
        return Err(io::Error::from_raw_os_error(libc::ESTALE));
    }
    Ok(flags)
}

#[cfg(test)]
type ChildProbe = Arc<dyn Fn() -> io::Result<()> + Send + Sync>;
fn prepare_hook(
    command: &mut Command,
    boundary: &SpawnBoundary,
    #[cfg(test)] after_close: Option<ChildProbe>,
) -> ActiveHook {
    let guards: Vec<_> = boundary.0.guards.values().copied().collect();
    let active = Arc::new(AtomicBool::new(true));
    if guards.is_empty() {
        #[cfg(not(test))]
        return ActiveHook(active);
        #[cfg(test)]
        if after_close.is_none() {
            return ActiveHook(active);
        }
    }
    let child_active = active.clone();
    // Command keeps pre-exec hooks when reused. Only this invocation's snapshot
    // may run; every previous hook is inert, including after a failed spawn.
    // Without this flag a stale snapshot could close a recycled descriptor.
    unsafe {
        command.pre_exec(move || {
            if child_active.load(Ordering::SeqCst) {
                close_unrelated_guards(&guards)?;
                #[cfg(test)]
                if let Some(probe) = after_close.as_ref() {
                    probe()?;
                }
            }
            Ok(())
        });
    }
    ActiveHook(active)
}

pub(crate) fn spawn_std(command: &mut Command, deadline: Option<Instant>) -> io::Result<Child> {
    let boundary = before(deadline)?;
    let _active = prepare_hook(
        command,
        &boundary,
        #[cfg(test)]
        None,
    );
    check_cutoff(deadline)?;
    command.spawn()
}
pub(crate) fn spawn_tokio(
    command: &mut tokio::process::Command,
    deadline: Option<Instant>,
) -> io::Result<tokio::process::Child> {
    let boundary = before(deadline)?;
    let _active = prepare_hook(
        command.as_std_mut(),
        &boundary,
        #[cfg(test)]
        None,
    );
    check_cutoff(deadline)?;
    command.spawn()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn expired_cutoff_after_interlock_wait_refuses_before_spawn() {
        let boundary = retirement().unwrap();
        let cutoff = Instant::now() + Duration::from_millis(50);
        let (entered, observed) = std::sync::mpsc::sync_channel(1);
        std::thread::scope(|threads| {
            let waiter = threads.spawn(move || {
                entered.send(()).unwrap();
                // No child should be attempted. Without the post-lock cutoff
                // check this returns NotFound, not the required TimedOut.
                spawn_std(
                    &mut Command::new("/kegari-nonexistent-interlock-fixture"),
                    Some(cutoff),
                )
                .unwrap_err()
                .kind()
            });
            observed.recv_timeout(Duration::from_secs(2)).unwrap();
            std::thread::sleep(
                cutoff.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
            );
            drop(boundary);
            assert_eq!(waiter.join().unwrap(), io::ErrorKind::TimedOut);
        });
    }

    fn guards() -> (tempfile::TempDir, crate::storage::PrivateDir) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap();
        let directory = crate::storage::PrivateDir::create(&path.join("guards")).unwrap();
        (root, directory)
    }

    fn shell() -> Command {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "exit 0"])
            .env_clear()
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command
    }

    #[test]
    fn child_closes_both_unrelated_guard_fds_before_exec_and_preserves_selected_guard() {
        let (_root, directory) = guards();
        let unrelated = RegisteredGuard::acquire(|| directory.lock("unrelated")).unwrap();
        let selected = RegisteredGuard::acquire(|| directory.lock("selected")).unwrap();
        let unrelated_fds = unrelated.descriptors;
        let selected_fds = selected.descriptors;
        let mut command = shell();
        // Component equivalent of the actual SpawnGuard hook, not a native
        // identity, group-exit or journal capability constructor.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(selected_fds.inherited, libc::F_GETFD);
                if flags < 0
                    || libc::fcntl(
                        selected_fds.inherited,
                        libc::F_SETFD,
                        flags & !libc::FD_CLOEXEC,
                    ) < 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let probe: ChildProbe = Arc::new(move || {
            // Runs in this actual fork after the production close routine and
            // BEFORE exec, so the assertion does not depend on CLOEXEC timing.
            for fd in [
                unrelated_fds.original,
                unrelated_fds.inherited,
                selected_fds.original,
            ] {
                if unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1
                    || io::Error::last_os_error().raw_os_error() != Some(libc::EBADF)
                {
                    return Err(io::Error::from_raw_os_error(libc::EIO));
                }
            }
            let flags = unsafe { libc::fcntl(selected_fds.inherited, libc::F_GETFD) };
            if flags < 0 || flags & libc::FD_CLOEXEC != 0 {
                return Err(io::Error::from_raw_os_error(libc::EIO));
            }
            Ok(())
        });
        let mut child = {
            let boundary = before(None).unwrap();
            let _active = prepare_hook(&mut command, &boundary, Some(probe));
            command.spawn().unwrap()
        };
        assert!(child.wait().unwrap().success());
        // Closing the child's copies must neither close nor unlock the host's.
        for pair in [unrelated_fds, selected_fds] {
            for fd in [pair.original, pair.inherited] {
                assert_ne!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
            }
        }
        assert_eq!(
            directory.lock("unrelated").err().unwrap().code,
            "ownership_conflict"
        );
        assert_eq!(
            directory.lock("selected").err().unwrap().code,
            "ownership_conflict"
        );
    }

    #[test]
    fn reused_command_disarms_prior_snapshots_after_success_and_failed_spawn() {
        let (_root, directory) = guards();
        let mut command = shell();
        for index in 0..3 {
            // Each prior guard is destroyed before the next invocation. Its
            // descriptor numbers may be closed or recycled to the new file;
            // neither old snapshot may execute in the next child.
            let guard =
                RegisteredGuard::acquire(|| directory.lock(&format!("guard-{index}"))).unwrap();
            if index == 1 {
                command.current_dir(directory.path().join("absent"));
                assert_eq!(
                    spawn_std(&mut command, None).unwrap_err().kind(),
                    io::ErrorKind::NotFound
                );
            } else {
                command.current_dir(directory.path());
                let mut child = spawn_std(&mut command, None).unwrap();
                assert!(child.wait().unwrap().success());
            }
            drop(guard);
        }
    }

    #[test]
    fn failed_guard_acquisition_keeps_no_registry_or_lock_owner() {
        let (_root, directory) = guards();
        let result = RegisteredGuard::acquire(|| {
            let _lock = directory.lock("failed")?;
            Err(Error::new("fixture_error", "after actual guard lock"))
        });
        assert_eq!(result.err().unwrap().code, "fixture_error");
        let guard = RegisteredGuard::acquire(|| directory.lock("failed")).unwrap();
        let descriptors = guard.descriptors;
        assert_eq!(
            retirement().unwrap().0.guards.get(&descriptors.original),
            Some(&descriptors)
        );
        drop(guard);
        // Registry membership/actual lock absence after Drop are both checked.
        assert!(
            !retirement()
                .unwrap()
                .0
                .guards
                .values()
                .any(|entry| *entry == descriptors)
        );
        let _lock = directory.lock("failed").unwrap();
    }
}
