//! Exact provider objects for admitted delivery effects. These methods retain
//! original receipt lineage and do not replace staging or release proof checks.
use super::*;
use crate::remote_objects::{
    AcquisitionServices, AuthorizedObjectRequest, ObjectAcquisition, ObjectLimits,
    VerifiedObjectSource,
};

impl DeliveryCoordinator {
    pub(crate) fn retained_object_read_roots(
        &self,
        configuration: &crate::config::Config,
        workspace: Uuid,
    ) -> Result<Vec<crate::git::VerifiedHostReadRoot>> {
        let contracts: Vec<_> = self.contracts.values().cloned().collect();
        crate::remote_objects::retained_sources(
            &self.records,
            &self.host,
            &self.key,
            self.github()?,
            &contracts,
        )?
        .into_iter()
        .map(|source| {
            source.host_read_root(
                configuration,
                workspace,
                &self.contracts[source.repository()],
            )
        })
        .collect()
    }
    pub(super) fn retained_delta_object_source(
        &self,
        delta: &crate::git::DeltaManifest,
    ) -> Result<Option<VerifiedObjectSource>> {
        let contracts: Vec<_> = self.contracts.values().cloned().collect();
        let sources = crate::remote_objects::retained_sources(
            &self.records,
            &self.host,
            &self.key,
            self.github()?,
            &contracts,
        )?;
        crate::remote_objects::select_delta_source(sources, &self.git, delta)
    }

    pub(super) fn object_acquisition(
        &self,
        admission: &DeliveryAdmission,
        processes: Arc<crate::process_records::ProcessJournal>,
        executions: Arc<crate::worker::ExecutionRegistry>,
    ) -> Result<ObjectAcquisition> {
        self.current(admission, &[admission.scope.phase])?;
        let expected = admission.clone();
        let gate = self.current_gate.clone();
        let contract_digest = self.contract(admission)?.digest()?;
        ObjectAcquisition::new(AcquisitionServices {
            control: self.host.clone(),
            github: self.github()?.clone(),
            git: self.git.as_ref().clone(),
            records: self.records.child("object-sources")?,
            key: self.key.clone(),
            processes,
            executions,
            current: Arc::new(move |request| {
                if request.permit().operation_id != expected.operation_id()
                    || request.permit().scope_digest != expected.permit.scope_digest
                    || request.repository() != &expected.scope.repository
                    || request.contract_digest()? != contract_digest
                    || request.authority_digest() != expected.gate.digest()?
                {
                    return Err(Error::new(
                        "scope_violation",
                        "object import differs from the original delivery admission",
                    ));
                }
                gate(&expected.gate)
            }),
            limits: ObjectLimits::default(),
        })
    }
    pub async fn acquire_staging_source(
        &self,
        admission: &DeliveryAdmission,
        pull_request: &ChildPullRequest,
        pending: &PendingStaging,
        processes: Arc<crate::process_records::ProcessJournal>,
        executions: Arc<crate::worker::ExecutionRegistry>,
    ) -> Result<VerifiedObjectSource> {
        self.current(admission, &[DeliveryPhase::Staging])?;
        pull_request.matches(admission)?;
        pending.matches(admission)?;
        let contract = self.contract(admission)?;
        let current = self
            .github()?
            .pull_request(&contract.provider, pull_request.number())
            .await?;
        let commit = current
            .merge_commit_sha
            .filter(|_| current.merged == Some(true))
            .ok_or_else(|| {
                Error::new(
                    "operation_pending",
                    "staging merge has no actual completed source",
                )
            })?;
        let request = AuthorizedObjectRequest::for_staging_merge(
            contract,
            crate::git::Revision {
                commit,
                tree: admission.scope.revision.tested_tree.clone(),
            },
            &admission.permit,
            admission,
            pull_request,
            pending,
        )?;
        self.object_acquisition(admission, processes, executions)?
            .acquire(request)
            .await
    }
    pub async fn acquire_release_objects(
        &self,
        admission: &DeliveryAdmission,
        observed: &ObservedReleaseStep,
        processes: Arc<crate::process_records::ProcessJournal>,
        executions: Arc<crate::worker::ExecutionRegistry>,
    ) -> Result<VerifiedObjectSource> {
        self.current(
            admission,
            &[DeliveryPhase::Release, DeliveryPhase::Recovery],
        )?;
        let request = AuthorizedObjectRequest::for_release_merge(
            self.contract(admission)?,
            &admission.permit,
            admission,
            observed,
        )?;
        self.object_acquisition(admission, processes, executions)?
            .acquire(request)
            .await
    }
}
