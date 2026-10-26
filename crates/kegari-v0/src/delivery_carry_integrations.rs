//! Exact current carry integrations reproduced from genuine retained imports.
//! No serde constructor, source acquisition, model admission or fresh baseline.
use super::*;

#[derive(Clone)]
pub(crate) struct RetainedCarryIntegrations {
    subject_digest: String,
    authority: FreshGate,
    predecessor: Arc<crate::runtime_delivery::KnownPredecessorBaseline>,
    proofs: Vec<crate::git::VerifiedProductionIntegration>,
    sources: Vec<crate::remote_objects::RetainedObjectReadHistory>,
}
impl RetainedCarryIntegrations {
    pub(crate) fn proofs(&self) -> &[crate::git::VerifiedProductionIntegration] {
        &self.proofs
    }
    /// Only immutable protected history and filesystem reads. Fresh current or
    /// pinned active authority is independently required by the carry consumer.
    pub(crate) fn validate_retained(&self, subject: &RetainedCarrySubject) -> Result<()> {
        subject.validate_retained()?;
        if subject.digest()? != self.subject_digest
            || self.authority.work() != &subject.current_evidence().work
        {
            return Err(invalid(
                "retained integrations belong to another carry subject",
            ));
        }
        self.predecessor.require_historical_gate(&self.authority)?;
        for source in &self.sources {
            source.validate()?;
        }
        require_complete_records(
            subject.current_evidence().candidates.as_slice(),
            self.proofs.iter().map(|p| p.record()),
        )
    }
    pub(crate) fn require_input(
        &self,
        input: &[crate::git::VerifiedProductionIntegration],
    ) -> Result<()> {
        if input.len() != self.proofs.len()
            || input
                .iter()
                .zip(&self.proofs)
                .any(|(a, b)| a.record() != b.record())
        {
            return Err(invalid(
                "current carry input lost its exact observed integration proofs",
            ));
        }
        Ok(())
    }
}
impl DeliveryCoordinator {
    pub(crate) fn retained_carry_integrations(
        &self,
        gate: &FreshGate,
        subject: &RetainedCarrySubject,
        predecessor: Arc<crate::runtime_delivery::KnownPredecessorBaseline>,
    ) -> Result<RetainedCarryIntegrations> {
        (self.current_gate)(gate)?;
        subject.validate_retained()?;
        predecessor.revalidate()?;
        if gate.work() != &subject.current_evidence().work || predecessor.work() != gate.work() {
            return Err(invalid(
                "carry predecessor differs from actual current subject",
            ));
        }
        let mut proofs = Vec::new();
        let mut sources = BTreeMap::new();
        for baseline in predecessor.baselines() {
            let production = baseline.production();
            let observation = self.observation(
                gate.clone(),
                evidence::RevisionBinding {
                    repository: baseline.repository().clone(),
                    head: production.commit.clone(),
                    base: production.commit.clone(),
                    tested_commit: production.commit.clone(),
                    tested_tree: production.tree.clone(),
                },
            )?;
            // Same genuine Git/object lineage reader as normal current parent
            // hydration; only this path retains the extra immutable source closure.
            let read =
                self.read_retained_integration_inventory(&observation, &predecessor, false, true)?;
            proofs.extend(read.proofs);
            for source in read.history {
                sources.insert(source.digest()?, source);
            }
        }
        require_complete_records(
            &subject.current_evidence().candidates,
            proofs.iter().map(|p| p.record()),
        )?;
        let value = RetainedCarryIntegrations {
            subject_digest: subject.digest()?,
            authority: gate.clone(),
            predecessor,
            proofs,
            sources: sources.into_values().collect(),
        };
        value.validate_retained(subject)?;
        value.predecessor.revalidate()?;
        (self.current_gate)(gate)?;
        Ok(value)
    }
}

fn require_complete_records<'a>(
    candidates: &[crate::git::CandidateManifest],
    actual: impl IntoIterator<Item = &'a crate::git::ProductionIntegration>,
) -> Result<()> {
    let mut declared = BTreeMap::new();
    for candidate in candidates {
        for record in &candidate.production_integrations {
            if record.repository != candidate.repository
                || record.parent_id != candidate.parent_id
                || record.observed_main != candidate.production
                || declared
                    .insert(
                        (record.release_operation, record.repository.clone()),
                        record,
                    )
                    .is_some()
            {
                return Err(invalid(
                    "signed carry integration inventory is duplicate or changes its exact candidate source",
                ));
            }
        }
    }
    let mut observed = BTreeMap::new();
    for record in actual {
        let key = (record.release_operation, record.repository.clone());
        if observed.insert(key.clone(), record).is_some()
            || declared
                .get(&key)
                .is_none_or(|expected| *expected != record)
        {
            return Err(invalid(
                "observed carry integration is extra, duplicate or differs from its signed full record",
            ));
        }
    }
    if observed.len() != declared.len() {
        return Err(invalid(
            "current carry lacks its complete declared production integrations",
        ));
    }
    Ok(())
}
fn invalid(message: &str) -> Error {
    Error::new("production_provenance", message)
}

#[cfg(test)]
#[path = "delivery_carry_integrations_tests.rs"]
mod tests;
