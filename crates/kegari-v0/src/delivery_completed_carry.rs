//! Actual terminal release origin used solely for a known released baseline.
//! Different-parent production changes never become current acceptance or a
//! new release permit. Every root below comes from an authenticated old slice.
use super::*;
use crate::runtime_delivery::VerifiedReleaseCompletion;
#[derive(Clone)]
pub(crate) struct CompletedCarryOrigin {
    completion: VerifiedReleaseCompletion,
    origin: VerifiedReleaseOrigin,
    configuration: crate::config::Config,
    parent_authority: FreshGate,
    selected: BTreeSet<RepositoryId>,
}
impl CompletedCarryOrigin {
    fn validate(&self, gate: &FreshGate, repository: &RepositoryId) -> Result<()> {
        self.parent_authority.compare_admitted_authority(gate)?;
        self.completion
            .validate_for_configuration(&self.configuration)?;
        if !self.selected.contains(repository)
            || self.configuration.revision()? != gate.config_revision()
            || self.completion.work().parent == gate.work().parent
            || self.completion.work().linear_endpoint != gate.work().linear_endpoint
            || self.completion.work().linear_workspace != gate.work().linear_workspace
            || self.completion.workspace_id() != gate.workspace_id()
            || self.origin.plan() != &self.completion.terminal().plan
            || !self
                .origin
                .manifest()
                .candidates
                .iter()
                .any(|c| &c.repository == repository)
        {
            return Err(Error::new(
                "acceptance_carry_baseline",
                "completed baseline origin differs from current native scope",
            ));
        }
        Ok(())
    }
}
impl DeliveryCoordinator {
    pub(crate) async fn completed_carry_origin(
        &self,
        gate: &FreshGate,
        completion: VerifiedReleaseCompletion,
        configuration: &crate::config::Config,
        selected: BTreeSet<RepositoryId>,
    ) -> Result<CompletedCarryOrigin> {
        (self.current_gate)(gate)?;
        let (parent, acceptance) = completion.carry_publications(configuration)?;
        let origin = self
            .release_origin(&completion.terminal().plan, parent, acceptance)
            .await?;
        completion.validate_for_configuration(configuration)?;
        (self.current_gate)(gate)?;
        Ok(CompletedCarryOrigin {
            completion,
            origin,
            configuration: configuration.clone(),
            parent_authority: gate.clone(),
            selected,
        })
    }
    pub(crate) fn completed_carry_roots(
        &self,
        contract: &ParentContract,
        origins: &[CompletedCarryOrigin],
    ) -> Result<Vec<crate::git::VerifiedHostReadRoot>> {
        (self.current_gate)(&contract.gate)?;
        let mut roots = Vec::new();
        for origin in origins {
            for repository in contract.repositories() {
                if !origin.selected.contains(repository)
                    || !origin
                        .origin
                        .manifest()
                        .candidates
                        .iter()
                        .any(|c| &c.repository == repository)
                {
                    continue;
                }
                origin.validate(&contract.gate, repository)?;
                let record = self.retained_origin_slice_record(repository, &origin.origin)?;
                let r = &record.payload;
                roots.push(crate::git::retained_candidate_read_root(
                    repository.clone(),
                    &r.intent
                        .execution_root
                        .join(format!("candidate-{}", r.intent.operation_id)),
                    contract.gate.config_revision().into(),
                    record.digest()?,
                    &r.candidate,
                )?);
            }
        }
        Ok(roots)
    }
    /// Uses a host-scoped read adapter; absence of already acquired exact merge
    /// objects is a source prerequisite, never an inferred integration.
    pub(crate) async fn observe_completed_carry_integrations(
        &self,
        contract: &ParentContract,
        origins: &[CompletedCarryOrigin],
        baselines: &[Arc<crate::deployment::VerifiedProductionBaseline>],
        observer: &crate::deployment::ProductionObserver,
    ) -> Result<
        Vec<(
            VerifiedReleaseCompletion,
            crate::git::VerifiedProductionIntegration,
        )>,
    > {
        (self.current_gate)(&contract.gate)?;
        let mut result = Vec::new();
        for origin in origins {
            let plan = origin.origin.plan();
            for step in &plan.steps {
                let crate::release::StepKind::Merge {
                    repository,
                    pull_request,
                    ..
                } = &step.kind
                else {
                    continue;
                };
                if !contract.repositories().contains(repository)
                    || !origin.selected.contains(repository)
                {
                    continue;
                }
                let baseline = baselines
                    .iter()
                    .find(|b| &b.baseline().repository == repository)
                    .ok_or_else(|| {
                        Error::new("production_baseline", "complete carry baseline missing")
                    })?;
                origin.validate(&contract.gate, repository)?;
                let observed = self
                    .recover_owned_release_step(
                        plan,
                        &step.id,
                        origin.completion.terminal().step_attempt(&step.id)?,
                        observer,
                    )
                    .await?;
                let Some(crate::release::ObservedEffect::Merge { commit, .. }) = observed.effect()
                else {
                    return Err(Error::new(
                        "acceptance_carry_baseline",
                        "completed origin lacks its exact actual merge",
                    ));
                };
                if !matches!(origin.completion.terminal().steps.get(&step.id),Some(crate::release::StepState::Succeeded {effect,..}) if Some(effect.as_ref())==observed.effect())
                    || observed.plan_digest() != plan.digest()?
                    || !matches!(observed.outcome(), ReleaseStepOutcome::Succeeded)
                {
                    return Err(Error::new(
                        "acceptance_carry_baseline",
                        "baseline merge differs from actual completed terminal step",
                    ));
                }
                let commit = commit.clone();
                let record = self.retained_origin_slice_record(repository, &origin.origin)?;
                let candidate = self.git.inspect_candidate(
                    &record.payload.intent.execution_root,
                    record.payload.intent.operation_id,
                )?;
                if candidate.manifest != record.payload.candidate {
                    return Err(Error::new(
                        "candidate_provenance",
                        "original accepted carry candidate changed",
                    ));
                }
                let accepted =
                    evidence::recorded_candidate_authority(&origin.origin, &candidate.manifest)?;
                let selected = self
                    .contracts
                    .get(repository)
                    .ok_or_else(|| Error::new("scope_violation", "carry repository disappeared"))?;
                let source_proof = CompletedCarryObjects {
                    origin: origin.clone(),
                    repository: selected.clone(),
                    candidate: candidate.manifest.clone(),
                    observed: Arc::new(observed),
                    current_main: baseline.baseline().production.clone(),
                };
                source_proof.validate(&contract.gate)?;
                let source = self.completed_carry_source(&source_proof)?.ok_or_else(|| {
                    Error::new(
                        "carry_objects_pending",
                        "exact completed merge objects require an owned import",
                    )
                })?;
                let integration = self.git.observe_production_integration(
                    &source,
                    crate::git::ObserveProductionIntegration {
                        release_operation: plan.operation_id,
                        pull_request: *pull_request,
                        candidate: &candidate,
                        authority: &accepted,
                        merged_commit: &commit.commit,
                        current_main: &baseline.baseline().production.commit,
                    },
                )?;
                // Re-read the exact native origin after Git; local read-only
                // object inspection never replaces current provider provenance.
                let ids = origin
                    .completion
                    .carry_publications(&origin.configuration)?;
                let current = self.release_origin(plan, ids.0, ids.1).await?;
                if current.admission_digest() != origin.origin.admission_digest()
                    || current.manifest() != origin.origin.manifest()
                {
                    return Err(Error::new(
                        "stale_result",
                        "completed carry origin changed during Git observation",
                    ));
                }
                origin.validate(&contract.gate, repository)?;
                result.push((origin.completion.clone(), integration));
            }
        }
        (self.current_gate)(&contract.gate)?;
        Ok(result)
    }
}
/// An exact missing-object prerequisite derived from a completed release and
/// its actual native merge readback. This is read provenance, not acceptance,
/// a deployment proof, or a permit. Fields cannot be deserialized or supplied.
#[derive(Clone)]
pub(crate) struct CompletedCarryObjects {
    origin: CompletedCarryOrigin,
    repository: RepositoryContract,
    candidate: crate::git::CandidateManifest,
    observed: Arc<ObservedReleaseStep>,
    current_main: crate::git::Revision,
}
impl CompletedCarryObjects {
    pub(crate) fn repository(&self) -> &RepositoryContract {
        &self.repository
    }
    pub(crate) fn observed(&self) -> &ObservedReleaseStep {
        &self.observed
    }
    pub(crate) fn current_main(&self) -> &crate::git::Revision {
        &self.current_main
    }
    pub(crate) fn accepted_head(&self) -> &crate::git::Revision {
        self.candidate
            .result
            .as_ref()
            .expect("actual complete accepted candidate")
    }
    pub(crate) fn required_revisions(&self) -> Result<Vec<crate::git::Revision>> {
        let Some(crate::release::ObservedEffect::Merge { commit, .. }) = self.observed.effect()
        else {
            return Err(Error::new(
                "acceptance_carry_baseline",
                "completed carry merge missing",
            ));
        };
        Ok(vec![
            self.candidate.production.clone(),
            self.accepted_head().clone(),
            commit.clone(),
            self.current_main.clone(),
        ])
    }
    pub(crate) fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &json!({"purpose":"completed_carry_objects","completion":self.origin.completion.digest(),"origin":self.origin.origin.admission_digest(),"authority":self.origin.parent_authority.digest()?,"contract":self.repository.digest()?,"candidate":self.candidate,"merge":self.observed.receipt(),"required":self.required_revisions()?}),
        )
    }
    pub(crate) fn validate(&self, gate: &FreshGate) -> Result<()> {
        self.origin.validate(gate, &self.repository.repository)?;
        let plan = self.origin.origin.plan();
        let Some(crate::release::ObservedEffect::Merge {
            repository,
            commit,
            parent_commit,
            ..
        }) = self.observed.effect()
        else {
            return Err(Error::new(
                "acceptance_carry_baseline",
                "completed carry merge missing",
            ));
        };
        if repository != &self.repository.repository
            || self.candidate.repository != *repository
            || !self
                .origin
                .origin
                .manifest()
                .candidates
                .contains(&self.candidate)
            || self
                .candidate
                .result
                .as_ref()
                .is_none_or(|head| head.tree != commit.tree)
            || self.candidate.production.commit != *parent_commit
            || self.observed.plan_digest() != plan.digest()?
            || !matches!(self.observed.outcome(), ReleaseStepOutcome::Succeeded)
            || !matches!(self.origin.completion.terminal().steps.get(self.observed.step_id()), Some(crate::release::StepState::Succeeded { effect, .. }) if Some(effect.as_ref())==self.observed.effect())
            || self
                .origin
                .completion
                .terminal()
                .step_attempt(self.observed.step_id())?
                != self.observed.attempt()
        {
            return Err(Error::new(
                "acceptance_carry_baseline",
                "carry object source differs from actual completed origin",
            ));
        }
        for revision in self.required_revisions()? {
            revision.validate()?;
        }
        self.repository.digest()?;
        Ok(())
    }
    pub(crate) fn verify_admission(
        &self,
        admission: &DeliveryAdmission,
        contract: &RepositoryContract,
    ) -> Result<()> {
        self.validate(admission.fresh_gate())?;
        let scope = admission.scope();
        if scope.phase != DeliveryPhase::CandidateAssembly
            || scope.work.child.is_some()
            || scope.work != *self.origin.parent_authority.work()
            || scope.repository != self.repository.repository
            || scope.revision.repository != self.repository.repository
            || contract.digest()? != self.repository.digest()?
            || scope.candidate_vector_digest.is_some()
            || scope.revision.head != self.current_main.commit
            || scope.revision.base != self.current_main.commit
            || scope.revision.tested_commit != self.current_main.commit
            || scope.revision.tested_tree != self.current_main.tree
        {
            return Err(Error::new(
                "scope_violation",
                "carry object import differs from exact current parent assembly source",
            ));
        }
        Ok(())
    }
}
impl DeliveryCoordinator {
    fn completed_carry_source(&self, proof: &CompletedCarryObjects) -> Result<Option<PathBuf>> {
        let revisions = proof.required_revisions()?;
        let repository = &proof.repository.repository;
        let configured = self
            .retained_git_sources
            .get(repository)
            .cloned()
            .unwrap_or_else(|| PathBuf::from(&proof.repository.configuration.source));
        let complete = |root: &Path| -> Result<bool> {
            for revision in &revisions {
                if !self.git.contains_exact_revision(root, revision)? {
                    return Ok(false);
                }
            }
            Ok(true)
        };
        if complete(&configured)? {
            return Ok(Some(configured));
        }
        let contracts = self.contracts.values().cloned().collect::<Vec<_>>();
        for source in crate::remote_objects::retained_sources(
            &self.records,
            &self.host,
            &self.key,
            self.github()?,
            &contracts,
        )? {
            if source.repository() == repository
                && revisions
                    .iter()
                    .all(|r| source.required_revision(&r.commit).as_ref() == Some(r))
            {
                source.verify_for_clone(&self.git)?;
                if complete(source.root())? {
                    return Ok(Some(source.root().to_owned()));
                }
            }
        }
        Ok(None)
    }
    /// Detect absence through actual full-SHA Git reads. Nonzero/error Git
    /// results are never reinterpreted as permission to download another source.
    pub(crate) async fn missing_completed_carry_objects(
        &self,
        contract: &ParentContract,
        origins: &[CompletedCarryOrigin],
        baselines: &[Arc<crate::deployment::VerifiedProductionBaseline>],
        observer: &crate::deployment::ProductionObserver,
    ) -> Result<Option<CompletedCarryObjects>> {
        (self.current_gate)(&contract.gate)?;
        for origin in origins {
            for step in &origin.origin.plan().steps {
                let crate::release::StepKind::Merge { repository, .. } = &step.kind else {
                    continue;
                };
                if !contract.repositories().contains(repository)
                    || !origin.selected.contains(repository)
                {
                    continue;
                }
                let baseline = baselines
                    .iter()
                    .find(|b| &b.baseline().repository == repository)
                    .ok_or_else(|| {
                        Error::new("production_baseline", "complete carry baseline missing")
                    })?;
                origin.validate(&contract.gate, repository)?;
                let observed = self
                    .recover_owned_release_step(
                        origin.origin.plan(),
                        &step.id,
                        origin.completion.terminal().step_attempt(&step.id)?,
                        observer,
                    )
                    .await?;
                let record = self.retained_origin_slice_record(repository, &origin.origin)?;
                let candidate = self.git.inspect_candidate(
                    &record.payload.intent.execution_root,
                    record.payload.intent.operation_id,
                )?;
                if candidate.manifest != record.payload.candidate
                    || candidate.manifest.result.is_none()
                {
                    return Err(Error::new(
                        "candidate_provenance",
                        "original accepted carry candidate changed",
                    ));
                }
                let proof = CompletedCarryObjects {
                    origin: origin.clone(),
                    repository: self
                        .contracts
                        .get(repository)
                        .ok_or_else(|| {
                            Error::new("scope_violation", "carry repository disappeared")
                        })?
                        .clone(),
                    candidate: candidate.manifest,
                    observed: Arc::new(observed),
                    current_main: baseline.baseline().production.clone(),
                };
                proof.validate(&contract.gate)?;
                let missing = self.completed_carry_source(&proof)?.is_none();
                (self.current_gate)(&contract.gate)?;
                if missing {
                    return Ok(Some(proof));
                }
            }
        }
        Ok(None)
    }
    pub(crate) async fn reobserve_carry_import_acceptance(
        &self,
        admission: &DeliveryAdmission,
        original: &AcceptedParentOrigin,
        configuration: &crate::config::Config,
    ) -> Result<()> {
        self.current(admission, &[DeliveryPhase::CandidateAssembly])?;
        original.reobserve_native(configuration).await?;
        self.current(admission, &[DeliveryPhase::CandidateAssembly])
    }
    async fn reobserve_completed_carry_origin(
        &self,
        admission: &DeliveryAdmission,
        proof: &CompletedCarryObjects,
    ) -> Result<()> {
        self.current(admission, &[DeliveryPhase::CandidateAssembly])?;
        let ids = proof
            .origin
            .completion
            .carry_publications(&proof.origin.configuration)?;
        let current = self
            .release_origin(proof.origin.origin.plan(), ids.0, ids.1)
            .await?;
        if current.admission_digest() != proof.origin.origin.admission_digest()
            || current.manifest() != proof.origin.origin.manifest()
        {
            return Err(Error::new(
                "stale_result",
                "completed carry origin changed before object read",
            ));
        }
        proof.validate(admission.fresh_gate())?;
        self.current(admission, &[DeliveryPhase::CandidateAssembly])
    }
    pub(crate) fn completed_carry_object_request(
        &self,
        admission: &DeliveryAdmission,
        proof: &CompletedCarryObjects,
    ) -> Result<crate::remote_objects::AuthorizedObjectRequest> {
        self.current(admission, &[DeliveryPhase::CandidateAssembly])?;
        crate::remote_objects::AuthorizedObjectRequest::for_completed_carry(
            self.contract(admission)?,
            &admission.permit,
            admission,
            proof,
        )
    }
    pub(crate) async fn acquire_completed_carry_objects(
        &self,
        admission: &DeliveryAdmission,
        proof: Arc<CompletedCarryObjects>,
    ) -> Result<crate::remote_objects::VerifiedObjectSource> {
        self.completed_carry_objects(admission, proof, false).await
    }
    /// Readback cannot dispatch or allocate a new object effect, even if the
    /// original process/ACK is absent. Unknown ownership remains retained.
    pub(crate) async fn observe_completed_carry_objects(
        &self,
        admission: &DeliveryAdmission,
        proof: Arc<CompletedCarryObjects>,
    ) -> Result<crate::remote_objects::VerifiedObjectSource> {
        self.completed_carry_objects(admission, proof, true).await
    }
    async fn completed_carry_objects(
        &self,
        admission: &DeliveryAdmission,
        proof: Arc<CompletedCarryObjects>,
        readback_only: bool,
    ) -> Result<crate::remote_objects::VerifiedObjectSource> {
        self.reobserve_completed_carry_origin(admission, &proof)
            .await?;
        let request = self.completed_carry_object_request(admission, &proof)?;
        let retained = proof.clone();
        let acquisition = self.object_acquisition_with_gate(
            admission,
            self.processes.clone(),
            self.executions.clone(),
            Some(Arc::new(move |gate| retained.validate(gate))),
        )?;
        let source = if readback_only {
            acquisition.observe_cached(&request).await?
        } else {
            acquisition.acquire(request.clone()).await?
        };
        source.verify_for_clone(&self.git)?;
        for revision in proof.required_revisions()? {
            if self.git.exact_revision(source.root(), &revision.commit)? != revision {
                return Err(Error::new(
                    "provenance_mismatch",
                    "carry import lost its exact full-SHA root",
                ));
            }
        }
        acquisition.observe_origin(&request).await?;
        self.reobserve_completed_carry_origin(admission, &proof)
            .await?;
        proof.validate(admission.fresh_gate())?;
        self.current(admission, &[DeliveryPhase::CandidateAssembly])?;
        Ok(source)
    }
}
