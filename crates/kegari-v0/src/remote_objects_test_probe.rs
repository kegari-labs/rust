//! Deterministic boot identity for owned process fixtures. This deliberately
//! does not qualify native host boot identity or detached-process containment.
use crate::{
    error::{Error, Result},
    platform::{ProcessIdentity, ProcessObservation, ProcessProbe},
};
use std::path::{Path, PathBuf};
use uuid::Uuid;
pub struct FixtureBootProbe {
    boot: Uuid,
}
impl FixtureBootProbe {
    pub fn new() -> Self {
        Self {
            boot: Uuid::new_v4(),
        }
    }
}
impl FixtureBootProbe {
    fn process(&self, pid: u32) -> Result<Option<ProcessIdentity>> {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        let length = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if length == 0 {
            return Ok(None);
        }
        if length != size {
            return Err(Error::invalid("incomplete fixture process observation"));
        }
        let info = unsafe { info.assume_init() };
        let mut path = [0u8; 4096];
        let length =
            unsafe { libc::proc_pidpath(pid as i32, path.as_mut_ptr().cast(), path.len() as u32) };
        if length <= 0 {
            return Ok(None);
        }
        let executable = PathBuf::from(
            std::str::from_utf8(&path[..length as usize])
                .map_err(|_| Error::invalid("fixture process path"))?
                .trim_end_matches('\0'),
        );
        Ok(Some(ProcessIdentity {
            pid,
            parent_pid: info.pbi_ppid,
            process_group: info.pbi_pgid,
            uid: info.pbi_uid,
            boot_id: self.boot,
            started_seconds: info.pbi_start_tvsec,
            started_microseconds: info.pbi_start_tvusec,
            executable,
        }))
    }
}
impl ProcessProbe for FixtureBootProbe {
    fn current(&self) -> Result<ProcessIdentity> {
        self.process(std::process::id())?
            .ok_or_else(|| Error::invalid("fixture host absent"))
    }
    fn observe(&self, expected: &ProcessIdentity) -> Result<ProcessObservation> {
        Ok(match self.process(expected.pid)? {
            Some(current) if &current == expected => ProcessObservation::Running,
            Some(_) => ProcessObservation::DifferentIdentity,
            None => ProcessObservation::Gone,
        })
    }
    fn matching_executable(&self, path: &Path) -> Result<Vec<ProcessIdentity>> {
        let mut pids = [0i32; 4096];
        let bytes = std::mem::size_of_val(&pids) as i32;
        // PROC_PPID_ONLY=6, verified from the installed Apple SDK proc_info.h.
        let length =
            unsafe { libc::proc_listpids(6, std::process::id(), pids.as_mut_ptr().cast(), bytes) };
        if length < 0 || length >= bytes || !(length as usize).is_multiple_of(4) {
            return Err(Error::invalid("fixture child inventory incomplete"));
        }
        let mut found = Vec::new();
        for pid in &pids[..length as usize / 4] {
            if *pid > 0
                && let Some(p) = self.process(*pid as u32)?
                && p.executable == path
            {
                found.push(p);
            }
        }
        Ok(found)
    }
}
