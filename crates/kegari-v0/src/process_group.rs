//! Owned Unix process groups. Observe terminal leaders without reaping, finish
//! their group while the leader PID is still reserved, then let the Child owner
//! reap. This proves group cleanup, not containment of descendants which escape
//! into a new session/group; that remains an execution-boundary requirement.
use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug)]
pub enum Termination {
    Terminate,
    Kill,
}

/// Construct immediately after spawning a Child with process_group(0)/setsid,
/// and keep beside that exact Child under the same ownership mutex. Never
/// reconstruct this capability from stored PIDs or call Child::try_wait first.
pub struct OwnedGroup {
    pid: libc::pid_t,
    ready_to_reap: bool,
}
impl OwnedGroup {
    #[cfg(unix)]
    pub fn spawn_std(command: &mut std::process::Command) -> Result<(std::process::Child, Self)> {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        let child = command
            .spawn()
            .map_err(|error| Error::io("spawn owned process group", error))?;
        let group = Self::created(child.id());
        Ok((child, group))
    }
    #[cfg(unix)]
    pub fn spawn_tokio(
        command: &mut tokio::process::Command,
    ) -> Result<(tokio::process::Child, Self)> {
        command.process_group(0);
        let child = command
            .spawn()
            .map_err(|error| Error::io("spawn owned async process group", error))?;
        // A freshly spawned, never-waited Tokio Child necessarily has its ID.
        let group = Self::created(child.id().expect("new unreaped Tokio child has an ID"));
        Ok((child, group))
    }

    pub fn from_spawned_std(child: &std::process::Child) -> Result<Self> {
        Self::from_spawn(child.id())
    }
    pub fn from_spawned_tokio(child: &tokio::process::Child) -> Result<Self> {
        Self::from_spawn(
            child.id().ok_or_else(|| {
                Error::new("ownership_conflict", "spawned child was already reaped")
            })?,
        )
    }
    fn created(pid: u32) -> Self {
        // The OS supplies pid_t here. Successful spawn with process_group(0)
        // establishes creation provenance without a fallible metadata probe
        // that could otherwise discard the only actual Child owner.
        Self {
            pid: pid as libc::pid_t,
            ready_to_reap: false,
        }
    }
    fn from_spawn(pid: u32) -> Result<Self> {
        let pid = i32::try_from(pid)
            .ok()
            .filter(|pid| *pid > 1)
            .ok_or_else(|| Error::new("ownership_conflict", "spawned child PID is invalid"))?;
        let group = Self {
            pid,
            ready_to_reap: false,
        };
        // Also establish that the unreaped process is our actual child. WNOWAIT
        // does not consume a quick child's status during construction.
        group.terminal_without_reaping()?;
        group.verify_group()?;
        Ok(group)
    }
    #[cfg(unix)]
    fn verify_group(&self) -> Result<()> {
        let actual = unsafe { libc::getpgid(self.pid) };
        if actual != self.pid {
            let error = std::io::Error::last_os_error();
            return Err(Error::new(
                "ownership_conflict",
                "spawned child is not the independently owned process-group leader",
            )
            .detail(serde_json::json!({"actual_group":actual,"os_error":error.raw_os_error()})));
        }
        Ok(())
    }

    /// Darwin may remove a dying process from getpgid before waitid exposes
    /// its terminal status. Keep its owned Child unreaped, and only tolerate
    /// this bounded transition after fresh WNOWAIT confirms terminal state.
    #[cfg(unix)]
    fn verify_live_group_or_terminal(&self) -> Result<bool> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
        loop {
            if self.terminal_without_reaping()? {
                return Ok(true);
            }
            let actual = unsafe { libc::getpgid(self.pid) };
            if actual == self.pid {
                return Ok(false);
            }
            let error = std::io::Error::last_os_error();
            if actual != -1
                || error.raw_os_error() != Some(libc::ESRCH)
                || std::time::Instant::now() >= deadline
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "owned process-group identity is unavailable",
                )
                .detail(
                    serde_json::json!({"actual_group":actual,"os_error":error.raw_os_error()}),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    #[cfg(not(unix))]
    fn verify_live_group_or_terminal(&self) -> Result<bool> {
        self.verify_group()?;
        Ok(false)
    }
    #[cfg(not(unix))]
    fn verify_group(&self) -> Result<()> {
        Err(Error::new(
            "prerequisite",
            "owned groups require the supported Unix host",
        ))
    }

    #[cfg(unix)]
    fn terminal_without_reaping(&self) -> Result<bool> {
        let mut status = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.pid as libc::id_t,
                status.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            return Err(Error::new(
                "ownership_conflict",
                "owned group leader is no longer demonstrably unreaped",
            )
            .detail(serde_json::json!({"os_error":error.raw_os_error()})));
        }
        let status = unsafe { status.assume_init() };
        let observed = unsafe { status.si_pid() };
        if observed == 0 {
            return Ok(false);
        }
        if observed != self.pid
            || !matches!(
                status.si_code,
                libc::CLD_EXITED | libc::CLD_KILLED | libc::CLD_DUMPED
            )
        {
            return Err(Error::new(
                "ownership_conflict",
                "owned group wait returned an unexpected child identity or state",
            ));
        }
        Ok(true)
    }
    #[cfg(not(unix))]
    fn terminal_without_reaping(&self) -> Result<bool> {
        Err(Error::new(
            "prerequisite",
            "owned groups require the supported Unix host",
        ))
    }

    /// True authorizes the caller to reap this exact Child. Natural leader exit
    /// first terminates remaining members while PID/group identity is reserved.
    pub fn observe_exit_before_reap(&mut self) -> Result<bool> {
        if self.ready_to_reap {
            return Ok(true);
        }
        if !self.terminal_without_reaping()? {
            return Ok(false);
        }
        #[cfg(target_os = "macos")]
        if !self.live_group_members()? {
            self.ready_to_reap = true;
            return Ok(true);
        }
        self.signal(Termination::Kill, true)?;
        #[cfg(target_os = "macos")]
        return Ok(false);
        #[cfg(not(target_os = "macos"))]
        {
            self.ready_to_reap = true;
            Ok(true)
        }
    }
    pub fn terminate(&mut self, signal: Termination) -> Result<()> {
        if self.ready_to_reap {
            return Ok(());
        }
        let terminal = self.terminal_without_reaping()?;
        if terminal {
            self.observe_exit_before_reap()?;
            return Ok(());
        }
        self.signal(signal, terminal)
    }
    #[cfg(unix)]
    fn signal(&self, signal: Termination, terminal: bool) -> Result<()> {
        // Darwin cannot getpgid a zombie. WNOWAIT still reserves the PID and
        // its creation-bound group identity until the Child owner reaps it.
        let mut terminal = terminal;
        if !terminal {
            terminal = self.verify_live_group_or_terminal()?;
        }
        let signal = match signal {
            Termination::Terminate => libc::SIGTERM,
            Termination::Kill => libc::SIGKILL,
        };
        let result = unsafe { libc::kill(-self.pid, signal) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            #[cfg(target_os = "macos")]
            if matches!(error.raw_os_error(), Some(libc::ESRCH | libc::EPERM)) {
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
                loop {
                    if self.terminal_without_reaping()? && !self.live_group_members()? {
                        // A running leader may have exited between verification and
                        // signal. Only fresh WNOWAIT plus a complete empty live-member
                        // inventory proves that no termination request remains needed.
                        return Ok(());
                    }
                    if std::time::Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
            if error.raw_os_error() != Some(libc::ESRCH)
                || (!terminal && !self.terminal_without_reaping()?)
            {
                return Err(Error::io("signal owned unreaped process group", error));
            }
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn live_group_members(&self) -> Result<bool> {
        // Read-only PROC_PGRP_ONLY/PROC_PIDTBSDINFO, verified against the installed
        // Apple SDK sys/proc_info.h. A saturated/denied inventory proves nothing.
        for capacity in [256usize, 1024, 4096, 16384, 65536] {
            let mut pids = vec![0i32; capacity];
            let bytes = (capacity * std::mem::size_of::<i32>()) as i32;
            unsafe {
                *libc::__error() = 0;
            }
            let length =
                unsafe { libc::proc_listpids(2, self.pid as u32, pids.as_mut_ptr().cast(), bytes) };
            let error = std::io::Error::last_os_error();
            if length < 0 || length == 0 && error.raw_os_error() != Some(0) {
                return Err(Error::io("enumerate owned process group", error));
            }
            if length >= bytes {
                continue;
            }
            if !(length as usize).is_multiple_of(std::mem::size_of::<i32>()) {
                return Err(Error::new(
                    "ownership_conflict",
                    "owned group inventory was incomplete",
                ));
            }
            pids.truncate(length as usize / std::mem::size_of::<i32>());
            for pid in pids.into_iter().filter(|pid| *pid > 0 && *pid != self.pid) {
                let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
                let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
                let length = unsafe {
                    libc::proc_pidinfo(
                        pid,
                        libc::PROC_PIDTBSDINFO,
                        0,
                        info.as_mut_ptr().cast(),
                        size,
                    )
                };
                if length == 0
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                {
                    continue;
                }
                if length != size {
                    return Err(Error::new(
                        "ownership_conflict",
                        "owned group member identity is unavailable",
                    ));
                }
                let info = unsafe { info.assume_init() };
                if info.pbi_pid != pid as u32 || info.pbi_pgid != self.pid as u32 {
                    return Err(Error::new(
                        "ownership_conflict",
                        "observed group member changed identity or escaped",
                    ));
                }
                // SZOMB=5 from installed sys/proc.h; terminal members cannot
                // execute and require their own parent's eventual reap.
                if info.pbi_status != 5 {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        Err(Error::new(
            "ownership_conflict",
            "owned process group exceeds complete inventory bound",
        ))
    }
    #[cfg(not(unix))]
    fn signal(&self, _: Termination, _: bool) -> Result<()> {
        Err(Error::new(
            "prerequisite",
            "owned groups require the supported Unix host",
        ))
    }
}
