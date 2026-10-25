//! Clone only verified private object substrates while preserving configured
//! operator attribution in the immutable owned execution identity.
use super::*;
use crate::remote_objects::VerifiedObjectSource;
impl Git {
    fn acquired_clone_source(
        &self,
        source: &VerifiedObjectSource,
        repository: &RepositoryId,
        operator: &Path,
    ) -> Result<()> {
        if self.host_scope.is_some() {
            return Err(Error::new(
                "scope_violation",
                "host metadata authority cannot prepare an execution clone",
            ));
        }
        if source.repository() != repository || source.operator_checkout() != operator {
            return Err(Error::new(
                "execution_ownership",
                "acquired clone source differs from repository or configured operator attribution",
            ));
        }
        source.verify_for_clone(self)
    }
    pub fn prepare_acquired_execution_clone(
        &self,
        source: &VerifiedObjectSource,
        execution_root: &Path,
        identity: &crate::execution::DirectoryIdentity,
    ) -> Result<PathBuf> {
        self.acquired_clone_source(source, &identity.repository, &identity.operator_checkout)?;
        identity.work.validate()?;
        identity.repository.validate()?;
        if identity.schema_version != 1
            || identity.runtime_id.is_nil()
            || identity.directory_id.is_nil()
        {
            return Err(Error::new(
                "execution_ownership",
                "execution identity is incomplete",
            ));
        }
        self.prepare_owned_clone(
            source.root(),
            execution_root,
            CloneOwner::Child(identity.clone()),
            source.revision(),
            true,
            Some(source.digest()?),
        )
    }
    pub fn prepare_acquired_independent_review_clone(
        &self,
        source: &VerifiedObjectSource,
        execution_root: &Path,
        identity: &IndependentCloneIdentity,
    ) -> Result<PathBuf> {
        self.acquired_clone_source(source, &identity.repository, &identity.operator_checkout)?;
        validate_independent_clone_identity(identity)?;
        source.validate_review_clone(identity)?;
        self.prepare_owned_clone(
            source.root(),
            execution_root,
            CloneOwner::IndependentReview(identity.clone()),
            &identity.head,
            true,
            Some(source.digest()?),
        )
    }
    pub fn observe_acquired_independent_review_clone(
        &self,
        source: &VerifiedObjectSource,
        execution_root: &Path,
        identity: &IndependentCloneIdentity,
    ) -> Result<VerifiedIndependentClone> {
        self.acquired_clone_source(source, &identity.repository, &identity.operator_checkout)?;
        validate_independent_clone_identity(identity)?;
        source.validate_review_clone(identity)?;
        self.prepare_owned_clone(
            source.root(),
            execution_root,
            CloneOwner::IndependentReview(identity.clone()),
            &identity.head,
            false,
            Some(source.digest()?),
        )?;
        if self.exact_revision(&identity.repository_path, &identity.base.commit)? != identity.base {
            return Err(Error::new(
                "stale_revision",
                "retained independent clone lost exact acquired base",
            ));
        }
        let root = owner_directory(execution_root)?;
        let paths = [
            root.as_path(),
            identity.repository_path.as_path(),
            identity.operator_checkout.as_path(),
            source.root(),
            &identity.repository_path.join(".git"),
        ]
        .into_iter()
        .map(clone_directory_stamp)
        .collect::<Result<_>>()?;
        let marker = root.join(format!("prepared-clone-{}.json", identity.directory_id));
        let proof = VerifiedIndependentClone {
            identity: identity.clone(),
            root,
            paths,
            marker_digest: canonical_digest(1, &read_clone_record(&marker)?)?,
        };
        proof.revalidate_filesystem()?;
        Ok(proof)
    }
}
