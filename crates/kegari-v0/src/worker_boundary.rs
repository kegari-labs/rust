//! Actual contract/worker diagnostic, never a permission/admission consumer.
use super::*;
use crate::evaluation::{
    IndependentBoundaryCheck, IndependentBoundaryIdentity, IndependentBoundaryObservation,
};
impl WorkerContract {
    pub(crate) fn independent_boundary_identity(
        &self,
        permit: &Permit,
        request_digest: String,
        current: &IndependentReviewGate,
    ) -> Result<IndependentBoundaryIdentity> {
        let WorkerAuthority::Independent(original) = &self.authority else {
            return Err(Error::new(
                "boundary_observation_unavailable",
                "worker is not an independent review",
            ));
        };
        original.compare_current(current, Utc::now(), Duration::seconds(30))?;
        let manifest = &self.assembled.manifest;
        let target = self.review.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "independent contract has no exact PR target",
            )
        })?;
        let revision = manifest.repository_revision.as_ref().ok_or_else(|| {
            Error::new(
                "worker_binding",
                "independent contract has no admitted source revision",
            )
        })?;
        if manifest.role != AgentRole::IndependentReview
            || manifest.phase != "independent_review"
            || manifest.operation_id != permit.operation_id
            || manifest.workspace_id != original.workspace_id()
            || self.scope_digest != permit.scope_digest
            || self.work_key()? != permit.work_key
            || target.repository.id != original.repository_id()
            || target.pull_request != original.pull_request().number
            || target.head != original.pull_request().head_sha
            || target.base != original.pull_request().base_sha
            || revision != &target.head
            || manifest.context_digest != canonical_digest(1, &self.context)?
            || self.assembled.manifest_digest
                != canonical_digest(crate::instructions::ASSEMBLY_SCHEMA, manifest)?
        {
            return Err(Error::new(
                "worker_binding",
                "independent diagnostic does not match original contract and permit",
            ));
        }
        let context = self.context.as_object().ok_or_else(|| {
            Error::new("worker_binding", "complete worker context is not an object")
        })?;
        let mut context_keys: Vec<_> = context.keys().cloned().collect();
        context_keys.sort();
        Ok(IndependentBoundaryIdentity {
            operation: permit.operation_id,
            runtime: permit.lease.runtime_id,
            workspace: manifest.workspace_id,
            owner_generation: permit.lease.owner_generation,
            owner: permit.lease.owner_id,
            checked_at: Utc::now(),
            permit_digest: canonical_digest(1, permit)?,
            admission_request_digest: request_digest,
            contract_scope_digest: self.scope_digest.clone(),
            authority_digest: original.digest()?,
            current_authority_digest: current.digest()?,
            review_identity: original.review_identity().into(),
            target: target.clone(),
            source_revision: revision.clone(),
            instruction_manifest_digest: self.assembled.manifest_digest.clone(),
            context_digest: manifest.context_digest.clone(),
            context_keys,
            tool_contract_digest: self.tools.digest().into(),
            isolation_digest: self.isolation.digest().into(),
            permission_digest: canonical_digest(1, &manifest.permissions)?,
        })
    }
}
impl ScopedWorker {
    pub(super) fn independent_boundary_observation(
        &self,
        revision: &str,
        candidate_digest: &str,
    ) -> Option<IndependentBoundaryObservation> {
        if !matches!(&self.contract.authority, WorkerAuthority::Independent(_)) {
            return None;
        }
        let check = match self.host.observe_independent_boundary(
            &self.control,
            &self.permit,
            &self.contract,
        ) {
            Ok(identity) => IndependentBoundaryCheck::MatchedAtCollection {
                identity: Box::new(identity),
            },
            Err(error) => IndependentBoundaryCheck::Unavailable { error },
        };
        let mut observation = IndependentBoundaryObservation {
            schema_version: 1,
            source_revision: revision.into(),
            candidate_digest: candidate_digest.into(),
            check,
        };
        if let Err(error) = observation.validate() {
            // A separate diagnostic failure must not discard actual source/check facts.
            observation.check = IndependentBoundaryCheck::Unavailable { error };
        }
        Some(observation)
    }
}
