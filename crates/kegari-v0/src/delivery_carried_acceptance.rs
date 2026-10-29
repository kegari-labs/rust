//! Current acceptance carry is issued only from actual reassembly, new complete
//! parent evidence, and full production gates. Historical records below never
//! construct a current VerifiedCarry or relabel the human-reviewed token.
use super::accepted_origin::{AcceptedOriginIdentity, AcceptedOriginRef};
use super::*;
use crate::control::{ReceiptPhase, ReviewReceiptReference};
use crate::evidence::{CarriedAcceptance, VerifiedAcceptance, VerifiedCarry};
use crate::runtime_delivery::ActiveParentGrantObserver;
use crate::worker_run::RunCompletion;
use kegari_config::Config;

type ParentGrantObserver = Arc<
    dyn Fn(&crate::evidence::WorkIdentity) -> Result<Vec<crate::decisions::VerifiedRetryGrant>>
        + Send
        + Sync,
>;

const PURPOSE: &str = "carried_acceptance";
const SAVED_PURPOSE: &str = "carried_acceptance_publication";
fn invalid(message: &str) -> Error {
    Error::new("acceptance_carry", message)
}
fn leaf(token: &str) -> Result<String> {
    evidence::validate_digest(token)?;
    Ok(format!("carried-acceptance-{}.json", &token[7..]))
}

#[derive(Clone)]
pub(crate) struct ReassembledParent {
    source: Arc<ParentReassemblySource>,
    parent: PublishedParent,
    references: CarryReferences,
    phases: Vec<ParentReassemblyWork>,
    results: Vec<Arc<RunCompletion>>,
    current_grants: ParentGrantObserver,
}
impl std::fmt::Debug for ReassembledParent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReassembledParent")
            .field("token", &self.parent.verified().token())
            .finish()
    }
}
impl ReassembledParent {
    pub(crate) fn source(&self) -> &Arc<ParentReassemblySource> {
        &self.source
    }
    pub(crate) fn validate(&self, config: &Config) -> Result<()> {
        self.validate_with_grants(config, &|authority| (self.current_grants)(authority.work()))
    }
    fn validate_with_grants(
        &self,
        config: &Config,
        current_grants: &dyn Fn(&FreshGate) -> Result<Vec<crate::decisions::VerifiedRetryGrant>>,
    ) -> Result<()> {
        self.source.validate(config)?;
        let receipts = self.source.retained_receipts()?;
        for (proof, result) in self.phases.iter().zip(&self.results) {
            proof.verify_result(result, &receipts)?;
            // Empty retained grant ancestry still checks for a newly superseding
            // native grant; it is never a reason to skip current observation.
            proof.verify_grants(&receipts, &current_grants(proof.authority())?)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CarryRecord {
    schema_version: u32,
    runtime: Uuid,
    workspace: Uuid,
    publication_operation: Uuid,
    publication_scope: DeliveryScope,
    original_parent: Signed<ParentResultRecord>,
    original_acceptance: Signed<evidence::Acceptance>,
    original: AcceptedOriginRef,
    current_parent: Signed<ParentResultRecord>,
    current_publication: Publication,
    source: ReviewReceiptReference,
    baseline_digest: String,
    carry: CarriedAcceptance,
    references: CarryReferences,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery: Option<recovery::CarryRecoveryBinding>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CarryPublicationRecord {
    signed: Signed<CarryRecord>,
    publication: Publication,
    receipt: ReviewReceiptReference,
}

#[derive(Clone)]
enum CarryBasis {
    Live(Box<ReassembledParent>),
    Reobserved(Box<recovery::ReobservedCarry>),
}
/// The receipt/native publication is part of this capability. A merely computed
/// semantic carry is never sufficient for runtime readiness.
#[derive(Clone)]
pub(crate) struct PublishedCarry {
    basis: CarryBasis,
    verified: VerifiedCarry,
    saved: Signed<CarryPublicationRecord>,
    records: Arc<PrivateDir>,
    root: Arc<PrivateDir>,
    host: Arc<ControlHost>,
    key: Arc<AttestationKey>,
}
impl std::fmt::Debug for PublishedCarry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublishedCarry")
            .field("record", self.verified.record())
            .finish()
    }
}
impl PublishedCarry {
    pub(crate) fn workspace_id(&self) -> Uuid {
        self.saved.payload.signed.payload.workspace
    }
    pub(crate) fn verified(&self) -> &VerifiedCarry {
        &self.verified
    }
    pub(crate) fn acceptance(&self) -> &VerifiedAcceptance {
        match &self.basis {
            CarryBasis::Live(value) => value.source.original().acceptance(),
            CarryBasis::Reobserved(value) => &value.acceptance,
        }
    }
    pub(crate) fn publication_reference(&self) -> Result<(Uuid, String, Uuid)> {
        Ok((
            self.saved
                .payload
                .publication
                .record_id
                .parse()
                .map_err(|_| invalid("carry comment identity is not canonical"))?,
            bytes_digest(
                evidence::encode_signed_publication(PURPOSE, &self.saved.payload.signed)?
                    .as_bytes(),
            ),
            self.saved
                .payload
                .publication
                .writer_account_id
                .parse()
                .map_err(|_| invalid("carry writer identity is not canonical"))?,
        ))
    }
    pub(crate) fn publication(&self) -> &Publication {
        &self.saved.payload.publication
    }
    pub(crate) fn parent(&self) -> &PublishedParent {
        match &self.basis {
            CarryBasis::Live(value) => &value.parent,
            CarryBasis::Reobserved(value) => &value.parent,
        }
    }
    pub(crate) fn validate(&self, config: &Config) -> Result<()> {
        match &self.basis {
            CarryBasis::Live(value) => value.validate(config)?,
            CarryBasis::Reobserved(value) => value.validate(config, &self.host, &|authority| {
                (value.current_grants)(authority.work())
            })?,
        }
        self.validate_publication()
    }
    fn validate_publication(&self) -> Result<()> {
        let saved: Signed<CarryPublicationRecord> = self.records.read(&leaf(
            self.verified.record().current_evidence_token.as_str(),
        )?)?;
        self.key.verify(SAVED_PURPOSE, &saved)?;
        if saved.digest()? != self.saved.digest()? {
            return Err(invalid("published carry record changed"));
        }
        validate_saved(&saved, &self.root, &self.records, &self.host, &self.key)?;
        closed(
            &self
                .host
                .operation(saved.payload.signed.payload.publication_operation)?,
        )?;
        Ok(())
    }
    pub(crate) fn verify_current_projection(
        &self,
        config: &Config,
        parent: &crate::reconcile::ParentProjection,
    ) -> Result<()> {
        self.validate(config)?;
        self.verify_native_projection(parent)
    }
    /// Only the actual owner-bound adapter selects retained active semantics.
    /// Its caller is already inside this same release permit's final gate.
    pub(crate) fn verify_active_projection(
        &self,
        active: &ActiveParentGrantObserver,
    ) -> Result<()> {
        match &self.basis {
            CarryBasis::Live(value) => value
                .validate_with_grants(active.configuration(), &|authority| {
                    active.observe_in_gate(authority)
                })?,
            CarryBasis::Reobserved(value) => {
                value.validate(active.configuration(), &self.host, &|authority| {
                    active.observe_in_gate(authority)
                })?
            }
        }
        self.validate_publication()?;
        self.verify_native_projection(&active.parent_in_gate()?)
    }
    fn verify_native_projection(&self, parent: &crate::reconcile::ParentProjection) -> Result<()> {
        let work = &self.parent().verified().manifest().work;
        if parent.work.as_ref() != Some(work)
            || parent.workspace_id != self.saved.payload.signed.payload.workspace
        {
            return Err(invalid(
                "carry differs from current exact parent generation",
            ));
        }
        let observation = parent
            .observations
            .get(&parent.key.issue)
            .ok_or_else(|| invalid("carry parent not currently observed"))?;
        self.verify_native_comments(&observation.comments)
    }
    fn verify_native_comments(&self, comments: &[kegari_providers::Comment]) -> Result<()> {
        verify_saved_native_comments(&self.saved, &self.key, comments)
    }
}

fn verify_saved_native_comments(
    saved: &Signed<CarryPublicationRecord>,
    key: &AttestationKey,
    comments: &[kegari_providers::Comment],
) -> Result<()> {
    let work = &saved
        .payload
        .signed
        .payload
        .current_parent
        .payload
        .evidence
        .work;
    let publications = [
        (
            &saved.payload.signed.payload.current_publication,
            evidence::encode_signed_publication(
                "parent_result",
                &saved.payload.signed.payload.current_parent,
            )?,
        ),
        (
            &saved.payload.publication,
            evidence::encode_signed_publication(PURPOSE, &saved.payload.signed)?,
        ),
    ];
    for (publication, body) in publications {
        if comments
            .iter()
            .filter(|c| {
                c.id.to_string() == publication.record_id
                    && c.issue_id == work.parent
                    && !c.bot
                    && !c.delegated
                    && c.user_id.map(|v| v.to_string()).as_deref()
                        == Some(publication.writer_account_id.as_str())
                    && c.body == body
                    && c.body_digest == bytes_digest(body.as_bytes())
            })
            .count()
            != 1
        {
            return Err(invalid(
                "current parent/carry native publication disappeared or changed",
            ));
        }
    }
    let selected_parent = comments
        .iter()
        .find(|comment| {
            comment.id.to_string() == saved.payload.signed.payload.current_publication.record_id
        })
        .ok_or_else(|| invalid("current parent publication disappeared"))?;
    DeliveryCoordinator::reject_superseding_parent_publication(
        key,
        saved.payload.signed.payload.runtime,
        comments,
        selected_parent,
    )?;
    let original = &saved.payload.signed.payload;
    let (old_parent, old_acceptance) = original.original.publication_ids();
    for (id, body) in [
        (
            old_parent,
            evidence::encode_signed_publication("parent_result", &original.original_parent)?,
        ),
        (
            old_acceptance,
            evidence::encode_signed_publication("human_acceptance", &original.original_acceptance)?,
        ),
    ] {
        if comments
            .iter()
            .filter(|c| {
                c.id == id
                    && c.issue_id == work.parent
                    && !c.bot
                    && !c.delegated
                    && c.user_id.map(|v| v.to_string()).as_deref()
                        == Some(
                            original
                                .original_acceptance
                                .payload
                                .provider_writer
                                .as_str(),
                        )
                    && c.body == body
                    && c.body_digest == bytes_digest(body.as_bytes())
            })
            .count()
            != 1
        {
            return Err(invalid(
                "original human carry publication disappeared or changed",
            ));
        }
    }
    require_original_acceptance_selection(
        comments,
        &original.original_acceptance,
        old_acceptance,
        key,
    )?;
    Ok(())
}

pub(super) fn require_original_acceptance_selection(
    comments: &[kegari_providers::Comment],
    selected: &Signed<evidence::Acceptance>,
    selected_id: Uuid,
    key: &AttestationKey,
) -> Result<()> {
    for comment in comments {
        if comment.id == selected_id
            || comment.bot
            || comment.delegated
            || comment.issue_id != selected.payload.work.parent
            || comment.user_id.map(|id| id.to_string()).as_deref()
                != Some(selected.payload.provider_writer.as_str())
        {
            continue;
        }
        if let Ok(other) = evidence::decode_signed_publication::<evidence::Acceptance>(
            &comment.body,
            "human_acceptance",
            key,
        ) && other.payload.runtime_id == selected.payload.runtime_id
            && other.payload.work == selected.payload.work
            && other.payload.reviewed_token == selected.payload.reviewed_token
        {
            return Err(invalid("original acceptance selection became ambiguous"));
        }
    }
    Ok(())
}

fn closed(receipt: &Receipt) -> Result<()> {
    if receipt.kind != "admission"
        || receipt.phase != ReceiptPhase::Reconciled
        || receipt.observed_result.as_ref().is_none_or(|v| {
            v["local_execution"] != "exited"
                || v["unresolved_effects"]
                    .as_array()
                    .is_none_or(|v| !v.is_empty())
        })
    {
        return Err(invalid("carry source or worker is not actually closed"));
    }
    Ok(())
}
fn validate_saved(
    saved: &Signed<CarryPublicationRecord>,
    root: &PrivateDir,
    records: &PrivateDir,
    host: &ControlHost,
    key: &AttestationKey,
) -> Result<()> {
    key.verify(SAVED_PURPOSE, saved)?;
    let stored = &saved.payload;
    key.verify(PURPOSE, &stored.signed)?;
    let record = &stored.signed.payload;
    key.verify("parent_result", &record.original_parent)?;
    key.verify("parent_result", &record.current_parent)?;
    key.verify("human_acceptance", &record.original_acceptance)?;
    let original = &record.original_parent.payload.evidence;
    let current = &record.current_parent.payload.evidence;
    let accepted = &record.original_acceptance.payload;
    if record.schema_version != 1
        || record.runtime.is_nil()
        || record.workspace.is_nil()
        || record.publication_operation.is_nil()
        || record.original_parent.payload.runtime_id != record.runtime
        || record.current_parent.payload.runtime_id != record.runtime
        || accepted.runtime_id != record.runtime
        || original.work != current.work
        || accepted.work != original.work
        || original.work.child.is_some()
        || original.token()? != accepted.reviewed_token
        || accepted.reviewed_token != record.carry.original_reviewed_token
        || record.original_acceptance.digest()? != record.carry.original_acceptance_digest
        || current.token()? != record.carry.current_evidence_token
        || original.token()? == current.token()?
        || accepted.accepted_at < original.created_at
        || record.carry.created_at < current.created_at
        || record.carry.schema_version != 1
    {
        return Err(invalid(
            "historical carry does not preserve original human and current machine subjects",
        ));
    }
    record.original.validate_retained_workspace(
        records,
        host,
        key,
        AcceptedOriginIdentity {
            runtime: record.runtime,
            workspace: record.workspace,
            work: &original.work,
            token: &accepted.reviewed_token,
            acceptance_digest: &record.carry.original_acceptance_digest,
        },
    )?;
    record.references.validate(root, host, key)?;
    let source = host.operation(record.source.operation)?;
    closed(&source)?;
    if canonical_digest(1, &source)? != record.source.receipt_digest
        || source.observed_result.as_ref().is_none_or(|v| {
            v["observation"]["parent_reassembly_vector"]
                != record.current_parent.payload.vector_digest
                || v["observation"]["parent_reassembly_baseline"] != record.baseline_digest
                || v["observation"]["original_reviewed_token"] != accepted.reviewed_token
        })
    {
        return Err(invalid(
            "historical carry source is not exact retired reassembly",
        ));
    }
    let parent_receipt = exact_parent_publication(
        host,
        key,
        &record.current_parent,
        &record.current_publication,
    )?;
    record.references.require_receipt(&parent_receipt)?;
    let parent_owner = host.operation(current.publication_operation)?;
    closed(&parent_owner)?;
    record.references.require_receipt(&parent_owner)?;
    let owner = host.operation(record.publication_operation)?;
    if owner.kind != "admission"
        || owner.runtime_id != record.runtime
        || owner.expected.get("scope") != Some(&record.publication_scope.digest()?)
        || record.publication_scope.phase != DeliveryPhase::ParentEvidence
        || record.publication_scope.work != current.work
        || record.publication_scope.candidate_vector_digest.as_deref()
            != Some(record.current_parent.payload.vector_digest.as_str())
        || owner.canonical_targets
            != [crate::authority::IssueKey::new(
                &current.work.linear_endpoint,
                current.work.linear_workspace,
                current.work.parent,
            )?
            .identity()?]
        || [
            "run",
            "worker_preparation",
            "metadata",
            "continuation",
            "source_advanced",
            "review_round",
            "review_round_preparation",
            "review_round_resume",
            "review_round_recovery",
            "session_resume",
            "session_recovery_preparation",
            "release_resume",
            "parent_reassembly",
        ]
        .iter()
        .any(|key| owner.expected.contains_key(*key))
    {
        return Err(invalid(
            "carry publication has no exact actual parent-evidence owner",
        ));
    }
    stored
        .publication
        .verify(&stored.signed, "linear", &accepted.provider_writer)?;
    let effect = host.operation(stored.receipt.operation)?;
    if canonical_digest(1, &effect)? != stored.receipt.receipt_digest
        || effect.kind != "linear_comment"
        || !matches!(
            effect.phase,
            ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
        )
        || effect.runtime_id != record.runtime
        || effect.expected.get("admission_operation")
            != Some(&record.publication_operation.to_string())
        || effect.remote_ids.get("comment_id") != Some(&stored.publication.record_id)
        || effect.expected.get("provider_writer") != Some(&accepted.provider_writer)
        || effect.expected.get("provider_body_digest")
            != Some(&bytes_digest(
                evidence::encode_signed_publication(PURPOSE, &stored.signed)?.as_bytes(),
            ))
        || effect.canonical_targets
            != [format!(
                "linear:{}:{}:{}",
                current.work.linear_endpoint, current.work.linear_workspace, current.work.parent
            )]
    {
        return Err(invalid(
            "historical carry lacks exact acknowledged native publication receipt",
        ));
    }
    Ok(())
}

/// Every applicable completed phase is required together before carry retains
/// their source, grant and actual joined-result closure.
pub(crate) struct CompletedParentReassemblyPhases<'a> {
    pub integrated: (&'a ParentReassemblyWork, &'a Arc<RunCompletion>),
    pub review: (&'a ParentReassemblyWork, &'a Arc<RunCompletion>),
    pub adequacy: Option<(&'a ParentReassemblyWork, &'a Arc<RunCompletion>)>,
}

impl DeliveryCoordinator {
    pub(crate) fn completed_parent_reassembly(
        &self,
        source: Arc<ParentReassemblySource>,
        parent: PublishedParent,
        phases: CompletedParentReassemblyPhases<'_>,
        configuration: &Config,
        retained: &impl Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
        grants: (ParentGrantObserver, ActiveParentGrantObserver),
    ) -> Result<ReassembledParent> {
        let CompletedParentReassemblyPhases {
            integrated,
            review,
            adequacy,
        } = phases;
        let discovered = parent.verified().manifest().integrated_validation.is_some()
            || parent
                .verified()
                .manifest()
                .archived_integrated_validation
                .is_some();
        if discovered != adequacy.is_some()
            || adequacy.is_some_and(|(proof, _)| proof.phase().phase != "parent_integrated_review")
        {
            return Err(invalid(
                "reassembled discovered validation requires its distinct independent adequacy phase",
            ));
        }
        let mut all_phases = vec![integrated];
        if let Some(pair) = adequacy {
            all_phases.push(pair);
        }
        all_phases.push(review);
        let (current_grants, active_grants) = grants;
        active_grants.require_configuration(configuration)?;
        source.validate(configuration)?;
        let receipts = self.host.operations()?;
        let mut refs = source.transition().carry_references(configuration)?;
        refs.extend(&source.original().references().retain_carry_references(
            &self.records,
            &self.host,
            &self.key,
        )?)?;
        for (proof, completion) in all_phases.iter().copied() {
            if proof.reference().source != *source.source_reference()
                || retained(completion.operation_id)?
                    .as_ref()
                    .is_none_or(|actual| !Arc::ptr_eq(actual, completion))
            {
                return Err(invalid(
                    "reassembled parent lost actual joined phase/source identity",
                ));
            }
            proof.verify_joined(retained, &self.host.snapshot()?)?;
            proof.verify_grants(&receipts, &active_grants.observe(proof.authority())?)?;
            proof.verify_result(completion, &receipts)?;
            proof.retain_history(&mut refs, &receipts)?;
            refs.retain_receipt(&self.host.operation(completion.operation_id)?)?;
            let receipt = self.host.operation(completion.operation_id)?;
            let prep: ReviewReceiptReference = serde_json::from_str(
                receipt
                    .expected
                    .get("parent_reassembly_preparation")
                    .ok_or_else(|| invalid("parent preparation absent"))?,
            )
            .map_err(|_| invalid("parent preparation invalid"))?;
            refs.retain_receipt(&self.host.operation(prep.operation)?)?;
        }
        if integrated.0.phase().phase != "parent_integrated_validation"
            || review.0.phase().phase != "parent_evidence_review"
            || !source
                .transition()
                .binds_current_parent(source.original().parent(), parent.verified())
            || source.vector().digest() != parent.signed.payload.vector_digest
        {
            return Err(invalid(
                "current parent did not consume every exact bounded reassembly phase",
            ));
        }
        refs.retain_receipt(&self.host.operation(source.source_reference().operation)?)?;
        let previous = source.previous_parent();
        refs.retain_file(
            CarryNamespace::Workspace {
                workspace: source.transition().workspace(),
            },
            &self.records,
            &format!("parent-result-{}.json", &previous.verified().token()[7..]),
            "parent_result",
            &self.key,
        )?;
        refs.retain_receipt(&exact_parent_publication(
            &self.host,
            &self.key,
            &previous.signed,
            previous.publication(),
        )?)?;
        let previous_owner = self
            .host
            .operation(previous.verified().manifest().publication_operation)?;
        closed(&previous_owner)?;
        refs.retain_receipt(&previous_owner)?;
        refs.retain_receipt(&exact_parent_publication(
            &self.host,
            &self.key,
            &parent.signed,
            parent.publication(),
        )?)?;
        refs.retain_file(
            CarryNamespace::Workspace {
                workspace: source.transition().workspace(),
            },
            &self.records,
            &format!("parent-vector-{}.json", &source.vector().digest()[7..]),
            "parent_candidate_vector",
            &self.key,
        )?;
        refs.retain_file(
            CarryNamespace::Workspace {
                workspace: source.transition().workspace(),
            },
            &self.records,
            &format!("parent-result-{}.json", &parent.verified().token()[7..]),
            "parent_result",
            &self.key,
        )?;
        Ok(ReassembledParent {
            source,
            parent,
            references: refs,
            phases: all_phases
                .iter()
                .map(|(proof, _)| (*proof).clone())
                .collect(),
            results: all_phases
                .iter()
                .map(|(_, result)| (*result).clone())
                .collect(),
            current_grants,
        })
    }
    pub(crate) async fn publish_carried_acceptance(
        &self,
        admission: &DeliveryAdmission,
        reassembled: &ReassembledParent,
        production: &[crate::delivery::CarryProduction],
        configuration: &Config,
        root: Arc<PrivateDir>,
        active_grants: &ActiveParentGrantObserver,
    ) -> Result<PublishedCarry> {
        active_grants.require_admission(
            &admission.permit,
            admission.fresh_gate(),
            configuration,
        )?;
        self.current(admission, &[DeliveryPhase::ParentEvidence])?;
        reassembled.source.refresh_current(configuration).await?;
        reassembled
            .validate_with_grants(configuration, &|authority| active_grants.observe(authority))?;
        let current = reassembled.parent.verified();
        if admission.scope.work != current.manifest().work
            || admission.scope.candidate_vector_digest.as_deref()
                != Some(reassembled.source.vector().digest())
            || crate::config::config_revision(configuration)?
                != admission.fresh_gate().config_revision()
        {
            return Err(invalid(
                "carry publication differs from exact admitted parent vector",
            ));
        }
        let proofs = production
            .iter()
            .map(|p| p.proof().clone())
            .collect::<Vec<_>>();
        let mut references = reassembled.references.clone();
        // The model/native parent publication is already retired by runtime
        // before production checks can complete. Retain its final owner bytes.
        let parent_owner = self
            .host
            .operation(current.manifest().publication_operation)?;
        closed(&parent_owner)?;
        references.retain_receipt(&parent_owner)?;
        for proof in production {
            references.extend(proof.references())?;
        }
        references.validate(&root, &self.host, &self.key)?;
        let timestamp = self.operation_time(admission, "carried-acceptance")?;
        let verified = evidence::carry_verified_acceptance(
            reassembled.source.original().parent(),
            current,
            reassembled.source.original().acceptance(),
            reassembled.source.transition(),
            configuration,
            &proofs,
            timestamp,
        )?;
        let signed = self.key.sign(
            PURPOSE,
            &CarryRecord {
                schema_version: 1,
                runtime: admission.scope.runtime_id,
                workspace: admission.fresh_gate().workspace_id(),
                publication_operation: admission.operation_id(),
                publication_scope: admission.scope.clone(),
                original_parent: reassembled.source.original().published().signed.clone(),
                original_acceptance: reassembled.source.original().signed_acceptance().clone(),
                original: reassembled.source.original().references().clone(),
                current_parent: reassembled.parent.signed.clone(),
                current_publication: reassembled.parent.publication().clone(),
                source: reassembled.source.source_reference().clone(),
                baseline_digest: reassembled.source.transition().digest().into(),
                carry: verified.record().clone(),
                references: references.clone(),
                recovery: Some(recovery::CarryRecoveryBinding::capture(
                    reassembled,
                    production,
                    configuration,
                    &self.host,
                )?),
            },
        )?;
        let source = reassembled.source.clone();
        let parent_guard = reassembled.clone();
        let active_guard = active_grants.clone();
        let config = configuration.clone();
        let host = self.host.clone();
        let key = self.key.clone();
        let refs = references;
        let root_guard = root.clone();
        let extra: GateCheck = Arc::new(move |gate| {
            source.verify_authority(gate)?;
            parent_guard.validate_with_grants(&config, &|authority| {
                active_guard.observe_in_gate(authority)
            })?;
            source.validate_current(&config)?;
            refs.validate(&root_guard, &host, &key)
        });
        let receipt = self
            .effect_guarded(
                admission,
                "publish-carried-acceptance",
                ProviderMutation::LinearComment {
                    issue: current.manifest().work.parent,
                    body: evidence::encode_signed_publication(PURPOSE, &signed)?,
                },
                Some(extra),
            )
            .await?;
        let publication = self
            .linear_publication(
                admission,
                current.manifest().work.parent,
                &remote_id(&receipt, "comment")?,
                PURPOSE,
                &signed,
            )
            .await?;
        self.current(admission, &[DeliveryPhase::ParentEvidence])?;
        reassembled.source.refresh_current(configuration).await?;
        reassembled
            .validate_with_grants(configuration, &|authority| active_grants.observe(authority))?;
        let saved = self.key.sign(
            SAVED_PURPOSE,
            &CarryPublicationRecord {
                signed,
                publication,
                receipt: ReviewReceiptReference {
                    operation: receipt.operation_id,
                    receipt_digest: canonical_digest(1, &receipt)?,
                },
            },
        )?;
        validate_saved(&saved, &root, &self.records, &self.host, &self.key)?;
        let name = leaf(current.token())?;
        let mut retained = None;
        self.host.validate_permit(&admission.permit, || {
            (self.current_gate)(admission.fresh_gate())?;
            reassembled.validate_with_grants(configuration, &|authority| {
                active_grants.observe_in_gate(authority)
            })?;
            reassembled.source.validate_current(configuration)?;
            match self.records.read::<Signed<CarryPublicationRecord>>(&name) {
                Ok(prior) => {
                    validate_saved(&prior, &root, &self.records, &self.host, &self.key)?;
                    if !same_published_carry(&prior.payload, &saved.payload)? {
                        return Err(invalid(
                            "current candidate already has another carry publication",
                        ));
                    }
                    // A fresh native read changes observation time only. Keep
                    // the original authenticated publication and its timestamp.
                    retained = Some(prior);
                }
                Err(error) if error.code == "record_missing" => {
                    self.records.write(&name, &saved)?
                }
                Err(error) => return Err(error),
            }
            Ok(())
        })?;
        let saved = retained.unwrap_or(saved);
        Ok(PublishedCarry {
            basis: CarryBasis::Live(Box::new(reassembled.clone())),
            verified,
            saved,
            records: self.records.clone(),
            root,
            host: self.host.clone(),
            key: self.key.clone(),
        })
    }
}

fn exact_parent_publication(
    host: &ControlHost,
    key: &AttestationKey,
    parent: &Signed<ParentResultRecord>,
    publication: &Publication,
) -> Result<Receipt> {
    key.verify("parent_result", parent)?;
    publication.verify(parent, "linear", &publication.writer_account_id)?;
    let evidence = &parent.payload.evidence;
    let target = format!(
        "linear:{}:{}:{}",
        evidence.work.linear_endpoint, evidence.work.linear_workspace, evidence.work.parent
    );
    let body =
        bytes_digest(evidence::encode_signed_publication("parent_result", parent)?.as_bytes());
    let mut matches = host.operations()?.into_iter().filter(|receipt| {
        receipt.kind == "linear_comment"
            && receipt.remote_ids.get("comment_id") == Some(&publication.record_id)
    });
    let receipt = matches
        .next()
        .ok_or_else(|| invalid("current parent publication receipt is missing"))?;
    if matches.next().is_some()
        || receipt.runtime_id != parent.payload.runtime_id
        || !matches!(
            receipt.phase,
            ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
        )
        || receipt.canonical_targets != [target]
        || receipt.expected.get("admission_operation")
            != Some(&evidence.publication_operation.to_string())
        || receipt.expected.get("provider_body_digest") != Some(&body)
        || receipt.expected.get("provider_writer") != Some(&publication.writer_account_id)
    {
        return Err(invalid(
            "current parent publication has no exact owned native receipt",
        ));
    }
    Ok(receipt)
}

fn same_published_carry(
    previous: &CarryPublicationRecord,
    observed: &CarryPublicationRecord,
) -> Result<bool> {
    Ok(previous.signed.digest()? == observed.signed.digest()?
        && previous.receipt == observed.receipt
        && same_publication_identity(&previous.publication, &observed.publication))
}
fn same_publication_identity(previous: &Publication, observed: &Publication) -> bool {
    let mut observed = observed.clone();
    observed.observed_at = previous.observed_at;
    previous == &observed
}

/// Authenticated historical closure of a link already admitted and published.
/// No method returns a VerifiedCarry from this record.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CarriedOriginRef {
    runtime: Uuid,
    workspace: Uuid,
    current_token: String,
    original_acceptance_digest: String,
    carried: CarriedAcceptance,
    references: CarryReferences,
}
impl CarriedOriginRef {
    pub(crate) fn validate_retained(
        &self,
        root: &PrivateDir,
        control: &ControlHost,
        key: &AttestationKey,
        plan: &crate::release::ReleasePlan,
    ) -> Result<()> {
        if self.runtime != plan.runtime_id
            || self.current_token != plan.evidence_token
            || self.original_acceptance_digest != plan.acceptance_digest
            || plan.carried_acceptance.as_ref() != Some(&self.carried)
        {
            return Err(invalid(
                "carried release history differs from exact admitted plan",
            ));
        }
        self.references.validate(root, control, key)
    }
    pub(crate) fn references(&self) -> &CarryReferences {
        &self.references
    }
}
impl DeliveryCoordinator {
    pub(super) fn carried_release_origin(
        &self,
        plan: &crate::release::ReleasePlan,
        current: &Signed<ParentResultRecord>,
        accepted: &Signed<evidence::Acceptance>,
        comments: &[kegari_providers::Comment],
    ) -> Result<CarriedOriginRef> {
        let workspace = self
            .records
            .path()
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<Uuid>().ok())
            .ok_or_else(|| invalid("carry history namespace is not its actual workspace"))?;
        let root = PrivateDir::open(
            self.records
                .path()
                .parent()
                .ok_or_else(|| invalid("carry history root missing"))?,
        )?;
        let name = leaf(&plan.evidence_token)?;
        let saved: Signed<CarryPublicationRecord> = self.records.read(&name)?;
        validate_saved(&saved, &root, &self.records, &self.host, &self.key)?;
        let record = &saved.payload.signed.payload;
        if record.workspace != workspace
            || record.runtime != plan.runtime_id
            || record.current_parent.digest()? != current.digest()?
            || record.original_acceptance.digest()? != accepted.digest()?
            || plan.carried_acceptance.as_ref() != Some(&record.carry)
        {
            return Err(invalid(
                "saved carry differs from actual original/current release publications",
            ));
        }
        let body = evidence::encode_signed_publication(PURPOSE, &saved.payload.signed)?;
        if comments
            .iter()
            .filter(|c| {
                c.id.to_string() == saved.payload.publication.record_id
                    && c.issue_id == plan.parent_id
                    && !c.bot
                    && !c.delegated
                    && c.body == body
                    && c.body_digest == bytes_digest(body.as_bytes())
                    && c.user_id.map(|id| id.to_string()).as_deref()
                        == Some(accepted.payload.provider_writer.as_str())
            })
            .count()
            != 1
        {
            return Err(invalid(
                "exact carried acceptance native publication missing or duplicated",
            ));
        }
        let (original_comment, _) = record.original.publication_ids();
        let original_body =
            evidence::encode_signed_publication("parent_result", &record.original_parent)?;
        if comments
            .iter()
            .filter(|c| {
                c.id == original_comment
                    && c.issue_id == plan.parent_id
                    && !c.bot
                    && !c.delegated
                    && c.body == original_body
                    && c.body_digest == bytes_digest(original_body.as_bytes())
                    && c.user_id.map(|id| id.to_string()).as_deref()
                        == Some(accepted.payload.provider_writer.as_str())
            })
            .count()
            != 1
        {
            return Err(invalid("original human-reviewed native parent disappeared"));
        }
        let owner = self.host.operation(record.publication_operation)?;
        closed(&owner)?;
        let mut references = record.references.clone();
        references.retain_file(
            CarryNamespace::Workspace { workspace },
            &self.records,
            &name,
            SAVED_PURPOSE,
            &self.key,
        )?;
        references.retain_receipt(&self.host.operation(saved.payload.receipt.operation)?)?;
        references.retain_receipt(&owner)?;
        let result = CarriedOriginRef {
            runtime: record.runtime,
            workspace,
            current_token: plan.evidence_token.clone(),
            original_acceptance_digest: plan.acceptance_digest.clone(),
            carried: record.carry.clone(),
            references,
        };
        result.validate_retained(&root, &self.host, &self.key, plan)?;
        Ok(result)
    }
}

impl DeliveryCoordinator {
    /// Exact signed native projection only. This deliberately returns no
    /// current acceptance/carry capability.
    pub(crate) fn verify_carried_publication_for_plan(
        body: &str,
        key: &AttestationKey,
        plan: &crate::release::ReleasePlan,
    ) -> Result<()> {
        let signed: Signed<CarryRecord> = evidence::decode_signed_publication(body, PURPOSE, key)?;
        let record = &signed.payload;
        if record.schema_version != 1
            || record.runtime != plan.runtime_id
            || record.current_parent.payload.evidence.work.parent != plan.parent_id
            || record.current_parent.payload.evidence.work.linear_workspace != plan.linear_workspace
            || record.current_parent.payload.evidence.work.generation != plan.generation
            || record.current_parent.payload.evidence.token()? != plan.evidence_token
            || record.original_acceptance.digest()? != plan.acceptance_digest
            || plan.carried_acceptance.as_ref() != Some(&record.carry)
        {
            return Err(invalid(
                "native carry publication does not name the exact admitted candidate",
            ));
        }
        Ok(())
    }
}

#[path = "delivery_carried_recovery.rs"]
mod recovery;
pub(crate) use recovery::{ObservedCurrentCarry, RetainedCarrySubject};

#[cfg(test)]
mod readback_tests {
    use super::*;
    #[test]
    fn repeat_readback_preserves_only_original_observation_time() {
        let original = Publication {
            provider: "linear".into(),
            record_id: Uuid::new_v4().to_string(),
            writer_account_id: Uuid::new_v4().to_string(),
            payload_digest: canonical_digest(1, &"same signed payload").unwrap(),
            observed_at: Utc::now(),
            url: "https://linear.invalid/parent/comment".into(),
        };
        let mut later = original.clone();
        later.observed_at += chrono::Duration::seconds(10);
        assert!(same_publication_identity(&original, &later));
        for field in [
            "provider",
            "record_id",
            "writer_account_id",
            "payload_digest",
            "url",
        ] {
            let mut value = serde_json::to_value(&later).unwrap();
            value[field] = json!("different");
            let changed: Publication = serde_json::from_value(value).unwrap();
            assert!(!same_publication_identity(&original, &changed), "{field}");
        }
    }
}

#[path = "delivery_carried_no_release.rs"]
mod no_release;
pub(crate) use no_release::CarriedNoReleaseOrigin;
