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
    parent_reassembly: Option<Arc<ParentReassemblyWork>>,
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
    pub(crate) fn parent_reassembly(&self) -> Option<&ParentReassemblyWork> {
        self.parent_reassembly.as_deref()
    }
    pub(crate) fn with_parent_reassembly(
        mut self,
        proof: Arc<ParentReassemblyWork>,
    ) -> Result<Self> {
        if self.parent_reassembly.is_some() {
            return Err(Error::new(
                "worker_binding",
                "parent assignment already has a cause",
            ));
        }
        proof.verify_assignment(&self)?;
        self.parent_reassembly = Some(proof);
        Ok(self)
    }
    pub fn digest(&self) -> Result<String> {
        let base = self.base_digest()?;
        match &self.parent_reassembly {
            Some(proof) => canonical_digest(
                1,
                &json!({"assignment":base,"parent_reassembly":proof.reference().digest()?}),
            ),
            None => Ok(base),
        }
    }
    pub(crate) fn base_digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &json!({"authority":self.authority.digest()?,"phase":self.phase,"role":self.role,"context":self.context,
            "source":self.source.as_ref().map(DeliverySource::digest).transpose()?,"read_sources":self.read_sources.iter().map(DeliverySource::digest).collect::<Result<Vec<_>>>()?,
            "vector":self.candidate_vector_digest,"contract":self.contract_digest}),
        )
    }
    /// Preserve the complete native projection context and bind only this
    /// host-created substantive payload under the documented evidence object.
    pub fn bind_context(&self, native: Value, operation: Uuid) -> Result<Value> {
        let mut native = public_model_context(&native)?;
        verify_native_identity(&native, operation, &self.authority, &self.phase)?;
        let evidence = native
            .get_mut("evidence")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| {
                Error::new(
                    "worker_binding",
                    "native assignment context lacks evidence object",
                )
            })?;
        if evidence.contains_key("delivery_assignment") {
            return Err(Error::new(
                "worker_binding",
                "native context already contains a delivery assignment",
            ));
        }
        evidence.insert(
            "delivery_assignment".into(),
            public_model_context(&self.context)?,
        );
        self.verify_bound_context(&native, operation)?;
        Ok(native)
    }
    pub fn verify_bound_context(&self, context: &Value, operation: Uuid) -> Result<()> {
        verify_native_identity(context, operation, &self.authority, &self.phase)?;
        if context.pointer("/evidence/delivery_assignment")
            != Some(&public_model_context(&self.context)?)
        {
            return Err(Error::new(
                "worker_binding",
                "native envelope differs from exact delivery assignment",
            ));
        }
        Ok(())
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
/// Public evidence uses a digest-qualified name; credential-shaped values are
/// still refused by the unchanged instruction secret guard.
pub(crate) fn public_model_context(value: &Value) -> Result<Value> {
    match value {
        Value::Object(map) => {
            if map.contains_key("authorization") && map.contains_key("authorization_digest") {
                return Err(Error::new(
                    "worker_binding",
                    "public assignment provenance names conflict",
                ));
            }
            let generation_keys = ["authorization", "definition", "correction", "scope"];
            let generation = map.len() == generation_keys.len()
                && generation_keys.iter().all(|key| map.contains_key(*key));
            if generation {
                for key in generation_keys {
                    evidence::validate_digest(map[key].as_str().ok_or_else(|| {
                        Error::new(
                            "access_denied",
                            "generation provenance must contain exact digests",
                        )
                    })?)?;
                }
            }
            let mut public = serde_json::Map::new();
            for (key, value) in map {
                let name = if generation && key == "authorization" {
                    "authorization_digest"
                } else {
                    key.as_str()
                };
                public.insert(name.into(), public_model_context(value)?);
            }
            Ok(Value::Object(public))
        }
        Value::Array(values) => Ok(Value::Array(
            values
                .iter()
                .map(public_model_context)
                .collect::<Result<Vec<_>>>()?,
        )),
        _ => Ok(value.clone()),
    }
}
fn verify_native_identity(
    context: &Value,
    operation: Uuid,
    gate: &FreshGate,
    phase: &str,
) -> Result<()> {
    let keys = [
        "issue",
        "parent",
        "children",
        "workspace",
        "scope",
        "dependencies",
        "corrections",
        "evidence",
        "attempt",
        "phase",
        "profile",
        "operation",
    ];
    if context
        .as_object()
        .is_none_or(|m| m.len() != keys.len() || keys.iter().any(|k| !m.contains_key(*k)))
        || context.pointer("/operation/id").and_then(Value::as_str)
            != Some(operation.to_string().as_str())
        || context.pointer("/workspace/id").and_then(Value::as_str)
            != Some(gate.workspace_id().to_string().as_str())
        || context.get("phase").and_then(Value::as_str) != Some(phase)
        || context.get("profile")
            != Some(
                &serde_json::to_value(gate.profile())
                    .map_err(|_| Error::invalid("profile encoding"))?,
            )
    {
        return Err(Error::new(
            "worker_binding",
            "native context differs from actual assignment admission",
        ));
    }
    Ok(())
}
pub(super) fn verify_assignment_contract(
    contract: &crate::worker::WorkerContract,
    gate: &FreshGate,
    phase: &str,
    payload: &Value,
) -> Result<()> {
    let manifest = &contract.instructions().manifest;
    verify_native_identity(contract.context(), manifest.operation_id, gate, phase)?;
    if contract.context().pointer("/evidence/delivery_assignment")
        != Some(&public_model_context(payload)?)
        || manifest.context_digest != canonical_digest(1, contract.context())?
        || manifest.phase != phase
        || manifest.workspace_id != gate.workspace_id()
        || manifest.work_generation != gate.work().digest()?
        || manifest.workflow_digest != gate.workflow_digest()
        || canonical_digest(1, &manifest.profile)? != canonical_digest(1, gate.profile())?
    {
        return Err(Error::new(
            "worker_binding",
            "actual full contract context differs from delivery payload or authority",
        ));
    }
    Ok(())
}
pub fn repair_worker_assignment(
    unfinished: &UnfinishedChildWork,
) -> Result<DeliveryWorkerAssignment> {
    unfinished.source().revalidate_filesystem()?;
    Ok(DeliveryWorkerAssignment {
        authority: unfinished.authority().clone(),
        phase: unfinished.phase().phase.clone(),
        role: unfinished.role(),
        context: unfinished.remaining_task().clone(),
        source: Some(unfinished.source().clone()),
        read_sources: vec![],
        candidate_vector_digest: None,
        contract_digest: unfinished.contract_digest().into(),
        parent_reassembly: None,
    })
}
pub fn retried_repair_worker_assignment(
    retried: &RetriedUnfinishedWork,
) -> Result<DeliveryWorkerAssignment> {
    let mut assignment = repair_worker_assignment(retried.original())?;
    assignment.context = retried.remaining_task().clone();
    Ok(assignment)
}
impl DeliveryCoordinator {
    pub fn repair_assignment(
        &self,
        unfinished: &UnfinishedChildWork,
    ) -> Result<DeliveryWorkerAssignment> {
        (self.current_gate)(unfinished.authority())?;
        repair_worker_assignment(unfinished)
    }

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
            parent_reassembly: None,
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
            parent_reassembly: None,
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
            parent_reassembly: None,
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
            parent_reassembly: None,
        })
    }
}
pub(super) fn child_source(
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
    if contract.instructions().manifest_digest != claim.outcome().outcome().instruction_digest
        || claim
            .outcome()
            .outcome()
            .instructions
            .repository_root
            .as_deref()
            != Some(isolation.plan().repository.as_path())
        || execution.binding.isolation_digest != isolation.digest()
        || execution.binding.work != *claim.outcome().original_authority().work()
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

#[cfg(test)]
mod context_tests {
    use super::*;
    #[test]
    fn public_provenance_keeps_exact_digest_without_admitting_credentials_or_collision() {
        let digest = bytes_digest(b"public authorization record");
        let original = json!({"work":{"generation":{"authorization":digest,"definition":digest,"correction":digest,"scope":digest}},"task":"review exact source"});
        let projected = public_model_context(&original).unwrap();
        assert_eq!(
            projected,
            json!({"work":{"generation":{"authorization_digest":digest,"definition":digest,"correction":digest,"scope":digest}},"task":"review exact source"})
        );
        assert_eq!(projected, public_model_context(&projected).unwrap());
        assert_eq!(
            public_model_context(&json!({"authorization":"Bearer real-secret-shaped-value"}))
                .unwrap(),
            json!({"authorization":"Bearer real-secret-shaped-value"})
        );
        assert_eq!(
            public_model_context(&json!({"authorization":digest})).unwrap(),
            json!({"authorization":digest})
        );
        assert!(
            public_model_context(&json!({"authorization":digest,"authorization_digest":digest}))
                .is_err()
        );
        assert_eq!(
            public_model_context(&json!({"api_key":"must still be rejected by assembler"}))
                .unwrap()["api_key"],
            "must still be rejected by assembler"
        );
    }
}
