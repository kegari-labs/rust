//! Supplied disposable source is inspected through actual admitted, journaled
//! read-only Git. Synthetic provider DTOs cannot construct this capability.
use super::*;
use crate::{
    delivery::RepositoryContract,
    git::{Git, GitLimits, HostGitExecutionScope, VerifiedHostReadRoot},
    host_process::{HostProcessServices, LocalProcessPermit, MetadataRequest},
    process_records::ProcessJournal,
};
use kegari_git::{RepositoryId, Revision};
use kegari_platform::BinaryIdentity;
pub(crate) struct VerifiedEvaluationSource {
    workspace: Uuid,
    operation: Uuid,
    namespace_digest: String,
    subject: subject::TrialSubject,
    review_text: kegari_providers::PullRequestText,
    digest: String,
    root: VerifiedHostReadRoot,
    base: Revision,
    head: Revision,
    files_digest: String,
    receipt: Receipt,
}
impl VerifiedEvaluationSource {
    pub(crate) fn workspace(&self) -> Uuid {
        self.workspace
    }
    pub(crate) fn operation(&self) -> Uuid {
        self.operation
    }
    pub(crate) fn namespace_digest(&self) -> &str {
        &self.namespace_digest
    }
    pub(crate) fn review_text(&self) -> &kegari_providers::PullRequestText {
        &self.review_text
    }
    pub(crate) fn pull_request(&self) -> u64 {
        self.subject.number()
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn base(&self) -> &Revision {
        &self.base
    }
    pub(crate) fn head(&self) -> &Revision {
        &self.head
    }
    pub(super) fn verify_closed_metadata(&self, runtime: &RuntimeCoordinator) -> Result<()> {
        let actual = runtime.control.operation(self.receipt.operation_id)?;
        verify_closed_metadata_receipt(&actual, runtime.runtime_id, self.operation)?;
        if canonical_digest(1, &actual)? != canonical_digest(1, &self.receipt)? {
            return Err(Error::new(
                "record_corrupt",
                "retained evaluation metadata closure changed",
            ));
        }
        Ok(())
    }
    pub(super) fn metadata_operation(&self) -> Uuid {
        self.receipt.operation_id
    }
    pub(super) fn validate(&self, runtime: &RuntimeCoordinator) -> Result<()> {
        self.subject.verify(runtime)?;
        self.root.revalidate_filesystem()?;
        if crate::config::config_revision(&runtime.configuration.snapshot()?)?
            != self.root.config_revision()
            || canonical_digest(1, &runtime.control.operation(self.receipt.operation_id)?)?
                != canonical_digest(1, &self.receipt)?
            || crate::evaluation::tree_digest(self.root.root())? != self.files_digest
        {
            return Err(Error::new(
                "evaluation_fidelity",
                "actual admitted evaluation source or metadata receipt changed",
            ));
        }
        let providers = runtime
            .services
            .inputs
            .evaluation_providers()
            .ok_or_else(|| {
                Error::new(
                    "evaluation_unavailable",
                    "evaluation namespace registry is absent",
                )
            })?;
        if providers.declaration_digest(self.workspace)? != self.namespace_digest {
            return Err(Error::new(
                "stale_config",
                "evaluation namespace declaration changed",
            ));
        }
        Ok(())
    }
}
pub(super) async fn observe(
    runtime: Arc<RuntimeCoordinator>,
    request: Arc<VerifiedEvaluationRequest>,
    fixture: &crate::evaluation::PreparedFixture,
) -> Result<VerifiedEvaluationSource> {
    let review_text = fixture.independent_review_text()?;
    let review_text_digest = canonical_digest(1, &review_text)?;
    let configured = request
        .configuration
        .workspaces
        .values()
        .find(|w| w.id == request.workspace)
        .ok_or_else(|| Error::invalid("declared workspace absent"))?;
    let repository = configured
        .repositories
        .get(&request.binding.repository_id.to_string())
        .ok_or_else(|| Error::invalid("declared repository absent"))?;
    let providers = runtime
        .services
        .inputs
        .evaluation_providers()
        .ok_or_else(|| {
            Error::new(
                "evaluation_unavailable",
                "reserved provider registry is absent",
            )
        })?;
    let namespace_digest = providers.declaration_digest(request.workspace)?;
    let actor = request.configuration.connections[&configured.github_connection]
        .account_id
        .parse::<u64>()
        .map_err(|_| Error::invalid("synthetic GitHub identity is invalid"))?;
    let contract = RepositoryContract {
        configuration_name: request.binding.repository_id.to_string(),
        repository: RepositoryId {
            hostname: repository.hostname.clone(),
            id: request.binding.repository_id.to_string(),
        },
        provider: kegari_providers::RepositoryIdentity {
            id: request.binding.repository_id,
            owner: "evaluation".into(),
            name: "fixture".into(),
            owner_id: actor,
            organization_id: None,
        },
        configuration: repository.clone(),
    };
    let root = crate::git::configured_host_read_root(
        &request.configuration,
        request.workspace,
        &contract,
    )?;
    let git = Git::from_pinned_executable(resolve_git()?)?;
    let journal = Arc::new(ProcessJournal::new(
        runtime.registration.root().child("worker-processes")?,
        request.configuration.runtime.id,
    )?);
    let metadata = LocalProcessPermit::metadata_observed(
        HostProcessServices {
            control: runtime.control.clone(),
            registration: runtime.registration.clone(),
            configuration: runtime.configuration.clone(),
            processes: journal,
            executions: runtime.executions.clone(),
        },
        MetadataRequest {
            request_id: source_operation(request.request.request_id)?,
            work_key: format!(
                "evaluation:{}:{}",
                request.configuration.runtime.id, request.workspace
            ),
            roots: vec![root.clone()],
            git_binary: BinaryIdentity::inspect(git.executable(), true)?,
        },
        |snapshot| {
            request.current(&runtime, snapshot)?;
            request.subject.verify(&runtime)
        },
    )?;
    let source_root = root.root().to_path_buf();
    let base_commit = request.binding.base_commit.clone();
    let head_commit = request.binding.head_commit.clone();
    let expected = canonical_digest(1, &fixture.before)?;
    let guarded = metadata.clone();
    let read = tokio::task::spawn_blocking(move || {
        let git = git.with_host_execution_scope(HostGitExecutionScope {
            limits: GitLimits::default(),
            authority: guarded,
        })?;
        let base = git.exact_revision(&source_root, &base_commit)?;
        let head = git.exact_revision(&source_root, &head_commit)?;
        if !git.is_ancestor(&source_root, &base.commit, &head.commit)? {
            return Err(Error::new(
                "evaluation_fidelity",
                "declared base is not an actual ancestor of the supplied head",
            ));
        }
        // Actual first-slice source contract is one plain changed commit over its declared base.
        // The metadata reader verifies that edge rather than inventing parents in the adapter.
        git.publication_objects(&source_root, &base, &head)?;
        git.verify_execution_tree(&source_root, &head, &BTreeSet::new())?;
        let actual = crate::evaluation::tree_digest(&source_root)?;
        if actual != expected {
            return Err(Error::new(
                "evaluation_fidelity",
                "actual clean Git source differs from exact declared fixture files/modes",
            ));
        }
        git.verify_execution_tree(&source_root, &head, &BTreeSet::new())?;
        if crate::evaluation::tree_digest(&source_root)? != actual {
            return Err(Error::new(
                "unstable_observation",
                "evaluation source changed during admitted inspection",
            ));
        }
        Ok((base, head, actual))
    })
    .await
    .map_err(|_| {
        Error::new(
            "operation_uncertain",
            "admitted evaluation source reader did not join",
        )
    })
    .and_then(|r| r);
    let receipt=metadata.finish(json!({"action":"evaluation_source_read","operation":request.request.request_id,"subject":request.subject.reference(),"review_text_digest":review_text_digest,"result":read.as_ref().map(|(base,head,files)|json!({"base":base,"head":head,"files":files})).map_err(|error|json!({"code":error.code,"message":error.message}))}))?;
    let (base, head, files_digest) = read?;
    let digest = canonical_digest(
        1,
        &json!({"workspace":request.workspace,"operation":request.request.request_id,"namespace":namespace_digest,"subject":request.subject.reference(),"review_text_digest":review_text_digest,"root":root.digest()?,"base":base,"head":head,"files":files_digest,"receipt":receipt}),
    )?;
    let proof = VerifiedEvaluationSource {
        workspace: request.workspace,
        operation: request.request.request_id,
        namespace_digest,
        subject: request.subject.clone(),
        review_text,
        digest,
        root,
        base,
        head,
        files_digest,
        receipt,
    };
    runtime.control.serialize_observation(|snapshot| {
        request.current(&runtime, snapshot)?;
        proof.validate(&runtime)
    })?;
    Ok(proof)
}
pub(super) fn source_operation(operation: Uuid) -> Result<Uuid> {
    let digest = canonical_digest(
        1,
        &json!({"kind":"evaluation_source_metadata","worker":operation}),
    )?;
    let bytes = digest
        .rsplit(':')
        .next()
        .ok_or_else(|| Error::invalid("source identity digest missing"))?;
    Uuid::parse_str(&bytes[..32])
        .map_err(|_| Error::invalid("source metadata UUID encoding invalid"))
}
fn resolve_git() -> Result<PathBuf> {
    let path = std::env::var_os("PATH")
        .ok_or_else(|| Error::new("prerequisite", "Git search path unavailable"))?;
    for directory in std::env::split_paths(&path).filter(|p| p.is_absolute()) {
        let candidate = directory.join("git");
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .map_err(|e| Error::io("resolve evaluation Git", e));
        }
    }
    Err(Error::new("prerequisite", "Git executable unavailable"))
}

// Data validation only. The caller also retains the real source capability and
// the exact whole receipt produced after the metadata process handles joined.
fn verify_closed_metadata_receipt(receipt: &Receipt, runtime: Uuid, worker: Uuid) -> Result<()> {
    let result = receipt.observed_result.as_ref().ok_or_else(|| {
        Error::new(
            "record_corrupt",
            "evaluation metadata has no closed observation",
        )
    })?;
    if receipt.runtime_id != runtime
        || receipt.operation_id != source_operation(worker)?
        || receipt.request_id != receipt.operation_id
        || receipt.kind != "host_metadata"
        || receipt.phase != ReceiptPhase::Acknowledged
        || result["owned_handles_joined"] != json!(true)
        || result["delivery_evidence"] != json!(false)
        || result["metadata_observation"]["action"] != json!("evaluation_source_read")
        || result["metadata_observation"]["operation"] != json!(worker)
    {
        return Err(Error::new(
            "record_corrupt",
            "evaluation metadata is not the exact joined read result",
        ));
    }
    Ok(())
}
#[cfg(test)]
mod retirement_observation_tests {
    use super::*;
    #[test]
    fn metadata_retirement_data_requires_exact_worker_and_joined_read() {
        // Serialized observation checks only: this never constructs a source,
        // process token, namespace exit or native completion capability.
        let worker = Uuid::new_v4();
        let runtime = Uuid::new_v4();
        let id = source_operation(worker).unwrap();
        let receipt = Receipt {
            operation_id: id,
            request_id: id,
            runtime_id: runtime,
            owner_generation: 1,
            actor: "observed-host".into(),
            kind: "host_metadata".into(),
            canonical_targets: vec!["observed-scope".into()],
            payload_digest: kegari_digest::bytes_digest(b"payload"),
            expected: BTreeMap::new(),
            phase: ReceiptPhase::Acknowledged,
            remote_ids: BTreeMap::new(),
            observed_result: Some(
                json!({"owned_handles_joined":true,"delivery_evidence":false,"metadata_observation":{"action":"evaluation_source_read","operation":worker}}),
            ),
            cancellation: "not_requested".into(),
            updated_at: Utc::now(),
        };
        verify_closed_metadata_receipt(&receipt, runtime, worker).unwrap();
        for field in ["owned_handles_joined", "delivery_evidence"] {
            let mut changed = receipt.clone();
            changed.observed_result.as_mut().unwrap()[field] = json!(field == "delivery_evidence");
            assert!(verify_closed_metadata_receipt(&changed, runtime, worker).is_err());
        }
        let mut changed = receipt.clone();
        changed.phase = ReceiptPhase::Reconciled;
        assert!(verify_closed_metadata_receipt(&changed, runtime, worker).is_err());
        assert!(verify_closed_metadata_receipt(&receipt, runtime, Uuid::new_v4()).is_err());
        assert!(verify_closed_metadata_receipt(&receipt, Uuid::new_v4(), worker).is_err());
    }
}
