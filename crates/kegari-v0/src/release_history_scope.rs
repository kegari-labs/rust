//! Protective scope from authenticated completion identities. This is not
//! completion credit, a cleared hold, or a permission to execute anything.
use super::*;
use crate::evidence::WorkIdentity;
use crate::runtime_delivery::{
    ReleaseCompletionAnchor, VerifiedNoReleaseCompletion, VerifiedReleaseCompletion,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletionKind {
    Release,
    NoRelease,
}

#[derive(Clone, Debug)]
enum CompletionEnvelope {
    Release(std::sync::Arc<ReleaseCompletionAnchor>),
    NoRelease(std::sync::Arc<VerifiedNoReleaseCompletion>),
}

#[derive(Clone, Debug)]
pub(crate) struct CompletionScope {
    kind: CompletionKind,
    runtime: Uuid,
    workspace: Uuid,
    work: WorkIdentity,
    operation: Uuid,
    digest: String,
    outcome_digest: String,
    resources: BTreeSet<ResourceKey>,
    covered_operations: BTreeSet<Uuid>,
    envelope: CompletionEnvelope,
}
// Reader handles identify actual protected stores but are not mutable identity.
// A new facade may reopen that same store. Equality binds every authenticated
// immutable field; an equal scope never skips current envelope revalidation.
impl PartialEq for CompletionScope {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.runtime == other.runtime
            && self.workspace == other.workspace
            && self.work == other.work
            && self.operation == other.operation
            && self.digest == other.digest
            && self.outcome_digest == other.outcome_digest
            && self.resources == other.resources
            && self.covered_operations == other.covered_operations
    }
}
impl Eq for CompletionScope {}
impl CompletionScope {
    pub(crate) fn release(proof: &VerifiedReleaseCompletion) -> Result<Self> {
        Self::release_anchor(&proof.queue_anchor()?)
    }
    pub(crate) fn release_anchor(anchor: &ReleaseCompletionAnchor) -> Result<Self> {
        let result = Self {
            kind: CompletionKind::Release,
            runtime: anchor.runtime_id(),
            workspace: anchor.workspace_id(),
            work: anchor.work().clone(),
            operation: anchor.operation(),
            digest: anchor.digest().into(),
            outcome_digest: anchor.outcome_digest()?,
            resources: anchor.resources().clone(),
            covered_operations: anchor.covered_operations().collect(),
            envelope: CompletionEnvelope::Release(std::sync::Arc::new(anchor.clone())),
        };
        result.validate_identity()?;
        Ok(result)
    }
    pub(crate) fn no_release(proof: &VerifiedNoReleaseCompletion) -> Result<Self> {
        // The opaque cap was created from an authenticated outer seal and exact
        // signed outcome identity. Missing required references make it unusable
        // as credit, not an excuse to forget its protective resource inventory.
        let outcome = proof.record();
        let mut resources: BTreeSet<_> = proof
            .dependency_repositories()?
            .iter()
            .cloned()
            .map(|repository| ResourceKey::Repository { repository })
            .collect();
        resources.extend(
            outcome
                .baselines
                .iter()
                .map(|baseline| ResourceKey::Repository {
                    repository: baseline.repository.clone(),
                }),
        );
        resources.extend(outcome.targets.iter().map(|target| ResourceKey::Target {
            provider: target.provider.clone(),
            id: target.target.clone(),
        }));
        let result = Self {
            kind: CompletionKind::NoRelease,
            runtime: proof.runtime_id(),
            workspace: proof.workspace_id(),
            work: proof.work().clone(),
            operation: proof.operation(),
            digest: proof.digest().into(),
            outcome_digest: proof.outcome_digest().into(),
            resources,
            covered_operations: BTreeSet::from([proof.operation()]),
            envelope: CompletionEnvelope::NoRelease(std::sync::Arc::new(proof.clone())),
        };
        if outcome.work != result.work
            || outcome.runtime_id != result.runtime
            || outcome.operation_id != result.operation
        {
            return Err(Error::new(
                "no_release_completion",
                "authenticated completion scope differs from its outcome",
            ));
        }
        result.validate_identity()?;
        Ok(result)
    }
    fn validate_identity(&self) -> Result<()> {
        self.work.validate()?;
        validate_digest(&self.digest)?;
        validate_digest(&self.outcome_digest)?;
        if self.runtime.is_nil()
            || self.workspace.is_nil()
            || self.operation.is_nil()
            || self.work.child.is_some()
            || self.resources.is_empty()
            || self.resources.len() > 4096
            || !self.covered_operations.contains(&self.operation)
            || self.covered_operations.len() > 64
            || self.covered_operations.iter().any(Uuid::is_nil)
        {
            return Err(Error::new(
                "release_completion",
                "authenticated completion scope is incomplete",
            ));
        }
        for resource in &self.resources {
            match resource {
                ResourceKey::Repository { repository } => repository.validate()?,
                ResourceKey::Target { provider, id }
                    if provider.trim().is_empty() || id.trim().is_empty() =>
                {
                    return Err(Error::new(
                        "release_completion",
                        "authenticated target scope is empty",
                    ));
                }
                ResourceKey::Target { .. } => (),
            }
        }
        Ok(())
    }
    pub(crate) fn validate_queue_envelope(&self) -> Result<()> {
        match &self.envelope {
            CompletionEnvelope::Release(anchor) => anchor.validate_queue_envelope(),
            CompletionEnvelope::NoRelease(proof) => proof.validate_queue_envelope(),
        }
    }
    pub(crate) fn operation(&self) -> Uuid {
        self.operation
    }
    pub(crate) fn work(&self) -> &WorkIdentity {
        &self.work
    }
    pub(crate) fn resources(&self) -> &BTreeSet<ResourceKey> {
        &self.resources
    }
    pub(crate) fn covered_operations(&self) -> &BTreeSet<Uuid> {
        &self.covered_operations
    }
    /// Consuming A's exact ancestor holds cannot consume a failed B history
    /// identity. Shared resources alone do not make their immutable IDs equal.
    pub(crate) fn conflicts_consumption(&self, proof: &VerifiedReleaseCompletion) -> Result<bool> {
        Ok(proof
            .queue_anchor()?
            .covered_operations()
            .any(|operation| self.covered_operations().contains(&operation)))
    }
    pub(crate) fn affects(
        &self,
        resources: &BTreeSet<ResourceKey>,
        dependencies: &[DependencyRequirement],
    ) -> bool {
        !self.resources.is_disjoint(resources)
            || dependencies.iter().any(|dependency| {
                dependency.parent_id == self.work.parent
                    && dependency.generation == self.work.generation
                    && dependency.outcome_digest == self.outcome_digest
            })
    }
    pub(super) fn require_unaffected(
        &self,
        runtime: Uuid,
        resources: &BTreeSet<ResourceKey>,
        dependencies: &[DependencyRequirement],
        error: &Error,
    ) -> Result<()> {
        self.validate_identity()?;
        if self.runtime != runtime {
            return Err(Error::new(
                "identity_mismatch",
                "completion belongs to another runtime",
            ));
        }
        // Unsafe storage is not a scoped missing fact: the trust boundary itself
        // is unavailable. Never turn it into permission for unrelated execution.
        if error.code == "unsafe_storage" || self.affects(resources, dependencies) {
            return Err(error.clone());
        }
        Ok(())
    }
}
