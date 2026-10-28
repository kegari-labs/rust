//! Owned Unix process groups. Observe terminal leaders without reaping, finish
//! their group while the leader PID is still reserved, then let the Child owner
//! reap. This proves group cleanup, not containment of descendants which escape
//! into a new session/group. Such descendants are outside the managed v0 profile.
use kegari_types::error::{Error, Result};

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
    identity: uuid::Uuid,
    reaped: bool,
    forced_kill: bool,
}

/// Evidence from reaping the exact owned child after observing its group clear.
/// This is neither a live process capability nor arbitrary-descendant closure.
/// It cannot be deserialized or constructed from a retained PID/exit code.
pub struct OwnedGroupExit {
    identity: uuid::Uuid,
    pid: u32,
    status: std::process::ExitStatus,
}
fn spawn_error(context: &str, error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::TimedOut {
        Error::new("timeout", "original process launch deadline expired")
    } else {
        Error::io(context, error)
    }
}
impl OwnedGroupExit {
    pub fn status(&self) -> std::process::ExitStatus {
        self.status
    }
    pub(crate) fn binding(&self) -> (uuid::Uuid, u32) {
        (self.identity, self.pid)
    }
}
impl OwnedGroup {
    #[cfg(unix)]
    pub fn spawn_std(command: &mut std::process::Command) -> Result<(std::process::Child, Self)> {
        Self::spawn_std_before(command, None)
    }
    #[cfg(unix)]
    pub(crate) fn spawn_std_before(
        command: &mut std::process::Command,
        deadline: Option<std::time::Instant>,
    ) -> Result<(std::process::Child, Self)> {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        let child = crate::process_spawn::spawn_std(command, deadline)
            .map_err(|error| spawn_error("spawn owned process group", error))?;
        let group = Self::created(child.id());
        Ok((child, group))
    }
    #[cfg(unix)]
    pub fn spawn_tokio(
        command: &mut tokio::process::Command,
    ) -> Result<(tokio::process::Child, Self)> {
        Self::spawn_tokio_before(command, None)
    }
    #[cfg(unix)]
    pub(crate) fn spawn_tokio_before(
        command: &mut tokio::process::Command,
        deadline: Option<std::time::Instant>,
    ) -> Result<(tokio::process::Child, Self)> {
        command.process_group(0);
        let child = crate::process_spawn::spawn_tokio(command, deadline)
            .map_err(|error| spawn_error("spawn owned async process group", error))?;
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
            identity: uuid::Uuid::new_v4(),
            reaped: false,
            forced_kill: false,
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
            identity: uuid::Uuid::new_v4(),
            reaped: false,
            forced_kill: false,
        };
        // Also establish that the unreaped process is our actual child. WNOWAIT
        // does not consume a quick child's status during construction.
        group.terminal_without_reaping()?;
        group.verify_group()?;
        Ok(group)
    }
    pub(crate) fn bind_child(&self, pid: u32) -> Result<(uuid::Uuid, u32)> {
        if self.reaped || pid != self.pid as u32 {
            return Err(Error::new(
                "ownership_conflict",
                "child differs from the original unreaped owned group",
            ));
        }
        Ok((self.identity, pid))
    }
    /// Observe this original unreaped leader without treating a terminal leader
    /// with still-draining group members as a live service.
    pub(crate) fn leader_running_tokio(&self, child: &tokio::process::Child) -> Result<bool> {
        self.require_group_exit_observation()?;
        let pid = child.id().ok_or_else(|| {
            Error::new(
                "ownership_conflict",
                "owned service child was already reaped",
            )
        })?;
        self.bind_child(pid)?;
        Ok(!self.verify_live_group_or_terminal()?)
    }
    pub fn try_reap_std(
        &mut self,
        child: &mut std::process::Child,
    ) -> Result<Option<OwnedGroupExit>> {
        self.require_group_exit_observation()?;
        self.bind_child(child.id())?;
        if !self.observe_exit_before_reap()? {
            return Ok(None);
        }
        let status = child
            .try_wait()
            .map_err(|error| Error::io("reap original owned child", error))?;
        Ok(status.map(|status| self.record_reap(status)))
    }
    pub fn try_reap_tokio(
        &mut self,
        child: &mut tokio::process::Child,
    ) -> Result<Option<OwnedGroupExit>> {
        self.require_group_exit_observation()?;
        let pid = child.id().ok_or_else(|| {
            Error::new("ownership_conflict", "owned async child was already reaped")
        })?;
        self.bind_child(pid)?;
        if !self.observe_exit_before_reap()? {
            return Ok(None);
        }
        let status = child
            .try_wait()
            .map_err(|error| Error::io("reap original owned async child", error))?;
        Ok(status.map(|status| self.record_reap(status)))
    }
    fn record_reap(&mut self, status: std::process::ExitStatus) -> OwnedGroupExit {
        self.reaped = true;
        OwnedGroupExit {
            identity: self.identity,
            pid: self.pid as u32,
            status,
        }
    }
    fn require_group_exit_observation(&self) -> Result<()> {
        if !cfg!(target_os = "macos") {
            return Err(Error::new(
                "prerequisite",
                "owned group exit evidence requires the supported macOS group observation",
            ));
        }
        Ok(())
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
    /// Wait only for terminal/group closure of the original unreaped Child.
    /// A false result retains the existing owner and grants no live identity.
    /// The callback is local cancellation/deadline checking, not a new admission.
    pub(crate) fn await_exit_before_reap(
        &mut self,
        deadline: std::time::Instant,
        mut current: impl FnMut() -> Result<()>,
    ) -> Result<bool> {
        loop {
            current()?;
            if std::time::Instant::now() >= deadline {
                return Ok(false);
            }
            let terminal = self.observe_exit_before_reap()?;
            current()?;
            if std::time::Instant::now() >= deadline {
                return Ok(false);
            }
            if terminal {
                return Ok(true);
            }
            std::thread::sleep(
                std::time::Duration::from_millis(1)
                    .min(deadline.saturating_duration_since(std::time::Instant::now())),
            );
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
    pub(crate) fn forced_kill_requested(&self) -> bool {
        self.forced_kill
    }
    #[cfg(unix)]
    fn signal(&mut self, signal: Termination, terminal: bool) -> Result<()> {
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
        // Retain the actual attempted group signal even if its syscall result
        // becomes uncertain; a clean leader status cannot erase forced cleanup.
        if signal == libc::SIGKILL {
            self.forced_kill = true;
        }
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
        live_or_changed_members(self.pid, &mut MacGroupInventory(self.pid))
    }
    #[cfg(not(unix))]
    fn signal(&mut self, _: Termination, _: bool) -> Result<()> {
        Err(Error::new(
            "prerequisite",
            "owned groups require the supported Unix host",
        ))
    }
}

#[cfg(any(target_os = "macos", test))]
const MAX_GROUP_MEMBERS: usize = 65_536;

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GroupMember {
    pid: i32,
    group: i32,
    started_seconds: u64,
    started_microseconds: u64,
    terminal: bool,
}

#[cfg(any(target_os = "macos", test))]
trait GroupInventory {
    fn members(&mut self) -> Result<Vec<i32>>;
    fn observe(&mut self, pid: i32) -> Result<Option<GroupMember>>;
}

#[cfg(any(target_os = "macos", test))]
fn group_members(
    leader: i32,
    inventory: &mut impl GroupInventory,
) -> Result<std::collections::BTreeSet<i32>> {
    let members = inventory.members()?;
    if members.len() > MAX_GROUP_MEMBERS {
        return Err(Error::new(
            "ownership_conflict",
            "owned process group exceeds complete inventory bound",
        ));
    }
    let mut unique = std::collections::BTreeSet::new();
    for pid in members.into_iter().filter(|pid| *pid > 0 && *pid != leader) {
        if !unique.insert(pid) {
            return Err(Error::new(
                "ownership_conflict",
                "owned group inventory repeats a member",
            ));
        }
    }
    Ok(unique)
}

#[cfg(any(target_os = "macos", test))]
fn observe_member(
    leader: i32,
    pid: i32,
    inventory: &mut impl GroupInventory,
) -> Result<Option<GroupMember>> {
    let observed = inventory.observe(pid)?;
    if observed.is_some_and(|member| member.pid != pid || member.group != leader) {
        return Err(Error::new(
            "ownership_conflict",
            "observed group member changed identity or escaped",
        ));
    }
    Ok(observed)
}

#[cfg(any(target_os = "macos", test))]
fn live_or_changed_members(leader: i32, inventory: &mut impl GroupInventory) -> Result<bool> {
    let first = group_members(leader, inventory)?;
    let mut terminal = std::collections::BTreeMap::new();
    for pid in &first {
        let Some(member) = observe_member(leader, *pid, inventory)? else {
            // Disappearance is not enough: it could follow an unobserved fork.
            // Let the existing bounded owner loop obtain a fresh complete census.
            return Ok(true);
        };
        if !member.terminal {
            return Ok(true);
        }
        terminal.insert(*pid, member);
    }

    // A member can fork after the first PID list and become terminal before its
    // metadata read. Only after every listed member is terminal do we obtain a
    // second complete list. This is a conservative pair of observations, not an
    // atomic kernel snapshot or a claim about descendants outside this group.
    let second = group_members(leader, inventory)?;
    if second != first {
        return Ok(true);
    }
    for pid in second {
        let Some(member) = observe_member(leader, pid, inventory)? else {
            return Ok(true);
        };
        let original = &terminal[&pid];
        if member.started_seconds != original.started_seconds
            || member.started_microseconds != original.started_microseconds
        {
            return Err(Error::new(
                "ownership_conflict",
                "owned group member birth identity changed between observations",
            ));
        }
        if !member.terminal {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(target_os = "macos")]
struct MacGroupInventory(i32);

#[cfg(target_os = "macos")]
impl GroupInventory for MacGroupInventory {
    fn members(&mut self) -> Result<Vec<i32>> {
        // PROC_PGRP_ONLY/PROC_PIDTBSDINFO and birth fields are declared in the
        // installed Apple SDK sys/proc_info.h. Saturated/denied data prove nothing.
        for capacity in [256usize, 1024, 4096, 16384, 65536] {
            let mut pids = vec![0i32; capacity];
            let bytes = (capacity * std::mem::size_of::<i32>()) as i32;
            unsafe {
                *libc::__error() = 0;
            }
            let length =
                unsafe { libc::proc_listpids(2, self.0 as u32, pids.as_mut_ptr().cast(), bytes) };
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
            return Ok(pids);
        }
        Err(Error::new(
            "ownership_conflict",
            "owned process group exceeds complete inventory bound",
        ))
    }
    fn observe(&mut self, pid: i32) -> Result<Option<GroupMember>> {
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
        if length == 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return Ok(None);
        }
        if length != size {
            return Err(Error::new(
                "ownership_conflict",
                "owned group member identity is unavailable",
            ));
        }
        let info = unsafe { info.assume_init() };
        Ok(Some(GroupMember {
            pid: info.pbi_pid as i32,
            group: info.pbi_pgid as i32,
            started_seconds: info.pbi_start_tvsec,
            started_microseconds: info.pbi_start_tvusec,
            // SZOMB=5 from installed sys/proc.h; terminal members cannot execute.
            terminal: info.pbi_status == 5,
        }))
    }
}

#[cfg(test)]
mod inventory_tests {
    use super::*;
    use std::collections::VecDeque;

    enum Step {
        Members(Result<Vec<i32>>),
        Observe(i32, Result<Option<GroupMember>>),
    }
    struct Inventory(VecDeque<Step>);
    impl Inventory {
        fn new(steps: Vec<Step>) -> Self {
            Self(steps.into())
        }
        fn assert_consumed(&self) {
            assert!(self.0.is_empty(), "all prescribed observations must occur");
        }
    }
    impl GroupInventory for Inventory {
        fn members(&mut self) -> Result<Vec<i32>> {
            let Some(Step::Members(result)) = self.0.pop_front() else {
                panic!("unexpected inventory read or observation ordering");
            };
            result
        }
        fn observe(&mut self, pid: i32) -> Result<Option<GroupMember>> {
            let Some(Step::Observe(expected, result)) = self.0.pop_front() else {
                panic!("unexpected member observation or ordering");
            };
            assert_eq!(pid, expected);
            result
        }
    }
    fn terminal(pid: i32) -> GroupMember {
        GroupMember {
            pid,
            group: 10,
            started_seconds: 100,
            started_microseconds: 200,
            terminal: true,
        }
    }
    fn observed(member: GroupMember) -> Step {
        Step::Observe(member.pid, Ok(Some(member)))
    }
    fn members(pids: &[i32]) -> Step {
        Step::Members(Ok(pids.to_vec()))
    }

    #[test]
    fn fork_after_first_inventory_is_not_mistaken_for_group_exit() {
        // Parent 20 forks 21 after the first list, then becomes terminal before
        // its metadata read. A second list must follow that terminal observation.
        let mut inventory = Inventory::new(vec![
            members(&[10, 20]),
            observed(terminal(20)),
            members(&[10, 20, 21]),
        ]);
        assert!(live_or_changed_members(10, &mut inventory).unwrap());
        inventory.assert_consumed();

        // A fresh owner observation still finds the child live. These injected
        // census tests exercise only the predicate, never mint OwnedGroupExit.
        let mut inventory = Inventory::new(vec![
            members(&[10, 21]),
            observed(GroupMember {
                terminal: false,
                ..terminal(21)
            }),
        ]);
        assert!(live_or_changed_members(10, &mut inventory).unwrap());
        inventory.assert_consumed();
    }

    #[test]
    fn stable_terminal_members_require_two_matching_birth_observations() {
        let mut inventory = Inventory::new(vec![
            members(&[10, 20, 21]),
            observed(terminal(20)),
            observed(terminal(21)),
            members(&[21, 10, 20]),
            observed(terminal(20)),
            observed(terminal(21)),
        ]);
        assert!(!live_or_changed_members(10, &mut inventory).unwrap());
        inventory.assert_consumed();
    }

    #[test]
    fn same_pid_with_changed_birth_identity_cannot_complete_the_group() {
        for replacement in [
            GroupMember {
                started_seconds: 101,
                ..terminal(20)
            },
            GroupMember {
                started_microseconds: 201,
                ..terminal(20)
            },
        ] {
            let mut inventory = Inventory::new(vec![
                members(&[10, 20]),
                observed(terminal(20)),
                members(&[10, 20]),
                observed(replacement),
            ]);
            let error = live_or_changed_members(10, &mut inventory).unwrap_err();
            assert_eq!(error.code, "ownership_conflict");
            assert_eq!(
                error.message,
                "owned group member birth identity changed between observations"
            );
            inventory.assert_consumed();
        }
    }

    #[test]
    fn disappearing_or_unavailable_second_observation_does_not_prove_exit() {
        for steps in [
            vec![members(&[10, 20]), observed(terminal(20)), members(&[10])],
            vec![
                members(&[10, 20]),
                observed(terminal(20)),
                members(&[10, 20]),
                Step::Observe(20, Ok(None)),
            ],
        ] {
            let mut inventory = Inventory::new(steps);
            assert!(live_or_changed_members(10, &mut inventory).unwrap());
            inventory.assert_consumed();
        }
        let mut inventory = Inventory::new(vec![
            members(&[10]),
            Step::Members(Err(Error::new("inventory_denied", "second read denied"))),
        ]);
        assert_eq!(
            live_or_changed_members(10, &mut inventory)
                .unwrap_err()
                .code,
            "inventory_denied"
        );
        inventory.assert_consumed();
    }

    #[test]
    fn excessive_or_duplicate_inventory_refuses_before_member_reads() {
        for pids in [vec![20; MAX_GROUP_MEMBERS + 1], vec![10, 20, 20]] {
            let mut inventory = Inventory::new(vec![Step::Members(Ok(pids))]);
            assert_eq!(
                live_or_changed_members(10, &mut inventory)
                    .unwrap_err()
                    .code,
                "ownership_conflict"
            );
            inventory.assert_consumed();
        }
    }
}
