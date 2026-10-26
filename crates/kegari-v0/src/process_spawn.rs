//! Serialize host fork-to-exec windows with inherited process-guard retirement.
//!
//! CLOEXEC descriptors still exist in an unrelated fork until exec. A guard
//! release probe must not mistake that known host launch window for a surviving
//! owned descendant. Every host process start uses this boundary; it is released
//! as soon as synchronous spawn returns, before any child wait or async await.
//! Nothing acquires this Rust mutex in a post-fork child.
use std::{
    io,
    process::{Child, Command},
    sync::{Mutex, MutexGuard},
    time::Instant,
};

static SPAWN_RETIREMENT: Mutex<()> = Mutex::new(());

pub(crate) fn retirement() -> crate::error::Result<MutexGuard<'static, ()>> {
    SPAWN_RETIREMENT.lock().map_err(|_| {
        crate::error::Error::new(
            "operation_uncertain",
            "host spawn/guard-retirement boundary was interrupted",
        )
    })
}

fn before(deadline: Option<Instant>) -> io::Result<MutexGuard<'static, ()>> {
    let boundary = SPAWN_RETIREMENT
        .lock()
        .map_err(|_| io::Error::other("host spawn/guard-retirement boundary was interrupted"))?;
    if deadline.is_some_and(|cutoff| Instant::now() >= cutoff) {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "original process launch deadline expired while awaiting spawn boundary",
        ));
    }
    Ok(boundary)
}

pub(crate) fn spawn_std(command: &mut Command, deadline: Option<Instant>) -> io::Result<Child> {
    let _boundary = before(deadline)?;
    command.spawn()
}

pub(crate) fn spawn_tokio(
    command: &mut tokio::process::Command,
    deadline: Option<Instant>,
) -> io::Result<tokio::process::Child> {
    let _boundary = before(deadline)?;
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
}
