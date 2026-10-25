//! Finite delivery actions consume host-authenticated worker claims and sealed
//! execution/provider capabilities. A model claim never supplies a passed gate.
use super::*;
use crate::instructions::AgentRole;
use crate::runtime::VerifiedAuthorizedOutcome;
use crate::worker_run::{RunCompletion, RunOutcome};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChildClaimDisposition {
    Changed,
    NoChange { reason: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildResultClaim {
    pub schema_version: u32,
    pub summary: String,
    pub fulfilled_criteria: BTreeSet<String>,
    pub disposition: ChildClaimDisposition,
}
/// Pending product assertions from an actual completed authorized child session.
/// This capability has no completion/CI/review/acceptance semantics.
pub struct CompletedChildClaim {
    outcome: VerifiedAuthorizedOutcome,
    completion: Arc<RunCompletion>,
    claim: ChildResultClaim,
    correction: Option<Arc<crate::corrections::VerifiedCorrectionTransition>>,
}
impl CompletedChildClaim {
    pub fn claim(&self) -> &ChildResultClaim {
        &self.claim
    }
    pub fn operation_id(&self) -> Uuid {
        self.completion.operation_id
    }
    pub fn digest(&self) -> &str {
        self.outcome.digest()
    }
    pub(super) fn outcome(&self) -> &VerifiedAuthorizedOutcome {
        &self.outcome
    }
    pub(super) fn work(&self) -> &WorkIdentity {
        self.outcome.effective_authority().work()
    }
}
pub(super) fn completed_run(
    outcome: &VerifiedAuthorizedOutcome,
    completion: &RunCompletion,
) -> Result<()> {
    let claim = outcome.outcome();
    if completion.operation_id != claim.operation_id
        || !matches!(completion.outcome, RunOutcome::Completed)
        || completion.error.is_some()
        || completion.hook_error.is_some()
        || !completion.local_exit_confirmed
        || !completion.state.completion_is_current()
        || canonical_digest(1, &completion.state)? != canonical_digest(1, &claim.session)?
        || completion
            .checks
            .iter()
            .any(|check| check.record().binding.operation_id != claim.operation_id)
    {
        return Err(Error::new(
            "worker_binding",
            "claim has no matching completed and exited owned run",
        ));
    }
    Ok(())
}
impl DeliveryCoordinator {
    pub fn accept_child_completion(
        &self,
        gate: &FreshGate,
        outcome: VerifiedAuthorizedOutcome,
        completion: Arc<RunCompletion>,
    ) -> Result<CompletedChildClaim> {
        self.accept_child_completion_inner(gate, outcome, completion, None)
    }
    pub fn accept_corrected_child_completion(
        &self,
        gate: &FreshGate,
        outcome: VerifiedAuthorizedOutcome,
        completion: Arc<RunCompletion>,
        transition: Arc<crate::corrections::VerifiedCorrectionTransition>,
    ) -> Result<CompletedChildClaim> {
        self.accept_child_completion_inner(gate, outcome, completion, Some(transition))
    }
    fn accept_child_completion_inner(
        &self,
        gate: &FreshGate,
        outcome: VerifiedAuthorizedOutcome,
        completion: Arc<RunCompletion>,
        correction: Option<Arc<crate::corrections::VerifiedCorrectionTransition>>,
    ) -> Result<CompletedChildClaim> {
        (self.current_gate)(gate)?;
        match &correction {
            Some(transition)
                if outcome.correction_transition_digest() == Some(transition.digest())
                    && outcome.original_authority().digest()?
                        == transition.original_authority().digest()?
                    && outcome.effective_authority().digest()?
                        == transition.current_authority().digest()? => {}
            None if outcome.correction_transition_digest().is_none()
                && outcome.original_authority().digest()?
                    == outcome.effective_authority().digest()? => {}
            _ => {
                return Err(Error::new(
                    "correction_execution_binding",
                    "completed correction lacks its actual current native transition",
                ));
            }
        }
        outcome.effective_authority().compare_current(
            gate,
            Utc::now(),
            chrono::Duration::seconds(60),
        )?;
        completed_run(&outcome, &completion)?;
        if gate.work().child.is_none()
            || !matches!(
                outcome.outcome().instructions.role,
                AgentRole::Implementation | AgentRole::Testing | AgentRole::Documentation
            )
        {
            return Err(Error::new(
                "scope_violation",
                "only an implementation responsibility may propose child completion",
            ));
        }
        let claim: ChildResultClaim = serde_json::from_value(outcome.outcome().claim.clone())
            .map_err(|_| {
                Error::new(
                    "worker_claim",
                    "child result claim has no supported explicit schema",
                )
            })?;
        if claim.schema_version != 1
            || claim.summary.trim().is_empty()
            || claim.summary.len() > 65536
            || claim.fulfilled_criteria.is_empty()
            || claim.fulfilled_criteria.len() > 256
            || claim
                .fulfilled_criteria
                .iter()
                .any(|c| c.trim().is_empty() || c.len() > 8192)
            || matches!(&claim.disposition, ChildClaimDisposition::NoChange {reason} if reason.trim().is_empty() || reason.len()>65536)
        {
            return Err(Error::new(
                "worker_claim",
                "child result claim is incomplete or exceeds finite bounds",
            ));
        }
        Ok(CompletedChildClaim {
            outcome,
            completion,
            claim,
            correction,
        })
    }
    pub(super) fn current_child_claim(
        &self,
        admission: &DeliveryAdmission,
        claim: &CompletedChildClaim,
    ) -> Result<()> {
        self.current(admission, &[admission.scope.phase])?;
        if claim.work() != &admission.scope.work
            || claim.outcome.original_authority().config_revision()
                != admission.gate.config_revision()
            || claim.outcome.original_authority().workflow_digest()
                != admission.gate.workflow_digest()
            || claim.outcome.original_authority().child_scope_digest()
                != admission.gate.child_scope_digest()
        {
            return Err(Error::new(
                "stale_result",
                "pending child claim differs from current delivery authority",
            ));
        }
        completed_run(&claim.outcome, &claim.completion)
    }
}

/// Each variant performs one bounded phase; the caller supplies a fresh permit
/// with the corresponding immutable revision and phase before calling it.
pub enum ChildDeliveryAction<'a> {
    ValidateLocal {
        validation: &'a VerifiedAuthorizedOutcome,
        completion: &'a Arc<RunCompletion>,
    },
    PublishBranch {
        source: &'a SealedChildSource,
        local: &'a LocalPassed,
        branch: &'a crate::git::BranchName,
        previous: Option<&'a PublishedChildBranch>,
    },
    OpenPullRequest {
        branch: &'a PublishedChildBranch,
        local: &'a LocalPassed,
        title: &'a str,
    },
    DispatchExactValidation,
    ObserveExactValidation {
        pending: &'a PendingExactValidation,
        verifier: &'a HostedProofVerifier,
    },
    ObserveHosted {
        tokens: &'a [HostedToken],
        verifier: &'a HostedProofVerifier,
    },
    RequestStaging {
        gates: StagingGates<'a>,
    },
    ObserveStaging {
        source: &'a Path,
        change: &'a ScopedChange,
        pull_request: &'a ChildPullRequest,
        pending: &'a PendingStaging,
    },
    PublishChanged {
        local: &'a LocalPassed,
        hosted: &'a HostedPassed,
        review: &'a PublishedReview,
        staged: &'a StagedDelta,
    },
    PublishNoChange {
        local: &'a LocalPassed,
        exact: &'a ExactValidationPassed,
        review: &'a PublishedReview,
    },
    Close {
        result: &'a PublishedChild,
        configuration: &'a crate::config::Config,
    },
}
pub enum ChildDeliveryProgress {
    Local(LocalPassed),
    Branch(PublishedChildBranch),
    PullRequest(ChildPullRequest),
    ExactPending(PendingExactValidation),
    Exact(ExactValidationPassed),
    Hosted(HostedPassed),
    StagingPending(PendingStaging),
    Staged(StagedDelta),
    Published(Box<PublishedChild>),
    Closed(Receipt),
}
impl DeliveryCoordinator {
    pub async fn advance_child(
        &self,
        admission: &DeliveryAdmission,
        claim: &CompletedChildClaim,
        workflow: &Workflow,
        action: ChildDeliveryAction<'_>,
    ) -> Result<ChildDeliveryProgress> {
        self.current_child_claim(admission, claim)?;
        if workflow.digest != admission.gate.workflow_digest() {
            return Err(Error::new("stale_result", "phase workflow changed"));
        }
        Ok(match action {
            ChildDeliveryAction::ValidateLocal {
                validation,
                completion,
            } => {
                completed_run(validation, completion)?;
                if validation.outcome().instructions.role != AgentRole::Validation
                    || validation.original_authority().work() != &admission.scope.work
                    || validation
                        .outcome()
                        .instructions
                        .repository_revision
                        .as_deref()
                        != Some(&admission.scope.revision.tested_commit)
                    || validation.original_authority().config_revision()
                        != admission.gate.config_revision()
                    || validation.original_authority().workflow_digest() != workflow.digest
                {
                    return Err(Error::new(
                        "validation_gate",
                        "local validation needs a current completed exact-commit validation session",
                    ));
                }
                let required = local_requirements(self.contract(admission)?, workflow)?;
                let records: Vec<_> = completion
                    .checks
                    .iter()
                    .map(|c| (c.record(), false))
                    .chain(completion.failed_checks.iter().map(|c| (c.record(), true)))
                    .collect();
                match super::continuation::latest_required_failures(&required, &records) {
                    Ok(_) => {
                        return Err(Error::new(
                            "validation_failed",
                            "latest required check remains failed",
                        ));
                    }
                    Err(error) if error.code == "no_work_trigger" => {}
                    Err(error) => return Err(error),
                }
                ChildDeliveryProgress::Local(self.verify_local_checks(
                    admission,
                    workflow,
                    &completion.checks.iter().collect::<Vec<_>>(),
                )?)
            }
            ChildDeliveryAction::PublishBranch {
                source,
                local,
                branch,
                previous,
            } => {
                source.matches(admission, claim)?;
                let committed = source.committed.as_ref().ok_or_else(|| {
                    Error::new("no_change", "no-change work must not publish a branch")
                })?;
                let change = source.change.as_ref().ok_or_else(|| {
                    Error::new(
                        "candidate_provenance",
                        "committed source lacks reviewed delta identity",
                    )
                })?;
                ChildDeliveryProgress::Branch(
                    self.publish_child_objects(
                        admission,
                        ChildObjectPublication {
                            change,
                            committed,
                            local,
                            branch,
                            previous,
                        },
                    )
                    .await?,
                )
            }
            ChildDeliveryAction::OpenPullRequest {
                branch,
                local,
                title,
            } => {
                if branch.revision() != &admission.scope.revision {
                    return Err(Error::new(
                        "stale_result",
                        "published branch revision changed",
                    ));
                }
                ChildDeliveryProgress::PullRequest(
                    self.open_child_pull_request(
                        admission,
                        local,
                        branch.branch(),
                        title,
                        &claim.claim.summary,
                    )
                    .await?,
                )
            }
            ChildDeliveryAction::DispatchExactValidation => ChildDeliveryProgress::ExactPending(
                self.dispatch_exact_validation(admission).await?,
            ),
            ChildDeliveryAction::ObserveExactValidation { pending, verifier } => {
                ChildDeliveryProgress::Exact(
                    self.acquire_exact_validation(admission, pending, verifier)
                        .await?,
                )
            }
            ChildDeliveryAction::ObserveHosted { tokens, verifier } => {
                ChildDeliveryProgress::Hosted(
                    self.observe_hosted_checks(admission, CheckKind::HostedCi, tokens, verifier)
                        .await?,
                )
            }
            ChildDeliveryAction::RequestStaging { gates } => ChildDeliveryProgress::StagingPending(
                self.request_staging_merge(admission, gates).await?,
            ),
            ChildDeliveryAction::ObserveStaging {
                source,
                change,
                pull_request,
                pending,
            } => ChildDeliveryProgress::Staged(
                self.observe_staging_merge(admission, source, change, pull_request, pending)
                    .await?,
            ),
            ChildDeliveryAction::PublishChanged {
                local,
                hosted,
                review,
                staged,
            } => {
                if !matches!(claim.claim.disposition, ChildClaimDisposition::Changed) {
                    return Err(Error::new(
                        "worker_claim",
                        "a no-change claim cannot publish a changed completion",
                    ));
                }
                ChildDeliveryProgress::Published(Box::new(
                    self.publish_changed_child(
                        admission,
                        ChangedChildCompletion {
                            workflow,
                            criteria: &claim.claim.fulfilled_criteria,
                            local,
                            hosted,
                            review,
                            staged,
                        },
                    )
                    .await?,
                ))
            }
            ChildDeliveryAction::PublishNoChange {
                local,
                exact,
                review,
            } => {
                let ChildClaimDisposition::NoChange { reason } = &claim.claim.disposition else {
                    return Err(Error::new(
                        "worker_claim",
                        "changed work cannot silently become no-change completion",
                    ));
                };
                ChildDeliveryProgress::Published(Box::new(
                    self.publish_no_change_child(
                        admission,
                        NoChangeCompletion {
                            workflow,
                            criteria: &claim.claim.fulfilled_criteria,
                            reason,
                            local,
                            exact,
                            review,
                        },
                    )
                    .await?,
                ))
            }
            ChildDeliveryAction::Close {
                result,
                configuration,
            } => ChildDeliveryProgress::Closed(
                self.close_child(admission, result, configuration).await?,
            ),
        })
    }
}

/// Actual host commit/proposal from the child's owned source clone. No JSON
/// constructor exists; a fresh exact-head validation is still mandatory.
pub struct SealedChildSource {
    work: WorkIdentity,
    claim_digest: String,
    revision: RevisionBinding,
    committed: Option<crate::git::CommittedSource>,
    change: Option<ScopedChange>,
    receipt: Receipt,
}
impl SealedChildSource {
    pub(super) fn matches_worker(
        &self,
        admission: &DeliveryAdmission,
        claim: &CompletedChildClaim,
    ) -> Result<()> {
        self.matches(admission, claim)
    }
    pub fn revision(&self) -> &RevisionBinding {
        &self.revision
    }
    pub fn change(&self) -> Option<&ScopedChange> {
        self.change.as_ref()
    }
    pub fn receipt(&self) -> &Receipt {
        &self.receipt
    }
    fn matches(&self, admission: &DeliveryAdmission, claim: &CompletedChildClaim) -> Result<()> {
        if self.work != admission.scope.work
            || self.claim_digest != claim.digest()
            || self.revision != admission.scope.revision
        {
            return Err(Error::new(
                "stale_result",
                "sealed child source differs from current exact admission",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceSealIntent {
    schema_version: u32,
    operation_id: Uuid,
    request_id: Uuid,
    scope: DeliveryScope,
    claim_digest: String,
    source: PathBuf,
    binding: crate::execution::ExecutionBinding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    correction: Option<crate::execution::CorrectionExecutionProvenance>,
    metadata: crate::git::SourceCommitMetadata,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceSealRecord {
    intent: SourceSealIntent,
    revision: RevisionBinding,
    receipt: Uuid,
    review: Option<crate::git::ReviewDelta>,
}
impl DeliveryCoordinator {
    /// Commit only after the implementation process tree is actually known to
    /// have exited. Every subprocess starts under current fenced authority.
    pub async fn seal_child_source(
        &self,
        admission: &DeliveryAdmission,
        claim: &CompletedChildClaim,
        execution: &crate::worker::WorkerExecution,
        isolation: &crate::isolation::ValidatedIsolation,
        metadata: crate::git::SourceCommitMetadata,
    ) -> Result<SealedChildSource> {
        self.current(admission, &[DeliveryPhase::SourceSealing])?;
        self.current_child_claim(admission, claim)?;
        let scope = admission.gate.child_scope().ok_or_else(|| {
            Error::new("scope_violation", "source sealing lacks exact child scope")
        })?;
        let effective = claim
            .correction
            .as_ref()
            .map(|transition| {
                crate::execution::EffectiveExecutionBinding::new(
                    &execution.binding,
                    &execution.ownership,
                    isolation,
                    transition.clone(),
                )
            })
            .transpose()?;
        let effective_work = effective
            .as_ref()
            .map(|binding| &binding.binding().work)
            .unwrap_or(&execution.binding.work);
        if effective_work != &admission.scope.work
            || execution.binding.work != *claim.outcome.original_authority().work()
            || execution.binding.operation_id != claim.operation_id()
            || execution.binding.repository != admission.scope.repository
            || execution.binding.revision.commit != admission.scope.revision.head
            || execution.binding.revision.tree != admission.scope.revision.tested_tree
            || claim
                .outcome
                .outcome()
                .instructions
                .repository_root
                .as_deref()
                != Some(isolation.plan().repository.as_path())
            || scope.write_repository.as_deref()
                != Some(self.contract(admission)?.configuration_name.as_str())
            || isolation.digest() != execution.binding.isolation_digest
        {
            return Err(Error::new(
                "scope_violation",
                "source sealing is not the exited child's owned clone and scope",
            ));
        }
        execution.ownership.verify(&execution.binding, isolation)?;
        let intent = SourceSealIntent {
            schema_version: if effective.is_some() { 2 } else { 1 },
            operation_id: admission.operation_id(),
            request_id: Uuid::new_v4(),
            scope: admission.scope.clone(),
            claim_digest: claim.digest().into(),
            source: isolation.plan().repository.clone(),
            binding: execution.binding.clone(),
            correction: effective
                .as_ref()
                .map(crate::execution::EffectiveExecutionBinding::provenance)
                .transpose()?,
            metadata,
        };
        let leaf = format!("source-seal-intent-{}.json", admission.operation_id());
        let intent = {
            let _lock = self.records.lock("source-sealing.lock")?;
            match self.records.read::<Signed<SourceSealIntent>>(&leaf) {
                Ok(prior) => {
                    self.key.verify("child_source_intent", &prior)?;
                    let mut comparable = intent.clone();
                    comparable.request_id = prior.payload.request_id;
                    if canonical_digest(1, &comparable)? != canonical_digest(1, &prior.payload)? {
                        return Err(Error::new(
                            "operation_conflict",
                            "source seal identity was reused for different work",
                        ));
                    }
                    // A retained attempt is never silently committed again. Its
                    // durable control receipt determines explicit reconciliation.
                    return Err(Error::new(
                        "reconciliation_required",
                        "source seal already attempted; inspect retained Git objects and receipt before retry",
                    ));
                }
                Err(e) if e.code == "record_missing" => {
                    self.records
                        .write(&leaf, &self.key.sign("child_source_intent", &intent)?)?;
                    intent
                }
                Err(e) => return Err(e),
            }
        };
        let binding = execution.binding.clone();
        let ownership = execution.ownership.clone();
        let boundary = isolation.clone();
        let write_scope = crate::git::WriteScope {
            paths: scope.write_paths.clone(),
        };
        let metadata = intent.metadata.clone();
        let work = admission.scope.work.clone();
        let repository = admission.scope.repository.clone();
        let source = intent.source.clone();
        let child_scope_digest = admission
            .gate
            .child_scope_digest()
            .ok_or_else(|| Error::new("scope_violation", "child scope digest missing"))?
            .to_owned();
        let expected_changed = matches!(claim.claim.disposition, ChildClaimDisposition::Changed);
        let host = self.host.clone();
        let permit = admission.permit.clone();
        let gate = admission.gate.clone();
        let check = self.current_gate.clone();
        let start_gate: crate::git::GitStartGate = Arc::new(move |spawn| {
            host.validate_permit(&permit, || {
                check(&gate)?;
                spawn()
            })
        });
        let (effect,running)=self.host.begin_effect(MutationRequest {
            request_id:intent.request_id,runtime_id:admission.scope.runtime_id,
            actor:Actor{principal:None,identity:format!("owner:{}",admission.permit.lease.owner_id),roles:BTreeSet::new()},
            kind:"child_source_sealing".into(),canonical_targets:vec![admission.scope.repository.id.clone()],
            expected:BTreeMap::from([("delivery_scope".into(),admission.scope.digest()?)]),
            payload:serde_json::to_value(&intent).map_err(|_|Error::invalid("cannot encode source seal"))?,
        },&admission.permit,||(self.current_gate)(&admission.gate),|receipt|{
            let running=self.git.start_journaled_operation(crate::git::GitLimits::default(),start_gate,
                self.processes.clone(),admission.permit.clone(),move|git|{
                    if !expected_changed {
                        git.verify_execution_tree(&source,&binding.revision,&BTreeSet::new())?;
                    }
                    let committed=git.commit_execution_changes(&ownership,&binding,&boundary,&write_scope,metadata)?;
                    if committed.is_some()!=expected_changed {
                        return Err(Error::new("worker_claim","actual source change differs from completed child assertion; retain produced objects"));
                    }
                    let result=committed.as_ref().map(|c|c.revision().clone()).unwrap_or_else(||binding.revision.clone());
                    let revision=RevisionBinding{repository:repository.clone(),head:result.commit.clone(),base:binding.revision.commit.clone(),
                        tested_commit:result.commit.clone(),tested_tree:result.tree.clone()};
                    let change=if committed.is_some(){Some(ScopedChange{proposal:git.propose_delta(&source,crate::git::ProposeDelta {
                        repository,parent_id:work.parent,child_id:work.child.ok_or_else(||Error::new("scope_violation","child missing"))?,
                        work_generation:&work.generation.definition,correction_generation:&work.generation.correction,
                        scope:write_scope,base:&binding.revision.commit,head:&result.commit,
                    })?,child_scope_digest})}else{None};
                    Ok((committed,change,revision))
                })?;
            self.executions.register(admission.operation_id(),receipt.operation_id,Arc::new(running.cancellation()))?;
            Ok(running)
        })?;
        let cancellation = running.cancellation();
        let result = running.wait().await;
        if !matches!(cancellation.exited(), Ok(true)) {
            self.host.fence(admission.operation_id())?;
            self.executions
                .cancel_fenced(&self.host, admission.operation_id())?;
            return Err(Error::new(
                "operation_uncertain",
                "source sealing process exit is unconfirmed; retain source and receipt",
            ));
        }
        self.executions
            .remove(admission.operation_id(), effect.operation_id())?;
        let receipt = self.host.complete_effect(
            effect,
            &admission.permit,
            || (self.current_gate)(&admission.gate),
            match &result {
                Ok((_, _, revision)) => json!({"revision":revision}),
                Err(error) => json!({"error":error,"source":"retained"}),
            },
        )?;
        let (committed, change, revision) = result?;
        self.records.write(
            &format!("source-seal-result-{}.json", admission.operation_id()),
            &self.key.sign(
                "child_source_result",
                &SourceSealRecord {
                    intent,
                    revision: revision.clone(),
                    receipt: receipt.request_id,
                    review: change.as_ref().map(|c| c.review().clone()),
                },
            )?,
        )?;
        Ok(SealedChildSource {
            work: admission.scope.work.clone(),
            claim_digest: claim.digest().into(),
            revision,
            committed,
            change,
            receipt,
        })
    }
}

pub enum ChildReviewTarget<'a> {
    Changed {
        change: &'a ScopedChange,
        pull_request: &'a ChildPullRequest,
    },
    NoChange {
        source: &'a SealedChildSource,
    },
}
#[derive(Clone, Debug)]
pub struct ChildReviewAssignment {
    work: WorkIdentity,
    revision: RevisionBinding,
    delta_digest: String,
    pull_request: Option<u64>,
    context: Value,
    config_revision: String,
    workflow_digest: String,
}
impl ChildReviewAssignment {
    pub(super) fn matches_worker(&self, admission: &DeliveryAdmission) -> Result<()> {
        if self.work != admission.scope.work
            || self.revision != admission.scope.revision
            || self.config_revision != admission.gate.config_revision()
            || self.workflow_digest != admission.gate.workflow_digest()
        {
            return Err(Error::new(
                "review_gate",
                "review assignment differs from current worker scope",
            ));
        }
        Ok(())
    }
    pub fn context(&self) -> &Value {
        &self.context
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewResultClaim {
    pub schema_version: u32,
    pub verdict: evidence::Verdict,
    pub unresolved_findings: Vec<String>,
    pub supporting_links: Vec<String>,
}
impl DeliveryCoordinator {
    pub fn prepare_child_review(
        &self,
        admission: &DeliveryAdmission,
        claim: &CompletedChildClaim,
        target: ChildReviewTarget<'_>,
        base_context: Value,
    ) -> Result<ChildReviewAssignment> {
        self.current(
            admission,
            &[DeliveryPhase::ReviewPublication, DeliveryPhase::NoChange],
        )?;
        self.current_child_claim(admission, claim)?;
        let (delta_digest, pull_request) = match target {
            ChildReviewTarget::Changed {
                change,
                pull_request,
            } => {
                change.matches(admission)?;
                pull_request.matches(admission)?;
                if !matches!(claim.claim.disposition, ChildClaimDisposition::Changed) {
                    return Err(Error::new(
                        "worker_claim",
                        "review target differs from completed changed claim",
                    ));
                }
                (change.review_digest()?, Some(pull_request.number()))
            }
            ChildReviewTarget::NoChange { source } => {
                source.matches(admission, claim)?;
                let ChildClaimDisposition::NoChange { reason } = &claim.claim.disposition else {
                    return Err(Error::new(
                        "worker_claim",
                        "review target is not completed no-change work",
                    ));
                };
                if source.committed.is_some() {
                    return Err(Error::new(
                        "candidate_provenance",
                        "no-change review source has actual changes",
                    ));
                }
                (
                    evidence::no_change_digest(
                        &admission.scope.work,
                        &admission.scope.revision,
                        &claim.claim.fulfilled_criteria,
                        reason,
                    )?,
                    None,
                )
            }
        };
        let context = json!({"task":base_context,"delivery_review":{"schema_version":1,
            "work":admission.scope.work,"revision":admission.scope.revision,"delta_digest":delta_digest,
            "pull_request":pull_request,"child_claim":claim.claim,"claim_provenance":claim.digest(),
            "required_result_schema":{"schema_version":1,"verdict":"pass | findings | blocked",
                "unresolved_findings":[],"supporting_links":[]}}});
        if serde_json::to_vec(&context)
            .map_err(|_| Error::invalid("review context encoding"))?
            .len()
            > 8 * 1024 * 1024
        {
            return Err(Error::new(
                "bounded_input",
                "review context exceeds finite limit",
            ));
        }
        Ok(ChildReviewAssignment {
            work: admission.scope.work.clone(),
            revision: admission.scope.revision.clone(),
            delta_digest,
            pull_request,
            context,
            config_revision: admission.gate.config_revision().into(),
            workflow_digest: admission.gate.workflow_digest().into(),
        })
    }
    pub(super) fn completed_review_claim(
        admission: &DeliveryAdmission,
        assignment: &ChildReviewAssignment,
        outcome: &VerifiedAuthorizedOutcome,
        completion: &RunCompletion,
    ) -> Result<ReviewResultClaim> {
        completed_run(outcome, completion)?;
        let worker = outcome.outcome();
        if assignment.work != admission.scope.work
            || assignment.revision != admission.scope.revision
            || assignment.config_revision != admission.gate.config_revision()
            || assignment.workflow_digest != admission.gate.workflow_digest()
            || outcome.original_authority().work() != &admission.scope.work
            || outcome.original_authority().config_revision() != admission.gate.config_revision()
            || canonical_digest(1, &worker.instructions.profile)?
                != canonical_digest(1, admission.gate.profile())?
            || worker.instructions.role != AgentRole::Review
            || worker.instructions.workflow_digest != assignment.workflow_digest
            || worker.instructions.repository_revision.as_deref() != Some(&assignment.revision.head)
            || worker.instructions.permissions.repository_code_execution
            || !worker
                .instructions
                .permissions
                .source_write_paths
                .is_empty()
            || !worker
                .instructions
                .permissions
                .resource_write_paths
                .is_empty()
        {
            return Err(Error::new(
                "review_gate",
                "completed review does not match host-prepared exact patch context",
            ));
        }
        let contract = completion.contract.as_ref().ok_or_else(|| {
            Error::new("worker_binding", "review lacks actual assembled contract")
        })?;
        if contract.instructions().manifest_digest != worker.instruction_digest
            || contract.tools().digest() != worker.tool_contract_digest
        {
            return Err(Error::new(
                "worker_binding",
                "review contract differs from protected outcome",
            ));
        }
        assignment::verify_assignment_contract(
            contract,
            &admission.gate,
            "child_codex_review",
            &assignment.context,
        )?;
        let claim: ReviewResultClaim = serde_json::from_value(worker.claim.clone())
            .map_err(|_| Error::new("worker_claim", "review result has no supported schema"))?;
        if claim.schema_version != 1
            || claim.unresolved_findings.len() > 256
            || claim.supporting_links.len() > 256
            || claim
                .unresolved_findings
                .iter()
                .chain(&claim.supporting_links)
                .any(|s| s.trim().is_empty() || s.len() > 65536)
            || claim.verdict == evidence::Verdict::Pass && !claim.unresolved_findings.is_empty()
        {
            return Err(Error::new(
                "review_gate",
                "review result is inconsistent or exceeds finite bounds",
            ));
        }
        Ok(claim)
    }
    pub async fn publish_completed_review(
        &self,
        admission: &DeliveryAdmission,
        assignment: &ChildReviewAssignment,
        outcome: &VerifiedAuthorizedOutcome,
        completion: &Arc<RunCompletion>,
    ) -> Result<PublishedReview> {
        self.publish_completed_review_guarded(admission, assignment, outcome, completion, None)
            .await
    }
    pub(super) async fn publish_completed_review_guarded(
        &self,
        admission: &DeliveryAdmission,
        assignment: &ChildReviewAssignment,
        outcome: &VerifiedAuthorizedOutcome,
        completion: &Arc<RunCompletion>,
        extra: Option<GateCheck>,
    ) -> Result<PublishedReview> {
        self.current(
            admission,
            &[DeliveryPhase::ReviewPublication, DeliveryPhase::NoChange],
        )?;
        let claim = Self::completed_review_claim(admission, assignment, outcome, completion)?;
        let worker = outcome.outcome();
        let signed =
            self.key.sign(
                "codex_review",
                &evidence::ReviewAttestation {
                    schema_version: 1,
                    runtime_id: admission.scope.runtime_id,
                    owner_generation: admission.permit.lease.owner_generation,
                    operation_id: admission.operation_id(),
                    work: assignment.work.clone(),
                    revision: assignment.revision.clone(),
                    pull_request: assignment.pull_request,
                    delta_digest: assignment.delta_digest.clone(),
                    verdict: claim.verdict,
                    unresolved_findings: claim.unresolved_findings,
                    supporting_links: claim.supporting_links,
                    review: evidence::ReviewKind::Agent {
                        session_id: worker.session.thread_id.clone().ok_or_else(|| {
                            Error::new("review_gate", "review lacks actual thread")
                        })?,
                        instruction_manifest: worker.instruction_digest.clone(),
                    },
                },
            )?;
        self.records.write(&format!("completed-review-{}.json",admission.operation_id()),
            &self.key.sign("completed_review_provenance",&json!({"claim":outcome.digest(),"review":signed,
                "context_digest":worker.instructions.context_digest,"completed_turn":worker.completed_turn}))?)?;
        self.publish_review_guarded(admission, &signed, extra).await
    }
}

impl DeliveryCoordinator {
    /// Reconstruct an acknowledged source seal from real retained Git objects.
    /// Missing/uncertain receipts remain held for explicit operator recovery.
    pub async fn recover_child_source(
        &self,
        admission: &DeliveryAdmission,
        claim: &CompletedChildClaim,
        operation: Uuid,
        ownership: Arc<crate::execution::OwnedDirectory>,
        isolation: crate::isolation::ValidatedIsolation,
    ) -> Result<SealedChildSource> {
        self.current_child_claim(admission, claim)?;
        let signed: Signed<SourceSealRecord> = self
            .records
            .read(&format!("source-seal-result-{operation}.json"))?;
        self.key.verify("child_source_result", &signed)?;
        let record = signed.payload;
        let original: Signed<SourceSealIntent> = self
            .records
            .read(&format!("source-seal-intent-{operation}.json"))?;
        self.key.verify("child_source_intent", &original)?;
        if canonical_digest(1, &record.intent)? != canonical_digest(1, &original.payload)?
            || !matches!(record.intent.schema_version, 1 | 2)
            || (record.intent.schema_version == 2) != record.intent.correction.is_some()
            || record.intent.operation_id != operation
            || record.intent.scope.work != admission.scope.work
            || record.intent.claim_digest != claim.digest()
            || record.intent.scope.runtime_id != admission.scope.runtime_id
            || record.intent.scope.repository != admission.scope.repository
            || record.intent.scope.contract_digest != admission.scope.contract_digest
            || record.intent.source != isolation.plan().repository
            || record.revision != admission.scope.revision
            || record.receipt != record.intent.request_id
        {
            return Err(Error::new(
                "stale_result",
                "retained source seal does not match current owned work and revision",
            ));
        }
        ownership.verify(&record.intent.binding, &isolation)?;
        let receipt = self.host.operation(record.receipt)?;
        let expected = BTreeMap::from([
            ("delivery_scope".to_owned(), record.intent.scope.digest()?),
            ("admitted_scope".to_owned(), record.intent.scope.digest()?),
            (
                "admission_operation".to_owned(),
                record.intent.operation_id.to_string(),
            ),
            (
                "admitted_work".to_owned(),
                crate::authority::IssueKey::new(
                    &record.intent.scope.work.linear_endpoint,
                    record.intent.scope.work.linear_workspace,
                    record.intent.scope.work.child.ok_or_else(|| {
                        Error::new("scope_violation", "source seal lacks child identity")
                    })?,
                )?
                .identity()?,
            ),
        ]);
        let expected_payload = canonical_digest(
            1,
            &json!({"kind":"child_source_sealing","targets":[record.intent.scope.repository.id],
            "expected":expected,"payload":record.intent,"principal":null}),
        )?;
        if receipt.runtime_id != admission.scope.runtime_id
            || receipt.kind != "child_source_sealing"
            || receipt.payload_digest != expected_payload
            || receipt.phase != crate::control::ReceiptPhase::Acknowledged
            || receipt
                .observed_result
                .as_ref()
                .and_then(|v| v.get("revision"))
                != Some(
                    &serde_json::to_value(&record.revision)
                        .map_err(|_| Error::invalid("source revision encoding"))?,
                )
        {
            return Err(Error::new(
                "reconciliation_required",
                "source seal lacks exact acknowledged owned effect",
            ));
        }
        let scope = admission
            .gate
            .child_scope()
            .ok_or_else(|| Error::new("scope_violation", "current child scope missing"))?;
        let write_scope = crate::git::WriteScope {
            paths: scope.write_paths.clone(),
        };
        let child_scope_digest = admission
            .gate
            .child_scope_digest()
            .ok_or_else(|| Error::new("scope_violation", "current child scope digest missing"))?
            .to_owned();
        let host = self.host.clone();
        let permit = admission.permit.clone();
        let gate = admission.gate.clone();
        let check = self.current_gate.clone();
        let start_gate: crate::git::GitStartGate = Arc::new(move |spawn| {
            host.validate_permit(&permit, || {
                check(&gate)?;
                spawn()
            })
        });
        let git = self.git.clone();
        let execution_scope = crate::git::GitExecutionScope {
            limits: crate::git::GitLimits::default(),
            start_gate,
            processes: self.processes.clone(),
            permit: admission.permit.clone(),
            executions: self.executions.clone(),
        };
        let expected_changed = matches!(claim.claim.disposition, ChildClaimDisposition::Changed);
        let work = admission.scope.work.clone();
        let revision = record.revision.clone();
        let (committed, change) = tokio::task::spawn_blocking(move || {
            let git = git.with_execution_scope(execution_scope)?;
            let head = crate::git::Revision {
                commit: revision.head.clone(),
                tree: revision.tested_tree.clone(),
            };
            git.verify_execution_tree(&record.intent.source, &head, &BTreeSet::new())?;
            if !expected_changed {
                if head != record.intent.binding.revision || record.review.is_some() {
                    return Err(Error::new(
                        "provenance_mismatch",
                        "retained no-change source has a delta",
                    ));
                }
                return Ok((None, None));
            }
            let committed = git.inspect_committed_source(
                &record.intent.source,
                &record.intent.binding.revision,
                &head,
                &write_scope,
                record.intent.metadata,
            )?;
            let proposal = git.propose_delta(
                &record.intent.source,
                crate::git::ProposeDelta {
                    repository: revision.repository,
                    parent_id: work.parent,
                    child_id: work
                        .child
                        .ok_or_else(|| Error::new("scope_violation", "source has no child"))?,
                    work_generation: &work.generation.definition,
                    correction_generation: &work.generation.correction,
                    scope: write_scope,
                    base: &record.intent.binding.revision.commit,
                    head: &head.commit,
                },
            )?;
            if record.review.as_ref() != Some(proposal.review()) {
                return Err(Error::new(
                    "provenance_mismatch",
                    "retained source review differs from actual Git patch",
                ));
            }
            Ok((
                Some(committed),
                Some(ScopedChange {
                    proposal,
                    child_scope_digest,
                }),
            ))
        })
        .await
        .map_err(|_| Error::new("operation_uncertain", "source recovery task did not finish"))??;
        self.current_child_claim(admission, claim)?;
        Ok(SealedChildSource {
            work: admission.scope.work.clone(),
            claim_digest: claim.digest().into(),
            revision: admission.scope.revision.clone(),
            committed,
            change,
            receipt,
        })
    }
}

pub(super) fn completed_claim_run(
    claim: &CompletedChildClaim,
    completion: &Arc<RunCompletion>,
) -> Result<()> {
    completed_run(&claim.outcome, completion)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        tempfile::TempDir,
        VerifiedAuthorizedOutcome,
        Arc<RunCompletion>,
    ) {
        let digest = bytes_digest(b"fixture");
        crate::runtime_claims::test_fixture(
            AgentRole::Implementation,
            WorkIdentity {
                linear_endpoint: "https://linear.invalid/graphql".into(),
                linear_workspace: Uuid::from_u128(2),
                parent: Uuid::from_u128(10),
                child: Some(Uuid::from_u128(11)),
                generation: evidence::Generation {
                    authorization: digest.clone(),
                    definition: digest.clone(),
                    correction: digest.clone(),
                    scope: digest,
                },
            },
            json!({"fixture":"protected child claim"}),
            Some("a".repeat(40)),
        )
    }
    #[test]
    fn completed_delivery_run_requires_same_actual_terminal_turn_and_exit() {
        let (_temp, claim, completion) = fixture();
        completed_run(&claim, &completion).unwrap();
        let mut completion =
            Arc::try_unwrap(completion).unwrap_or_else(|_| panic!("fixture completion is shared"));
        completion.local_exit_confirmed = false;
        assert_eq!(
            completed_run(&claim, &completion).unwrap_err().code,
            "worker_binding"
        );
        completion.local_exit_confirmed = true;
        completion.state.completed_turns.clear();
        assert_eq!(
            completed_run(&claim, &completion).unwrap_err().code,
            "worker_binding"
        );
    }
    #[test]
    fn successful_claim_cannot_hide_run_failure_or_another_operation() {
        let (_temp, claim, completion) = fixture();
        let mut completion =
            Arc::try_unwrap(completion).unwrap_or_else(|_| panic!("fixture completion is shared"));
        completion.operation_id = Uuid::new_v4();
        assert_eq!(
            completed_run(&claim, &completion).unwrap_err().code,
            "worker_binding"
        );
        completion.operation_id = claim.outcome().operation_id;
        completion.hook_error = Some(Error::new("hook_failure", "after_run failed"));
        assert_eq!(
            completed_run(&claim, &completion).unwrap_err().code,
            "worker_binding"
        );
    }
    #[test]
    fn exact_review_validator_rejects_other_head_context_correction_and_unknown_exit() {
        let (_original_temp, original, _) = fixture();
        let context = json!({"patch":"exact protected review context"});
        let (_temp, outcome, completion) = crate::runtime_claims::test_fixture_with_claim(
            AgentRole::Review,
            original.original_authority().work().clone(),
            context.clone(),
            Some("a".repeat(40)),
            json!({"schema_version":1,"verdict":"findings","unresolved_findings":["required null case fails"],"supporting_links":[]}),
        );
        let gate = outcome.original_authority().clone();
        let revision = RevisionBinding {
            repository: RepositoryId {
                hostname: "github.com".into(),
                id: "42".into(),
            },
            head: "a".repeat(40),
            base: "b".repeat(40),
            tested_commit: "a".repeat(40),
            tested_tree: "c".repeat(40),
        };
        let scope = DeliveryScope {
            schema_version: 1,
            runtime_id: Uuid::from_u128(1),
            phase: DeliveryPhase::ReviewPublication,
            work: gate.work().clone(),
            repository: revision.repository.clone(),
            revision: revision.clone(),
            fresh_gate_digest: gate.digest().unwrap(),
            contract_digest: bytes_digest(b"fixture contract"),
            candidate_vector_digest: None,
        };
        let permit = Permit {
            operation_id: Uuid::new_v4(),
            lease: crate::control::Lease {
                runtime_id: scope.runtime_id,
                owner_generation: 1,
                owner_id: Uuid::new_v4(),
            },
            control_generation: 1,
            work_key: gate.work_key().unwrap(),
            scope_digest: scope.digest().unwrap(),
            registered_at: Utc::now(),
        };
        let admission = DeliveryAdmission::bind(permit, gate.clone(), scope).unwrap();
        let assignment = ChildReviewAssignment {
            work: gate.work().clone(),
            revision,
            delta_digest: bytes_digest(b"actual fixture delta identity"),
            pull_request: Some(1),
            context,
            config_revision: gate.config_revision().into(),
            workflow_digest: gate.workflow_digest().into(),
        };
        assert_eq!(
            DeliveryCoordinator::completed_review_claim(
                &admission,
                &assignment,
                &outcome,
                &completion
            )
            .unwrap_err()
            .code,
            "worker_binding"
        );
        for mismatch in 0..3 {
            let mut wrong = assignment.clone();
            match mismatch {
                0 => wrong.revision.head = "d".repeat(40),
                1 => wrong.context = json!({"patch":"other"}),
                _ => wrong.work.generation.correction = bytes_digest(b"other correction"),
            }
            assert_eq!(
                DeliveryCoordinator::completed_review_claim(
                    &admission,
                    &wrong,
                    &outcome,
                    &completion
                )
                .unwrap_err()
                .code,
                if mismatch == 1 {
                    "worker_binding"
                } else {
                    "review_gate"
                }
            );
        }
        let mut completion =
            Arc::try_unwrap(completion).unwrap_or_else(|_| panic!("fixture completion shared"));
        completion.local_exit_confirmed = false;
        assert_eq!(
            DeliveryCoordinator::completed_review_claim(
                &admission,
                &assignment,
                &outcome,
                &completion
            )
            .unwrap_err()
            .code,
            "worker_binding"
        );
        completion.local_exit_confirmed = true;
        completion.state.binding.profile.model = "another-profile".into();
        assert_eq!(
            DeliveryCoordinator::completed_review_claim(
                &admission,
                &assignment,
                &outcome,
                &completion
            )
            .unwrap_err()
            .code,
            "worker_binding"
        );
        // This validates authenticated review input only. The fixture deliberately
        // has no native WorkerContract or source and cannot mint a repair trigger.
    }
    #[test]
    fn unsupported_claim_fields_cannot_smuggle_passed_gate_evidence() {
        assert!(
            serde_json::from_value::<ChildResultClaim>(json!({"schema_version":1,"summary":"done",
            "fulfilled_criteria":["criterion"],"disposition":{"kind":"changed"},"ci_passed":true}))
            .is_err()
        );
        assert!(
            serde_json::from_value::<ReviewResultClaim>(
                json!({"schema_version":1,"verdict":"pass",
            "unresolved_findings":[],"supporting_links":[],"revision":{"head":"arbitrary"}})
            )
            .is_err()
        );
    }
}
