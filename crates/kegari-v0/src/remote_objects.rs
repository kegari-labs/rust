//! Exact Git objects acquired into an owned private substrate. HTTP only reads
//! the selected provider; actual Git import is an admitted, journaled local effect.
use crate::control::ControlHost;
use crate::control::MutationRequest;
use crate::control::Permit;
use crate::control::ReceiptPhase;
use crate::delivery::DeliveryAdmission;
use crate::delivery::RepositoryContract;
use crate::git::Git;
use crate::git::GitLimits;
use crate::process_records::ProcessJournal;
use crate::worker::ExecutionRegistry;
use crate::worker::WorkerAuthority;
use kegari_control_api::identity::Actor;
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_git::RepositoryId;
use kegari_git::Revision;
use kegari_providers::GithubClient;
use kegari_secrets::{AttestationKey, Signed};
use kegari_storage::PrivateDir;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;

#[path = "remote_objects_pack.rs"]
mod pack;
pub use pack::ObjectLimits;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Origin {
    Reference {
        reference: String,
    },
    PullRequest {
        expected: kegari_providers::PullRequest,
    },
    Merge {
        number: u64,
        parent: String,
        receipt: Uuid,
    },
    LineageMerge {
        number: u64,
        parent: String,
        receipt: Uuid,
        receipt_digest: String,
        current_main: Revision,
    },
    CompletedCarryMerge {
        number: u64,
        parent: String,
        receipt: Uuid,
        receipt_digest: String,
        current_main: Revision,
        accepted_head: Revision,
        completed_origin: String,
    },
    Candidate {
        reference: String,
        staged: Vec<(u64, kegari_git::DeltaManifest)>,
        integrations: Vec<kegari_git::ProductionIntegration>,
    },
    StagingMerge {
        expected: kegari_providers::PullRequest,
        parent: String,
        receipt: Uuid,
        receipt_digest: String,
    },
}
/// Its constructors require existing sealed worker/delivery authority. It is not
/// deserializable, and does not itself grant a process permit.
#[derive(Clone)]
pub struct AuthorizedObjectRequest {
    contract: RepositoryContract,
    revision: Revision,
    required_revisions: BTreeMap<String, String>,
    origin: Origin,
    permit: Permit,
    authority_digest: String,
}
impl AuthorizedObjectRequest {
    pub fn for_worker(
        contract: &RepositoryContract,
        revision: Revision,
        permit: &Permit,
        authority: &WorkerAuthority,
    ) -> Result<Self> {
        contract.digest()?;
        revision.validate()?;
        if permit.work_key != authority.work_key()? {
            return Err(Error::new(
                "scope_violation",
                "object request belongs to another admitted work item",
            ));
        }
        let origin = match authority {
            WorkerAuthority::Authorized(gate) => {
                let scope = gate.child_scope().ok_or_else(|| {
                    Error::new(
                        "scope_violation",
                        "coordination cannot acquire implementation sources",
                    )
                })?;
                if scope.write_repository.as_deref() != Some(&contract.repository.id)
                    && !scope.read_repositories.contains(&contract.repository.id)
                {
                    return Err(Error::new(
                        "scope_violation",
                        "object repository is outside child scope",
                    ));
                }
                Origin::Reference {
                    reference: format!("heads/{}", contract.configuration.staging),
                }
            }
            WorkerAuthority::Independent(gate) => {
                if gate.repository_id().to_string() != contract.repository.id
                    || gate.pull_request().head_sha != revision.commit
                {
                    return Err(Error::new(
                        "scope_violation",
                        "independent review source differs from its admitted PR",
                    ));
                }
                Origin::PullRequest {
                    expected: gate.pull_request().clone(),
                }
            }
        };
        Ok(Self {
            contract: contract.clone(),
            required_revisions: BTreeMap::from([(revision.commit.clone(), revision.tree.clone())]),
            revision,
            origin,
            permit: permit.clone(),
            authority_digest: authority.digest()?,
        })
    }
    pub fn for_delivery(
        contract: &RepositoryContract,
        revision: Revision,
        reference: &str,
        permit: &Permit,
        admission: &DeliveryAdmission,
    ) -> Result<Self> {
        Self::check_delivery(contract, permit, admission)?;
        revision.validate()?;
        if ![
            format!("heads/{}", contract.configuration.main),
            format!("heads/{}", contract.configuration.staging),
        ]
        .iter()
        .any(|r| r == reference)
        {
            return Err(Error::new(
                "scope_violation",
                "delivery acquisition requires a configured production/staging ref",
            ));
        }
        Ok(Self {
            contract: contract.clone(),
            required_revisions: BTreeMap::from([(revision.commit.clone(), revision.tree.clone())]),
            revision,
            origin: Origin::Reference {
                reference: reference.into(),
            },
            permit: permit.clone(),
            authority_digest: admission.fresh_gate().digest()?,
        })
    }
    pub fn for_release_merge(
        contract: &RepositoryContract,
        permit: &Permit,
        admission: &DeliveryAdmission,
        observed: &crate::delivery::ObservedReleaseStep,
    ) -> Result<Self> {
        Self::check_delivery(contract, permit, admission)?;
        let Some(crate::release::ObservedEffect::Merge {
            repository,
            pull_request,
            commit,
            parent_commit,
        }) = observed.effect()
        else {
            return Err(Error::new(
                "scope_violation",
                "verified release merge observation required",
            ));
        };
        if repository != &contract.repository
            || !matches!(
                observed.outcome(),
                crate::delivery::ReleaseStepOutcome::Succeeded
            )
        {
            return Err(Error::new(
                "scope_violation",
                "merge observation does not establish this repository source",
            ));
        }
        let receipt = observed
            .receipt()
            .ok_or_else(|| Error::new("missing_evidence", "merge receipt unavailable"))?;
        Ok(Self {
            contract: contract.clone(),
            revision: commit.clone(),
            required_revisions: BTreeMap::from([(commit.commit.clone(), commit.tree.clone())]),
            origin: Origin::Merge {
                number: *pull_request,
                parent: parent_commit.clone(),
                receipt: receipt.request_id,
            },
            permit: permit.clone(),
            authority_digest: admission.fresh_gate().digest()?,
        })
    }
    /// Import both roots only for a current corrected parent with an actual
    /// terminal predecessor effect. The old accepted candidate is provenance,
    /// never a new production effect authorization.
    pub(crate) fn for_successor_merge(
        contract: &RepositoryContract,
        permit: &Permit,
        admission: &DeliveryAdmission,
        predecessor: &crate::runtime_delivery::KnownPredecessorBaseline,
        observed: &crate::delivery::ObservedReleaseStep,
    ) -> Result<Self> {
        predecessor.require_effect(observed, admission.fresh_gate())?;
        if admission.scope().phase != crate::delivery::DeliveryPhase::CandidateAssembly
            || admission.scope().work.child.is_some()
            || predecessor.work() != &admission.scope().work
        {
            return Err(Error::new(
                "scope_violation",
                "successor source needs this corrected parent's assembly admission",
            ));
        }
        let mut request = Self::for_release_merge(contract, permit, admission, observed)?;
        let candidate = predecessor
            .integration_origin(observed)?
            .manifest()
            .candidates
            .iter()
            .find(|candidate| candidate.repository == contract.repository)
            .ok_or_else(|| Error::new("candidate_provenance", "predecessor candidate missing"))?;
        let result = candidate.result.as_ref().ok_or_else(|| {
            Error::new(
                "candidate_provenance",
                "predecessor candidate is incomplete",
            )
        })?;
        if result.tree != request.revision.tree {
            return Err(Error::new(
                "production_provenance",
                "observed predecessor merge is not exact current accepted product tree",
            ));
        }
        let current_main = predecessor
            .baseline(&contract.repository)?
            .production()
            .clone();
        let Origin::Merge {
            number,
            parent,
            receipt,
        } = request.origin
        else {
            unreachable!()
        };
        request.origin = Origin::LineageMerge {
            number,
            parent,
            receipt,
            receipt_digest: canonical_digest(
                1,
                observed.receipt().ok_or_else(|| {
                    Error::new("missing_evidence", "lineage merge receipt missing")
                })?,
            )?,
            current_main: current_main.clone(),
        };
        insert_revision(&mut request.required_revisions, result)?;
        insert_revision(&mut request.required_revisions, &candidate.production)?;
        insert_revision(&mut request.required_revisions, &current_main)?;
        Ok(request)
    }
    /// Exact already-completed, different-parent merge objects for a current
    /// parent assembly. The sealed origin authorizes reads only; the actual
    /// current assembly permit owns the local import and its process journal.
    pub(crate) fn for_completed_carry(
        contract: &RepositoryContract,
        permit: &Permit,
        admission: &DeliveryAdmission,
        proof: &crate::delivery::CompletedCarryObjects,
    ) -> Result<Self> {
        proof.verify_admission(admission, contract)?;
        let mut request = Self::for_release_merge(contract, permit, admission, proof.observed())?;
        let Origin::Merge {
            number,
            parent,
            receipt,
        } = request.origin
        else {
            unreachable!()
        };
        request.origin = Origin::CompletedCarryMerge {
            number,
            parent,
            receipt,
            receipt_digest: canonical_digest(
                1,
                proof.observed().receipt().ok_or_else(|| {
                    Error::new("missing_evidence", "completed carry merge receipt missing")
                })?,
            )?,
            current_main: proof.current_main().clone(),
            accepted_head: proof.accepted_head().clone(),
            completed_origin: proof.digest()?,
        };
        for revision in proof.required_revisions()? {
            insert_revision(&mut request.required_revisions, &revision)?;
        }
        Ok(request)
    }
    /// The squash and the reviewed head are distinct roots after squash merge.
    /// Both must be retained so the original patch can be verified unchanged.
    pub fn for_staging_merge(
        contract: &RepositoryContract,
        revision: Revision,
        permit: &Permit,
        admission: &DeliveryAdmission,
        pull_request: &crate::delivery::ChildPullRequest,
        pending: &crate::delivery::PendingStaging,
    ) -> Result<Self> {
        Self::check_delivery(contract, permit, admission)?;
        revision.validate()?;
        pull_request.matches(admission)?;
        pending.matches(admission)?;
        if admission.scope().phase != crate::delivery::DeliveryPhase::Staging
            || revision.tree != admission.scope().revision.tested_tree
            || pending.receipt().kind != "github_merge"
            || pending.receipt().runtime_id != permit.lease.runtime_id
        {
            return Err(Error::new(
                "scope_violation",
                "staging object request differs from the exact admitted merge",
            ));
        }
        Ok(Self {
            contract: contract.clone(),
            required_revisions: BTreeMap::from([
                (revision.commit.clone(), revision.tree.clone()),
                (
                    admission.scope().revision.head.clone(),
                    admission.scope().revision.tested_tree.clone(),
                ),
            ]),
            revision,
            origin: Origin::StagingMerge {
                expected: pull_request.provider().clone(),
                parent: admission.scope().revision.base.clone(),
                receipt: pending.receipt().request_id,
                receipt_digest: canonical_digest(1, pending.receipt())?,
            },
            permit: permit.clone(),
            authority_digest: admission.fresh_gate().digest()?,
        })
    }
    pub fn for_candidate(
        contract: &RepositoryContract,
        production: Revision,
        permit: &Permit,
        admission: &DeliveryAdmission,
        staged: &[&crate::delivery::StagedDelta],
        integrations: &[crate::git::VerifiedProductionIntegration],
    ) -> Result<Self> {
        Self::check_delivery(contract, permit, admission)?;
        production.validate()?;
        if admission.scope().phase != crate::delivery::DeliveryPhase::CandidateAssembly
            || admission.scope().work.child.is_some()
            || admission.scope().revision.base != production.commit
        {
            return Err(Error::new(
                "scope_violation",
                "candidate objects require the exact admitted parent production base",
            ));
        }
        let mut required_revisions = BTreeMap::new();
        insert_revision(&mut required_revisions, &production)?;
        let mut selected = Vec::new();
        for item in staged {
            let delta = &item.delta().manifest;
            if delta.repository != contract.repository
                || delta.parent_id != admission.scope().work.parent
                || delta.work_generation != admission.scope().work.generation.definition
                || delta.correction_generation != admission.scope().work.generation.correction
                || !admission
                    .fresh_gate()
                    .required_children()
                    .contains(&delta.child_id)
            {
                return Err(Error::new(
                    "scope_violation",
                    "candidate object contribution is outside current parent authority",
                ));
            }
            collect_delta_revisions(delta, 0, &mut required_revisions)?;
            selected.push((item.pull_request(), delta.clone()));
        }
        let mut produced = Vec::new();
        for integration in integrations {
            let record = integration.record();
            if record.repository != contract.repository
                || record.parent_id != admission.scope().work.parent
            {
                return Err(Error::new(
                    "scope_violation",
                    "prior production integration belongs to another parent or repository",
                ));
            }
            for revision in [
                &record.production_parent,
                &record.candidate,
                &record.merged,
                &record.observed_main,
            ] {
                insert_revision(&mut required_revisions, revision)?;
            }
            produced.push(record.clone());
        }
        Ok(Self {
            contract: contract.clone(),
            revision: production,
            required_revisions,
            origin: Origin::Candidate {
                reference: format!("heads/{}", contract.configuration.main),
                staged: selected,
                integrations: produced,
            },
            permit: permit.clone(),
            authority_digest: admission.fresh_gate().digest()?,
        })
    }
    fn check_delivery(
        contract: &RepositoryContract,
        permit: &Permit,
        admission: &DeliveryAdmission,
    ) -> Result<()> {
        if contract.digest()? != admission.scope().contract_digest
            || contract.repository != admission.scope().repository
            || permit.operation_id != admission.operation_id()
            || permit.work_key != admission.fresh_gate().work_key()?
        {
            return Err(Error::new(
                "scope_violation",
                "object acquisition differs from admitted delivery scope",
            ));
        }
        Ok(())
    }
    pub fn repository(&self) -> &RepositoryId {
        &self.contract.repository
    }
    pub fn contract_digest(&self) -> Result<String> {
        self.contract.digest()
    }
    pub fn revision(&self) -> &Revision {
        &self.revision
    }
    pub fn permit(&self) -> &Permit {
        &self.permit
    }
    pub fn authority_digest(&self) -> &str {
        &self.authority_digest
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &json!({"purpose":"exact_git_objects","contract":self.contract.digest()?,"revision":self.revision,"required_revisions":self.required_revisions,"origin":self.origin,"runtime":self.permit.lease.runtime_id,"operation":self.permit.operation_id,"scope":self.permit.scope_digest,"authority":self.authority_digest}),
        )
    }
    pub(crate) fn effect_id(&self) -> Result<Uuid> {
        let digest = self.digest()?;
        let hex = digest
            .rsplit(':')
            .next()
            .ok_or_else(|| Error::invalid("digest encoding"))?;
        let mut bytes: [u8; 16] = hex::decode(&hex[..32])
            .map_err(|_| Error::invalid("digest encoding"))?
            .try_into()
            .map_err(|_| Error::invalid("digest size"))?;
        bytes[6] = (bytes[6] & 15) | 128;
        bytes[8] = (bytes[8] & 63) | 128;
        Ok(Uuid::from_bytes(bytes))
    }
}

/// Callback is cache-only and must revalidate the original sealed authority and
/// repository contract. It is invoked inside the serialized final spawn gate.
pub type CurrentObjectAuthority = Arc<dyn Fn(&AuthorizedObjectRequest) -> Result<()> + Send + Sync>;
pub struct AcquisitionServices {
    pub control: Arc<ControlHost>,
    pub github: GithubClient,
    pub git: Git,
    pub records: PrivateDir,
    pub key: Arc<AttestationKey>,
    pub processes: Arc<ProcessJournal>,
    pub executions: Arc<ExecutionRegistry>,
    pub current: CurrentObjectAuthority,
    pub limits: ObjectLimits,
}
pub struct ObjectAcquisition {
    services: AcquisitionServices,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceRecord {
    schema_version: u32,
    request_digest: String,
    contract_digest: String,
    operator_checkout: PathBuf,
    provider_endpoint: String,
    provider_account: String,
    effect: Uuid,
    repository: RepositoryId,
    revision: Revision,
    required_revisions: BTreeMap<String, String>,
    origin: Origin,
    pack_digest: String,
    objects_digest: String,
    expanded_bytes: usize,
    directory_device: u64,
    directory_inode: u64,
    filesystem_digest: String,
    git_version: String,
    git_binary: kegari_platform::BinaryIdentity,
}
pub struct VerifiedObjectSource {
    directory: Arc<PrivateDir>,
    root: PathBuf,
    record: SourceRecord,
}
impl VerifiedObjectSource {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn repository(&self) -> &RepositoryId {
        &self.record.repository
    }
    pub fn revision(&self) -> &Revision {
        &self.record.revision
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(1, &self.record)
    }
    pub fn required_revision(&self, commit: &str) -> Option<Revision> {
        self.record
            .required_revisions
            .get(commit)
            .map(|tree| Revision {
                commit: commit.into(),
                tree: tree.clone(),
            })
    }
    pub(crate) fn operator_checkout(&self) -> &Path {
        &self.record.operator_checkout
    }
    pub(crate) fn validate_review_clone(
        &self,
        identity: &crate::git::IndependentCloneIdentity,
    ) -> Result<()> {
        let Origin::PullRequest { expected } = &self.record.origin else {
            return Err(Error::new(
                "scope_violation",
                "independent clone requires an admitted PR source",
            ));
        };
        if expected.repository_id.to_string() != identity.repository.id
            || expected.number != identity.pull_request
            || expected.head_sha != identity.head.commit
            || expected.base_sha != identity.base.commit
            || self.required_revision(&identity.head.commit).as_ref() != Some(&identity.head)
            || self.required_revision(&identity.base.commit).as_ref() != Some(&identity.base)
        {
            return Err(Error::new(
                "scope_violation",
                "independent clone differs from exact acquired PR head/base",
            ));
        }
        Ok(())
    }

    pub fn verify_for_clone(&self, git: &Git) -> Result<()> {
        self.revalidate_filesystem()?;
        if git.object_import_binary() != &self.record.git_binary {
            return Err(Error::new(
                "ownership_conflict",
                "source verifier differs from the originally pinned Git executable",
            ));
        }
        let objects: BTreeSet<String> =
            serde_json::from_slice(&self.directory.read_bytes("objects.json", 8 * 1024 * 1024)?)
                .map_err(|_| {
                    Error::new("record_corrupt", "retained object inventory is malformed")
                })?;
        if canonical_digest(1, &objects)? != self.record.objects_digest {
            return Err(Error::new(
                "provenance_mismatch",
                "retained object inventory changed",
            ));
        }
        git.verify_imported_objects(&self.root, &objects, &self.record.required_revisions)
    }
    pub(crate) fn revalidate_filesystem(&self) -> Result<()> {
        self.directory.revalidate()?;
        self.record.git_binary.revalidate(false)?;
        let metadata = fs::symlink_metadata(&self.root)
            .map_err(|e| Error::io("inspect acquired source", e))?;
        if metadata.dev() != self.record.directory_device
            || metadata.ino() != self.record.directory_inode
            || filesystem_digest(&self.root)? != self.record.filesystem_digest
        {
            return Err(Error::new(
                "ownership_conflict",
                "acquired Git source was replaced or changed",
            ));
        }
        Ok(())
    }
    pub(crate) fn marker(&self) -> PathBuf {
        self.directory.path().join("source.json")
    }
    pub(crate) fn contains_delta(&self, delta: &kegari_git::DeltaManifest) -> Result<bool> {
        let mut required = BTreeMap::new();
        collect_delta_revisions(delta, 0, &mut required)?;
        Ok(self.repository() == &delta.repository
            && required
                .iter()
                .all(|(oid, tree)| self.record.required_revisions.get(oid) == Some(tree)))
    }
}
impl ObjectAcquisition {
    pub fn new(services: AcquisitionServices) -> Result<Self> {
        services.limits.validate()?;
        services.records.revalidate()?;
        Ok(Self { services })
    }
    fn current(&self, request: &AuthorizedObjectRequest) -> Result<()> {
        self.services
            .control
            .validate_permit(&request.permit, || (self.services.current)(request))
    }
    pub(crate) async fn observe_origin(&self, request: &AuthorizedObjectRequest) -> Result<()> {
        self.current(request)?;
        let github = &self.services.github;
        let repo = &request.contract.provider;
        match &request.origin {
            Origin::Reference { reference } => {
                if github.reference(repo, reference).await? != request.revision.commit {
                    return Err(Error::new(
                        "stale_generation",
                        "selected source ref moved during acquisition",
                    ));
                }
            }
            Origin::PullRequest { expected } => {
                let current = github.pull_request(repo, expected.number).await?;
                verify_review_source(expected, &current)?;
            }
            Origin::Merge {
                number, receipt, ..
            } => {
                let retained = self.services.control.operation(*receipt)?;
                let current = github.pull_request(repo, *number).await?;
                if !matches!(
                    retained.phase,
                    ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                ) || current.merged != Some(true)
                    || current.merge_commit_sha.as_deref() != Some(&request.revision.commit)
                {
                    return Err(Error::new(
                        "stale_result",
                        "retained merge lineage is not currently substantiated",
                    ));
                }
            }
            Origin::LineageMerge {
                number,
                receipt,
                receipt_digest,
                current_main,
                ..
            }
            | Origin::CompletedCarryMerge {
                number,
                receipt,
                receipt_digest,
                current_main,
                ..
            } => {
                let retained = self.services.control.operation(*receipt)?;
                let current = github.pull_request(repo, *number).await?;
                if matches!(&request.origin, Origin::CompletedCarryMerge { accepted_head, .. } if current.head_sha != accepted_head.commit)
                    || canonical_digest(1, &retained)? != *receipt_digest
                    || !matches!(
                        retained.phase,
                        ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                    )
                    || current.merged != Some(true)
                    || current.merge_commit_sha.as_deref() != Some(request.revision.commit.as_str())
                    || current.base_ref != request.contract.configuration.main
                    || github
                        .reference(
                            repo,
                            &format!("heads/{}", request.contract.configuration.main),
                        )
                        .await?
                        != current_main.commit
                {
                    return Err(Error::new(
                        "stale_result",
                        "historical merge receipt or exact current production ref changed",
                    ));
                }
            }
            Origin::Candidate {
                reference,
                staged,
                integrations,
            } => {
                if github.reference(repo, reference).await? != request.revision.commit {
                    return Err(Error::new(
                        "stale_generation",
                        "candidate production base moved during acquisition",
                    ));
                }
                for (number, delta) in staged {
                    let current = github.pull_request(repo, *number).await?;
                    if current.merged != Some(true)
                        || current.merge_commit_sha.as_deref()
                            != Some(delta.staging_commit.commit.as_str())
                        || current.head_sha != delta.validated_head.commit
                        || current.base_ref != request.contract.configuration.staging
                    {
                        return Err(Error::new(
                            "stale_result",
                            "candidate contribution differs from actual staged PR",
                        ));
                    }
                    let object = github.commit(repo, &delta.staging_commit.commit).await?;
                    verify_single_parent(&object, &delta.staging_parent.commit)?;
                }
                for integration in integrations {
                    let current = github.pull_request(repo, integration.pull_request).await?;
                    if current.merged != Some(true)
                        || current.merge_commit_sha.as_deref()
                            != Some(integration.merged.commit.as_str())
                        || current.head_sha != integration.candidate.commit
                        || current.base_ref != request.contract.configuration.main
                    {
                        return Err(Error::new(
                            "stale_result",
                            "prior production contribution differs from actual merged PR",
                        ));
                    }
                    let object = github.commit(repo, &integration.merged.commit).await?;
                    verify_single_parent(&object, &integration.production_parent.commit)?;
                }
            }
            Origin::StagingMerge {
                expected,
                receipt,
                receipt_digest,
                ..
            } => {
                let retained = self.services.control.operation(*receipt)?;
                let current = github.pull_request(repo, expected.number).await?;
                if canonical_digest(1, &retained)? != *receipt_digest
                    || !matches!(
                        retained.phase,
                        ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                    )
                    || current.merged != Some(true)
                    || current.merge_commit_sha.as_deref() != Some(&request.revision.commit)
                    || current.id != expected.id
                    || current.number != expected.number
                    || current.head_sha != expected.head_sha
                    || current.head_ref != expected.head_ref
                    || current.base_ref != expected.base_ref
                    || current.head_repository_id != expected.head_repository_id
                    || current.author_id != expected.author_id
                {
                    return Err(Error::new(
                        "stale_result",
                        "current staging squash differs from the exact retained PR and receipt",
                    ));
                }
            }
        }
        for (commit, tree) in &request.required_revisions {
            let object = github.commit(repo, commit).await?;
            if object
                .pointer("/tree/sha")
                .and_then(serde_json::Value::as_str)
                != Some(tree)
            {
                return Err(Error::new(
                    "stale_generation",
                    "required provider source tree changed during acquisition",
                ));
            }
        }
        let commit = github.commit(repo, &request.revision.commit).await?;
        if commit
            .pointer("/tree/sha")
            .and_then(serde_json::Value::as_str)
            != Some(&request.revision.tree)
        {
            return Err(Error::new(
                "stale_generation",
                "provider source tree changed during acquisition",
            ));
        }
        if let Origin::Merge { parent, .. }
        | Origin::StagingMerge { parent, .. }
        | Origin::LineageMerge { parent, .. }
        | Origin::CompletedCarryMerge { parent, .. } = &request.origin
        {
            let parents = commit
                .get("parents")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| Error::new("provider_response", "merge parents missing"))?;
            if parents.len() != 1
                || parents[0].get("sha").and_then(serde_json::Value::as_str) != Some(parent)
            {
                return Err(Error::new(
                    "stale_result",
                    "observed squash ancestry differs from original effect",
                ));
            }
        }
        self.current(request)
    }
    async fn complete_roots(
        &self,
        mut request: AuthorizedObjectRequest,
    ) -> Result<AuthorizedObjectRequest> {
        self.current(&request)?;
        if let Origin::PullRequest { expected } = &request.origin {
            let object = self
                .services
                .github
                .commit(&request.contract.provider, &expected.base_sha)
                .await?;
            let tree = object
                .pointer("/tree/sha")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| Error::new("provider_response", "PR base tree unavailable"))?;
            insert_revision(
                &mut request.required_revisions,
                &Revision {
                    commit: expected.base_sha.clone(),
                    tree: tree.into(),
                },
            )?;
        }
        self.current(&request)?;
        Ok(request)
    }
    pub async fn acquire(&self, request: AuthorizedObjectRequest) -> Result<VerifiedObjectSource> {
        let request = self.complete_roots(request).await?;
        self.observe_origin(&request).await?;
        let id = request.effect_id()?;
        match self.services.control.operation(id) {
            Ok(_) => return self.observe_cached(&request).await,
            Err(error) if error.code == "record_missing" => (),
            Err(error) => return Err(error),
        }
        let session = self
            .services
            .github
            .object_session(
                &request.contract.provider,
                &request.contract.repository.hostname,
            )
            .await?;
        self.current(&request)?;
        let advertisement = session
            .advertisement(self.services.limits.protocol_bytes)
            .await?;
        pack::validate_advertisement(&advertisement, &self.services.limits)?;
        self.current(&request)?;
        let response = session
            .fetch(
                pack::fetch_request(&request.required_revisions.keys().cloned().collect())?,
                self.services.limits.pack_bytes + self.services.limits.protocol_bytes,
            )
            .await?;
        let limits = self.services.limits.clone();
        let validated = tokio::task::spawn_blocking(move || {
            pack::validate_pack(pack::extract_pack(&response, &limits)?, &limits)
        })
        .await
        .map_err(|_| {
            Error::new(
                "object_validation",
                "bounded object validator did not complete",
            )
        })??;
        self.observe_origin(&request).await?;
        let pack_digest = bytes_digest(&validated.bytes);
        let objects_digest = canonical_digest(1, &validated.objects)?;
        let expanded_bytes = validated.expanded_bytes;
        let request = Arc::new(request);
        let permit = request.permit.clone();
        let gate_control = self.services.control.clone();
        let gate_current = self.services.current.clone();
        let gate_request = request.clone();
        let start_gate: crate::git::GitStartGate = Arc::new(move |spawn| {
            gate_control.validate_permit(&gate_request.permit, || {
                gate_current(&gate_request)?;
                spawn()
            })
        });
        let records = &self.services.records;
        let git = self.services.git.clone();
        let processes = self.services.processes.clone();
        let executions = self.services.executions.clone();
        let expected = request.required_revisions.clone();
        let expected_for_record = validated.objects.clone();
        let mutation = MutationRequest {
            request_id: id,
            runtime_id: permit.lease.runtime_id,
            actor: Actor {
                identity: "kegari-object-acquisition".into(),
                roles: BTreeSet::from(["operate".into()]),
                principal: None,
            },
            kind: "git/object-import".into(),
            canonical_targets: vec![
                permit.work_key.clone(),
                format!(
                    "github:{}/{}",
                    request.repository().hostname,
                    request.repository().id
                ),
            ],
            expected: BTreeMap::from([
                ("object_request".into(), request.digest()?),
                ("pack_digest".into(), pack_digest.clone()),
            ]),
            payload: json!({"request":request.digest()?,"pack":pack_digest,"objects":objects_digest,"expanded_bytes":expanded_bytes}),
        };
        let (effect, (running, directory)) = self.services.control.begin_effect(
            mutation,
            &permit,
            || (self.services.current)(&request),
            |_| {
                let directory = Arc::new(records.child(&format!("objects-{id}"))?);
                directory.create_bytes_exclusive(
                    "intent.json",
                    &serde_json::to_vec(&self.services.key.sign(
                        "object_acquisition_intent",
                        &json!({"request":request.digest()?,"effect":id}),
                    )?)
                    .map_err(|_| Error::invalid("object intent encoding"))?,
                )?;
                let quarantine = directory.child("quarantine")?;
                let running = git.start_journaled_operation(
                    GitLimits::default(),
                    start_gate,
                    processes,
                    permit.clone(),
                    move |owned| {
                        owned.import_verified_object_pack(
                            &quarantine,
                            &validated.bytes,
                            &validated.objects,
                            &expected,
                        )
                    },
                )?;
                executions.register(permit.operation_id, id, Arc::new(running.cancellation()))?;
                Ok((running, directory))
            },
        )?;
        let cancellation = running.cancellation();
        let result = running.wait().await;
        if !cancellation.exited()? {
            return Err(Error::new(
                "operation_uncertain",
                "object import retains an unknown owned process",
            ));
        }
        self.services.executions.remove(permit.operation_id, id)?;
        if let Err(error) = result {
            self.services.control.complete_effect(
                effect,
                &permit,
                || (self.services.current)(&request),
                json!({"object_import":"failed","error":error}),
            )?;
            return Err(error);
        }
        let origin = self.observe_origin(&request).await;
        if let Err(error) = origin {
            self.services.control.complete_effect(
                effect,
                &permit,
                || (self.services.current)(&request),
                json!({"object_import":"rejected","error":error}),
            )?;
            return Err(error);
        }
        let root = directory.path().join("quarantine/repository.git");
        let metadata =
            fs::symlink_metadata(&root).map_err(|e| Error::io("inspect imported source", e))?;
        let fingerprint_root = root.clone();
        let filesystem_digest =
            tokio::task::spawn_blocking(move || filesystem_digest(&fingerprint_root))
                .await
                .map_err(|_| Error::new("object_validation", "object inventory join failed"))??;
        let record = SourceRecord {
            schema_version: 1,
            request_digest: request.digest()?,
            contract_digest: request.contract.digest()?,
            operator_checkout: PathBuf::from(&request.contract.configuration.source),
            provider_endpoint: self.services.github.object_origin_identity().0,
            provider_account: self.services.github.object_origin_identity().1,
            effect: id,
            repository: request.repository().clone(),
            revision: request.revision.clone(),
            required_revisions: request.required_revisions.clone(),
            origin: request.origin.clone(),
            pack_digest,
            objects_digest,
            expanded_bytes,
            directory_device: metadata.dev(),
            directory_inode: metadata.ino(),
            filesystem_digest,
            git_version: kegari_git::SUPPORTED_GIT_VERSION.into(),
            git_binary: self.services.git.object_import_binary().clone(),
        };
        directory.create_bytes_exclusive(
            "objects.json",
            &serde_json::to_vec(&expected_for_record)
                .map_err(|_| Error::invalid("object inventory encoding"))?,
        )?;
        directory.create_bytes_exclusive(
            "source.json",
            &serde_json::to_vec(&self.services.key.sign("verified_object_source", &record)?)
                .map_err(|_| Error::invalid("source record encoding"))?,
        )?;
        self.services.control.complete_effect(
            effect,
            &permit,
            || (self.services.current)(&request),
            json!({"object_import":"verified","source_digest":canonical_digest(1,&record)?}),
        )?;
        Ok(VerifiedObjectSource {
            directory,
            root,
            record,
        })
    }
    /// A missing/unacknowledged artifact never performs hidden work or acquires a
    /// new permit. Current-source freshness is reread independently of bytes.
    pub async fn observe_cached(
        &self,
        request: &AuthorizedObjectRequest,
    ) -> Result<VerifiedObjectSource> {
        let request = self.complete_roots(request.clone()).await?;
        self.observe_origin(&request).await?;
        let id = request.effect_id()?;
        let directory = Arc::new(PrivateDir::open(
            &self.services.records.path().join(format!("objects-{id}")),
        )?);
        let signed: Signed<SourceRecord> =
            serde_json::from_slice(&directory.read_bytes("source.json", 256 * 1024)?)
                .map_err(|_| Error::new("record_corrupt", "acquired source record is invalid"))?;
        self.services
            .key
            .verify("verified_object_source", &signed)?;
        let record = signed.payload;
        let receipt = self.services.control.operation(id)?;
        if record.schema_version != 1
            || record.request_digest != request.digest()?
            || record.contract_digest != request.contract.digest()?
            || (
                record.provider_endpoint.clone(),
                record.provider_account.clone(),
            ) != self.services.github.object_origin_identity()
            || record.effect != id
            || record.repository != *request.repository()
            || record.revision != request.revision
            || !matches!(
                receipt.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || receipt
                .observed_result
                .as_ref()
                .and_then(|v| v.get("source_digest"))
                .and_then(serde_json::Value::as_str)
                != Some(&canonical_digest(1, &record)?)
        {
            return Err(Error::new(
                "reconciliation_required",
                "object source lacks exact acknowledged promotion",
            ));
        }
        record.git_binary.revalidate(false)?;
        let objects: BTreeSet<String> =
            serde_json::from_slice(&directory.read_bytes("objects.json", 8 * 1024 * 1024)?)
                .map_err(|_| Error::new("record_corrupt", "object inventory invalid"))?;
        if canonical_digest(1, &objects)? != record.objects_digest {
            return Err(Error::new("record_corrupt", "object inventory changed"));
        }
        let root = directory.path().join("quarantine/repository.git");
        let metadata =
            fs::symlink_metadata(&root).map_err(|e| Error::io("inspect cached source", e))?;
        let check_root = root.clone();
        let actual = tokio::task::spawn_blocking(move || filesystem_digest(&check_root))
            .await
            .map_err(|_| Error::new("object_validation", "source inventory join failed"))??;
        if metadata.dev() != record.directory_device
            || metadata.ino() != record.directory_inode
            || actual != record.filesystem_digest
        {
            return Err(Error::new(
                "ownership_conflict",
                "cached source was changed or replaced",
            ));
        }
        self.current(&request)?;
        Ok(VerifiedObjectSource {
            directory,
            root,
            record,
        })
    }
}

fn insert_revision(revisions: &mut BTreeMap<String, String>, revision: &Revision) -> Result<()> {
    revision.validate()?;
    if revisions
        .get(&revision.commit)
        .is_some_and(|tree| tree != &revision.tree)
    {
        return Err(Error::new(
            "provenance_mismatch",
            "one required commit has conflicting trees",
        ));
    }
    revisions.insert(revision.commit.clone(), revision.tree.clone());
    if revisions.len() > 64 {
        return Err(Error::new(
            "source_limit_exceeded",
            "exact source request exceeds 64 revision roots",
        ));
    }
    Ok(())
}
fn collect_delta_revisions(
    delta: &kegari_git::DeltaManifest,
    depth: usize,
    revisions: &mut BTreeMap<String, String>,
) -> Result<()> {
    if depth > 8 {
        return Err(Error::new(
            "source_limit_exceeded",
            "replacement history exceeds supported depth",
        ));
    }
    for revision in [
        &delta.validated_base,
        &delta.validated_head,
        &delta.staging_parent,
        &delta.staging_commit,
    ] {
        insert_revision(revisions, revision)?;
    }
    if let Some(replacement) = &delta.replacement {
        for original in &replacement.originals {
            if original.repository != delta.repository
                || original.parent_id != delta.parent_id
                || original.child_id != delta.child_id
            {
                return Err(Error::new(
                    "provenance_mismatch",
                    "replacement crosses its original scope",
                ));
            }
            collect_delta_revisions(original, depth + 1, revisions)?;
        }
    }
    Ok(())
}
fn verify_single_parent(object: &serde_json::Value, expected: &str) -> Result<()> {
    let parents = object
        .get("parents")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Error::new("provider_response", "merge parents missing"))?;
    if parents.len() != 1
        || parents[0].get("sha").and_then(serde_json::Value::as_str) != Some(expected)
    {
        return Err(Error::new(
            "provenance_mismatch",
            "actual squash parent differs from retained contribution",
        ));
    }
    Ok(())
}
fn filesystem_digest(root: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    let mut total = 0u64;
    let mut count = 0usize;
    while let Some(path) = pending.pop() {
        count += 1;
        if count > 200_000 {
            return Err(Error::new(
                "source_limit_exceeded",
                "object cache has too many entries",
            ));
        }
        let metadata =
            fs::symlink_metadata(&path).map_err(|e| Error::io("inspect object cache entry", e))?;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.file_type().is_symlink()
            || metadata.mode() & 0o022 != 0
        {
            return Err(Error::new(
                "unsafe_storage",
                "object cache contains unsafe ownership, links or write permissions",
            ));
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(&path).map_err(|e| Error::io("enumerate object cache", e))? {
                if pending.len().saturating_add(count) >= 200_000 {
                    return Err(Error::new(
                        "source_limit_exceeded",
                        "object cache enumeration exceeds its bound",
                    ));
                }
                pending.push(
                    entry
                        .map_err(|e| Error::io("read object cache entry", e))?
                        .path(),
                );
            }
        } else if metadata.is_file() && metadata.nlink() == 1 {
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| Error::invalid("object cache size overflow"))?;
            if total > 512 * 1024 * 1024 {
                return Err(Error::new(
                    "source_limit_exceeded",
                    "object cache exceeds its filesystem bound",
                ));
            }
            let mut file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)
                .map_err(|e| Error::io("open object cache file", e))?;
            let opened = file
                .metadata()
                .map_err(|e| Error::io("inspect opened cache file", e))?;
            if (
                metadata.dev(),
                metadata.ino(),
                metadata.len(),
                metadata.mode(),
            ) != (opened.dev(), opened.ino(), opened.len(), opened.mode())
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "object cache entry changed during open",
                ));
            }
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 65536];
            let mut bytes = 0u64;
            loop {
                let n = file
                    .read(&mut buffer)
                    .map_err(|e| Error::io("hash object cache", e))?;
                if n == 0 {
                    break;
                }
                bytes += n as u64;
                if bytes > metadata.len() {
                    return Err(Error::new("ownership_conflict", "object cache file grew"));
                }
                hash.update(&buffer[..n]);
            }
            if bytes != metadata.len() {
                return Err(Error::new("ownership_conflict", "object cache file shrank"));
            }
            let after = file
                .metadata()
                .map_err(|e| Error::io("recheck hashed cache file", e))?;
            let named = fs::symlink_metadata(&path)
                .map_err(|e| Error::io("recheck named cache file", e))?;
            let identity = |m: &fs::Metadata| {
                (
                    m.dev(),
                    m.ino(),
                    m.len(),
                    m.mode(),
                    m.mtime(),
                    m.mtime_nsec(),
                    m.ctime(),
                    m.ctime_nsec(),
                )
            };
            if identity(&after) != identity(&metadata) || identity(&named) != identity(&metadata) {
                return Err(Error::new(
                    "ownership_conflict",
                    "object cache file changed while hashing",
                ));
            }
            files.insert(
                path.strip_prefix(root)
                    .map_err(|_| Error::invalid("object root containment"))?
                    .to_path_buf(),
                (metadata.mode(), bytes, hex::encode(hash.finalize())),
            );
        } else {
            return Err(Error::new(
                "unsafe_storage",
                "object cache contains a special or linked file",
            ));
        }
    }
    canonical_digest(1, &files)
}

fn verify_review_source(
    expected: &kegari_providers::PullRequest,
    current: &kegari_providers::PullRequest,
) -> Result<()> {
    if current.state != "open"
        || current.draft
        || current.merged == Some(true)
        || current.head_repository_id.is_none()
        || canonical_digest(1, current)? != canonical_digest(1, expected)?
    {
        return Err(Error::new(
            "stale_generation",
            "selected review PR changed during acquisition",
        ));
    }
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
#[path = "remote_objects_tests.rs"]
mod tests;

#[path = "remote_objects_retained.rs"]
mod retained;
pub(crate) use retained::{retained_sources, select_delta_source};

#[path = "remote_objects_read_history.rs"]
mod read_history;
pub(crate) use read_history::RetainedObjectReadHistory;
