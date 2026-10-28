//! Reconstruct original production gates only for an actually observed owned
//! squash. Current ordinary readiness and new-merge gates remain unchanged.
use super::*;
pub struct MergedProductionRecovery<'a> {
    pub observation: &'a DeliveryObservation,
    pub parent: &'a evidence::VerifiedParent,
    pub preparation: Uuid,
    pub plan: &'a ReleasePlan,
    pub merge_step: &'a str,
    pub attempt: u32,
    pub observer: &'a ProductionObserver,
}
impl DeliveryCoordinator {
    pub async fn hydrate_owned_merge_production(
        &self,
        input: MergedProductionRecovery<'_>,
    ) -> Result<(PreparedProductionCandidate, evidence::VerifiedProduction)> {
        let observation = input.observation;
        self.current(observation, &[DeliveryPhase::Recovery])?;
        let observed = self
            .recover_owned_release_step(input.plan, input.merge_step, input.attempt, input.observer)
            .await?;
        let Some(ObservedEffect::Merge {
            repository,
            pull_request,
            commit,
            parent_commit,
        }) = observed.effect()
        else {
            return Err(production_error(
                "historical production validation requires an actual successful owned merge",
            ));
        };
        if observed.outcome() != &ReleaseStepOutcome::Succeeded
            || input.plan.runtime_id != observation.scope.runtime_id
            || input.plan.parent_id != observation.scope.work.parent
            || input.plan.linear_workspace != observation.scope.work.linear_workspace
            || input.plan.generation != observation.scope.work.generation
            || input.plan.evidence_token != input.parent.token()
            || input.parent.manifest().work != observation.scope.work
            || repository != &observation.scope.repository
            || parent_commit != &observation.scope.revision.base
            || commit.tree != observation.scope.revision.tested_tree
        {
            return Err(production_error(
                "owned merge is not the original accepted production candidate",
            ));
        }
        let prepared = PreparedProductionCandidate {
            signed: self.records.read(&production_leaf(input.preparation))?,
        };
        if prepared.operation_id() != input.preparation || prepared.pull_request() != *pull_request
        {
            return Err(production_error(
                "original production preparation differs from owned merge",
            ));
        }
        let review = self
            .observe_production_preparation_at(observation, input.parent, &prepared, Some(commit))
            .await?;
        let signed: Signed<ProductionValidationRecord> = self
            .records
            .read(&format!("production-validation-{}.json", input.preparation))?;
        self.key.verify("production_validation", &signed)?;
        if signed.payload.schema_version != 1
            || signed.payload.preparation_digest != prepared.signed.digest()?
        {
            return Err(production_error(
                "historical production checks differ from original preparation",
            ));
        }
        let hosted = HostedPassed {
            work: observation.scope.work.clone(),
            revision: observation.scope.revision.clone(),
            kind: CheckKind::ProductionCi,
            checks: signed.payload.checks,
            proofs: signed.payload.proofs,
        };
        hosted.matches(observation, CheckKind::ProductionCi)?;
        let mut production = observation.clone();
        production.scope.phase = DeliveryPhase::Release;
        self.reobserve_hosted(&production, &hosted).await?;
        let requirements = hosted_requirements(
            self.contract(observation)?
                .configuration
                .production_ci_policy()?,
            CheckKind::ProductionCi,
        )?;
        let contexts = requirements
            .iter()
            .map(|check| check.id.clone())
            .chain(std::iter::once(evidence::REVIEW_CONTEXT.into()))
            .collect();
        // Preserve the original signed observation. Separately require current
        // policy at the actual owned squash; never relabel the current base as
        // the old base merely to satisfy the original proof validator.
        let current = self
            .current_repository_protection(observation, "main")
            .await?;
        let mut current_revision = observation.scope.revision.clone();
        current_revision.base = commit.commit.clone();
        current.verify(&current_revision, "main", &contexts)?;
        let verified = evidence::verify_production(
            evidence::ProductionRequirements {
                parent: input.parent,
                revision: &observation.scope.revision,
                checks: &requirements,
                required_contexts: &contexts,
            },
            &CheckSnapshot {
                pagination_complete: true,
                observed_at: Utc::now(),
                runs: hosted.checks,
            },
            &review.verified,
            &signed.payload.protection,
        )?;
        if self.main_revision(observation).await? != *commit {
            return Err(production_error(
                "production moved beyond the accounted owned squash during recovery",
            ));
        }
        self.current(observation, &[DeliveryPhase::Recovery])?;
        Ok((prepared, verified))
    }
}
