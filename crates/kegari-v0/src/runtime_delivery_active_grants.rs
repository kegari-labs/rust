//! Native grant observation for an actual already-admitted parent phase.
//! New admissions/readiness keep the ordinary current projection reader.
use super::child::ActivePhase;
use super::*;
use crate::control::Permit;

/// Contains no supplied ParentProjection. Every read obtains the actual retained
/// snapshot under the exact pinned owner binding. The in-gate methods never
/// reenter Control; the surrounding callback validates this same permit.
#[derive(Clone)]
pub(crate) struct ActiveParentGrantObserver {
    runtime: Arc<RuntimeCoordinator>,
    permit: Permit,
    gate: FreshGate,
    configuration: Config,
}
impl ActiveParentGrantObserver {
    pub(crate) fn new(
        runtime: Arc<RuntimeCoordinator>,
        permit: Permit,
        gate: FreshGate,
        configuration: Config,
    ) -> Result<Self> {
        if configuration.runtime.id != permit.lease.runtime_id
            || crate::config::config_revision(&configuration)? != gate.config_revision()
            || permit.work_key != gate.work_key()?
            || gate.work().child.is_some()
        {
            return Err(Error::new(
                "scope_violation",
                "active grant reader differs from actual pinned parent admission",
            ));
        }
        Ok(Self {
            runtime,
            permit,
            gate,
            configuration,
        })
    }
    pub(crate) fn configuration(&self) -> &Config {
        &self.configuration
    }
    pub(crate) fn require_configuration(&self, configuration: &Config) -> Result<()> {
        if crate::config::config_revision(&self.configuration)?
            != crate::config::config_revision(configuration)?
        {
            return Err(Error::new(
                "stale_config",
                "active grant configuration differs",
            ));
        }
        Ok(())
    }
    pub(crate) fn require_admission(
        &self,
        permit: &Permit,
        gate: &FreshGate,
        configuration: &Config,
    ) -> Result<()> {
        self.require_configuration(configuration)?;
        if self.permit.operation_id != permit.operation_id
            || self.permit.lease != permit.lease
            || self.permit.scope_digest != permit.scope_digest
            || self.permit.work_key != permit.work_key
            || self.permit.control_generation != permit.control_generation
            || self.gate.digest()? != gate.digest()?
        {
            return Err(Error::new(
                "scope_violation",
                "active grant permit or gate differs",
            ));
        }
        Ok(())
    }
    pub(crate) fn observe(
        &self,
        authority: &FreshGate,
    ) -> Result<Vec<crate::decisions::VerifiedRetryGrant>> {
        let mut grants = None;
        self.runtime.control.validate_permit(&self.permit, || {
            grants = Some(self.observe_in_gate(authority)?);
            Ok(())
        })?;
        grants.ok_or_else(|| Error::new("ownership_conflict", "active grant observation absent"))
    }
    /// Only invoked by the same actual permit's serialized final predicate (or
    /// its read-only preflight, which confers no authority). No Control reentry.
    pub(crate) fn observe_in_gate(
        &self,
        authority: &FreshGate,
    ) -> Result<Vec<crate::decisions::VerifiedRetryGrant>> {
        let parent = self.parent_in_gate()?;
        let work = authority.work();
        if authority.workspace_id() != self.gate.workspace_id()
            || authority.config_revision() != self.gate.config_revision()
            || authority.workflow_digest() != self.gate.workflow_digest()
            || work.linear_endpoint != self.gate.work().linear_endpoint
            || work.linear_workspace != self.gate.work().linear_workspace
            || work.parent != self.gate.work().parent
        {
            return Err(Error::new(
                "scope_violation",
                "active worker differs from pinned parent context",
            ));
        }
        match work.child {
            None if work == self.gate.work() => {}
            Some(child)
                if parent
                    .metadata
                    .as_ref()
                    .is_some_and(|meta| meta.e2e_child == child)
                    && self.gate.required_children().contains(&child)
                    && parent.required_children.contains(&child)
                    && parent
                        .children
                        .get(&child)
                        .and_then(|child| child.work.as_ref())
                        == Some(work) => {}
            _ => {
                return Err(Error::new(
                    "scope_violation",
                    "active grant reader requires exact parent or required E2E worker",
                ));
            }
        }
        // The integrated proof belongs to the E2E child, not the parent permit.
        // Validate its actual retained gate against this same authenticated
        // parent snapshot: original profile/scope/dependencies/corrections remain
        // bound, rather than accepting any sibling with the same parent ID.
        parent.retain_active_gate(
            authority,
            &self.runtime.dependency_outcomes()?,
            Utc::now(),
            Duration::seconds(30),
        )?;
        crate::runtime_workflow::retained_parent_retry_grants(
            &self.configuration,
            &parent,
            work,
            &self.runtime.control.operations()?,
            &self.runtime.services.key,
        )
    }
    pub(crate) fn parent_in_gate(&self) -> Result<ParentProjection> {
        self.runtime
            .registration
            .verify_active_owner(&self.permit.lease)?;
        // The registry independently checks current authority fences, accounts,
        // provider access, original work/profile/workflow and snapshot freshness.
        self.runtime
            .observations
            .active_parent(&self.gate, &self.runtime.dependency_outcomes()?)
    }
}
impl RuntimeDelivery {
    pub(super) fn active_parent_grant_observer(
        &self,
        phase: &ActivePhase,
    ) -> Result<ActiveParentGrantObserver> {
        if phase.admission.operation_id() != phase.permit.operation_id
            || phase.admission.scope().digest()? != phase.permit.scope_digest
            || phase.workspace.workflow.digest != phase.admission.fresh_gate().workflow_digest()
        {
            return Err(Error::new(
                "scope_violation",
                "active parent phase binding differs",
            ));
        }
        ActiveParentGrantObserver::new(
            self.runtime.clone(),
            phase.permit.clone(),
            phase.admission.fresh_gate().clone(),
            phase.workspace.configuration.clone(),
        )
    }
}
