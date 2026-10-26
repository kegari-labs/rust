//! Same-generation parent reassembly from an authenticated human-reviewed
//! parent and actual released-baseline transition. No public record can mint
//! this trigger, and it never renews an implementation/parent phase budget.
use super::accepted_origin::{AcceptedOriginIdentity, VerifiedAcceptedOriginRefs};
use super::*;
use crate::{
    config::Config,
    control::{ParentReassemblyReference, ReviewReceiptReference, RunReference},
    decisions::VerifiedRetryGrant,
    evidence::VerifiedParent,
    runtime_delivery::VerifiedBaselineTransition,
    scheduler::PhaseKey,
    worker::{WorkerAuthority, WorkerContract},
    worker_run::{RunCompletion, RunOutcome},
};

#[path = "delivery_parent_reassembly_grants.rs"]
mod grant_lineage;

fn blocked(message: &str) -> Error {
    Error::new("parent_reassembly", message)
}

#[derive(Clone)]
pub(crate) struct AcceptedParentOrigin {
    published: PublishedParent,
    acceptance: evidence::VerifiedAcceptance,
    signed_acceptance: Signed<evidence::Acceptance>,
    references: VerifiedAcceptedOriginRefs,
    records: Arc<PrivateDir>,
    control: Arc<ControlHost>,
    key: Arc<AttestationKey>,
    linear: crate::providers::LinearClient,
    workspace: Uuid,
    configuration_revision: String,
    digest: String,
}
impl AcceptedParentOrigin {
    pub(crate) fn published(&self) -> &PublishedParent {
        &self.published
    }
    pub(crate) fn parent(&self) -> &VerifiedParent {
        self.published.verified()
    }
    pub(super) fn signed_acceptance(&self) -> &Signed<evidence::Acceptance> {
        &self.signed_acceptance
    }
    pub(super) fn references(&self) -> &super::accepted_origin::AcceptedOriginRef {
        self.references.reference()
    }
    pub(crate) fn acceptance(&self) -> &evidence::VerifiedAcceptance {
        &self.acceptance
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn publication_ids(&self) -> (Uuid, Uuid) {
        self.references.reference().publication_ids()
    }
    pub(super) async fn reobserve_native(&self, config: &Config) -> Result<()> {
        self.validate(config)?;
        let identity = self.linear.identity().await?;
        let comments = self
            .linear
            .comments(self.parent().manifest().work.parent)
            .await?;
        let (parent_id, acceptance_id) = self.publication_ids();
        let select = |id| -> Result<&crate::providers::Comment> {
            let found = comments.iter().filter(|c| c.id == id).collect::<Vec<_>>();
            let [comment] = found.as_slice() else {
                return Err(Error::new(
                    "evidence_unpublished",
                    "original carry publication missing or duplicated",
                ));
            };
            if comment.issue_id != self.parent().manifest().work.parent
                || comment.bot
                || comment.delegated
                || comment.user_id.map(|id| id.to_string()).as_deref()
                    != Some(identity.account_id.as_str())
                || bytes_digest(comment.body.as_bytes()) != comment.body_digest
            {
                return Err(Error::new(
                    "evidence_unpublished",
                    "original carry publication writer/body changed",
                ));
            }
            Ok(*comment)
        };
        let parent: Signed<ParentResultRecord> = evidence::decode_signed_publication(
            &select(parent_id)?.body,
            "parent_result",
            &self.key,
        )?;
        let acceptance: Signed<evidence::Acceptance> = evidence::decode_signed_publication(
            &select(acceptance_id)?.body,
            "human_acceptance",
            &self.key,
        )?;
        if parent.digest()? != self.published.signed.digest()?
            || acceptance.digest()? != self.acceptance.digest()
            || identity.account_id != self.acceptance.acceptance().provider_writer
        {
            return Err(Error::new(
                "evidence_unpublished",
                "original exact human-reviewed evidence changed",
            ));
        }
        super::carried_acceptance::require_original_acceptance_selection(
            &comments,
            &self.signed_acceptance,
            acceptance_id,
            &self.key,
        )?;
        self.validate(config)
    }
    pub(crate) fn validate(&self, config: &Config) -> Result<()> {
        if config.revision()? != self.configuration_revision {
            return Err(Error::new(
                "stale_config",
                "parent reassembly configuration changed",
            ));
        }
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == self.workspace)
            .ok_or_else(|| blocked("parent reassembly workspace disappeared"))?;
        if !workspace
            .actors
            .acceptors
            .iter()
            .any(|id| id.to_string() == self.acceptance.acceptance().linear_actor)
        {
            return Err(Error::new(
                "access_denied",
                "original human acceptor is no longer permitted",
            ));
        }
        self.references.reference().validate_retained_workspace(
            &self.records,
            &self.control,
            &self.key,
            AcceptedOriginIdentity {
                runtime: self.acceptance.acceptance().runtime_id,
                workspace: self.workspace,
                work: &self.parent().manifest().work,
                token: self.parent().token(),
                acceptance_digest: self.acceptance.digest(),
            },
        )
    }
}

/// Retains the actual assembly result and its immutable closed source receipt.
/// This source does not authorize a model session until a fresh per-phase proof
/// below binds the actual joined predecessor and complete prior phase inventory.
#[derive(Clone)]
pub(crate) struct ParentReassemblySource {
    original: Arc<AcceptedParentOrigin>,
    previous_parent: PublishedParent,
    transition: Arc<VerifiedBaselineTransition>,
    vector: ParentCandidateVector,
    source: ReviewReceiptReference,
    parent_authority: FreshGate,
    records: Arc<PrivateDir>,
    key: Arc<AttestationKey>,
    observer: Arc<ProductionObserver>,
    contracts: Vec<RepositoryContract>,
    current_baselines: Arc<std::sync::Mutex<Vec<VerifiedProductionBaseline>>>,
    digest: String,
}
impl ParentReassemblySource {
    pub(super) fn source_reference(&self) -> &ReviewReceiptReference {
        &self.source
    }
    pub(super) fn retained_receipts(&self) -> Result<Vec<Receipt>> {
        self.original.control.operations()
    }
    pub(crate) fn original(&self) -> &AcceptedParentOrigin {
        &self.original
    }
    pub(super) fn previous_parent(&self) -> &PublishedParent {
        &self.previous_parent
    }
    pub(crate) fn vector(&self) -> &ParentCandidateVector {
        &self.vector
    }
    pub(crate) fn transition(&self) -> &VerifiedBaselineTransition {
        &self.transition
    }
    pub(crate) fn validate(&self, config: &Config) -> Result<()> {
        self.original.validate(config)?;
        self.transition.revalidate(config)?;
        let previous: Signed<ParentResultRecord> = self.records.read(&format!(
            "parent-result-{}.json",
            &self.previous_parent.verified().token()[7..],
        ))?;
        self.key.verify("parent_result", &previous)?;
        if previous.digest()? != self.previous_parent.signed.digest()? {
            return Err(blocked("latest preceding parent publication changed"));
        }
        let leaf = format!("parent-vector-{}.json", &self.vector.digest()[7..]);
        let current: Signed<VectorRecord> = self.records.read(&leaf)?;
        self.key.verify("parent_candidate_vector", &current)?;
        if current.digest()? != self.vector.signed.digest()? {
            return Err(blocked("retained reassembly vector disappeared or changed"));
        }
        Ok(())
    }
    pub(crate) async fn refresh_current(&self, config: &Config) -> Result<()> {
        self.validate(config)?;
        self.original.reobserve_native(config).await?;
        let mut observed = Vec::new();
        for contract in &self.contracts {
            observed.push(self.observer.observe(contract).await?);
        }
        self.transition.verify_current(config, &observed)?;
        *self
            .current_baselines
            .lock()
            .map_err(|_| blocked("parent baseline observation lock failed"))? = observed;
        Ok(())
    }
    pub(super) fn validate_current(&self, config: &Config) -> Result<()> {
        let observed = self
            .current_baselines
            .lock()
            .map_err(|_| blocked("parent baseline observation lock failed"))?;
        self.transition.verify_current(config, &observed)
    }
    pub(crate) fn verify_authority(&self, current: &FreshGate) -> Result<()> {
        self.parent_authority
            .compare_current(current, Utc::now(), chrono::Duration::seconds(30))
    }
}

/// One actual next attempt of the existing finite parent phase. The assignment
/// owns this proof only after its ordinary payload digest has been sealed.
#[derive(Clone)]
pub(crate) struct ParentReassemblyWork {
    source: Arc<ParentReassemblySource>,
    assignment: DeliveryWorkerAssignment,
    predecessor: Arc<RunCompletion>,
    joined: Vec<Arc<RunCompletion>>,
    grants: Vec<VerifiedRetryGrant>,
    reference: ParentReassemblyReference,
}
impl ParentReassemblyWork {
    pub(crate) async fn refresh_current(&self, config: &Config) -> Result<()> {
        self.source.refresh_current(config).await
    }
    pub(crate) fn reference(&self) -> &ParentReassemblyReference {
        &self.reference
    }
    pub(crate) fn authority(&self) -> &FreshGate {
        self.assignment.authority()
    }
    pub(crate) fn parent_authority(&self) -> &FreshGate {
        &self.source.parent_authority
    }
    pub(crate) fn phase(&self) -> &PhaseKey {
        &self.reference.phase
    }
    pub(crate) fn verify_joined(
        &self,
        retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        for completion in &self.joined {
            if retained(completion.operation_id)?
                .as_ref()
                .is_none_or(|actual| !Arc::ptr_eq(actual, completion))
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "parent reassembly lost actual joined ancestry",
                ));
            }
            terminal(completion)?;
        }
        self.verify_predecessor()
    }
    fn verify_predecessor(&self) -> Result<()> {
        if self.reference.transient_of.is_some() {
            transient_failure(&self.predecessor)
        } else if self.reference.retry_of.is_some() {
            failed(&self.predecessor)
        } else {
            completed(&self.predecessor)
        }
    }
    pub(crate) fn verify_grants(
        &self,
        receipts: &[Receipt],
        current: &[VerifiedRetryGrant],
    ) -> Result<()> {
        let (grants, references) = grant_lineage::capture(
            self.phase(),
            self.authority(),
            self.reference.runtime,
            receipts,
            current,
        )?;
        if references != self.reference.grants
            || grants.len() != self.grants.len()
            || grants
                .iter()
                .zip(&self.grants)
                .any(|(a, b)| a.digest() != b.digest())
        {
            return Err(blocked("parent native grant lineage changed"));
        }
        Ok(())
    }
    pub(crate) fn retain_history(
        &self,
        refs: &mut super::carry_history::CarryReferences,
        receipts: &[Receipt],
    ) -> Result<()> {
        for operation in self.reference.history.keys().copied().chain(
            self.reference
                .grants
                .iter()
                .flat_map(|g| [g.publication.operation, g.command]),
        ) {
            let receipt = receipts
                .iter()
                .find(|r| r.operation_id == operation)
                .ok_or_else(|| blocked("parent ancestry receipt disappeared"))?;
            if self.reference.grants.iter().any(|g| g.command == operation)
                && !matches!(
                    receipt.phase,
                    crate::control::ReceiptPhase::Acknowledged
                        | crate::control::ReceiptPhase::Reconciled
                )
            {
                return Err(Error::new(
                    "operation_pending",
                    "parent grant command has not finalized for immutable carry history",
                ));
            }
            refs.retain_receipt(receipt)?;
        }
        Ok(())
    }

    pub(crate) fn verify_assignment(&self, assignment: &DeliveryWorkerAssignment) -> Result<()> {
        if assignment.base_digest()? != self.reference.assignment_digest {
            return Err(blocked(
                "parent reassembly assignment changed after sealing",
            ));
        }
        for source in assignment
            .source()
            .into_iter()
            .chain(assignment.read_sources())
        {
            source.revalidate_filesystem()?;
        }
        Ok(())
    }
    pub(crate) fn verify_contract(&self, contract: &WorkerContract) -> Result<()> {
        let WorkerAuthority::Authorized(gate) = contract.authority() else {
            return Err(blocked(
                "parent reassembly needs authorized worker authority",
            ));
        };
        self.authority().compare_admitted_authority(gate)?;
        let manifest = &contract.instructions().manifest;
        if manifest.operation_id != self.reference.operation
            || manifest.role != self.assignment.role()
            || manifest.repository_revision.as_deref()
                != self
                    .assignment
                    .source()
                    .map(|s| s.revision().commit.as_str())
        {
            return Err(blocked(
                "actual parent contract differs from sealed role/source/vector",
            ));
        }
        super::super::assignment::verify_assignment_contract(
            contract,
            self.authority(),
            self.assignment.phase(),
            self.assignment.context(),
        )
    }
    pub(crate) fn verify_current(
        &self,
        config: &Config,
        snapshot: &crate::control::ControlSnapshot,
        receipts: &[Receipt],
        grants: &[VerifiedRetryGrant],
    ) -> Result<()> {
        self.source.validate(config)?;
        self.source.validate_current(config)?;
        self.verify_grants(receipts, grants)?;
        crate::control::verify_parent_reassembly_reference(snapshot, &self.reference, receipts)?;
        self.verify_assignment(&self.assignment)?;
        let receipt = receipts
            .iter()
            .find(|r| r.operation_id == self.predecessor.operation_id)
            .ok_or_else(|| blocked("actual parent predecessor receipt disappeared"))?;
        if receipt
            .observed_result
            .as_ref()
            .and_then(|v| v.get("observation"))
            != Some(&self.predecessor.observation())
        {
            return Err(blocked(
                "retained parent completion differs from actual protected terminal observation",
            ));
        }
        self.verify_predecessor()
    }
    pub(crate) fn verify_result(
        &self,
        completion: &Arc<RunCompletion>,
        receipts: &[Receipt],
    ) -> Result<()> {
        completed(completion)?;
        self.verify_terminal_result(completion, receipts)
    }
    fn verify_terminal_result(
        &self,
        completion: &Arc<RunCompletion>,
        receipts: &[Receipt],
    ) -> Result<()> {
        terminal(completion)?;
        if completion.operation_id != self.reference.operation {
            return Err(blocked("parent result belongs to another readmission"));
        }
        self.verify_contract(
            completion
                .contract
                .as_deref()
                .ok_or_else(|| blocked("completed parent contract absent"))?,
        )?;
        let receipt = receipts
            .iter()
            .find(|r| r.operation_id == completion.operation_id)
            .ok_or_else(|| blocked("parent result admission absent"))?;
        if receipt.phase != crate::control::ReceiptPhase::Reconciled
            || receipt.observed_result.as_ref().is_none_or(|result| {
                result["local_execution"] != "exited"
                    || result["unresolved_effects"]
                        .as_array()
                        .is_none_or(|effects| !effects.is_empty())
            })
            || ParentReassemblyReference::from_receipt(receipt)?.as_ref() != Some(&self.reference)
            || receipt
                .observed_result
                .as_ref()
                .and_then(|v| v.get("observation"))
                != Some(&completion.observation())
        {
            return Err(blocked(
                "parent result lost exact causal admission or terminal observation",
            ));
        }
        let encoded = receipt
            .expected
            .get("parent_reassembly_preparation")
            .ok_or_else(|| blocked("parent result has no actual preparation link"))?;
        let link: ReviewReceiptReference = serde_json::from_str(encoded)
            .map_err(|_| blocked("parent result preparation link is invalid"))?;
        let preparation = receipts
            .iter()
            .find(|r| r.operation_id == link.operation)
            .ok_or_else(|| blocked("parent result preparation disappeared"))?;
        crate::control::require_successful_parent_preparation(preparation)?;
        if canonical_digest(1, preparation)? != link.receipt_digest
            || preparation.phase != crate::control::ReceiptPhase::Reconciled
            || preparation.owner_generation != receipt.owner_generation
            || preparation.actor != receipt.actor
            || ParentReassemblyReference::from_receipt(preparation)?.as_ref()
                != Some(&self.reference)
            || preparation.observed_result.as_ref().is_none_or(|result| {
                result["local_execution"] != "exited"
                    || result["unresolved_effects"]
                        .as_array()
                        .is_none_or(|effects| !effects.is_empty())
            })
        {
            return Err(blocked("parent result lost its exact closed preparation"));
        }
        let prior = receipts
            .iter()
            .filter(|r| r.operation_id != completion.operation_id)
            .cloned()
            .collect::<Vec<_>>();
        if crate::control::parent_reassembly_inventory(&prior, &self.reference)?
            != self.reference.inventory_digest
        {
            return Err(blocked(
                "parent result is superseded or its complete admission inventory changed",
            ));
        }
        for (operation, digest) in &self.reference.history {
            let matches = receipts
                .iter()
                .filter(|r| r.operation_id == *operation)
                .collect::<Vec<_>>();
            let [actual] = matches.as_slice() else {
                return Err(blocked("parent result ancestry missing or duplicated"));
            };
            if canonical_digest(1, *actual)? != *digest {
                return Err(blocked("parent result ancestry changed"));
            }
        }
        Ok(())
    }
}
fn terminal(completion: &RunCompletion) -> Result<()> {
    match &completion.outcome {
        RunOutcome::Completed => completed(completion),
        RunOutcome::Failed if completion.state.terminal.is_none() => transient_failure(completion),
        RunOutcome::Failed => failed(completion),
        _ => Err(Error::new(
            "reconciliation_required",
            "parent ancestry has unknown or interrupted exit",
        )),
    }
}
fn failed(completion: &RunCompletion) -> Result<()> {
    if !matches!(completion.outcome, RunOutcome::Failed)
        || !completion.local_exit_confirmed
        || completion.hook_error.is_some()
        || completion.state.terminal.as_deref() != Some("failed")
        || completion.state.turn_id.is_some()
        || completion.state.unresolved_turn.is_some()
        || completion.state.input_required
        || completion.state.intervention.is_some()
    {
        return Err(Error::new(
            "reconciliation_required",
            "parent retry requires actual known terminal failed local exit",
        ));
    }
    Ok(())
}
fn transient_failure(completion: &RunCompletion) -> Result<()> {
    if !matches!(completion.outcome, RunOutcome::Failed)
        || !completion.local_exit_confirmed
        || completion.hook_error.is_some()
        || completion.state.turn_id.is_some()
        || completion.state.unresolved_turn.is_some()
        || completion.state.input_required
        || completion.state.intervention.is_some()
    {
        return Err(Error::new(
            "reconciliation_required",
            "parent transient retry requires known closed local/provider ownership",
        ));
    }
    let error = completion
        .error
        .as_ref()
        .ok_or_else(|| blocked("failed parent has no actual classified error"))?;
    match error.code.as_str() {
        "process_exit" | "provider_transport" | "timeout" | "stream_timeout" | "stall_timeout"
            if completion
                .state
                .terminal
                .as_deref()
                .is_none_or(|s| s == "failed") =>
        {
            Ok(())
        }
        "turn_incomplete" if completion.state.terminal_transient_failure()?.is_some() => Ok(()),
        _ => Err(Error::new(
            "persistent_blocker",
            "parent failure requires explicit resolution before automatic retry",
        )),
    }
}

fn completed(completion: &RunCompletion) -> Result<()> {
    if !matches!(completion.outcome, RunOutcome::Completed)
        || !completion.local_exit_confirmed
        || completion.error.is_some()
        || completion.hook_error.is_some()
        || completion.state.terminal.as_deref() != Some("completed")
        || completion.state.turn_id.is_some()
        || completion.state.unresolved_turn.is_some()
        || completion.state.input_required
        || completion.state.intervention.is_some()
    {
        return Err(Error::new(
            "reconciliation_required",
            "parent predecessor needs actual known completed local exit",
        ));
    }
    Ok(())
}

/// The original human-reviewed subject and the exact released-baseline change
/// consumed by one actually closed parent source assembly.
pub(crate) struct ParentReassemblySourceOrigin<'a> {
    pub original: Arc<AcceptedParentOrigin>,
    pub previous_parent: &'a PublishedParent,
    pub transition: Arc<VerifiedBaselineTransition>,
}

/// Actual prior run plus its optional already-sealed reassembly ancestry.
pub(crate) struct ParentReassemblyPredecessor<'a> {
    pub completion: Arc<RunCompletion>,
    pub previous: Option<&'a ParentReassemblyWork>,
}

impl DeliveryCoordinator {
    /// Original human evidence remains original. This reads its exact native
    /// publications and current acceptor policy; it never labels a newer result
    /// human reviewed or reconstructs a completed worker.
    pub(crate) async fn capture_parent_acceptance(
        &self,
        contract: &ParentContract,
        published: &PublishedParent,
        config: &Config,
    ) -> Result<AcceptedParentOrigin> {
        let gate = &contract.gate;
        (self.current_gate)(gate)?;
        if config.revision()? != gate.config_revision()
            || gate.work() != &published.verified().manifest().work
        {
            return Err(blocked(
                "original parent belongs to another current authority",
            ));
        }
        self.key.verify("parent_result", &published.signed)?;
        if published.signed.payload.evidence != *published.verified().manifest()
            || published.signed.payload.contract_digest != contract.digest()?
        {
            return Err(blocked(
                "original parent capability differs from its signed publication",
            ));
        }
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == gate.workspace_id())
            .ok_or_else(|| blocked("original workspace absent"))?;
        let permitted = workspace
            .actors
            .acceptors
            .iter()
            .map(|id| id.to_string())
            .collect();
        let linear = self
            .broker
            .linear
            .as_ref()
            .ok_or_else(|| blocked("original Linear connection absent"))?;
        let identity = linear.identity().await?;
        let comments = linear.comments(gate.work().parent).await?;
        let mut selected = Vec::new();
        for comment in &comments {
            if comment.issue_id != gate.work().parent
                || comment.bot
                || comment.delegated
                || comment.user_id.map(|id| id.to_string()).as_deref()
                    != Some(identity.account_id.as_str())
                || !comment
                    .body
                    .starts_with("<!-- kegari:v0:human_acceptance -->\n")
            {
                continue;
            }
            let Ok(signed) = evidence::decode_signed_publication::<evidence::Acceptance>(
                &comment.body,
                "human_acceptance",
                &self.key,
            ) else {
                continue;
            };
            let publication = Publication {
                provider: "linear".into(),
                record_id: comment.id.to_string(),
                writer_account_id: identity.account_id.clone(),
                payload_digest: canonical_digest(1, &signed)?,
                observed_at: Utc::now(),
                url: comment.url.clone(),
            };
            if let Ok(acceptance) = evidence::verify_acceptance(
                &signed,
                &publication,
                published.verified(),
                gate.work(),
                false,
                &permitted,
                &self.key,
            ) {
                selected.push((comment.id, acceptance, signed));
            }
        }
        let [(acceptance_id, acceptance, signed_acceptance)] = selected.as_slice() else {
            return Err(Error::new(
                "acceptance_required",
                "carry needs one exact current original human acceptance",
            ));
        };
        let parent_id = published
            .publication()
            .record_id
            .parse()
            .map_err(|_| blocked("original parent publication ID is not canonical"))?;
        let references = self
            .accepted_carry_origin_references(
                gate,
                published.verified(),
                acceptance,
                parent_id,
                *acceptance_id,
            )
            .await?;
        let digest = canonical_digest(
            1,
            &json!({"origin":references.reference(),"original_parent":published.publication_digest()?,"acceptance":acceptance.digest(),"configuration":config.revision()?}),
        )?;
        let result = AcceptedParentOrigin {
            published: published.clone(),
            acceptance: acceptance.clone(),
            signed_acceptance: signed_acceptance.clone(),
            references,
            records: self.records.clone(),
            control: self.host.clone(),
            key: self.key.clone(),
            linear: linear.clone(),
            workspace: gate.workspace_id(),
            configuration_revision: config.revision()?,
            digest,
        };
        result.validate(config)?;
        (self.current_gate)(gate)?;
        Ok(result)
    }
    pub(crate) fn seal_parent_reassembly_source(
        &self,
        contract: &ParentContract,
        admission: &DeliveryAdmission,
        origin: ParentReassemblySourceOrigin<'_>,
        vector: &ParentCandidateVector,
        config: &Config,
        observer: Arc<ProductionObserver>,
    ) -> Result<ParentReassemblySource> {
        let ParentReassemblySourceOrigin {
            original,
            previous_parent,
            transition,
        } = origin;
        (self.current_gate)(&contract.gate)?;
        original.validate(config)?;
        transition.revalidate(config)?;
        admission.gate.compare_admitted_authority(&contract.gate)?;
        self.key.verify("parent_candidate_vector", &vector.signed)?;
        if vector.signed.payload.work != *contract.work()
            || vector.signed.payload.contract_digest != contract.digest()?
            || vector.baselines() != transition.baselines()
            || transition.original_token() != original.parent().token()
            || transition.work() != contract.work()
            || original.parent().manifest().work != *contract.work()
            || previous_parent.verified().manifest().work != *contract.work()
        {
            return Err(blocked(
                "new parent source differs from original exact work or released baseline",
            ));
        }
        let old = &original.parent().manifest().candidates;
        if old.len() != vector.candidates().len()
            || old.iter().zip(vector.candidates()).any(|(a, b)| {
                a.repository != b.repository
                    || a.ordered_delta_digests != b.ordered_delta_digests
                    || a.ordered_patch_digests != b.ordered_patch_digests
            })
        {
            return Err(Error::new(
                "acceptance_required",
                "reassembled parent changed exact ordered reviewed patches",
            ));
        }
        let receipt = self.host.operation(admission.operation_id())?;
        if receipt.expected.get("scope") != Some(&admission.scope.digest()?)
            || receipt.phase != crate::control::ReceiptPhase::Reconciled
            || receipt.observed_result.as_ref().is_none_or(|v| {
                v["local_execution"] != "exited"
                    || v["unresolved_effects"]
                        .as_array()
                        .is_none_or(|x| !x.is_empty())
                    || v["observation"]["parent_reassembly_vector"] != vector.digest()
                    || v["observation"]["parent_reassembly_baseline"] != transition.digest()
                    || v["observation"]["original_reviewed_token"] != original.parent().token()
            })
        {
            return Err(blocked(
                "new parent vector has no exact closed actual assembly admission",
            ));
        }
        let source = ReviewReceiptReference {
            operation: receipt.operation_id,
            receipt_digest: canonical_digest(1, &receipt)?,
        };
        let digest = canonical_digest(
            1,
            &json!({"original":original.digest(),"previous_parent":previous_parent.publication_digest()?,"transition":transition.digest(),"vector":vector.digest(),"source":source}),
        )?;
        Ok(ParentReassemblySource {
            original,
            previous_parent: previous_parent.clone(),
            transition,
            vector: vector.clone(),
            source,
            parent_authority: contract.gate.clone(),
            records: self.records.clone(),
            key: self.key.clone(),
            observer,
            contracts: contract
                .repositories()
                .iter()
                .map(|id| {
                    self.contracts
                        .get(id)
                        .cloned()
                        .ok_or_else(|| blocked("parent source contract disappeared"))
                })
                .collect::<Result<_>>()?,
            current_baselines: Arc::new(std::sync::Mutex::new(Vec::new())),
            digest,
        })
    }
    pub(crate) fn parent_reassembly_assignment(
        &self,
        operation: Uuid,
        source: Arc<ParentReassemblySource>,
        assignment: DeliveryWorkerAssignment,
        predecessor: ParentReassemblyPredecessor<'_>,
        config: &Config,
        grants: &[VerifiedRetryGrant],
    ) -> Result<DeliveryWorkerAssignment> {
        let ParentReassemblyPredecessor {
            completion: predecessor,
            previous,
        } = predecessor;
        (self.current_gate)(&source.parent_authority)?;
        (self.current_gate)(assignment.authority())?;
        source.validate(config)?;
        completed(&predecessor)?;
        let receipts = self.host.operations()?;
        let receipt = receipts
            .iter()
            .find(|r| r.operation_id == predecessor.operation_id)
            .ok_or_else(|| blocked("parent predecessor admission missing"))?;
        let prior = RunReference::from_receipt(receipt)?
            .ok_or_else(|| blocked("parent predecessor has no actual worker run"))?;
        let original_contract = predecessor
            .contract
            .as_deref()
            .ok_or_else(|| blocked("parent predecessor has no actual contract"))?;
        let WorkerAuthority::Authorized(old_gate) = original_contract.authority() else {
            return Err(blocked("parent predecessor was not authorized"));
        };
        old_gate.compare_admitted_authority(assignment.authority())?;
        let phase = prior.phase.clone();
        let (retained_grants, grant_references) = grant_lineage::capture(
            &phase,
            assignment.authority(),
            receipt.runtime_id,
            &receipts,
            grants,
        )?;
        if phase.work != assignment.authority().work_key()?
            || phase.generation != canonical_digest(1, &assignment.authority().work().generation)?
            || phase.phase != assignment.phase()
            || prior.instruction_digest != original_contract.instructions().manifest_digest
            || prior.account != original_contract.account_id()
            || original_contract.instructions().manifest.operation_id != predecessor.operation_id
            || original_contract.instructions().manifest.phase != assignment.phase()
            || original_contract.instructions().manifest.role != assignment.role()
            || assignment.candidate_vector_digest() != Some(source.vector.digest())
            || assignment.parent_reassembly().is_some()
        {
            return Err(blocked(
                "parent reassembly changed original phase, role, or candidate vector",
            ));
        }
        let [parent, acceptance] = source
            .original
            .references
            .reference()
            .publication_receipts();
        let mut joined = if let Some(previous) = previous {
            previous.verify_result(&predecessor, &receipts)?;
            previous.joined.clone()
        } else {
            if ParentReassemblyReference::from_receipt(receipt)?.is_some() {
                return Err(blocked(
                    "marked prior parent requires its actual retained assignment",
                ));
            }
            Vec::new()
        };
        joined.push(predecessor.clone());
        let mut reference = ParentReassemblyReference {
            grants: grant_references,
            retry_of: None,
            transient_of: None,
            history: grant_lineage::history(&receipts, &phase)?,
            schema_version: 1,
            runtime: receipt.runtime_id,
            workspace: prior.workspace,
            account: prior.account.clone(),
            operation,
            parent_work: source.parent_authority.work().clone(),
            worker_work: assignment.authority().work().clone(),
            phase,
            predecessor: ReviewReceiptReference {
                operation: receipt.operation_id,
                receipt_digest: canonical_digest(1, receipt)?,
            },
            source: source.source.clone(),
            original_parent: parent,
            original_acceptance: acceptance,
            original_token: source.original.parent().token().into(),
            vector_digest: source.vector.digest().into(),
            baseline_digest: source.transition.digest().into(),
            assignment_digest: assignment.base_digest()?,
            progress_digest: source.digest.clone(),
            inventory_digest: canonical_digest(1, &"pending bounded inventory")?,
            attempt: prior
                .attempt
                .checked_add(1)
                .ok_or_else(|| Error::new("attempts_exhausted", "parent attempt overflow"))?,
            max_attempts: config
                .runtime
                .max_attempts_per_phase
                .min(prior.max_attempts),
            max_turns: config.runtime.max_turns.min(prior.max_turns),
            max_review_rounds: config
                .runtime
                .max_review_rounds
                .min(prior.max_review_rounds),
        };
        for r in &receipts {
            if let Some(run) = RunReference::from_receipt(r)?
                && run.phase == reference.phase
            {
                reference.max_attempts = reference.max_attempts.min(run.max_attempts);
                reference.max_turns = reference.max_turns.min(run.max_turns);
                reference.max_review_rounds =
                    reference.max_review_rounds.min(run.max_review_rounds);
            }
        }
        if let Some(grant) = reference.grants.last() {
            reference.max_attempts = reference.max_attempts.min(grant.max_attempts);
            reference.max_turns = reference.max_turns.min(grant.max_turns);
            reference.max_review_rounds = reference.max_review_rounds.min(grant.max_review_rounds);
        }
        if reference.attempt > reference.max_attempts {
            return Err(Error::new(
                "attempts_exhausted",
                "parent reassembly exhausted the original finite phase budget",
            ));
        }
        reference.inventory_digest =
            crate::control::parent_reassembly_inventory(&receipts, &reference)?;
        let proof = Arc::new(ParentReassemblyWork {
            source,
            assignment: assignment.clone(),
            predecessor,
            joined,
            grants: retained_grants,
            reference,
        });
        proof.verify_current(config, &self.host.snapshot()?, &receipts, grants)?;
        assignment.with_parent_reassembly(proof)
    }
    /// An explicit native grant retries the exact failed marked assignment.
    /// The new phase budget changes; its original accepted source and ancestry do not.
    pub(crate) fn retry_parent_reassembly(
        &self,
        operation: Uuid,
        previous: &ParentReassemblyWork,
        failed_completion: Arc<RunCompletion>,
        grant: &VerifiedRetryGrant,
        config: &Config,
        current_grants: &[VerifiedRetryGrant],
    ) -> Result<DeliveryWorkerAssignment> {
        (self.current_gate)(&previous.source.parent_authority)?;
        (self.current_gate)(previous.authority())?;
        previous.source.validate(config)?;
        previous.source.validate_current(config)?;
        failed(&failed_completion)?;
        let receipts = self.host.operations()?;
        previous.verify_terminal_result(&failed_completion, &receipts)?;
        if grant.phase() != previous.phase() {
            return Err(blocked("parent grant replaces another phase budget"));
        }
        let phase = grant.phase_key();
        let (grants, references) = grant_lineage::capture(
            &phase,
            previous.authority(),
            previous.reference.runtime,
            &receipts,
            current_grants,
        )?;
        let predecessor = receipts
            .iter()
            .find(|r| r.operation_id == failed_completion.operation_id)
            .ok_or_else(|| blocked("failed parent admission missing"))?;
        let mut reference = previous.reference.clone();
        reference.operation = operation;
        reference.phase = phase.clone();
        reference.predecessor = ReviewReceiptReference {
            operation: predecessor.operation_id,
            receipt_digest: canonical_digest(1, predecessor)?,
        };
        reference.retry_of = Some(previous.reference.digest()?);
        reference.transient_of = None;
        reference.grants = references;
        reference.history = grant_lineage::history(&receipts, &phase)?;
        reference.attempt = 1;
        reference.max_attempts = config
            .runtime
            .max_attempts_per_phase
            .min(grant.new_budget().max_attempts);
        reference.max_turns = config.runtime.max_turns.min(grant.new_budget().max_turns);
        reference.max_review_rounds = config
            .runtime
            .max_review_rounds
            .min(grant.new_budget().max_review_rounds);
        reference.inventory_digest =
            crate::control::parent_reassembly_inventory(&receipts, &reference)?;
        let mut joined = previous.joined.clone();
        joined.push(failed_completion.clone());
        let proof = Arc::new(ParentReassemblyWork {
            source: previous.source.clone(),
            assignment: previous.assignment.clone(),
            predecessor: failed_completion,
            joined,
            grants,
            reference,
        });
        proof.verify_current(config, &self.host.snapshot()?, &receipts, current_grants)?;
        proof.assignment.clone().with_parent_reassembly(proof)
    }
    pub(crate) fn continue_parent_reassembly(
        &self,
        operation: Uuid,
        previous: &ParentReassemblyWork,
        failed_completion: Arc<RunCompletion>,
        config: &Config,
        current_grants: &[VerifiedRetryGrant],
    ) -> Result<DeliveryWorkerAssignment> {
        if previous.phase().retry_grant.is_none() {
            return Err(blocked(
                "automatic parent continuation requires an actual existing grant",
            ));
        }
        (self.current_gate)(&previous.source.parent_authority)?;
        (self.current_gate)(previous.authority())?;
        previous.source.validate(config)?;
        previous.source.validate_current(config)?;
        transient_failure(&failed_completion)?;
        let receipts = self.host.operations()?;
        previous.verify_terminal_result(&failed_completion, &receipts)?;
        previous.verify_grants(&receipts, current_grants)?;
        let predecessor = receipts
            .iter()
            .find(|r| r.operation_id == failed_completion.operation_id)
            .ok_or_else(|| blocked("parent transient predecessor disappeared"))?;
        let mut reference = previous.reference.clone();
        reference.operation = operation;
        reference.predecessor = ReviewReceiptReference {
            operation: predecessor.operation_id,
            receipt_digest: canonical_digest(1, predecessor)?,
        };
        reference.retry_of = None;
        reference.transient_of = Some(previous.reference.digest()?);
        reference.attempt = reference
            .attempt
            .checked_add(1)
            .ok_or_else(|| Error::new("attempts_exhausted", "parent attempt overflow"))?;
        reference.max_attempts = reference
            .max_attempts
            .min(config.runtime.max_attempts_per_phase);
        reference.max_turns = reference.max_turns.min(config.runtime.max_turns);
        reference.max_review_rounds = reference
            .max_review_rounds
            .min(config.runtime.max_review_rounds);
        if reference.attempt > reference.max_attempts {
            return Err(Error::new(
                "attempts_exhausted",
                "parent transient retry exhausted its existing granted budget",
            ));
        }
        reference.history = grant_lineage::history(&receipts, &reference.phase)?;
        reference.inventory_digest =
            crate::control::parent_reassembly_inventory(&receipts, &reference)?;
        let mut joined = previous.joined.clone();
        joined.push(failed_completion.clone());
        let proof = Arc::new(ParentReassemblyWork {
            source: previous.source.clone(),
            assignment: previous.assignment.clone(),
            predecessor: failed_completion,
            joined,
            grants: previous.grants.clone(),
            reference,
        });
        proof.verify_current(config, &self.host.snapshot()?, &receipts, current_grants)?;
        proof.assignment.clone().with_parent_reassembly(proof)
    }
}

#[cfg(test)]
#[path = "delivery_parent_grant_tests.rs"]
mod grant_tests;
