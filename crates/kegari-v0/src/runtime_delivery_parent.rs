//! Parent progression consumes completed child capabilities and the entire
//! isolated candidate vector before issuing integrated and coordination work.
use super::child::{ActivePhase, PhaseRequest};
use super::*;
use crate::{
    control::Lease,
    delivery::*,
    deployment::VerifiedProductionBaseline,
    digest::canonical_digest,
    evidence::{RevisionBinding, VerifiedParent},
    git::Revision,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Mutex};
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ParentStage {
    Assemble,
    Vector,
    LaunchIntegrated,
    Integrated,
    LaunchReview,
    Publish,
    Complete,
    Blocked,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParentReference {
    schema_version: u32,
    runtime: Uuid,
    workspace: Uuid,
    work: crate::evidence::WorkIdentity,
    contract_digest: String,
    stage: ParentStage,
    phase_operation: Option<Uuid>,
    slices: Vec<Uuid>,
    vector: Option<String>,
    integrated: Option<Uuid>,
    review: Option<Uuid>,
    publication: Option<String>,
    error: Option<Error>,
}
struct ParentPipeline {
    reference: ParentReference,
    children: Vec<PublishedChild>,
    child_authorities: BTreeMap<Uuid, FreshGate>,
    staged: Vec<StagedDelta>,
    baselines: Vec<Arc<VerifiedProductionBaseline>>,
    integrations: Vec<crate::git::VerifiedProductionIntegration>,
    slices: Vec<Arc<IsolatedSlice>>,
    vector: Option<ParentCandidateVector>,
    integrated: Option<IntegratedEvidence>,
    assignment: Option<ParentReviewAssignment>,
    published: Option<PublishedParent>,
    phase: Option<ActivePhase>,
}
type ParentKey = (Uuid, Uuid, String);
type RetainedParents = BTreeMap<ParentKey, Arc<tokio::sync::Mutex<ParentPipeline>>>;
#[derive(Default)]
pub(super) struct Parents {
    pipelines: Mutex<RetainedParents>,
    advancing: tokio::sync::Mutex<()>,
    integrated: Mutex<BTreeMap<ParentKey, (ParentCandidateVector, IntegratedEvidence)>>,
}
/// Actual retained preparation capabilities, never reconstructed from a public
/// result digest. The release adapter reobserves providers before every use.
pub struct ParentSources {
    pub(super) parent: VerifiedParent,
    pub(super) slices: Vec<Arc<IsolatedSlice>>,
    pub(super) children: Vec<PublishedChild>,
    pub(super) baselines: Vec<Arc<VerifiedProductionBaseline>>,
    pub(super) vector_digest: String,
    pub(super) integrations: Vec<crate::git::VerifiedProductionIntegration>,
}
impl ParentSources {
    pub fn parent(&self) -> &VerifiedParent {
        &self.parent
    }
    pub fn slices(&self) -> &[Arc<IsolatedSlice>] {
        &self.slices
    }
    pub fn children(&self) -> &[PublishedChild] {
        &self.children
    }
    pub fn baselines(&self) -> &[Arc<VerifiedProductionBaseline>] {
        &self.baselines
    }
    pub fn vector_digest(&self) -> &str {
        &self.vector_digest
    }
    pub fn integrations(&self) -> &[crate::git::VerifiedProductionIntegration] {
        &self.integrations
    }
}
impl RuntimeDelivery {
    fn persist_parent(&self, parent: &ParentPipeline) -> Result<()> {
        let key = canonical_digest(
            1,
            &parent_key(parent.reference.workspace, &parent.reference.work)?,
        )?;
        self.records.write(
            &format!("parent-pipeline-{}.json", key.rsplit(':').next().unwrap()),
            &self
                .runtime
                .services
                .key
                .sign("runtime_parent_pipeline", &parent.reference)?,
        )
    }
    pub async fn parent_sources(
        &self,
        workspace: Uuid,
        parent: Uuid,
    ) -> Result<Option<ParentSources>> {
        let projection = self.runtime.observations.admission_snapshot()?;
        let observed = projection.parent(workspace, parent)?;
        let Some(work) = &observed.work else {
            return Ok(None);
        };
        let key = parent_key(workspace, work)?;
        let pipeline = self
            .parents
            .pipelines
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "parent pipeline lock failed"))?
            .get(&key)
            .cloned();
        let Some(pipeline) = pipeline else {
            return Ok(None);
        };
        let pipeline = pipeline.lock().await;
        let Some(published) = &pipeline.published else {
            return Ok(None);
        };
        let current = self.runtime.current_delivery(&observed.key)?;
        let Some(current) = current.verified_parent() else {
            return Ok(None);
        };
        if current.token() != published.verified().token() {
            return Err(Error::new(
                "stale_result",
                "retained candidate sources differ from current published parent",
            ));
        }
        Ok(Some(ParentSources {
            parent: current.clone(),
            slices: pipeline.slices.clone(),
            children: pipeline.children.clone(),
            baselines: pipeline.baselines.clone(),
            vector_digest: required_parent(&pipeline.vector, "candidate vector")?
                .digest()
                .into(),
            integrations: pipeline.integrations.clone(),
        }))
    }
    pub(super) async fn integrated_responsibility_ready(
        &self,
        workspace: &WorkspaceDelivery,
        parent: &ParentProjection,
        gate: &FreshGate,
    ) -> Result<bool> {
        let key = parent_key(gate.workspace_id(), gate.work())?;
        let retained = self
            .parents
            .integrated
            .lock()
            .map_err(|_| {
                Error::new(
                    "runtime_unavailable",
                    "integrated responsibility lock failed",
                )
            })?
            .get(&key)
            .cloned();
        let Some((vector, proof)) = retained else {
            return Ok(false);
        };
        let parent_gate = parent.fresh_gate(
            None,
            GatePurpose::NewOperation,
            &self.runtime.dependency_outcomes()?,
            Utc::now(),
            Duration::seconds(30),
        )?;
        let contract =
            workspace
                .coordinator
                .parent_contract(parent_gate, parent, &workspace.workflow)?;
        workspace
            .coordinator
            .reobserve_integrated_checks(&contract, gate, &vector, &workspace.observer, &proof)
            .await?;
        Ok(true)
    }
    async fn require_prior_parent_generations_clear(&self, key: &ParentKey) -> Result<()> {
        let prior: Vec<_> = self
            .parents
            .pipelines
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "parent pipeline lock failed"))?
            .iter()
            .filter(|(old, _)| old.0 == key.0 && old.1 == key.1 && old.2 != key.2)
            .map(|(_, pipeline)| pipeline.clone())
            .collect();
        for retained in prior {
            let parent = retained.lock().await;
            self.parent_generation_clear(&parent.reference, parent.phase.is_some())?;
        }
        // Retained signed references survive process restarts. They cannot be
        // ignored merely because no in-memory capability was restored.
        for name in self.records.records_bounded(65_536)? {
            if !name.starts_with("parent-pipeline-") || !name.ends_with(".json") {
                continue;
            }
            let signed: crate::evidence::Signed<ParentReference> = self.records.read(&name)?;
            self.runtime
                .services
                .key
                .verify("runtime_parent_pipeline", &signed)?;
            let old = parent_key(signed.payload.workspace, &signed.payload.work)?;
            if old.0 == key.0 && old.1 == key.1 && old.2 != key.2 {
                self.parent_generation_clear(&signed.payload, false)?;
            }
        }
        Ok(())
    }
    fn parent_generation_clear(
        &self,
        reference: &ParentReference,
        retained_phase: bool,
    ) -> Result<()> {
        if retained_phase {
            return Err(Error::new(
                "operation_pending",
                "previous parent generation retains an admitted phase",
            ));
        }
        let state = self.runtime.control.snapshot()?;
        for id in reference
            .phase_operation
            .iter()
            .chain(reference.integrated.iter())
            .chain(reference.review.iter())
        {
            for operation in [*id, crate::runtime_workers::preparation_operation(*id)?] {
                if state
                    .active_operations
                    .iter()
                    .any(|permit| permit.operation_id == operation)
                    || state.unresolved_operations.contains_key(&operation)
                    || self
                        .runtime
                        .executions
                        .active_operations()?
                        .contains(&operation)
                {
                    return Err(Error::new(
                        "operation_uncertain",
                        "previous parent generation still owns live or unknown effects",
                    ));
                }
                match self.runtime.control.operation(operation) {
                    Ok(receipt)
                        if !matches!(
                            receipt.phase,
                            crate::control::ReceiptPhase::Acknowledged
                                | crate::control::ReceiptPhase::Reconciled
                        ) =>
                    {
                        return Err(Error::new(
                            "operation_uncertain",
                            "previous parent generation has an unresolved operation receipt",
                        ));
                    }
                    Ok(receipt) => {
                        if state.unresolved_operations.values().any(|targets| {
                            targets
                                .iter()
                                .any(|target| receipt.canonical_targets.contains(target))
                        }) {
                            return Err(Error::new(
                                "operation_uncertain",
                                "previous parent phase retains unresolved dependent effects",
                            ));
                        }
                    }
                    Err(error) if error.code == "record_missing" => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(())
    }
    fn block_terminal_parent(&self, parent: &mut ParentPipeline, error: &Error) -> Result<()> {
        if !super::child::terminal_gate_failure(error) {
            return Ok(());
        }
        let Some(phase) = &parent.phase else {
            return Ok(());
        };
        self.retire_delivery_phase(phase, json!({"parent_delivery_stage":parent.reference.stage,"outcome":"failed","error":error}))?;
        parent.phase = None;
        parent.reference.stage = ParentStage::Blocked;
        Ok(())
    }
    pub(super) async fn maintain_parent_phases(&self) -> Result<()> {
        let _serial = self.parents.advancing.lock().await;
        let retained: Vec<_> = self
            .parents
            .pipelines
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "parent pipeline lock failed"))?
            .values()
            .cloned()
            .collect();
        let mut reporting_error = None;
        for retained in retained {
            let mut parent = retained.lock().await;
            let Some(phase) = &parent.phase else {
                continue;
            };
            let lease = phase.permit.lease.clone();
            let workspace = phase.workspace.clone();
            let expected = phase.admission.fresh_gate().clone();
            let result = async {
                let projection = self
                    .runtime
                    .observations
                    .active_parent(&expected, &self.runtime.dependency_outcomes()?)?;
                let gate = self
                    .runtime
                    .observations
                    .active_gate(&expected, &self.runtime.dependency_outcomes()?)?;
                if matches!(
                    parent.reference.stage,
                    ParentStage::LaunchIntegrated | ParentStage::LaunchReview
                ) {
                    let stage = parent.reference.stage;
                    self.finish_parent_phase(&mut parent, stage)
                } else {
                    self.advance_one_parent(&lease, &workspace, &projection, &gate, &mut parent)
                        .await
                }
            }
            .await;
            if let Err(error) = result {
                if let Err(retirement) = self.block_terminal_parent(&mut parent, &error) {
                    reporting_error.get_or_insert(retirement);
                }
                parent.reference.error = Some(error);
                if let Err(error) = self.persist_parent(&parent) {
                    reporting_error.get_or_insert(error);
                }
            }
        }
        reporting_error.map_or(Ok(()), Err)
    }
    pub(super) async fn advance_parents(
        &self,
        lease: &Lease,
        projection: &RuntimeProjection,
    ) -> Result<()> {
        let _serial = self.parents.advancing.lock().await;
        let mut reporting_error = None;
        for observed in projection.workspaces.values() {
            for parent in observed.parents.values() {
                let result = self.advance_parent_target(lease, projection, parent).await;
                if let Err(error) = result
                    && let Err(reporting) = self.runtime.invalidate_delivery(&parent.key, error)
                {
                    reporting_error.get_or_insert(reporting);
                }
            }
        }
        reporting_error.map_or(Ok(()), Err)
    }
    async fn advance_parent_target(
        &self,
        lease: &Lease,
        projection: &RuntimeProjection,
        projection_parent: &ParentProjection,
    ) -> Result<()> {
        let workspace = self.workspace(projection, projection_parent.workspace_id)?;
        let gate = projection_parent.fresh_gate(
            None,
            GatePurpose::NewOperation,
            &self.runtime.dependency_outcomes()?,
            Utc::now(),
            Duration::seconds(30),
        )?;
        let key = parent_key(gate.workspace_id(), gate.work())?;
        let prior = self
            .parents
            .pipelines
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "parent pipeline lock failed"))?
            .get(&key)
            .cloned();
        let pipeline = if let Some(prior) = prior {
            prior
        } else {
            self.require_prior_parent_generations_clear(&key).await?;
            let contract = workspace.coordinator.parent_contract(
                gate.clone(),
                projection_parent,
                &workspace.workflow,
            )?;
            let prepared = self
                .candidate_children(gate.work(), contract.e2e_child())
                .await?;
            let ids: BTreeSet<_> = prepared.iter().map(|child| child.child).collect();
            if !complete_inventory(&projection_parent.required_children, &ids, prepared.len()) {
                return Ok(());
            }
            let children = prepared
                .iter()
                .filter_map(|child| child.published.clone())
                .collect();
            let staged = prepared
                .into_iter()
                .filter_map(|child| child.staged)
                .collect();
            let parent = ParentPipeline {
                reference: ParentReference {
                    schema_version: 1,
                    runtime: lease.runtime_id,
                    workspace: gate.workspace_id(),
                    work: gate.work().clone(),
                    contract_digest: contract.digest()?,
                    stage: ParentStage::Assemble,
                    phase_operation: None,
                    slices: vec![],
                    vector: None,
                    integrated: None,
                    review: None,
                    publication: None,
                    error: None,
                },
                children,
                child_authorities: BTreeMap::new(),
                staged,
                baselines: vec![],
                integrations: vec![],
                slices: vec![],
                vector: None,
                integrated: None,
                assignment: None,
                published: None,
                phase: None,
            };
            let record = canonical_digest(1, &key)?;
            if self
                .records
                .path()
                .join(format!(
                    "parent-pipeline-{}.json",
                    record.rsplit(':').next().unwrap()
                ))
                .exists()
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "parent candidate preparation has retained phase references requiring exact recovery",
                ));
            }
            self.persist_parent(&parent)?;
            let retained = Arc::new(tokio::sync::Mutex::new(parent));
            self.parents
                .pipelines
                .lock()
                .map_err(|_| Error::new("runtime_unavailable", "parent pipeline lock failed"))?
                .insert(key, retained.clone());
            retained
        };
        let mut parent = pipeline.lock().await;
        if parent.reference.work != *gate.work() {
            return Err(Error::new(
                "stale_result",
                "parent preparation belongs to an older authorized generation",
            ));
        }
        let result = self
            .advance_one_parent(lease, &workspace, projection_parent, &gate, &mut parent)
            .await;
        if let Err(error) = &result {
            let retirement = self.block_terminal_parent(&mut parent, error);
            parent.reference.error = Some(error.clone());
            self.persist_parent(&parent)?;
            retirement?;
        }
        result
    }
    async fn parent_phase(
        &self,
        lease: &Lease,
        workspace: &WorkspaceDelivery,
        gate: &FreshGate,
        parent: &mut ParentPipeline,
        repository: &RepositoryContract,
        revision: Revision,
    ) -> Result<()> {
        if parent.phase.is_some() {
            return Ok(());
        }
        let operation = parent
            .reference
            .phase_operation
            .unwrap_or_else(Uuid::new_v4);
        parent.reference.phase_operation = Some(operation);
        self.persist_parent(parent)?;
        let candidate = parent.vector.as_ref().and_then(|vector| {
            vector
                .candidates()
                .iter()
                .find(|candidate| candidate.repository == repository.repository)
        });
        let result = candidate
            .and_then(|candidate| candidate.result.as_ref())
            .unwrap_or(&revision);
        let scope = DeliveryScope {
            schema_version: 1,
            runtime_id: lease.runtime_id,
            phase: if parent.vector.is_some() {
                DeliveryPhase::ParentEvidence
            } else {
                DeliveryPhase::CandidateAssembly
            },
            work: gate.work().clone(),
            repository: repository.repository.clone(),
            revision: RevisionBinding {
                repository: repository.repository.clone(),
                head: result.commit.clone(),
                base: revision.commit.clone(),
                tested_commit: result.commit.clone(),
                tested_tree: result.tree.clone(),
            },
            fresh_gate_digest: gate.digest()?,
            contract_digest: repository.digest()?,
            candidate_vector_digest: parent.vector.as_ref().map(|vector| vector.digest().into()),
        };
        parent.phase = Some(
            self.admit_delivery_phase(workspace, gate, lease, PhaseRequest { operation, scope })
                .await?,
        );
        Ok(())
    }
    fn finish_parent_phase(&self, parent: &mut ParentPipeline, next: ParentStage) -> Result<()> {
        if let Some(phase) = &parent.phase {
            self.retire_delivery_phase(phase, json!({"parent_delivery_next":next}))?;
        }
        parent.phase = None;
        parent.reference.phase_operation = None;
        parent.reference.stage = next;
        parent.reference.error = None;
        self.persist_parent(parent)
    }
    async fn advance_one_parent(
        &self,
        lease: &Lease,
        workspace: &WorkspaceDelivery,
        projection: &ParentProjection,
        gate: &FreshGate,
        parent: &mut ParentPipeline,
    ) -> Result<()> {
        if matches!(
            parent.reference.stage,
            ParentStage::Complete | ParentStage::Blocked
        ) {
            return Ok(());
        }
        let snapshot = self.runtime.observations.admission_snapshot()?;
        let predecessor = self.known_predecessor_baseline(&snapshot, gate).await?;
        let observer = predecessor
            .as_ref()
            .map(|proof| workspace.observer.with_successor(proof.clone()))
            .transpose()?;
        let observer = observer.as_ref().unwrap_or(workspace.observer.as_ref());
        let contract =
            workspace
                .coordinator
                .parent_contract(gate.clone(), projection, &workspace.workflow)?;
        if contract.digest() != Ok(parent.reference.contract_digest.clone()) {
            return Err(Error::new(
                "stale_result",
                "complete parent delivery contract changed",
            ));
        }
        let child_gates: BTreeMap<_, _> =
            if parent.phase.is_some() && !parent.child_authorities.is_empty() {
                parent
                    .child_authorities
                    .iter()
                    .map(|(id, expected)| {
                        Ok((
                            *id,
                            self.runtime
                                .observations
                                .active_gate(expected, &self.runtime.dependency_outcomes()?)?,
                        ))
                    })
                    .collect::<Result<_>>()?
            } else {
                projection
                    .required_children
                    .iter()
                    .map(|child| {
                        Ok((
                            *child,
                            projection.fresh_gate(
                                Some(*child),
                                GatePurpose::NewOperation,
                                &self.runtime.dependency_outcomes()?,
                                Utc::now(),
                                Duration::seconds(30),
                            )?,
                        ))
                    })
                    .collect::<Result<_>>()?
            };
        parent.child_authorities = child_gates.clone();
        let e2e = child_gates.get(&contract.e2e_child()).ok_or_else(|| {
            Error::new(
                "parent_incomplete",
                "integrated responsibility has no current authority",
            )
        })?;
        if matches!(
            parent.reference.stage,
            ParentStage::Integrated | ParentStage::Publish
        ) {
            let operation = if parent.reference.stage == ParentStage::Integrated {
                parent.reference.integrated
            } else {
                parent.reference.review
            }
            .ok_or_else(|| {
                Error::new(
                    "reconciliation_required",
                    "parent worker phase has no persisted launch identity",
                )
            })?;
            if self.completed_claim(operation)?.is_none() {
                return Ok(());
            }
        }
        let pending_launch = match parent.reference.stage {
            ParentStage::LaunchIntegrated => parent
                .reference
                .integrated
                .map(|id| (id, ParentStage::Integrated)),
            ParentStage::LaunchReview => {
                parent.reference.review.map(|id| (id, ParentStage::Publish))
            }
            _ => None,
        };
        if let Some((operation, next)) = pending_launch
            && self.worker_launch_admitted(operation)?
        {
            parent.reference.stage = next;
            return self.persist_parent(parent);
        }
        if parent.reference.stage == ParentStage::LaunchReview {
            let completed = self.delivered_children(gate.work()).await?;
            let ids: BTreeSet<_> = completed
                .iter()
                .filter_map(|(child, _)| child.verified().evidence().work.child)
                .collect();
            if !complete_inventory(contract.required_children(), &ids, completed.len()) {
                return Ok(());
            }
            parent.children = completed.into_iter().map(|(child, _)| child).collect();
        }
        let repository_id = if parent.reference.stage == ParentStage::Assemble {
            contract.repositories().iter().nth(parent.slices.len())
        } else {
            contract.repositories().iter().next()
        }
        .ok_or_else(|| {
            Error::new(
                "parent_incomplete",
                "parent has no remaining candidate repository",
            )
        })?;
        let repository = workspace
            .contracts
            .iter()
            .find(|repository| &repository.repository == repository_id)
            .ok_or_else(|| Error::new("scope_violation", "candidate repository disappeared"))?;
        let baseline = observer.observe(repository).await?;
        self.parent_phase(
            lease,
            workspace,
            gate,
            parent,
            repository,
            baseline.baseline().production.clone(),
        )
        .await?;
        let current = parent.phase.as_ref().unwrap();
        let coordinator = current.coordinator.clone();
        let admission = current.admission.clone();
        match parent.reference.stage {
            ParentStage::Assemble => {
                if let Some(predecessor) = &predecessor {
                    let integrations = coordinator
                        .acquire_successor_integrations(&admission, predecessor.clone())
                        .await?;
                    parent
                        .integrations
                        .retain(|proof| proof.record().repository != repository.repository);
                    parent.integrations.extend(integrations);
                }
                let git = current.git.clone();
                let source = PathBuf::from(&repository.configuration.source);
                let identity =
                    tokio::task::spawn_blocking(move || git.configured_commit_identity(&source))
                        .await
                        .map_err(|_| {
                            Error::new(
                                "operation_uncertain",
                                "candidate identity observation did not join",
                            )
                        })??;
                let execution_root = workspace.records.child("candidate-clones")?;
                let staged: Vec<_> = parent
                    .staged
                    .iter()
                    .filter(|staged| staged.delta().manifest.repository == repository.repository)
                    .collect();
                let dependencies = projection
                    .children
                    .iter()
                    .map(|(id, child)| (*id, child.prerequisites.clone()))
                    .collect();
                let mut satisfied: BTreeSet<_> = parent
                    .children
                    .iter()
                    .filter(|child| {
                        child.verified().evidence().revision.repository != repository.repository
                            || matches!(
                                child.verified().evidence().outcome,
                                crate::evidence::ChildOutcome::NoChange { .. }
                            )
                    })
                    .filter_map(|child| child.verified().evidence().work.child)
                    .collect();
                // A prepared no-change E2E plan is a satisfied assembly
                // prerequisite, never a completed child result.
                if !parent.staged.iter().any(|staged| {
                    staged.delta().manifest.child_id == contract.e2e_child()
                        && staged.delta().manifest.repository == repository.repository
                }) {
                    satisfied.insert(contract.e2e_child());
                }
                let slice = coordinator
                    .assemble_remote_isolated_slice(
                        &admission,
                        SliceAssembly {
                            execution_root: execution_root.path(),
                            staged: &staged,
                            required_children: contract.required_children(),
                            child_dependencies: &dependencies,
                            satisfied_children: &satisfied,
                            production_integrations: &parent.integrations,
                            identity,
                        },
                    )
                    .await?;
                parent.reference.slices.push(slice.operation_id());
                parent.slices.push(Arc::new(slice));
                parent.baselines.push(Arc::new(baseline));
                self.finish_parent_phase(
                    parent,
                    if parent.slices.len() == contract.repositories().len() {
                        ParentStage::Vector
                    } else {
                        ParentStage::Assemble
                    },
                )?;
            }
            ParentStage::Vector => {
                parent.vector = Some(
                    coordinator
                        .prepare_parent_vector(
                            &contract,
                            observer,
                            &parent.slices.iter().map(Arc::as_ref).collect::<Vec<_>>(),
                            &parent.baselines.iter().map(Arc::as_ref).collect::<Vec<_>>(),
                        )
                        .await?,
                );
                parent.reference.vector = Some(parent.vector.as_ref().unwrap().digest().into());
                self.finish_parent_phase(parent, ParentStage::LaunchIntegrated)?;
            }
            ParentStage::LaunchIntegrated => {
                let assignment=coordinator.integrated_worker_assignment(&contract,e2e,required_parent(&parent.vector,"candidate vector")?,observer,&parent.slices.iter().map(Arc::as_ref).collect::<Vec<_>>(),json!({"purpose":"execute every configured integrated check against this complete isolated parent vector","parent":projection.issue})).await?;
                self.finish_parent_phase(parent, ParentStage::LaunchIntegrated)?;
                let operation = parent.reference.integrated.unwrap_or_else(Uuid::new_v4);
                parent.reference.integrated = Some(operation);
                self.persist_parent(parent)?;
                self.launch_assignment(lease, operation, assignment).await?;
                parent.reference.stage = ParentStage::Integrated;
                self.persist_parent(parent)?;
            }
            ParentStage::Integrated => {
                let (_, completion) = self
                    .completed_claim(parent.reference.integrated.unwrap())?
                    .ok_or_else(|| {
                        Error::new("operation_pending", "integrated worker remains owned")
                    })?;
                parent.integrated = Some(
                    coordinator
                        .verify_integrated_checks(
                            &contract,
                            e2e,
                            required_parent(&parent.vector, "candidate vector")?,
                            observer,
                            &completion.checks.iter().collect::<Vec<_>>(),
                        )
                        .await?,
                );
                self.parents
                    .integrated
                    .lock()
                    .map_err(|_| {
                        Error::new(
                            "runtime_unavailable",
                            "integrated responsibility lock failed",
                        )
                    })?
                    .insert(
                        parent_key(parent.reference.workspace, &parent.reference.work)?,
                        (
                            required_parent(&parent.vector, "candidate vector")?.clone(),
                            required_parent(&parent.integrated, "integrated execution")?.clone(),
                        ),
                    );
                self.finish_parent_phase(parent, ParentStage::LaunchReview)?;
            }
            ParentStage::LaunchReview => {
                parent.assignment=Some(coordinator.prepare_parent_review(ParentEvidenceInputs{contract:&contract,observer,vector:required_parent(&parent.vector,"candidate vector")?.clone(),integrated:required_parent(&parent.integrated,"integrated execution")?.clone(),e2e_gate:e2e,child_gates:&child_gates,children:&parent.children,workflow:&workspace.workflow,production_integrations:&parent.integrations},json!({"parent":projection.issue,"purpose":"evaluate the complete canonical parent outcome and all acceptance criteria"})).await?);
                let assignment = coordinator.parent_review_worker_assignment(
                    &contract,
                    required_parent(&parent.assignment, "parent review assignment")?,
                )?;
                self.finish_parent_phase(parent, ParentStage::LaunchReview)?;
                let operation = parent.reference.review.unwrap_or_else(Uuid::new_v4);
                parent.reference.review = Some(operation);
                self.persist_parent(parent)?;
                self.launch_assignment(lease, operation, assignment).await?;
                parent.reference.stage = ParentStage::Publish;
                self.persist_parent(parent)?;
            }
            ParentStage::Publish => {
                let (outcome, completion) = self
                    .completed_claim(parent.reference.review.unwrap())?
                    .ok_or_else(|| {
                        Error::new("operation_pending", "parent review remains owned")
                    })?;
                let published = coordinator
                    .publish_parent_result(
                        &admission,
                        ParentPublicationInputs {
                            contract: &contract,
                            observer,
                            assignment: required_parent(
                                &parent.assignment,
                                "parent review assignment",
                            )?,
                            child_gates: &child_gates,
                            workflow: &workspace.workflow,
                            outcome: &outcome,
                            completion: &completion,
                            video: None,
                        },
                    )
                    .await?;
                parent.reference.publication = Some(published.publication().record_id.clone());
                parent.published = Some(published);
                self.finish_parent_phase(parent, ParentStage::Complete)?;
            }
            ParentStage::Complete | ParentStage::Blocked => {}
        }
        Ok(())
    }
}
fn required_parent<'a, T>(value: &'a Option<T>, what: &str) -> Result<&'a T> {
    value.as_ref().ok_or_else(|| {
        Error::new(
            "reconciliation_required",
            format!("parent delivery lacks actual retained {what}"),
        )
    })
}

fn parent_key(workspace: Uuid, work: &crate::evidence::WorkIdentity) -> Result<ParentKey> {
    let mut parent = work.clone();
    parent.child = None;
    Ok((workspace, parent.parent, parent.digest()?))
}

fn complete_inventory(required: &BTreeSet<Uuid>, observed: &BTreeSet<Uuid>, count: usize) -> bool {
    !required.is_empty() && required == observed && observed.len() == count
}
#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    fn reference(workspace: Uuid, work: crate::evidence::WorkIdentity) -> ParentReference {
        ParentReference {
            schema_version: 1,
            runtime: Uuid::from_u128(1),
            workspace,
            work,
            contract_digest: canonical_digest(1, &"contract").unwrap(),
            stage: ParentStage::Complete,
            phase_operation: None,
            slices: vec![],
            vector: None,
            integrated: None,
            review: None,
            publication: None,
            error: None,
        }
    }
    fn pipeline(reference: ParentReference) -> ParentPipeline {
        ParentPipeline {
            reference,
            children: vec![],
            child_authorities: BTreeMap::new(),
            staged: vec![],
            baselines: vec![],
            integrations: vec![],
            slices: vec![],
            vector: None,
            integrated: None,
            assignment: None,
            published: None,
            phase: None,
        }
    }
    #[tokio::test]
    async fn corrected_generation_has_separate_immutable_records_and_prior_unknown_blocks_it() {
        let (_root, fixture, service) = super::super::tests::fixture().await;
        let projection = fixture.runtime.observations.snapshot().unwrap();
        let observed = projection
            .parent(Uuid::from_u128(2), Uuid::from_u128(100))
            .unwrap();
        let work = observed.work.clone().unwrap();
        let old_key = parent_key(observed.workspace_id, &work).unwrap();
        let old = Arc::new(tokio::sync::Mutex::new(pipeline(reference(
            observed.workspace_id,
            work.clone(),
        ))));
        service.persist_parent(&*old.lock().await).unwrap();
        service
            .parents
            .pipelines
            .lock()
            .unwrap()
            .insert(old_key.clone(), old.clone());
        let path = service.records.path().join(format!(
            "parent-pipeline-{}.json",
            canonical_digest(1, &old_key)
                .unwrap()
                .rsplit(':')
                .next()
                .unwrap()
        ));
        let original = std::fs::read(&path).unwrap();
        let mut corrected = work;
        corrected.generation.correction =
            canonical_digest(1, &"authorized G2 correction fixture").unwrap();
        let next_key = parent_key(observed.workspace_id, &corrected).unwrap();
        assert_ne!(old_key, next_key);
        service
            .require_prior_parent_generations_clear(&next_key)
            .await
            .unwrap();
        let next = pipeline(reference(observed.workspace_id, corrected));
        service.persist_parent(&next).unwrap();
        service
            .parents
            .pipelines
            .lock()
            .unwrap()
            .insert(next_key.clone(), Arc::new(tokio::sync::Mutex::new(next)));
        assert_eq!(service.parents.pipelines.lock().unwrap().len(), 2);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        // A genuine retained prior admission blocks G2; no synthetic active
        // worker capability or rewritten G1 record is used to bypass it.
        let lease = super::super::tests::start_control(&fixture.runtime);
        let operation = Uuid::new_v4();
        fixture
            .runtime
            .control
            .admit(
                &lease,
                operation,
                "prior-parent-phase".into(),
                canonical_digest(1, &"prior-scope").unwrap(),
                || Ok(()),
            )
            .unwrap();
        old.lock().await.reference.phase_operation = Some(operation);
        assert_eq!(
            service
                .require_prior_parent_generations_clear(&next_key)
                .await
                .unwrap_err()
                .code,
            "operation_uncertain"
        );
        fixture
            .runtime
            .control
            .retire_local(operation, json!({"fixture":"never spawned"}))
            .unwrap();
        service
            .require_prior_parent_generations_clear(&next_key)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn terminal_gate_failure_retires_admission_without_authorizing_retry_and_unknown_stays_held()
     {
        let (_root, fixture, service) = super::super::tests::fixture_authorization(true).await;
        let projection = fixture.runtime.observations.snapshot().unwrap();
        let observed = projection
            .parent(Uuid::from_u128(2), Uuid::from_u128(100))
            .unwrap();
        let gate = observed
            .fresh_gate(
                None,
                GatePurpose::NewOperation,
                &crate::reconcile::DependencyOutcomes::default(),
                Utc::now(),
                Duration::seconds(30),
            )
            .unwrap();
        let workspace = service.workspace(&projection, gate.workspace_id()).unwrap();
        let lease = super::super::tests::start_control(&fixture.runtime);
        let lifecycle = |command| {
            let state = fixture.runtime.control.snapshot().unwrap();
            fixture
                .runtime
                .control
                .lifecycle(
                    crate::control::MutationRequest {
                        request_id: Uuid::new_v4(),
                        runtime_id: lease.runtime_id,
                        actor: crate::control::Actor {
                            identity: "fixture-operator".into(),
                            principal: None,
                            roles: BTreeSet::from(["lifecycle".into()]),
                        },
                        kind: "lifecycle".into(),
                        canonical_targets: vec![lease.runtime_id.to_string()],
                        expected: BTreeMap::new(),
                        payload: json!(null),
                    },
                    command,
                    state.intent.generation,
                )
                .unwrap();
        };
        let make_phase = |operation| {
            let repository = &workspace.contracts[0];
            let scope = DeliveryScope {
                schema_version: 1,
                runtime_id: lease.runtime_id,
                phase: DeliveryPhase::ParentEvidence,
                work: gate.work().clone(),
                repository: repository.repository.clone(),
                revision: RevisionBinding {
                    repository: repository.repository.clone(),
                    head: "a".repeat(40),
                    base: "a".repeat(40),
                    tested_commit: "a".repeat(40),
                    tested_tree: "b".repeat(40),
                },
                fresh_gate_digest: gate.digest().unwrap(),
                contract_digest: repository.digest().unwrap(),
                candidate_vector_digest: Some(
                    canonical_digest(1, &"fixture scope only; no vector proof issued").unwrap(),
                ),
            };
            let permit = fixture
                .runtime
                .control
                .admit(
                    &lease,
                    operation,
                    gate.work_key().unwrap(),
                    scope.digest().unwrap(),
                    || Ok(()),
                )
                .unwrap();
            ActivePhase {
                admission: DeliveryAdmission::bind(permit.clone(), gate.clone(), scope).unwrap(),
                permit,
                coordinator: workspace.coordinator.clone(),
                git: workspace.git.clone(),
                workspace: Arc::new(workspace.clone()),
                _active: fixture
                    .runtime
                    .observations
                    .capture_active(
                        gate.workspace_id(),
                        gate.config_revision(),
                        gate.workflow_digest(),
                    )
                    .unwrap(),
            }
        };
        let operation = Uuid::new_v4();
        let mut parent = pipeline(reference(gate.workspace_id(), gate.work().clone()));
        parent.reference.stage = ParentStage::Integrated;
        parent.reference.phase_operation = Some(operation);
        parent.phase = Some(make_phase(operation));
        lifecycle(crate::control::Lifecycle::Pause);
        assert!(!fixture.runtime.control.snapshot().unwrap().admission_open);
        let failure = Error::new("integrated_gate", "actual bounded fixture check failed");
        service
            .block_terminal_parent(&mut parent, &failure)
            .unwrap();
        parent.reference.error = Some(failure.clone());
        service.persist_parent(&parent).unwrap();
        assert!(parent.phase.is_none());
        assert!(parent.reference.stage == ParentStage::Blocked);
        assert!(
            fixture
                .runtime
                .control
                .snapshot()
                .unwrap()
                .active_operations
                .is_empty()
        );
        assert_eq!(
            fixture
                .runtime
                .control
                .operation(operation)
                .unwrap()
                .observed_result
                .unwrap()["observation"]["error"]["code"],
            "integrated_gate"
        );
        let before = fixture.runtime.control.operations().unwrap().len();
        service
            .advance_one_parent(&lease, &workspace, observed, &gate, &mut parent)
            .await
            .unwrap();
        assert_eq!(fixture.runtime.control.operations().unwrap().len(), before);
        lifecycle(crate::control::Lifecycle::Resume);
        let unknown = Uuid::new_v4();
        parent.reference.stage = ParentStage::Integrated;
        parent.reference.phase_operation = Some(unknown);
        parent.phase = Some(make_phase(unknown));
        let effect = Uuid::new_v4();
        let (_pending, ()) = fixture
            .runtime
            .control
            .begin_effect(
                crate::control::MutationRequest {
                    request_id: effect,
                    runtime_id: lease.runtime_id,
                    actor: crate::control::Actor {
                        identity: "fixture-owned-phase".into(),
                        principal: None,
                        roles: BTreeSet::new(),
                    },
                    kind: "fixture-unknown-effect".into(),
                    canonical_targets: vec!["fixture-resource".into()],
                    expected: BTreeMap::new(),
                    payload: json!({"fixture":"no actual process"}),
                },
                &parent.phase.as_ref().unwrap().permit,
                || Ok(()),
                |_| Ok(()),
            )
            .unwrap();
        lifecycle(crate::control::Lifecycle::Pause);
        assert_eq!(
            service
                .block_terminal_parent(&mut parent, &failure)
                .unwrap_err()
                .code,
            "operation_uncertain"
        );
        assert!(parent.phase.is_some());
        assert!(parent.reference.stage == ParentStage::Integrated);
        assert!(
            fixture
                .runtime
                .control
                .snapshot()
                .unwrap()
                .unresolved_operations
                .contains_key(&effect)
        );
        assert!(!super::super::child::terminal_gate_failure(&Error::new(
            "operation_pending",
            "still running"
        )));
        assert!(!super::super::child::terminal_gate_failure(&Error::new(
            "operation_uncertain",
            "lost acknowledgment"
        )));
    }
    #[test]
    fn candidate_inventory_can_precede_final_child_inventory_without_omitting_other_work() {
        let implementation = Uuid::from_u128(1);
        let e2e = Uuid::from_u128(2);
        let docs = Uuid::from_u128(3);
        let required = BTreeSet::from([implementation, e2e, docs]);
        let completed = BTreeSet::from([implementation, docs]);
        assert!(!complete_inventory(&required, &completed, completed.len()));
        let mut prepared = completed.clone();
        prepared.insert(e2e);
        assert!(complete_inventory(&required, &prepared, prepared.len()));
        prepared.remove(&docs);
        assert!(!complete_inventory(&required, &prepared, prepared.len()));
        assert!(!complete_inventory(&required, &required, 4));
        assert!(complete_inventory(&required, &required, 3));
    }
}
