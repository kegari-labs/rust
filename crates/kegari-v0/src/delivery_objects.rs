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
        self.object_acquisition_guarded(admission, processes, executions, None)
    }
    fn object_acquisition_guarded(
        &self,
        admission: &DeliveryAdmission,
        processes: Arc<crate::process_records::ProcessJournal>,
        executions: Arc<crate::worker::ExecutionRegistry>,
        predecessor: Option<Arc<crate::runtime_delivery::KnownPredecessorBaseline>>,
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
                gate(&expected.gate)?;
                if let Some(predecessor) = &predecessor {
                    predecessor.revalidate()?;
                    predecessor.gate().compare_current(
                        &expected.gate,
                        Utc::now(),
                        chrono::Duration::seconds(30),
                    )?;
                }
                Ok(())
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
    /// Recover actual already-integrated contributions into a corrected parent's
    /// isolated assembly. Every subprocess uses the current assembly permit.
    pub async fn acquire_successor_integrations(
        &self,
        admission: &DeliveryAdmission,
        predecessor: Arc<crate::runtime_delivery::KnownPredecessorBaseline>,
    ) -> Result<Vec<crate::git::VerifiedProductionIntegration>> {
        self.current(admission, &[DeliveryPhase::CandidateAssembly])?;
        let mut integrations = Vec::new();
        for effect in predecessor.effects() {
            let Some(crate::release::ObservedEffect::Merge {
                repository,
                pull_request,
                commit,
                ..
            }) = effect.effect()
            else {
                continue;
            };
            if repository != &admission.scope.repository {
                continue;
            }
            let request = AuthorizedObjectRequest::for_successor_merge(
                self.contract(admission)?,
                &admission.permit,
                admission,
                &predecessor,
                effect,
            )?;
            let source = self
                .object_acquisition_guarded(
                    admission,
                    self.processes.clone(),
                    self.executions.clone(),
                    Some(predecessor.clone()),
                )?
                .acquire(request)
                .await?;
            predecessor.require_effect(effect, admission.fresh_gate())?;
            let candidate = self.successor_slice(admission, &predecessor)?;
            let authority =
                evidence::recorded_candidate_authority(predecessor.origin(), candidate.manifest())?;
            let git = self.git.clone();
            let source = source.root().to_path_buf();
            let release_operation = predecessor.predecessor();
            let pull_request = *pull_request;
            let merged = commit.commit.clone();
            let main = predecessor
                .baseline(repository)?
                .production()
                .commit
                .clone();
            let proof = tokio::task::spawn_blocking(move || {
                git.observe_production_integration(
                    &source,
                    crate::git::ObserveProductionIntegration {
                        release_operation,
                        pull_request,
                        candidate: &candidate.candidate,
                        authority: &authority,
                        merged_commit: &merged,
                        current_main: &main,
                    },
                )
            })
            .await
            .map_err(|_| {
                Error::new(
                    "operation_uncertain",
                    "successor integration verification did not join",
                )
            })??;
            predecessor.require_effect(effect, admission.fresh_gate())?;
            integrations.push(proof);
        }
        Ok(integrations)
    }
    /// STOPPED-safe readback of already imported predecessor objects. Missing
    /// objects remain a prerequisite; this path never starts an acquisition.
    pub fn retained_successor_integrations(
        &self,
        observation: &DeliveryObservation,
        predecessor: &crate::runtime_delivery::KnownPredecessorBaseline,
    ) -> Result<Vec<crate::git::VerifiedProductionIntegration>> {
        self.current(observation, &[DeliveryPhase::Recovery])?;
        predecessor.revalidate()?;
        let contracts: Vec<_> = self.contracts.values().cloned().collect();
        let sources = crate::remote_objects::retained_sources(
            &self.records,
            &self.host,
            &self.key,
            self.github()?,
            &contracts,
        )?;
        let mut result = Vec::new();
        for effect in predecessor.effects() {
            let Some(crate::release::ObservedEffect::Merge {
                repository,
                pull_request,
                commit,
                ..
            }) = effect.effect()
            else {
                continue;
            };
            if repository != &observation.scope.repository {
                continue;
            }
            predecessor.require_effect(effect, observation.fresh_gate())?;
            let candidate = self.successor_slice(observation, predecessor)?;
            let head = candidate.manifest().result.as_ref().ok_or_else(|| {
                Error::new(
                    "candidate_provenance",
                    "predecessor candidate is incomplete",
                )
            })?;
            let source = sources
                .iter()
                .find(|source| {
                    source.repository() == repository
                        && source.revision() == commit
                        && source.required_revision(&head.commit).as_ref() == Some(head)
                })
                .ok_or_else(|| {
                    Error::new(
                        "source_unavailable",
                        "predecessor candidate and squash require an admitted exact object import",
                    )
                })?;
            source.verify_for_clone(&self.git)?;
            let authority =
                evidence::recorded_candidate_authority(predecessor.origin(), candidate.manifest())?;
            result.push(self.git.observe_production_integration(
                source.root(),
                crate::git::ObserveProductionIntegration {
                    release_operation: predecessor.predecessor(),
                    pull_request: *pull_request,
                    candidate: &candidate.candidate,
                    authority: &authority,
                    merged_commit: &commit.commit,
                    current_main: &predecessor.baseline(repository)?.production().commit,
                },
            )?);
        }
        predecessor.revalidate()?;
        self.current(observation, &[DeliveryPhase::Recovery])?;
        Ok(result)
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
