//! Protected worker proposals are provenance, never passed workflow gates.
use crate::{
    control::{Receipt, RunReference},
    evidence::{AttestationKey, Signed},
    instructions::AgentRole,
    reconcile::{FreshGate, GatePurpose},
    storage::PrivateDir,
    worker::{WorkerOutcome, recover_session},
};
use kegari_digest::canonical_digest;
use kegari_types::error::{Error, Result};
use serde_json::Value;
use uuid::Uuid;
#[path = "runtime_claims_transition.rs"]
mod transition_records;

/// An actual completed authorized proposal recovered from host-owned records.
/// Its historical authority does not authorize any new provider write: consumers
/// must obtain a fresh gate and their own current operation permit.
#[derive(Clone, Debug)]
pub struct VerifiedAuthorizedOutcome {
    outcome: WorkerOutcome,
    original_authority: FreshGate,
    authority: FreshGate,
    admission: Receipt,
    digest: String,
}
impl VerifiedAuthorizedOutcome {
    pub fn outcome(&self) -> &WorkerOutcome {
        &self.outcome
    }
    pub fn original_authority(&self) -> &FreshGate {
        &self.original_authority
    }
    /// Historical effective authority of this particular completed turn.
    /// Further effects still require a current native gate.
    pub fn effective_authority(&self) -> &FreshGate {
        &self.authority
    }
    pub fn correction_transition_digest(&self) -> Option<&str> {
        self.outcome
            .processed_correction
            .as_ref()
            .and_then(|processed| processed.transition_digest.as_deref())
    }
    pub fn run_receipt(&self) -> &Receipt {
        &self.admission
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

#[derive(Clone, Debug)]
pub struct VerifiedCoordinationOutcome(VerifiedAuthorizedOutcome);
impl VerifiedCoordinationOutcome {
    pub fn outcome(&self) -> &WorkerOutcome {
        self.0.outcome()
    }
    pub fn original_authority(&self) -> &FreshGate {
        self.0.original_authority()
    }
    pub fn run_receipt(&self) -> &Receipt {
        self.0.run_receipt()
    }
    pub fn digest(&self) -> &str {
        self.0.digest()
    }
}

pub(crate) fn recover_coordination(
    key: &AttestationKey,
    claims: &PrivateDir,
    records: &PrivateDir,
    runtime: Uuid,
    admission: Receipt,
    proposal: Uuid,
) -> Result<VerifiedCoordinationOutcome> {
    let verified = recover_authorized(key, claims, records, runtime, admission, proposal)?;
    let outcome = verified.outcome();
    let authority = verified.original_authority();
    if outcome.instructions.role != AgentRole::Coordination
        || !outcome
            .instructions
            .permissions
            .source_write_paths
            .is_empty()
        || !outcome
            .instructions
            .permissions
            .resource_write_paths
            .is_empty()
        || outcome.instructions.permissions.repository_code_execution
        || authority.purpose() != GatePurpose::Coordination
        || authority.work().child.is_some()
        || authority.child_scope().is_some()
    {
        return Err(Error::new(
            "worker_binding",
            "coordination requires an actual completed read-only parent session",
        ));
    }
    Ok(VerifiedCoordinationOutcome(verified))
}
pub(crate) fn recover_authorized(
    key: &AttestationKey,
    claims: &PrivateDir,
    records: &PrivateDir,
    runtime: Uuid,
    admission: Receipt,
    proposal: Uuid,
) -> Result<VerifiedAuthorizedOutcome> {
    let bytes = claims.read_bytes(&format!("pending-worker-{proposal}.json"), 8 * 1024 * 1024)?;
    let signed: Signed<Value> = serde_json::from_slice(&bytes)
        .map_err(|_| Error::new("record_corrupt", "pending worker claim is invalid"))?;
    key.verify("pending_worker_claim", &signed)?;
    let envelope = signed
        .payload
        .as_object()
        .ok_or_else(|| Error::new("record_corrupt", "pending worker claim envelope is missing"))?;
    if envelope.len() != 3 || envelope.get("schema_version") != Some(&Value::from(1)) {
        return Err(Error::new(
            "record_corrupt",
            "pending worker claim lacks its bound original authority",
        ));
    }
    let outcome: WorkerOutcome = serde_json::from_value(signed.payload["outcome"].clone())
        .map_err(|_| Error::new("record_corrupt", "pending worker outcome is invalid"))?;
    let run = RunReference::from_receipt(&admission)?
        .ok_or_else(|| Error::new("worker_binding", "proposal has no actual worker admission"))?;
    let authority = FreshGate::recover_host_claim(key, &signed, &outcome.fresh_gate_digest)?;
    let original =
        transition_records::original_authority(key, claims, &admission, &outcome, &authority)?;
    let contract: Value = records.read("contract.json")?;
    let recovered = recover_session(records, &admission)?.ok_or_else(|| {
        Error::new(
            "worker_binding",
            "proposal has no retained provider session",
        )
    })?;
    if !matches!(outcome.schema_version, 1 | 2)
        || outcome.runtime_id != runtime
        || admission.runtime_id != runtime
        || outcome.operation_id != admission.operation_id
        || outcome.proposal_id != proposal
        || proposal.is_nil()
        || outcome.instructions.role == AgentRole::IndependentReview
        || outcome.independent_review.is_some()
        || outcome.work.as_ref() != Some(authority.work())
        || authority.workspace_id() != run.workspace
        || authority.work_key()? != run.phase.work
        || canonical_digest(1, &original.work().generation)? != run.phase.generation
        || outcome.instructions.work_generation != original.work().digest()?
        || outcome.instructions.operation_id != admission.operation_id
        || outcome.instructions.workspace_id != run.workspace
        || outcome.instructions.phase != run.phase.phase
        || canonical_digest(crate::instructions::ASSEMBLY_SCHEMA, &outcome.instructions)?
            != run.instruction_digest
        || outcome.instruction_digest != run.instruction_digest
        || canonical_digest(1, &outcome.instructions.profile)? != run.profile_digest
        || canonical_digest(1, &contract["instructions"])? != outcome.instruction_digest
        || contract["tool_contract"].as_str() != Some(&outcome.tool_contract_digest)
        || canonical_digest(1, &outcome.session.binding)?
            != canonical_digest(1, &recovered.state.binding)?
        || outcome.session.thread_id.is_none()
        || outcome.session.thread_id != recovered.state.thread_id
        || outcome.session.max_turns > run.max_turns
        || outcome.session.turns_started > run.max_turns
        || !outcome.session.completion_is_current()
        || outcome.session.terminal.as_deref() != Some("completed")
        || outcome.session.turn_id.is_some()
        || outcome.session.input_required
        || outcome.session.intervention.is_some()
        || recovered.state.input_required
        || recovered.state.intervention.is_some()
        || recovered.pending_method.is_some()
        || !recovered.state.completion_is_current()
        || recovered.state.terminal.as_deref() != Some("completed")
        || recovered.state.turn_id.is_some()
        || recovered.state.turns_started != outcome.session.turns_started
        || recovered.state.completed_turns != outcome.session.completed_turns
        || recovered.state.correction_required != outcome.session.correction_required
        || recovered.state.correction_consumed != outcome.session.correction_consumed
        || outcome.schema_version == 2
            && (outcome.session.last_completed_turn.as_deref() != Some(&outcome.completed_turn)
                || recovered.state.last_completed_turn.as_deref() != Some(&outcome.completed_turn))
        || outcome
            .session
            .completed_turns
            .get(&outcome.completed_turn)
            .map(String::as_str)
            != Some("completed")
        || recovered
            .state
            .completed_turns
            .get(&outcome.completed_turn)
            .map(String::as_str)
            != Some("completed")
    {
        return Err(Error::new(
            "worker_binding",
            "authorized proposal differs from its actual admitted completed session",
        ));
    }
    let journal = PrivateDir::open(&records.path().join("journal"))?;
    let expected = canonical_digest(1, &outcome)?;
    let mut found = false;
    let mut delivered_at = None;
    let mut processed_at = None;
    let mut outcome_at = None;
    // recover_session has already checked complete contiguous bounded history.
    // Recheck this particular pending proposal; a signed model-shaped value
    // alone cannot stand in for an actually recorded completed-turn output.
    let files = journal.records_bounded(65_537)?;
    let mut total = 0usize;
    for (index, leaf) in files.into_iter().enumerate() {
        if leaf == "binding.json" {
            continue;
        }
        let event: Value = journal.read(&leaf)?;
        total = total.saturating_add(
            serde_json::to_vec(&event)
                .map_err(|_| Error::invalid("journal event encoding failed"))?
                .len(),
        );
        if total > 64 * 1024 * 1024 {
            return Err(Error::new(
                "journal_limit",
                "proposal journal exceeds its byte bound",
            ));
        }
        if let Some(processed) = &outcome.processed_correction {
            if event["kind"] == "correction_delivered"
                && event["record"]["thread"].as_str() == outcome.session.thread_id.as_deref()
                && event["record"]["turn"].as_str() == Some(&outcome.completed_turn)
                && event["record"]["transition"].as_str() == processed.transition_digest.as_deref()
                && event["record"]["input_digest"].as_str() == Some(&processed.input_digest)
                && event["record"]["watermark"].as_u64() == Some(processed.watermark)
            {
                delivered_at = Some(index);
            }
            if event["kind"] == "correction_processed"
                && event["record"]["thread"].as_str() == outcome.session.thread_id.as_deref()
                && event["record"]["turn"].as_str() == Some(&outcome.completed_turn)
                && canonical_digest(1, &event["record"]["processed"])?
                    == canonical_digest(1, processed)?
                && processed_at.replace(index).is_some()
            {
                return Err(Error::new(
                    "worker_binding",
                    "correction processing journal is duplicated",
                ));
            }
        }
        if event["kind"] == "outcome_pending"
            && event["record"]["proposal_id"].as_str() == Some(&proposal.to_string())
        {
            if found || canonical_digest(1, &event["record"])? != expected {
                return Err(Error::new(
                    "worker_binding",
                    "pending proposal journal is duplicate or differs from its signed record",
                ));
            }
            found = true;
            outcome_at = Some(index);
        }
    }
    if let Some(processed) = &outcome.processed_correction
        && processed.watermark > 0
        && (processed_at.is_none()
            || processed_at >= outcome_at
            || processed.transition_digest.is_some()
                && (delivered_at.is_none() || delivered_at >= processed_at))
    {
        return Err(Error::new(
            "worker_binding",
            "corrected outcome lacks ordered actual turn delivery and consumption",
        ));
    }
    if !found {
        return Err(Error::new(
            "worker_binding",
            "pending proposal is absent from the actual worker journal",
        ));
    }
    Ok(VerifiedAuthorizedOutcome {
        outcome,
        original_authority: original,
        authority,
        admission,
        digest: signed.digest()?,
    })
}

pub(crate) fn proposal_ids(records: &PrivateDir, admission: &Receipt) -> Result<Vec<Uuid>> {
    if recover_session(records, admission)?.is_none() {
        return Ok(vec![]);
    }
    let journal = PrivateDir::open(&records.path().join("journal"))?;
    let mut ids = std::collections::BTreeSet::new();
    let mut total = 0usize;
    for leaf in journal.records_bounded(65_537)? {
        if leaf == "binding.json" {
            continue;
        }
        let event: Value = journal.read(&leaf)?;
        total = total.saturating_add(
            serde_json::to_vec(&event)
                .map_err(|_| Error::invalid("journal event encoding failed"))?
                .len(),
        );
        if total > 64 * 1024 * 1024 {
            return Err(Error::new(
                "journal_limit",
                "proposal discovery exceeds its byte bound",
            ));
        }
        if event["kind"] != "outcome_pending" {
            continue;
        }
        let outcome: WorkerOutcome =
            serde_json::from_value(event["record"].clone()).map_err(|_| {
                Error::new(
                    "record_corrupt",
                    "pending outcome journal schema is invalid",
                )
            })?;
        if outcome.operation_id != admission.operation_id
            || outcome.runtime_id != admission.runtime_id
            || outcome.proposal_id.is_nil()
            || !ids.insert(outcome.proposal_id)
            || ids.len() > 128
        {
            return Err(Error::new(
                "worker_binding",
                "pending outcome inventory crosses binding or finite proposal bound",
            ));
        }
    }
    Ok(ids.into_iter().collect())
}

#[cfg(test)]
pub(crate) use tests::{
    test_coordination_fixture, test_coordination_fixture_with_proposal, test_fixture,
    test_fixture_with_claim, test_fixture_with_history_digest,
};
#[cfg(test)]
#[path = "runtime_claims_contract_fixture.rs"]
mod contract_fixture;
#[cfg(test)]
pub(crate) use contract_fixture::test_completed_contract_fixture;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        authority::{ChildScope, Corrections, IssueKey, Responsibility},
        codex::{CodexProfile, SessionBinding, SessionState},
        control::ReceiptPhase,
        evidence::{Generation, WorkIdentity},
        instructions::InstructionManifest,
        reconcile::{ChildProjection, DependencyOutcomes, ParentProjection},
        scheduler::PhaseKey,
    };
    use chrono::{Duration, Utc};
    use kegari_config::{Profile, ReleaseMode, Speed};
    use kegari_digest::bytes_digest;
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    struct Fixture {
        _temp: tempfile::TempDir,
        key: AttestationKey,
        claims: PrivateDir,
        records: PrivateDir,
        admission: Receipt,
        outcome: WorkerOutcome,
        authority: FreshGate,
    }
    impl Fixture {
        fn new() -> Self {
            Self::with(AgentRole::Coordination, None, json!({"fixture":true}), None)
        }
        fn with(
            role: AgentRole,
            supplied_work: Option<WorkIdentity>,
            context: Value,
            repository_revision: Option<String>,
        ) -> Self {
            let temp = tempfile::TempDir::new().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let claims = PrivateDir::create(&base.join("claims")).unwrap();
            let records = PrivateDir::create(&base.join("records")).unwrap();
            let digest = bytes_digest(b"fixture");
            let profile = Profile {
                tool: "codex".into(),
                model: "fixture-model".into(),
                reasoning: "high".into(),
                speed: Speed::Standard,
            };
            let work = supplied_work.unwrap_or(WorkIdentity {
                linear_endpoint: "https://linear.invalid/graphql".into(),
                linear_workspace: id(2),
                parent: id(10),
                child: None,
                generation: Generation {
                    authorization: digest.clone(),
                    definition: digest.clone(),
                    correction: digest.clone(),
                    scope: digest.clone(),
                },
            });
            let children = work
                .child
                .map(|child| {
                    (
                        child,
                        ChildProjection {
                            parent_reference: "fixture/10".into(),
                            key: IssueKey::new(&work.linear_endpoint, work.linear_workspace, child)
                                .unwrap(),
                            issue: None,
                            scope: Some(ChildScope {
                                schema_version: 1,
                                kind: "child".into(),
                                responsibility: Responsibility::Implementation,
                                write_repository: Some("42".into()),
                                write_paths: vec!["src/".into()],
                                read_repositories: vec![],
                                test_resources: vec![],
                            }),
                            profile: Some(profile.clone()),
                            work: Some(work.clone()),
                            blockers: vec![],
                            prerequisites: BTreeSet::new(),
                        },
                    )
                })
                .into_iter()
                .collect();
            let mut parent_work = work.clone();
            parent_work.child = None;
            let parent = ParentProjection {
                workspace_id: id(3),
                workspace_name: "fixture".into(),
                parent_reference: "fixture/10".into(),
                key: IssueKey::new(&work.linear_endpoint, work.linear_workspace, work.parent)
                    .unwrap(),
                issue: None,
                included: true,
                observed_at: Utc::now(),
                required_children: BTreeSet::from([work.child.unwrap_or(id(11))]),
                children,
                observations: BTreeMap::new(),
                metadata: None,
                authorization_candidate: None,
                authorization_published: true,
                corrections: Corrections::default(),
                observation_revision: 1,
                disappeared_children: BTreeMap::new(),
                work: Some(parent_work),
                blockers: vec![],
                prerequisites: BTreeSet::new(),
                source_digest: digest.clone(),
                config_revision: digest.clone(),
                workflow_digest: digest.clone(),
                coordination_profile: profile.clone(),
                drain_config_revision: digest.clone(),
                release_mode: ReleaseMode::Manual,
            };
            let authority = parent
                .fresh_gate(
                    work.child,
                    if work.child.is_none() {
                        GatePurpose::Coordination
                    } else {
                        GatePurpose::NewOperation
                    },
                    &DependencyOutcomes::default(),
                    Utc::now(),
                    Duration::seconds(30),
                )
                .unwrap();
            let isolation = json!({"fixture":"no process executed"});
            let manifest:InstructionManifest=serde_json::from_value(json!({"schema_version":1,"runtime_version":"fixture","runtime_build":digest,
                "asset_version":"fixture","assets":{},"workflow_path":base.join("WORKFLOW.md"),"workflow_digest":digest,"role":role,
                "phase":if role==AgentRole::Coordination {"correction_classification_fixture"}else{"review"},"workspace_id":id(3),"operation_id":id(7),"work_generation":work.digest().unwrap(),
                "correction_revision":work.generation.correction,"context_digest":canonical_digest(1,&context).unwrap(),"prompt_digest":digest,"repository_revision":repository_revision,"repository_root":null,
                "cwd":base,"codex_home":base.join("auth"),"codex_protocol":"fixture","codex_version":"fixture","profile":profile,
                "supplied_files":[],"provider_discovery":{"state":"unavailable","reason":"fixture"},"tools":[],
                "permissions":{"read_roots":[],"source_write_paths":[],"resource_write_paths":[],"scratch_roots":[],"repository_code_execution":false,
                    "enforcement_profile":canonical_digest(1,&isolation).unwrap(),"network":"denied"}})).unwrap();
            let instruction_digest =
                canonical_digest(crate::instructions::ASSEMBLY_SCHEMA, &manifest).unwrap();
            let mut session = SessionState::new(
                SessionBinding {
                    schema_version: 1,
                    account_id: "fixture-account".into(),
                    work_identity: authority.work_key().unwrap(),
                    operation_id: id(7).to_string(),
                    generation: manifest.work_generation.clone(),
                    instruction_digest: instruction_digest.clone(),
                    isolation_digest: manifest.permissions.enforcement_profile.clone(),
                    profile: CodexProfile {
                        model: profile.model.clone(),
                        reasoning: profile.reasoning.clone(),
                        speed: "standard".into(),
                    },
                    cwd: manifest.cwd.clone(),
                },
                1,
            )
            .unwrap();
            session.thread_id = Some("fixture-thread".into());
            session.provider_history = None; // synthetic claims fixture, no provider-history proof
            session.turns_started = 1;
            session.terminal = Some("completed".into());
            session
                .completed_turns
                .insert("fixture-turn".into(), "completed".into());
            let run = RunReference {
                workspace: id(3),
                account: "fixture-account".into(),
                normalized_state: Some("prepared".into()),
                phase: PhaseKey {
                    work: authority.work_key().unwrap(),
                    generation: canonical_digest(1, &work.generation).unwrap(),
                    phase: manifest.phase.clone(),
                    retry_grant: None,
                },
                profile_digest: canonical_digest(1, &profile).unwrap(),
                instruction_digest: instruction_digest.clone(),
                max_attempts: 1,
                max_turns: 1,
                max_review_rounds: 1,
                attempt: 1,
            };
            let admission = Receipt {
                operation_id: id(7),
                request_id: id(7),
                runtime_id: id(1),
                owner_generation: 1,
                actor: "fixture-host".into(),
                kind: "admission".into(),
                canonical_targets: vec![authority.work_key().unwrap()],
                payload_digest: digest.clone(),
                expected: BTreeMap::from([
                    ("scope".into(), digest.clone()),
                    ("run".into(), serde_json::to_string(&run).unwrap()),
                ]),
                phase: ReceiptPhase::Reconciled,
                remote_ids: BTreeMap::new(),
                observed_result: None,
                cancellation: "none".into(),
                updated_at: Utc::now(),
            };
            let outcome = WorkerOutcome {
                validation_history_digest: None,
                schema_version: 1,
                runtime_id: id(1),
                operation_id: id(7),
                proposal_id: id(8),
                processed_correction: None,
                completed_turn: "fixture-turn".into(),
                work: Some(work),
                independent_review: None,
                session,
                instructions: manifest,
                instruction_digest,
                tool_contract_digest: digest.clone(),
                fresh_gate_digest: authority.digest().unwrap(),
                claim: json!({"kind":"correction_classification","classifications":[]}),
                artifact_refs: vec![],
                checks: vec![],
            };
            let f = Self {
                _temp: temp,
                key: AttestationKey::new("fixture".into(), vec![7; 32]).unwrap(),
                claims,
                records,
                admission,
                outcome,
                authority,
            };
            f.records.write("contract.json",&json!({"scope":digest,"instructions":f.outcome.instructions,"tool_contract":digest,"isolation":isolation})).unwrap();
            let journal = f.records.child("journal").unwrap();
            journal.write("binding.json",&json!({"schema_version":1,"scope_digest":digest,"instruction_digest":f.outcome.instruction_digest,"tool_contract_digest":digest})).unwrap();
            journal.write("event-00000000.json",&json!({"kind":"session","record":{"state":f.outcome.session,"pending_method":null}})).unwrap();
            f.save();
            f
        }
        fn save(&self) {
            let signed = self
                .key
                .sign(
                    "pending_worker_claim",
                    &json!({"schema_version":1,"outcome":self.outcome,"authority":self.authority}),
                )
                .unwrap();
            self.claims
                .write_bytes(
                    &format!("pending-worker-{}.json", self.outcome.proposal_id),
                    &serde_json::to_vec(&signed).unwrap(),
                )
                .unwrap();
            self.records
                .child("journal")
                .unwrap()
                .write(
                    "event-00000001.json",
                    &json!({"kind":"outcome_pending","record":self.outcome}),
                )
                .unwrap();
        }
        fn recover(&self) -> Result<VerifiedCoordinationOutcome> {
            recover_coordination(
                &self.key,
                &self.claims,
                &self.records,
                id(1),
                self.admission.clone(),
                self.outcome.proposal_id,
            )
        }
    }
    async fn corrected_fixture() -> (tempfile::TempDir, Fixture) {
        let (native, transition) = crate::corrections::test_transition_fixture().await;
        let original = transition.original_authority();
        let mut fixture = Fixture::with(
            AgentRole::Testing,
            Some(original.work().clone()),
            json!({"fixture":true}),
            None,
        );
        fixture.authority = transition.current_authority().clone();
        fixture.outcome.schema_version = 2;
        fixture.outcome.work = Some(transition.effective_work().clone());
        fixture.outcome.fresh_gate_digest = fixture.authority.digest().unwrap();
        fixture.outcome.instructions.workspace_id = original.workspace_id();
        fixture.outcome.instructions.profile = original.profile().clone();
        fixture.outcome.instructions.workflow_digest = original.workflow_digest().into();
        fixture.outcome.instruction_digest = canonical_digest(
            crate::instructions::ASSEMBLY_SCHEMA,
            &fixture.outcome.instructions,
        )
        .unwrap();
        fixture.outcome.session.binding.instruction_digest =
            fixture.outcome.instruction_digest.clone();
        fixture.outcome.session.binding.profile = CodexProfile {
            model: original.profile().model.clone(),
            reasoning: original.profile().reasoning.clone(),
            speed: "standard".into(),
        };
        fixture.outcome.session.correction_required = transition.watermark();
        fixture.outcome.session.correction_consumed = transition.watermark();
        fixture.outcome.session.last_completed_turn = Some(fixture.outcome.completed_turn.clone());
        fixture.outcome.processed_correction = Some(crate::worker::ProcessedCorrection {
            watermark: transition.watermark(),
            input_digest: transition.input_digest(),
            transition_digest: Some(transition.digest().into()),
        });
        let mut run = RunReference::from_receipt(&fixture.admission)
            .unwrap()
            .unwrap();
        run.workspace = original.workspace_id();
        run.profile_digest = canonical_digest(1, original.profile()).unwrap();
        run.instruction_digest = fixture.outcome.instruction_digest.clone();
        fixture
            .admission
            .expected
            .insert("run".into(), serde_json::to_string(&run).unwrap());
        let mut contract: Value = fixture.records.read("contract.json").unwrap();
        contract["instructions"] = serde_json::to_value(&fixture.outcome.instructions).unwrap();
        fixture.records.write("contract.json", &contract).unwrap();
        let journal = fixture.records.child("journal").unwrap();
        journal.write("binding.json", &json!({"schema_version":1,"scope_digest":fixture.admission.expected["scope"],"instruction_digest":fixture.outcome.instruction_digest,"tool_contract_digest":fixture.outcome.tool_contract_digest})).unwrap();
        journal.write("event-00000000.json", &json!({"kind":"session","record":{"state":fixture.outcome.session,"pending_method":null}})).unwrap();
        fixture.save();
        journal.write("event-00000001.json", &json!({"kind":"correction_delivered","record":{"thread":fixture.outcome.session.thread_id,"turn":fixture.outcome.completed_turn,"transition":transition.digest(),"input_digest":transition.input_digest(),"watermark":transition.watermark()}})).unwrap();
        journal.write("event-00000002.json", &json!({"kind":"correction_processed","record":{"thread":fixture.outcome.session.thread_id,"turn":fixture.outcome.completed_turn,"processed":fixture.outcome.processed_correction}})).unwrap();
        journal
            .write(
                "event-00000003.json",
                &json!({"kind":"outcome_pending","record":fixture.outcome}),
            )
            .unwrap();
        let record = json!({"schema_version":1,"runtime":fixture.admission.runtime_id,"operation":fixture.admission.operation_id,"work":fixture.admission.canonical_targets[0],"scope":fixture.admission.expected["scope"],"instructions":fixture.outcome.instruction_digest,"tool_contract":fixture.outcome.tool_contract_digest,"transition":transition.record().unwrap()});
        let signed = fixture
            .key
            .sign("worker_correction_transition", &record)
            .unwrap();
        fixture
            .claims
            .write_bytes(
                &format!(
                    "worker-transition-{}-{}.json",
                    fixture.admission.operation_id,
                    transition.watermark()
                ),
                &serde_json::to_vec(&signed).unwrap(),
            )
            .unwrap();
        (native, fixture)
    }
    fn recover_corrected(fixture: &Fixture) -> Result<VerifiedAuthorizedOutcome> {
        recover_authorized(
            &fixture.key,
            &fixture.claims,
            &fixture.records,
            id(1),
            fixture.admission.clone(),
            fixture.outcome.proposal_id,
        )
    }
    #[tokio::test]
    async fn corrected_claim_recovers_original_assembly_and_effective_native_history() {
        let (_native, fixture) = corrected_fixture().await;
        let verified = recover_corrected(&fixture).unwrap();
        assert_ne!(
            verified.original_authority().work(),
            verified.effective_authority().work()
        );
        assert_eq!(
            verified.original_authority().work().digest().unwrap(),
            verified.outcome().instructions.work_generation
        );
        assert_eq!(
            Some(verified.effective_authority().work()),
            verified.outcome().work.as_ref()
        );
        assert_eq!(
            verified.correction_transition_digest(),
            fixture
                .outcome
                .processed_correction
                .as_ref()
                .unwrap()
                .transition_digest
                .as_deref()
        );
        let leaf = format!(
            "worker-transition-{}-{}.json",
            fixture.admission.operation_id,
            fixture.authority.correction_watermark()
        );
        std::fs::remove_file(fixture.claims.path().join(&leaf)).unwrap();
        assert_eq!(
            recover_corrected(&fixture).unwrap_err().code,
            "worker_binding"
        );
    }
    #[tokio::test]
    async fn corrected_claim_rejects_unprocessed_or_reordered_turn_evidence() {
        let (_native, fixture) = corrected_fixture().await;
        let journal = fixture.records.child("journal").unwrap();
        let processed: Value = journal.read("event-00000002.json").unwrap();
        let outcome: Value = journal.read("event-00000003.json").unwrap();
        journal.write("event-00000002.json", &outcome).unwrap();
        journal.write("event-00000003.json", &processed).unwrap();
        assert_eq!(
            recover_corrected(&fixture).unwrap_err().code,
            "worker_binding"
        );
        journal.write("event-00000002.json", &processed).unwrap();
        journal.write("event-00000003.json", &outcome).unwrap();
        let mut delivered: Value = journal.read("event-00000001.json").unwrap();
        delivered["record"]["turn"] = json!("other-turn");
        journal.write("event-00000001.json", &delivered).unwrap();
        assert_eq!(
            recover_corrected(&fixture).unwrap_err().code,
            "worker_binding"
        );
    }
    /// Protected deterministic host fixture, not actual provider/process evidence.
    /// It goes through the production signature/contract/journal recovery path.
    pub(crate) fn test_fixture(
        role: AgentRole,
        work: WorkIdentity,
        context: Value,
        repository_revision: Option<String>,
    ) -> (
        tempfile::TempDir,
        VerifiedAuthorizedOutcome,
        std::sync::Arc<crate::worker_run::RunCompletion>,
    ) {
        test_fixture_with_claim(
            role,
            work,
            context,
            repository_revision,
            json!({"kind":"correction_classification","classifications":[]}),
        )
    }
    pub(crate) fn test_fixture_with_claim(
        role: AgentRole,
        work: WorkIdentity,
        context: Value,
        repository_revision: Option<String>,
        claim: Value,
    ) -> (
        tempfile::TempDir,
        VerifiedAuthorizedOutcome,
        std::sync::Arc<crate::worker_run::RunCompletion>,
    ) {
        test_fixture_with_history_digest(role, work, context, repository_revision, claim, None)
    }
    /// Signed descriptive marker fixture. It never creates a worker history seal.
    pub(crate) fn test_fixture_with_history_digest(
        role: AgentRole,
        work: WorkIdentity,
        context: Value,
        repository_revision: Option<String>,
        claim: Value,
        history: Option<String>,
    ) -> (
        tempfile::TempDir,
        VerifiedAuthorizedOutcome,
        std::sync::Arc<crate::worker_run::RunCompletion>,
    ) {
        let mut fixture = Fixture::with(role, Some(work), context, repository_revision);
        fixture.outcome.claim = claim;
        fixture.outcome.validation_history_digest = history;
        fixture.save();
        let verified = recover_authorized(
            &fixture.key,
            &fixture.claims,
            &fixture.records,
            id(1),
            fixture.admission.clone(),
            fixture.outcome.proposal_id,
        )
        .unwrap();
        let completion = std::sync::Arc::new(crate::worker_run::RunCompletion {
            contract: None,
            execution: None,
            operation_id: fixture.outcome.operation_id,
            state: fixture.outcome.session.clone(),
            outcome: crate::worker_run::RunOutcome::Completed,
            error: None,
            elapsed_ms: 1,
            hook_error: None,
            checks: vec![],
            failed_checks: vec![],
            validation_plan: None,
            validation_artifacts: vec![],
            validation_attempts: vec![],
            validation_error: None,
            validation_history: None,
            local_exit_confirmed: true,
        });
        (fixture._temp, verified, completion)
    }
    pub(crate) fn test_coordination_fixture(
        authority: FreshGate,
        claim: Value,
        account: &str,
    ) -> (tempfile::TempDir, VerifiedCoordinationOutcome) {
        test_coordination_fixture_with_proposal(authority, claim, account, id(8))
    }
    pub(crate) fn test_coordination_fixture_with_proposal(
        authority: FreshGate,
        claim: Value,
        account: &str,
        proposal: Uuid,
    ) -> (tempfile::TempDir, VerifiedCoordinationOutcome) {
        let mut fixture = Fixture::new();
        fixture.outcome.proposal_id = proposal;
        fixture.outcome.work = Some(authority.work().clone());
        fixture.outcome.fresh_gate_digest = authority.digest().unwrap();
        fixture.outcome.claim = claim;
        let manifest = &mut fixture.outcome.instructions;
        manifest.workspace_id = authority.workspace_id();
        manifest.work_generation = authority.work().digest().unwrap();
        manifest.correction_revision = authority.work().generation.correction.clone();
        manifest.workflow_digest = authority.workflow_digest().into();
        manifest.profile = authority.profile().clone();
        fixture.outcome.instruction_digest =
            canonical_digest(crate::instructions::ASSEMBLY_SCHEMA, manifest).unwrap();
        let state = &mut fixture.outcome.session;
        state.binding.account_id = account.into();
        state.binding.work_identity = authority.work_key().unwrap();
        state.binding.generation = manifest.work_generation.clone();
        state.binding.instruction_digest = fixture.outcome.instruction_digest.clone();
        state.binding.profile = CodexProfile {
            model: manifest.profile.model.clone(),
            reasoning: manifest.profile.reasoning.clone(),
            speed: match manifest.profile.speed {
                Speed::Standard => "standard",
                Speed::Fast => "fast",
            }
            .into(),
        };
        state.correction_required = authority.correction_watermark();
        state.correction_consumed = state.correction_required;
        let mut run = RunReference::from_receipt(&fixture.admission)
            .unwrap()
            .unwrap();
        run.workspace = authority.workspace_id();
        run.account = account.into();
        run.phase.work = authority.work_key().unwrap();
        run.phase.generation = canonical_digest(1, &authority.work().generation).unwrap();
        run.profile_digest = canonical_digest(1, &manifest.profile).unwrap();
        run.instruction_digest = fixture.outcome.instruction_digest.clone();
        fixture.admission.canonical_targets = vec![run.phase.work.clone()];
        fixture
            .admission
            .expected
            .insert("run".into(), serde_json::to_string(&run).unwrap());
        fixture.authority = authority;
        let mut contract: Value = fixture.records.read("contract.json").unwrap();
        contract["instructions"] = serde_json::to_value(&fixture.outcome.instructions).unwrap();
        fixture.records.write("contract.json", &contract).unwrap();
        let journal = fixture.records.child("journal").unwrap();
        let mut binding: Value = journal.read("binding.json").unwrap();
        binding["instruction_digest"] = json!(fixture.outcome.instruction_digest);
        journal.write("binding.json", &binding).unwrap();
        journal.write("event-00000000.json",&json!({"kind":"session","record":{"state":fixture.outcome.session,"pending_method":null}})).unwrap();
        fixture.save();
        let verified = fixture.recover().unwrap();
        (fixture._temp, verified)
    }
    #[test]
    fn signed_coordination_claim_requires_actual_bound_completed_journal() {
        let f = Fixture::new();
        let verified = f.recover().unwrap();
        assert_eq!(verified.outcome().proposal_id, id(8));
        assert_eq!(verified.run_receipt().operation_id, id(7));
        assert_eq!(
            verified.original_authority().digest().unwrap(),
            f.authority.digest().unwrap()
        );
        f.records
            .child("journal")
            .unwrap()
            .write(
                "event-00000001.json",
                &json!({"kind":"diagnostic","record":{}}),
            )
            .unwrap();
        assert_eq!(f.recover().unwrap_err().code, "worker_binding");
        f.save();
        f.records
            .child("journal")
            .unwrap()
            .write(
                "event-00000002.json",
                &json!({"kind":"outcome_pending","record":f.outcome}),
            )
            .unwrap();
        assert_eq!(f.recover().unwrap_err().code, "worker_binding");
    }
    #[test]
    fn signed_claim_cannot_supply_an_unobserved_turn_or_cross_run_binding() {
        for mutation in [
            "turn",
            "operation",
            "account",
            "instructions",
            "write_permission",
            "stale_watermark",
        ] {
            let mut f = Fixture::new();
            match mutation {
                "turn" => {
                    f.outcome.completed_turn = "invented-turn".into();
                    f.outcome.session.completed_turns.clear();
                    f.outcome
                        .session
                        .completed_turns
                        .insert("invented-turn".into(), "completed".into());
                }
                "operation" => f.outcome.operation_id = id(99),
                "account" => f.outcome.session.binding.account_id = "another-account".into(),
                "instructions" => f.outcome.instructions.phase = "another_phase".into(),
                "write_permission" => f
                    .outcome
                    .instructions
                    .permissions
                    .source_write_paths
                    .push("/tmp/forbidden".into()),
                "stale_watermark" => f.outcome.session.correction_required = 1,
                _ => unreachable!(),
            }
            f.save();
            assert_eq!(
                f.recover().unwrap_err().code,
                "worker_binding",
                "{mutation}"
            );
        }
    }
    #[test]
    fn host_signature_and_prior_input_hold_are_not_replaced_by_later_success() {
        let f = Fixture::new();
        let leaf = format!("pending-worker-{}.json", f.outcome.proposal_id);
        let mut signed: Signed<Value> =
            serde_json::from_slice(&f.claims.read_bytes(&leaf, 8 * 1024 * 1024).unwrap()).unwrap();
        signed.payload["outcome"]["claim"] = json!({"forged":true});
        f.claims
            .write_bytes(&leaf, &serde_json::to_vec(&signed).unwrap())
            .unwrap();
        assert_eq!(f.recover().unwrap_err().code, "invalid_attestation");
        f.save();
        let mut held = f.outcome.session.clone();
        held.input_required = true;
        let journal = f.records.child("journal").unwrap();
        journal
            .write(
                "event-00000000.json",
                &json!({"kind":"session","record":{"state":held,"pending_method":null}}),
            )
            .unwrap();
        journal.write("event-00000002.json",&json!({"kind":"session","record":{"state":f.outcome.session,"pending_method":null}})).unwrap();
        assert_eq!(f.recover().unwrap_err().code, "worker_binding");
    }
    #[test]
    fn later_turn_or_pending_protocol_request_supersedes_completed_claim() {
        for terminal in ["active", "failed", "interrupted", "completed", "pending"] {
            let mut f = Fixture::new();
            let mut run = RunReference::from_receipt(&f.admission).unwrap().unwrap();
            run.max_turns = 2;
            f.admission
                .expected
                .insert("run".into(), serde_json::to_string(&run).unwrap());
            f.outcome.session.max_turns = 2;
            let journal = f.records.child("journal").unwrap();
            journal.write("event-00000000.json",&json!({"kind":"session","record":{"state":f.outcome.session,"pending_method":null}})).unwrap();
            f.save();
            f.recover().unwrap();
            let mut later = f.outcome.session.clone();
            if terminal != "pending" {
                later.turns_started = 2;
                later.terminal = Some(terminal.into());
                if terminal == "active" {
                    later.turn_id = Some("later-turn".into());
                } else {
                    later
                        .completed_turns
                        .insert("later-turn".into(), terminal.into());
                }
            }
            journal.write("event-00000002.json",&json!({"kind":"session","record":{"state":later,"pending_method":if terminal=="pending"{Some("turn/start")}else{None}}})).unwrap();
            assert_eq!(
                f.recover().unwrap_err().code,
                "worker_binding",
                "{terminal}"
            );
        }
    }
}
