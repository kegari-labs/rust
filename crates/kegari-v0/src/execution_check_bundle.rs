//! Finite configured service/check membership. Descriptive records cannot
//! replace the original Control launch token, command owner or joined result.
use super::*;
use crate::control::{CheckBundleLaunch, CheckBundlePlan, CheckBundleStepSpec};
use crate::process_records::CheckMemberKind;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

const CLEANUP_BOUND: Duration = Duration::from_secs(13);
type Waiter = tokio::task::JoinHandle<Result<CommandOutcome>>;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckBundleMemberResult {
    pub ordinal: u32,
    pub process_id: Uuid,
    pub kind: CheckMemberKind,
    pub service: Option<String>,
    pub outcome: Option<CommandOutcome>,
    pub error: Option<Error>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceExecutionRecord {
    pub name: String,
    pub service_process: Uuid,
    pub readiness_process: Uuid,
    pub readiness: CommandOutcome,
    pub terminal: CommandOutcome,
    pub stopped: isolation::ServiceObservation,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckBundleExecutionRecord {
    pub schema_version: u32,
    pub effect_id: Uuid,
    pub plan_digest: String,
    pub services: Vec<ServiceExecutionRecord>,
}
impl CheckBundleExecutionRecord {
    // Descriptive consistency only; the private FailedCheckExecution producer
    // still supplies the actual confined command/unchanged-source authority.
    pub(super) fn services_completed_cleanly(&self) -> bool {
        self.schema_version == 1
            && !self.services.is_empty()
            && self.services.iter().all(|service| {
                [&service.readiness, &service.terminal]
                    .into_iter()
                    .all(|outcome| {
                        outcome.exit_code == Some(0)
                            && outcome.termination_signal.is_none()
                            && !outcome.timed_out
                            && !outcome.output_truncated
                    })
                    && !service.stopped.leader_running
                    && service.stopped.stop_requested
                    && !service.stopped.kill_escalated
            })
    }
}
struct ResolvedMember {
    argv: Vec<String>,
    timeout: Duration,
}
pub(crate) struct PreparedCheckBundle {
    prepared: PreparedCheck,
    plan: CheckBundlePlan,
    members: Vec<ResolvedMember>,
    owner: Arc<CheckBundleOwner>,
}
impl PreparedCheckBundle {
    pub(crate) fn plan(&self) -> &CheckBundlePlan {
        &self.plan
    }
    pub(crate) fn owner(&self) -> Arc<CheckBundleOwner> {
        self.owner.clone()
    }
}
#[derive(Default)]
struct Slot {
    cancellation: Option<Arc<dyn CommandCancellation>>,
    waiter: Option<Waiter>,
    result: Option<Result<CommandOutcome>>,
    joined: bool,
}
struct OwnerState {
    attempted: usize,
    sealed: bool,
    cancel_requested: bool,
    slots: Vec<Slot>,
}
/// Original same-process aggregate. No public constructor or serialized form
/// exists; a query grants neither launch authority nor completed evidence.
pub struct CheckBundleOwner {
    plan: CheckBundlePlan,
    deadline: Instant,
    cleanup_deadline: OnceLock<Instant>,
    uncertain: AtomicBool,
    output_budget: Arc<isolation::CommandOutputBudget>,
    state: Mutex<OwnerState>,
}
impl CheckBundleOwner {
    fn new(plan: CheckBundlePlan, timeout: Duration, output_limit: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(OwnerState {
                attempted: 0,
                sealed: false,
                cancel_requested: false,
                slots: (0..plan.steps().len()).map(|_| Slot::default()).collect(),
            }),
            plan,
            deadline: Instant::now() + timeout,
            cleanup_deadline: OnceLock::new(),
            uncertain: AtomicBool::new(false),
            output_budget: Arc::new(isolation::CommandOutputBudget::new(output_limit)),
        })
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, OwnerState>> {
        self.state
            .lock()
            .map_err(|_| Error::new("operation_uncertain", "check bundle ownership lock failed"))
    }
    fn remaining(&self) -> Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| Error::new("timeout", "original check bundle deadline expired"))
    }
    pub(crate) fn plan(&self) -> &CheckBundlePlan {
        &self.plan
    }
    /// Called only inside the original Control serialized pending-inventory
    /// read, which also binds receipt, exact registry key and durable prefix.
    /// This proves retained ownership, never health, readiness or completion.
    pub(crate) fn verify_live_prefix(&self, plan: &CheckBundlePlan) -> Result<usize> {
        if self.plan.digest() != plan.digest() {
            return Err(Error::new(
                "operation_uncertain",
                "pending bundle plan differs from original owner",
            ));
        }
        let state = self.lock()?;
        if state
            .slots
            .iter()
            .take(state.attempted)
            .any(|slot| slot.cancellation.is_none())
            || self.uncertain.load(Ordering::SeqCst)
        {
            return Err(Error::new(
                "operation_uncertain",
                "pending bundle lacks an actual retained member owner",
            ));
        }
        Ok(state.attempted)
    }
    pub(crate) fn attempted_prefix(&self) -> usize {
        self.state
            .lock()
            .map_or(usize::MAX, |state| state.attempted)
    }
    pub(crate) fn verify_settled(&self, plan: &CheckBundlePlan) -> Result<()> {
        if self.plan.digest() != plan.digest() || !self.exited()? {
            return Err(Error::new(
                "operation_uncertain",
                "check bundle lacks exact settled original ownership",
            ));
        }
        Ok(())
    }
    fn service(&self, ordinal: usize) -> Result<isolation::ServiceObservation> {
        let cancellation = self
            .lock()?
            .slots
            .get(ordinal)
            .and_then(|slot| slot.cancellation.clone())
            .ok_or_else(|| {
                Error::new("operation_uncertain", "service has no actual command owner")
            })?;
        cancellation.service_observation()
    }
    fn require_service_live(&self, ordinal: usize) -> Result<()> {
        let observed = self.service(ordinal)?;
        if !observed.leader_running || observed.stop_requested || observed.kill_escalated {
            return Err(Error::new(
                "service_not_ready",
                "original service leader is not running within its declared lifecycle",
            ));
        }
        Ok(())
    }
    fn result(&self, ordinal: usize) -> Result<CommandOutcome> {
        self.lock()?
            .slots
            .get(ordinal)
            .and_then(|slot| slot.result.clone())
            .ok_or_else(|| {
                Error::new("operation_uncertain", "bundle member result is unavailable")
            })?
    }
    async fn wait(self: &Arc<Self>, ordinal: usize, deadline: Instant) -> Result<CommandOutcome> {
        let waiter = {
            let mut state = self.lock()?;
            let slot = state
                .slots
                .get_mut(ordinal)
                .ok_or_else(|| Error::invalid("bundle member outside plan"))?;
            if let Some(result) = &slot.result {
                return result.clone();
            }
            slot.waiter.take().ok_or_else(|| {
                Error::new(
                    "operation_uncertain",
                    "bundle waiter is absent or already borrowed",
                )
            })?
        };
        let mut retained = RetainedWaiter {
            owner: self.clone(),
            ordinal,
            waiter: Some(waiter),
        };
        let left = deadline.saturating_duration_since(Instant::now());
        let joined = tokio::time::timeout(
            left,
            retained.waiter.as_mut().expect("retained actual waiter"),
        )
        .await
        .map_err(|_| {
            Error::new(
                "timeout",
                "bundle member did not settle within its original bound",
            )
        })?;
        retained.waiter.take();
        let result = match joined {
            Ok(result) => result,
            Err(_) => {
                self.uncertain.store(true, Ordering::SeqCst);
                Err(Error::new(
                    "operation_uncertain",
                    "owned bundle waiter interrupted",
                ))
            }
        };
        let mut state = self.lock()?;
        state.slots[ordinal].joined = true;
        state.slots[ordinal].result = Some(result.clone());
        result
    }
    fn records(&self) -> Result<Vec<CheckBundleMemberResult>> {
        let state = self.lock()?;
        Ok(self
            .plan
            .steps()
            .iter()
            .take(state.attempted)
            .map(|step| {
                let result = state.slots[step.ordinal() as usize].result.as_ref();
                CheckBundleMemberResult {
                    ordinal: step.ordinal(),
                    process_id: step.process_id(),
                    kind: step.kind().clone(),
                    service: step.service().map(str::to_owned),
                    outcome: result.and_then(|r| r.as_ref().ok()).cloned(),
                    error: result.and_then(|r| r.as_ref().err()).cloned(),
                }
            })
            .collect())
    }
    async fn cleanup(self: &Arc<Self>) -> Result<()> {
        // Resuming a dropped cleanup future does not renew the original budget.
        let deadline = *self
            .cleanup_deadline
            .get_or_init(|| Instant::now() + CLEANUP_BOUND);
        self.lock()?.sealed = true;
        let mut first = self.cancel().err();
        for ordinal in (0..self.attempted_prefix().min(self.plan.steps().len())).rev() {
            if let Err(error) = self.wait(ordinal, deadline).await {
                first.get_or_insert(error);
                // A timeout cannot turn a lost result into a known outcome.
                // Abort only an unjoined retained waiter; the native supervisor
                // retains its independent finite child/output cleanup obligation.
                let waiter = self.lock()?.slots[ordinal].waiter.take();
                if let Some(mut waiter) = waiter {
                    self.uncertain.store(true, Ordering::SeqCst);
                    waiter.abort();
                    let _ = tokio::time::timeout(Duration::from_secs(1), &mut waiter).await;
                }
            }
        }
        if let Err(error) = self.verify_settled(&self.plan) {
            first.get_or_insert(error);
        }
        first.map_or(Ok(()), Err)
    }
}
impl CommandCancellation for CheckBundleOwner {
    fn check_bundle_owner(&self) -> Option<&CheckBundleOwner> {
        Some(self)
    }
    fn cancel(&self) -> Result<()> {
        let handles = {
            let mut state = self.lock()?;
            state.cancel_requested = true;
            state
                .slots
                .iter()
                .rev()
                .filter_map(|slot| slot.cancellation.clone())
                .collect::<Vec<_>>()
        };
        let mut failure = None;
        for handle in handles {
            if let Err(error) = handle.cancel() {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
    fn exited(&self) -> Result<bool> {
        if self.uncertain.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let handles = {
            let state = self.lock()?;
            if !state.sealed
                || state
                    .slots
                    .iter()
                    .take(state.attempted)
                    .any(|slot| !slot.joined)
            {
                return Ok(false);
            }
            let handles = state
                .slots
                .iter()
                .take(state.attempted)
                .filter_map(|slot| slot.cancellation.clone())
                .collect::<Vec<_>>();
            if handles.len() != state.attempted {
                return Ok(false);
            }
            handles
        };
        for handle in handles {
            if !handle.exited()? {
                return Ok(false);
            }
        }
        Ok(!self.uncertain.load(Ordering::SeqCst))
    }
}
struct RetainedWaiter {
    owner: Arc<CheckBundleOwner>,
    ordinal: usize,
    waiter: Option<Waiter>,
}
impl Drop for RetainedWaiter {
    fn drop(&mut self) {
        if let Some(waiter) = self.waiter.take() {
            match self.owner.state.lock() {
                Ok(mut state) => state.slots[self.ordinal].waiter = Some(waiter),
                Err(_) => {
                    self.owner.uncertain.store(true, Ordering::SeqCst);
                    waiter.abort();
                }
            }
        }
    }
}
struct CleanupOnDrop {
    owner: Arc<CheckBundleOwner>,
    armed: bool,
}
impl Drop for CleanupOnDrop {
    fn drop(&mut self) {
        if self.armed {
            let owner = self.owner.clone();
            let _ = owner.cancel();
            tokio::spawn(async move {
                let _ = owner.cleanup().await;
            });
        }
    }
}
pub(super) fn launch_digest(isolation: &ValidatedIsolation, argv: &[String]) -> Result<String> {
    canonical_digest(1, &(isolation.digest(), &isolation.plan().cwd, argv))
}
impl LocalExecutor<'_> {
    pub(crate) fn prepare_bundle(
        &self,
        prepared: PreparedCheck,
        effect_id: Uuid,
    ) -> Result<PreparedCheckBundle> {
        if prepared.resolved.services.is_empty()
            || self
                .isolation
                .plan()
                .managed_execution
                .as_ref()
                .is_none_or(|policy| !policy.supports_check_bundles())
        {
            return Err(Error::new(
                "unsupported_check_bundle",
                "check services require explicit check_bundle_v1 policy",
            ));
        }
        self.preflight_check(&prepared)?;
        let mut specs = Vec::new();
        let mut members = Vec::new();
        for service in &prepared.resolved.services {
            for (kind, argv, timeout) in [
                (
                    CheckMemberKind::Service,
                    &service.argv,
                    service.max_lifetime_ms,
                ),
                (
                    CheckMemberKind::Readiness,
                    &service.readiness.argv,
                    service.readiness.timeout_ms,
                ),
            ] {
                self.isolation.resolve_executable(&argv[0])?;
                specs.push(CheckBundleStepSpec {
                    process_id: Uuid::new_v4(),
                    kind,
                    service: Some(service.name.clone()),
                    launch_digest: launch_digest(self.isolation, argv)?,
                });
                members.push(ResolvedMember {
                    argv: argv.clone(),
                    timeout: Duration::from_millis(timeout.min(self.limits.command_timeout_ms)),
                });
            }
        }
        specs.push(CheckBundleStepSpec {
            process_id: Uuid::new_v4(),
            kind: CheckMemberKind::Check,
            service: None,
            launch_digest: launch_digest(self.isolation, &prepared.resolved.argv)?,
        });
        members.push(ResolvedMember {
            argv: prepared.resolved.argv.clone(),
            timeout: Duration::from_millis(self.limits.command_timeout_ms),
        });
        let plan = CheckBundlePlan::new(
            effect_id,
            prepared.binding.operation_id,
            prepared.binding.runtime_id,
            canonical_digest(
                1,
                &(
                    &prepared.binding,
                    &prepared.workflow_digest,
                    &prepared.check_name,
                    &prepared.resolved.services,
                    // Only check execution/capture bounds belong to this
                    // identity. Media-tool limits include floating point
                    // duration and do not govern any bundle member or snapshot.
                    serde_json::json!({
                        "command_timeout_ms": self.limits.command_timeout_ms,
                        "output_bytes": self.limits.output_bytes,
                        "artifact_bytes": self.limits.artifact_bytes,
                        "source_bytes": self.limits.source_bytes,
                        "source_files": self.limits.source_files,
                    }),
                ),
            )?,
            self.isolation.digest().into(),
            specs,
        )?;
        let owner = CheckBundleOwner::new(
            plan.clone(),
            Duration::from_millis(self.limits.command_timeout_ms),
            self.limits.output_bytes,
        );
        Ok(PreparedCheckBundle {
            prepared,
            plan,
            members,
            owner,
        })
    }
    pub(crate) fn preflight_bundle(&self, bundle: &PreparedCheckBundle) -> Result<()> {
        self.preflight_check(&bundle.prepared)?;
        if self
            .isolation
            .plan()
            .managed_execution
            .as_ref()
            .is_none_or(|policy| !policy.supports_check_bundles())
            || bundle.plan.isolation_digest() != self.isolation.digest()
        {
            return Err(Error::new(
                "unsupported_check_bundle",
                "prepared bundle lost its explicit managed policy",
            ));
        }
        bundle.owner.remaining()?;
        for member in &bundle.members {
            self.isolation.resolve_executable(&member.argv[0])?;
        }
        Ok(())
    }
    pub(crate) fn launch_bundle_member(
        &self,
        bundle: &PreparedCheckBundle,
        launch: CheckBundleLaunch,
    ) -> Result<()> {
        let ordinal = launch.ordinal() as usize;
        let step = bundle
            .plan
            .steps()
            .get(ordinal)
            .ok_or_else(|| Error::invalid("bundle ordinal outside plan"))?;
        let resolved = &bundle.members[ordinal];
        if launch.plan_digest() != bundle.plan.digest()
            || launch.process_id() != step.process_id()
            || launch.launch_digest() != launch_digest(self.isolation, &resolved.argv)?
            || launch.permit().operation_id != bundle.prepared.binding.operation_id
            || launch.permit().lease.runtime_id != bundle.prepared.binding.runtime_id
            || self.isolation.digest() != bundle.plan.isolation_digest()
        {
            return Err(Error::new(
                "check_bundle_identity",
                "launch token differs from actual prepared bundle",
            ));
        }
        let timeout = resolved.timeout.min(bundle.owner.remaining()?);
        self.boundary.preflight(self.isolation)?;
        let mut state = bundle.owner.lock()?;
        if state.sealed || state.cancel_requested || ordinal != state.attempted {
            return Err(Error::new(
                "admission_closed",
                "bundle is cancelled, sealed or out of order",
            ));
        }
        // The original aggregate owns the attempted slot before fallible spawn.
        // No await or fallible lock separates the returned handle and retention.
        state.attempted += 1;
        let command = match self.boundary.start_bundle_member(
            self.isolation,
            &resolved.argv,
            isolation::BundleCommandLimits {
                timeout,
                deadline: bundle.owner.deadline,
                output_limit: self.limits.output_bytes,
                output_budget: bundle.owner.output_budget.clone(),
            },
            launch,
        ) {
            Ok(command) => command,
            Err(error) => {
                state.slots[ordinal].result = Some(Err(error.clone()));
                bundle.owner.uncertain.store(true, Ordering::SeqCst);
                return Err(error);
            }
        };
        state.slots[ordinal].cancellation = Some(command.cancellation());
        state.slots[ordinal].waiter = Some(tokio::spawn(command.wait()));
        Ok(())
    }
    fn check_member_outcome(
        &self,
        bundle: &PreparedCheckBundle,
        ordinal: usize,
        outcome: &CommandOutcome,
    ) -> Result<()> {
        if outcome.argv != bundle.members[ordinal].argv
            || outcome.cwd != bundle.prepared.resolved.cwd
            || outcome.isolation_digest != self.isolation.digest()
        {
            return Err(Error::new(
                "execution_identity",
                "bundle result differs from exact member identity",
            ));
        }
        if outcome.exit_code != Some(0)
            || outcome.termination_signal.is_some()
            || outcome.timed_out
            || outcome.output_truncated
        {
            return Err(Error::new(
                "check_bundle_failed",
                "bundle readiness/service failed, timed out or exceeded output bound",
            ));
        }
        Ok(())
    }
    fn finished_bundle(
        &self,
        bundle: &PreparedCheckBundle,
    ) -> Result<(CommandOutcome, CheckBundleExecutionRecord)> {
        let mut services = Vec::new();
        for ordinal in (0..bundle.members.len() - 1).step_by(2) {
            let terminal = bundle.owner.result(ordinal)?;
            self.check_member_outcome(bundle, ordinal, &terminal)?;
            let readiness = bundle.owner.result(ordinal + 1)?;
            self.check_member_outcome(bundle, ordinal + 1, &readiness)?;
            let stopped = bundle.owner.service(ordinal)?;
            if stopped.leader_running || stopped.kill_escalated || !stopped.stop_requested {
                return Err(Error::new(
                    "check_bundle_failed",
                    "service did not complete graceful owned shutdown",
                ));
            }
            services.push(ServiceExecutionRecord {
                name: bundle.plan.steps()[ordinal]
                    .service()
                    .expect("validated service step")
                    .into(),
                service_process: bundle.plan.steps()[ordinal].process_id(),
                readiness_process: bundle.plan.steps()[ordinal + 1].process_id(),
                readiness,
                terminal,
                stopped,
            });
        }
        let command = bundle.owner.result(bundle.members.len() - 1)?;
        let record = CheckBundleExecutionRecord {
            schema_version: 1,
            effect_id: bundle.plan.effect_id(),
            plan_digest: bundle.plan.digest().into(),
            services,
        };
        Ok((command, record))
    }
    pub(crate) async fn complete_bundle(
        &self,
        bundle: PreparedCheckBundle,
        mut launch: impl FnMut(&PreparedCheckBundle, u32) -> Result<()>,
        mut persist: impl FnMut(&CheckJournalEntry) -> Result<()>,
    ) -> Result<CheckCompletion> {
        let mut cleanup = CleanupOnDrop {
            owner: bundle.owner.clone(),
            armed: true,
        };
        let run = async {
            for ordinal in 0..bundle.members.len() {
                if ordinal + 1 == bundle.members.len() {
                    for service in (0..ordinal).step_by(2) {
                        bundle.owner.require_service_live(service)?;
                    }
                } else if ordinal % 2 == 1 {
                    bundle.owner.require_service_live(ordinal - 1)?;
                }
                bundle.owner.remaining()?;
                launch(&bundle, ordinal as u32)?;
                if ordinal % 2 == 1 || ordinal + 1 == bundle.members.len() {
                    let outcome = bundle.owner.wait(ordinal, bundle.owner.deadline).await?;
                    if ordinal + 1 != bundle.members.len() {
                        self.check_member_outcome(&bundle, ordinal, &outcome)?;
                        bundle.owner.require_service_live(ordinal - 1)?;
                    }
                }
            }
            for service in (0..bundle.members.len() - 1).step_by(2) {
                bundle.owner.require_service_live(service)?;
            }
            Ok::<(), Error>(())
        }
        .await;
        let closed = bundle.owner.cleanup().await;
        cleanup.armed = false;
        let result = run.and(closed).and_then(|()| self.finished_bundle(&bundle));
        persist(&CheckJournalEntry::BundleStopped {
            operation_id: bundle.prepared.binding.operation_id,
            plan_digest: bundle.plan.digest().into(),
            members: bundle.owner.records()?,
            error: result.as_ref().err().cloned(),
        })?;
        let (command, record) = result?;
        // Every declared writer is stopped and joined before this source/artifact
        // snapshot. Readiness was only the finite configured check observation.
        self.complete_check_result(bundle.prepared, command, Some(record), persist)
    }
}

#[cfg(test)]
#[path = "execution_check_bundle_tests.rs"]
mod tests;
