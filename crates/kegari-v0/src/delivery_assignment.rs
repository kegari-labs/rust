//! Opaque host assignments for validation and review. Model/CLI JSON cannot
//! select a role, replace context, or grant an arbitrary source directory.
use super::*;
use crate::{instructions::AgentRole, isolation::ValidatedIsolation, worker::WorkerExecution};

#[derive(Clone)]
enum SourceOwnership {
    Child {
        execution: Arc<WorkerExecution>,
        isolation: ValidatedIsolation,
    },
    Candidate(crate::git::VerifiedHostReadRoot),
}
#[derive(Clone)]
pub struct DeliverySource {
    repository: RepositoryId,
    root: PathBuf,
    revision: crate::git::Revision,
    ownership: SourceOwnership,
}
impl DeliverySource {
    pub fn repository(&self) -> &RepositoryId {
        &self.repository
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn revision(&self) -> &crate::git::Revision {
        &self.revision
    }
    pub fn revalidate_filesystem(&self) -> Result<()> {
        match &self.ownership {
            SourceOwnership::Child {
                execution,
                isolation,
            } => {
                execution.ownership.verify(&execution.binding, isolation)?;
                if isolation.plan().repository != self.root
                    || execution.binding.repository != self.repository
                {
                    return Err(Error::new(
                        "execution_ownership",
                        "delivery source differs from actual completed worker root",
                    ));
                }
                Ok(())
            }
            SourceOwnership::Candidate(root) => root.revalidate_filesystem(),
        }
    }
    /// Invoke only through the new preparation operation's journaled Git handle.
    pub fn verify_for_clone(&self, git: &Git) -> Result<()> {
        self.revalidate_filesystem()?;
        let outputs = match &self.ownership {
            SourceOwnership::Child { execution, .. } => execution
                .check_outputs
                .values()
                .flatten()
                .cloned()
                .collect(),
            SourceOwnership::Candidate(_) => BTreeSet::new(),
        };
        git.verify_execution_tree(&self.root, &self.revision, &outputs)
    }
    pub fn digest(&self) -> Result<String> {
        let ownership = match &self.ownership {
            SourceOwnership::Child {
                execution,
                isolation,
            } => {
                json!({"owned_directory":execution.ownership.digest(),"binding":execution.binding,"isolation":isolation.digest()})
            }
            SourceOwnership::Candidate(root) => json!({"candidate_root":root.digest()?}),
        };
        canonical_digest(
            1,
            &json!({"repository":self.repository,"root":self.root,"revision":self.revision,"ownership":ownership}),
        )
    }
}
#[derive(Clone)]
pub struct DeliveryWorkerAssignment {
    authority: FreshGate,
    phase: String,
    role: AgentRole,
    context: Value,
    source: Option<DeliverySource>,
    read_sources: Vec<DeliverySource>,
    candidate_vector_digest: Option<String>,
    contract_digest: String,
}
impl DeliveryWorkerAssignment {
    pub fn authority(&self) -> &FreshGate {
        &self.authority
    }
    pub fn phase(&self) -> &str {
        &self.phase
    }
    pub fn role(&self) -> AgentRole {
        self.role
    }
    pub fn context(&self) -> &Value {
        &self.context
    }
    pub fn source(&self) -> Option<&DeliverySource> {
        self.source.as_ref()
    }
    pub fn read_sources(&self) -> &[DeliverySource] {
        &self.read_sources
    }
    pub fn candidate_vector_digest(&self) -> Option<&str> {
        self.candidate_vector_digest.as_deref()
    }
    pub fn contract_digest(&self) -> &str {
        &self.contract_digest
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &json!({"authority":self.authority.digest()?,"phase":self.phase,"role":self.role,"context":self.context,
            "source":self.source.as_ref().map(DeliverySource::digest).transpose()?,"read_sources":self.read_sources.iter().map(DeliverySource::digest).collect::<Result<Vec<_>>>()?,
            "vector":self.candidate_vector_digest,"contract":self.contract_digest}),
        )
    }
    pub fn verify_binding(&self, gate: &FreshGate, phase: &str) -> Result<()> {
        self.authority
            .compare_current(gate, Utc::now(), chrono::Duration::seconds(30))?;
        if self.phase != phase {
            return Err(Error::new(
                "worker_binding",
                "delivery assignment is for another finite phase",
            ));
        }
        for source in self.source.iter().chain(&self.read_sources) {
            source.revalidate_filesystem()?;
        }
        Ok(())
    }
}
impl DeliveryCoordinator {
    pub fn child_validation_assignment(
        &self,
        admission: &DeliveryAdmission,
        claim: &CompletedChildClaim,
        source: &SealedChildSource,
        completion: &Arc<crate::worker_run::RunCompletion>,
        mut context: Value,
    ) -> Result<DeliveryWorkerAssignment> {
        self.current(admission, &[DeliveryPhase::LocalValidation])?;
        let owned = child_source(admission, claim, source, completion)?;
        let object = context
            .as_object_mut()
            .ok_or_else(|| Error::invalid("validation base context must be an object"))?;
        object.insert("validation_assignment".into(),json!({"work":admission.scope.work,"revision":admission.scope.revision,"source_claim":claim.digest(),
            "kind":"local","configured_checks":self.contract(admission)?.configuration.local_checks,
            "outcome_claim":{"schema_version":1,"kind":"validation","summary":"actual configured check outcome"}}));
        Ok(DeliveryWorkerAssignment {
            authority: admission.gate.clone(),
            phase: "child_local_validation".into(),
            role: AgentRole::Validation,
            context,
            source: Some(owned),
            read_sources: vec![],
            candidate_vector_digest: None,
            contract_digest: admission.scope.contract_digest.clone(),
        })
    }
    pub fn child_review_worker_assignment(
        &self,
        admission: &DeliveryAdmission,
        claim: &CompletedChildClaim,
        source: &SealedChildSource,
        completion: &Arc<crate::worker_run::RunCompletion>,
        review: &ChildReviewAssignment,
    ) -> Result<DeliveryWorkerAssignment> {
        self.current(
            admission,
            &[DeliveryPhase::ReviewPublication, DeliveryPhase::NoChange],
        )?;
        review.matches_worker(admission)?;
        Ok(DeliveryWorkerAssignment {
            authority: admission.gate.clone(),
            phase: "child_codex_review".into(),
            role: AgentRole::Review,
            context: review.context().clone(),
            source: Some(child_source(admission, claim, source, completion)?),
            read_sources: vec![],
            candidate_vector_digest: None,
            contract_digest: admission.scope.contract_digest.clone(),
        })
    }
    pub fn parent_review_worker_assignment(
        &self,
        contract: &ParentContract,
        assignment: &ParentReviewAssignment,
    ) -> Result<DeliveryWorkerAssignment> {
        (self.current_gate)(&contract.gate)?;
        assignment.matches_worker(contract)?;
        Ok(DeliveryWorkerAssignment {
            authority: contract.gate.clone(),
            phase: "parent_evidence_review".into(),
            role: AgentRole::Coordination,
            context: assignment.context().clone(),
            source: None,
            read_sources: vec![],
            candidate_vector_digest: Some(assignment.vector_digest().into()),
            contract_digest: contract.digest()?,
        })
    }
    pub async fn integrated_worker_assignment(
        &self,
        contract: &ParentContract,
        gate: &FreshGate,
        vector: &ParentCandidateVector,
        observer: &crate::deployment::ProductionObserver,
        slices: &[&IsolatedSlice],
        mut context: Value,
    ) -> Result<DeliveryWorkerAssignment> {
        self.reobserve_parent_vector(contract, observer, vector)
            .await?;
        if gate.work().child != Some(contract.e2e_child())
            || gate.work().parent != contract.work().parent
            || gate.config_revision() != contract.gate.config_revision()
            || gate.workflow_digest() != contract.gate.workflow_digest()
        {
            return Err(Error::new(
                "integrated_gate",
                "integrated assignment differs from current required E2E child",
            ));
        }
        (self.current_gate)(gate)?;
        let mut sources = BTreeMap::new();
        for slice in slices {
            if !vector.candidates().contains(slice.manifest()) {
                return Err(Error::new(
                    "candidate_provenance",
                    "integrated source outside exact vector",
                ));
            }
            let revision = slice
                .manifest()
                .result
                .clone()
                .ok_or_else(|| Error::new("candidate_provenance", "candidate is unfinished"))?;
            let observation = self.observation(
                contract.gate.clone(),
                super::parent::candidate_binding(slice.manifest())?,
            )?;
            let root = self.retained_candidate_host_root(&observation, slice.operation_id())?;
            let source = DeliverySource {
                repository: slice.manifest().repository.clone(),
                root: slice.path().into(),
                revision,
                ownership: SourceOwnership::Candidate(root),
            };
            if sources.insert(source.repository.clone(), source).is_some() {
                return Err(Error::new(
                    "candidate_provenance",
                    "duplicate integrated source",
                ));
            }
        }
        if sources.keys().cloned().collect::<BTreeSet<_>>() != *contract.repositories() {
            return Err(Error::new(
                "candidate_provenance",
                "integrated assignment omits a candidate repository",
            ));
        }
        let primary = gate
            .child_scope()
            .and_then(|s| s.write_repository.as_ref())
            .ok_or_else(|| {
                Error::new(
                    "integrated_gate",
                    "E2E child requires its configured execution repository",
                )
            })?;
        let key = sources
            .keys()
            .find(|r| &r.id == primary)
            .cloned()
            .ok_or_else(|| {
                Error::new(
                    "integrated_gate",
                    "E2E execution repository is outside vector",
                )
            })?;
        let primary = sources.remove(&key).expect("selected source");
        context.as_object_mut().ok_or_else(||Error::invalid("integrated base context must be an object"))?.insert("validation_assignment".into(),json!({"kind":"integrated","vector":vector.digest(),"candidates":vector.candidates(),"test_resources":contract.test_resources(),"configured_checks":contract.integrated_commands()}));
        Ok(DeliveryWorkerAssignment {
            authority: gate.clone(),
            phase: "parent_integrated_validation".into(),
            role: AgentRole::Validation,
            context,
            source: Some(primary),
            read_sources: sources.into_values().collect(),
            candidate_vector_digest: Some(vector.digest().into()),
            contract_digest: contract.digest()?,
        })
    }
}
fn child_source(
    admission: &DeliveryAdmission,
    claim: &CompletedChildClaim,
    source: &SealedChildSource,
    completion: &Arc<crate::worker_run::RunCompletion>,
) -> Result<DeliverySource> {
    source.matches_worker(admission, claim)?;
    super::child::completed_claim_run(claim, completion)?;
    let execution = completion.execution.clone().ok_or_else(|| {
        Error::new(
            "execution_ownership",
            "completed claim lacks original actual source capability",
        )
    })?;
    let contract = completion.contract.as_ref().ok_or_else(|| {
        Error::new(
            "execution_ownership",
            "completed claim lacks actual worker contract",
        )
    })?;
    let isolation = contract.isolation().clone();
    execution.ownership.verify(&execution.binding, &isolation)?;
    if execution.binding.work != admission.scope.work
        || execution.binding.operation_id != claim.operation_id()
        || execution.binding.repository != admission.scope.repository
    {
        return Err(Error::new(
            "execution_ownership",
            "completed source capability differs from claim",
        ));
    }
    Ok(DeliverySource {
        repository: admission.scope.repository.clone(),
        root: isolation.plan().repository.clone(),
        revision: crate::git::Revision {
            commit: source.revision().head.clone(),
            tree: source.revision().tested_tree.clone(),
        },
        ownership: SourceOwnership::Child {
            execution,
            isolation,
        },
    })
}
