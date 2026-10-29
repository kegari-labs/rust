//! Durable completion of an exact supersede chain. The historical reference
//! records a fact already observed; it is never a dispatch or recovery permit.
use super::*;
use crate::control::Receipt;
use chrono::DateTime;

const PURPOSE: &str = "release_chain_completion";
const MAX_REFERENCES: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProofFile {
    leaf: String,
    purpose: String,
    digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginReference {
    receipts: BTreeSet<Uuid>,
    vector: ProofFile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    carried: Option<crate::delivery::CarriedOriginRef>,
}
use super::super::completion_binding::DependencyBinding;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionRecord {
    schema_version: u32,
    runtime: Uuid,
    workspace: Uuid,
    config_revision: String,
    terminal: Uuid,
    // Oldest ancestor first, successful terminal last.
    attempts: Vec<(Uuid, String)>,
    receipts: BTreeMap<Uuid, String>,
    intents: BTreeMap<Uuid, String>,
    effects: Vec<ProofFile>,
    origins: BTreeMap<Uuid, OriginReference>,
    target_proofs: Vec<ProofFile>,
    inventory_digest: String,
    observed_at: DateTime<Utc>,
    // Omission preserves the signed schema-1 representation. A legacy record
    // can settle queue history, but cannot establish the old GitHub account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dependency_binding: Option<DependencyBinding>,
}

#[derive(Clone)]
pub(crate) struct VerifiedReleaseCompletion {
    record: CompletionRecord,
    signed_digest: String,
    terminal: ReleaseAttempt,
    work: WorkIdentity,
    holds: BTreeMap<Uuid, domain::ResourceHold>,
    records: Arc<kegari_storage::PrivateDir>,
    control: Arc<crate::control::ControlHost>,
    key: Arc<kegari_secrets::AttestationKey>,
}
impl std::fmt::Debug for VerifiedReleaseCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedReleaseCompletion")
            .field("terminal", &self.record.terminal)
            .field("signed_digest", &self.signed_digest)
            .finish()
    }
}
fn leaf(operation: Uuid) -> String {
    format!("release-completion-{operation}.json")
}
fn invalid(message: &str) -> Error {
    Error::new("release_completion", message)
}
fn completion_context(
    mut error: Error,
    operation: Uuid,
    stage: &'static str,
    purpose: &str,
) -> Error {
    error.details = json!({"release_operation":operation,"completion_stage":stage,
        "reference_purpose":purpose,"cause":error.details});
    error
}

fn read_target_reference(
    workspace: &kegari_storage::PrivateDir,
    operation: Uuid,
    reference: &ProofFile,
) -> Result<Signed<Value>> {
    // RuntimeDelivery gives ProductionObserver this exact dedicated namespace.
    // Recovery must open it without creating a missing store or falling back to
    // a similarly named file in the workspace's release/vector store.
    kegari_storage::PrivateDir::open(&workspace.path().join("deployment"))
        .and_then(|deployment| deployment.read(&reference.leaf))
        .map_err(|error| {
            completion_context(error, operation, "target_reference", &reference.purpose)
        })
}

pub(super) fn validate_completed_owner(
    control: &crate::control::ControlHost,
    record: &AttemptRecord,
) -> Result<()> {
    record.attempt.validate_recovered()?;
    let owner = control.operation(record.admission_operation)?;
    if !record.admission_confirmed
        || !record.local_retired
        || !record.pending_receipts.is_empty()
        || !record.attempt.completed()
        || record.attempt.has_unknown()
        || record.attempt.hold_reason.is_some()
        || record.attempt.abandoned
        || record.attempt.resolved
        || record.attempt.assigned_successor.is_some()
        || owner.kind != "admission"
        || owner.runtime_id != record.runtime
        || owner.phase != ReceiptPhase::Reconciled
        || owner.owner_generation != record.attempt.owner_generation
        || owner.expected.get("scope") != Some(&admission_scope(&owner, &record.attempt.plan)?)
        || owner.canonical_targets != vec![issue(record)?.identity()?]
        || owner.observed_result.as_ref().is_none_or(|v| {
            v["local_execution"] != "exited"
                || v["unresolved_effects"]
                    .as_array()
                    .is_none_or(|a| !a.is_empty())
        })
    {
        return Err(invalid(
            "terminal release is not completely successful and actually retired",
        ));
    }
    Ok(())
}

fn retain_receipt_reference(refs: &mut BTreeMap<Uuid, String>, receipt: &Receipt) -> Result<()> {
    let digest = canonical_digest(1, receipt)?;
    if let Some(prior) = refs.get(&receipt.operation_id)
        && prior != &digest
    {
        return Err(invalid("completed receipt changed during observation"));
    }
    refs.insert(receipt.operation_id, digest);
    if refs.len() > MAX_REFERENCES {
        return Err(invalid("completed receipt inventory exceeds bound"));
    }
    Ok(())
}
fn require_receipts(expected: &BTreeMap<Uuid, String>, actual: &[Receipt]) -> Result<()> {
    for receipt in actual {
        if expected.get(&receipt.operation_id) != Some(&canonical_digest(1, receipt)?) {
            return Err(invalid("completed receipt ancestry is missing or changed"));
        }
    }
    Ok(())
}
fn closed_owner_references(
    control: &crate::control::ControlHost,
    record: &AttemptRecord,
) -> Result<Vec<Receipt>> {
    let mut unique = BTreeMap::<Uuid, Receipt>::new();
    let mut retain = |receipt: Receipt| -> Result<()> {
        if let Some(prior) = unique.get(&receipt.operation_id)
            && canonical_digest(1, prior)? != canonical_digest(1, &receipt)?
        {
            return Err(invalid("completed ancestry changed during collection"));
        }
        unique.insert(receipt.operation_id, receipt);
        if unique.len() > MAX_REFERENCES {
            return Err(invalid("completed recovery ancestry exceeds bound"));
        }
        Ok(())
    };
    let refs = control.closed_release_resume_receipts(record.admission_operation)?;
    for receipt in &refs {
        retain(receipt.clone())?;
    }
    let mut resumed = Vec::new();
    for receipt in &refs {
        if let Some(reference) = crate::control::ReleaseResumeReference::from_receipt(receipt)? {
            resumed.push(reference);
        }
    }
    if resumed.iter().any(|reference| {
        reference.plan_operation_id() != record.attempt.plan.operation_id
            || reference.plan_digest() != record.attempt.plan.digest().unwrap_or_default()
    }) || resumed.len()
        != record
            .attempt
            .recovery_records
            .iter()
            .filter(|r| r.action == "resume")
            .count()
        || record.recovery_receipts.len() != record.attempt.recovery_records.len()
    {
        return Err(invalid("completed release recovery inventory differs"));
    }
    for receipt in &refs {
        if crate::control::ReleaseResumeReference::from_receipt(receipt)?.is_some()
            && record.recovery_receipts.get(&receipt.operation_id) != Some(&receipt.payload_digest)
        {
            return Err(invalid("completed release omitted a resumed admission"));
        }
    }
    for (id, digest) in &record.recovery_receipts {
        let receipt = control.operation(*id)?;
        if receipt.payload_digest != *digest {
            return Err(invalid("completed recovery receipt changed"));
        }
        if crate::control::ReleaseResumeReference::from_receipt(&receipt)?.is_some()
            && let Some(previous) = refs.iter().find(|prior| prior.operation_id == *id)
        {
            // The latest closed ancestry already traversed this exact link and
            // its command/authorization. Reread its identity, not its full prefix.
            if canonical_digest(1, previous)? != canonical_digest(1, &receipt)? {
                return Err(invalid("completed resume changed during collection"));
            }
            retain(receipt)?;
            continue;
        }
        for receipt in control.closed_release_recovery_receipts(
            record.attempt.plan.operation_id,
            &record.attempt.plan.digest()?,
            *id,
        )? {
            retain(receipt)?;
        }
    }
    Ok(unique.into_values().collect())
}

fn validate_completion_envelope(
    records: &kegari_storage::PrivateDir,
    key: &kegari_secrets::AttestationKey,
    expected: &CompletionRecord,
) -> Result<()> {
    let signed: Signed<CompletionRecord> = match records.read(&leaf(expected.terminal)) {
        Ok(signed) => signed,
        Err(error) if error.code == "record_missing" => return Ok(()),
        Err(error) => return Err(error),
    };
    key.verify(PURPOSE, &signed)?;
    if signed.payload.terminal != expected.terminal
        || signed.payload.runtime != expected.runtime
        || signed.payload.workspace != expected.workspace
    {
        return Err(invalid("current completion envelope identity changed"));
    }
    Ok(())
}

/// Authenticate every signed scope-bearing attempt before deriving a protective
/// anchor. Owner/effect/origin references are checked only by full proof recovery;
/// an anchor can deny related work but never clear holds or supply credit.
fn authenticated_completion_attempts(
    delivery: &RuntimeDelivery,
    signed: &Signed<CompletionRecord>,
) -> Result<Vec<AttemptRecord>> {
    delivery.runtime.services.key.verify(PURPOSE, signed)?;
    let record = &signed.payload;
    if record.runtime != delivery.runtime.configuration.snapshot()?.runtime.id
        || record.runtime.is_nil()
        || record.workspace.is_nil()
        || record.terminal.is_nil()
    {
        return Err(invalid(
            "completion scope belongs to another runtime or has no identity",
        ));
    }
    if !matches!(
        (record.schema_version, record.dependency_binding.is_some()),
        (1, false) | (2 | 3, true)
    ) || record.attempts.is_empty()
        || record.attempts.len() > MAX_RELEASE_LINKS
        || record.attempts.last().map(|(id, _)| *id) != Some(record.terminal)
        || record.receipts.len() > MAX_REFERENCES
        || record.effects.len() > MAX_REFERENCES
        || record.target_proofs.len() > MAX_REFERENCES
        || record.intents.len() + 1 != record.attempts.len()
        || record.origins.keys().copied().collect::<BTreeSet<_>>()
            != record.attempts.iter().map(|(id, _)| *id).collect()
        || record.observed_at > Utc::now() + Duration::seconds(5)
    {
        return Err(invalid(
            "completion reference has an invalid finite chain inventory",
        ));
    }
    evidence::validate_digest(&record.inventory_digest)?;
    evidence::validate_digest(&record.config_revision)?;
    let mut seen = BTreeSet::new();
    let mut attempts = Vec::new();
    for (id, digest) in &record.attempts {
        if id.is_nil() || !seen.insert(*id) {
            return Err(invalid("completion chain is cyclic or repeats an attempt"));
        }
        let stored: Signed<AttemptRecord> = delivery.records.read(&attempt_leaf(*id))?;
        delivery
            .runtime
            .services
            .key
            .verify("runtime_release_attempt", &stored)?;
        if canonical_digest(1, &stored.payload)? != *digest
            || stored.payload.schema_version != 1
            || stored.payload.attempt.plan.operation_id != *id
            || stored.payload.runtime != record.runtime
            || stored.payload.workspace != record.workspace
            || stored.payload.work.child.is_some()
            || stored.payload.attempt.plan.runtime_id != record.runtime
            || stored.payload.attempt.plan.parent_id != stored.payload.work.parent
            || stored.payload.attempt.plan.linear_workspace != stored.payload.work.linear_workspace
            || stored.payload.attempt.plan.generation != stored.payload.work.generation
        {
            return Err(invalid(
                "completion attempt reference changed or belongs to another runtime",
            ));
        }
        stored.payload.work.validate()?;
        stored.payload.attempt.validate_recovered()?;
        attempts.push(stored.payload);
    }
    Ok(attempts)
}

#[derive(Clone)]
pub(crate) struct ReleaseCompletionAnchor {
    record: CompletionRecord,
    signed_digest: String,
    terminal: ReleaseAttempt,
    work: WorkIdentity,
    resources: BTreeSet<domain::ResourceKey>,
    records: Arc<kegari_storage::PrivateDir>,
    key: Arc<kegari_secrets::AttestationKey>,
}
impl std::fmt::Debug for ReleaseCompletionAnchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReleaseCompletionAnchor")
            .field("operation", &self.operation())
            .field("digest", &self.digest())
            .finish()
    }
}
impl ReleaseCompletionAnchor {
    fn from_signed(delivery: &RuntimeDelivery, signed: &Signed<CompletionRecord>) -> Result<Self> {
        let attempts = authenticated_completion_attempts(delivery, signed)?;
        let terminal = attempts
            .last()
            .expect("bounded nonempty authenticated chain");
        // Union every original scope, even if a broken relation prevents full
        // completion validation. Narrowing to the terminal could lose a hold.
        let resources = attempts
            .iter()
            .flat_map(|attempt| attempt.attempt.plan.resources.iter().cloned())
            .collect();
        let result = Self {
            record: signed.payload.clone(),
            signed_digest: signed.digest()?,
            terminal: terminal.attempt.clone(),
            work: terminal.work.clone(),
            resources,
            records: delivery.records.clone(),
            key: delivery.runtime.services.key.clone(),
        };
        result.queue_scope()?;
        Ok(result)
    }
    pub(crate) fn validate_queue_envelope(&self) -> Result<()> {
        validate_completion_envelope(&self.records, &self.key, &self.record)
    }
    pub(crate) fn operation(&self) -> Uuid {
        self.record.terminal
    }
    pub(crate) fn digest(&self) -> &str {
        &self.signed_digest
    }
    pub(crate) fn runtime_id(&self) -> Uuid {
        self.record.runtime
    }
    pub(crate) fn workspace_id(&self) -> Uuid {
        self.record.workspace
    }
    pub(crate) fn work(&self) -> &WorkIdentity {
        &self.work
    }
    pub(crate) fn resources(&self) -> &BTreeSet<domain::ResourceKey> {
        &self.resources
    }
    pub(crate) fn outcome_digest(&self) -> Result<String> {
        canonical_digest(1, &self.terminal.steps)
    }
    pub(crate) fn covered_operations(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.record.attempts.iter().map(|(operation, _)| *operation)
    }
    pub(crate) fn queue_scope(&self) -> Result<domain::CompletionScope> {
        domain::CompletionScope::release_anchor(self)
    }
}

pub(in crate::runtime_delivery::release) struct ReleaseCompletionQueueInventory {
    pub(in crate::runtime_delivery::release) proofs: BTreeMap<Uuid, VerifiedReleaseCompletion>,
    pub(in crate::runtime_delivery::release) anchors: BTreeMap<Uuid, ReleaseCompletionAnchor>,
    pub(in crate::runtime_delivery::release) errors: BTreeMap<Uuid, Error>,
}

impl VerifiedReleaseCompletion {
    fn recover(delivery: &RuntimeDelivery, signed: Signed<CompletionRecord>) -> Result<Self> {
        let attempts = authenticated_completion_attempts(delivery, &signed)?;
        let record = &signed.payload;
        let terminal = attempts.last().expect("bounded nonempty chain");
        validate_completed_owner(&delivery.runtime.control, terminal)?;
        let mut holds = BTreeMap::new();
        for pair in attempts.windows(2) {
            let ancestor = &pair[0];
            let child = &pair[1];
            super::super::baseline::validate_known_owner(&delivery.runtime.control, ancestor)?;
            if ancestor.attempt.assigned_successor != Some(child.attempt.plan.operation_id)
                || ancestor.attempt.plan.resources != terminal.attempt.plan.resources
                || issue(ancestor)? != issue(terminal)?
                || ancestor.work.generation.scope != terminal.work.generation.scope
                || !record
                    .intents
                    .contains_key(&child.attempt.plan.operation_id)
            {
                return Err(invalid(
                    "completion does not bind every exact ancestor hold assignment",
                ));
            }
            holds.insert(
                ancestor.attempt.plan.operation_id,
                domain::ResourceHold {
                    operation_id: ancestor.attempt.plan.operation_id,
                    parent_id: ancestor.attempt.plan.parent_id,
                    resources: ancestor.attempt.plan.resources.clone(),
                    assigned_successor: ancestor.attempt.assigned_successor,
                },
            );
        }
        let result = Self {
            record: record.clone(),
            signed_digest: signed.digest()?,
            terminal: terminal.attempt.clone(),
            work: terminal.work.clone(),
            holds,
            records: delivery.records.clone(),
            control: delivery.runtime.control.clone(),
            key: delivery.runtime.services.key.clone(),
        };
        result.validate_references()?;
        Ok(result)
    }
    fn validate_references(&self) -> Result<()> {
        let mut expected_files = BTreeMap::new();
        let workspace = kegari_storage::PrivateDir::open(
            &self.records.path().join(self.record.workspace.to_string()),
        )?;
        for (id, digest) in &self.record.attempts {
            let stored: Signed<AttemptRecord> = self.records.read(&attempt_leaf(*id))?;
            self.key.verify("runtime_release_attempt", &stored)?;
            if canonical_digest(1, &stored.payload)? != *digest {
                return Err(invalid("completed chain attempt changed"));
            }
            require_receipts(
                &self.record.receipts,
                &closed_owner_references(&self.control, &stored.payload).map_err(|error| {
                    completion_context(error, *id, "owner_references", "release_recovery")
                })?,
            )?;
            for (step, state) in &stored.payload.attempt.steps {
                let max = stored.payload.attempt.step_attempt(step)?;
                for ordinal in 1..=max {
                    let mut found = false;
                    for (prefix, purpose) in [
                        ("release-effect", "release_effect"),
                        ("release-no-dispatch", "release_no_dispatch"),
                    ] {
                        let leaf = format!(
                            "{prefix}-{id}-{}-attempt-{ordinal}.json",
                            &kegari_digest::bytes_digest(step.as_bytes())[7..]
                        );
                        match workspace.read::<Signed<Value>>(&leaf) {
                            Ok(signed) => {
                                self.key.verify(purpose, &signed)?;
                                found = true;
                                expected_files.insert(leaf, (purpose.to_owned(), signed.digest()?));
                            }
                            Err(error) if error.code == "record_missing" => (),
                            Err(error) => return Err(error),
                        }
                    }
                    if !found
                        && (ordinal < max
                            || !matches!(state, StepState::NotStarted | StepState::Retired { .. }))
                    {
                        return Err(invalid(
                            "completed chain omitted a required prior step attempt",
                        ));
                    }
                    if expected_files.len() > MAX_REFERENCES {
                        return Err(invalid("completed effect history exceeds bound"));
                    }
                }
            }
            let origin = self
                .record
                .origins
                .get(id)
                .ok_or_else(|| invalid("completed origin missing"))?;
            if origin.receipts.len() != 2
                || origin
                    .receipts
                    .iter()
                    .any(|id| !self.record.receipts.contains_key(id))
                || origin.vector.purpose != "parent_candidate_vector"
            {
                return Err(invalid("completed publication inventory differs"));
            }
            match (
                &stored.payload.attempt.plan.carried_acceptance,
                &origin.carried,
            ) {
                (Some(_), Some(carried)) => carried.validate_retained(
                    &self.records,
                    &self.control,
                    &self.key,
                    &stored.payload.attempt.plan,
                )?,
                (None, None) => (),
                _ => return Err(invalid("completion lost exact carried acceptance history")),
            }
            let vector: Signed<Value> = workspace.read(&origin.vector.leaf).map_err(|error| {
                completion_context(error, *id, "origin_reference", "parent_candidate_vector")
            })?;
            self.key.verify("parent_candidate_vector", &vector)?;
            if vector.digest()? != origin.vector.digest {
                return Err(invalid("completed parent vector changed"));
            }
            if *id == self.record.terminal {
                validate_completed_owner(&self.control, &stored.payload)?;
            } else {
                super::super::baseline::validate_known_owner(&self.control, &stored.payload)?;
            }
        }
        for (id, digest) in &self.record.receipts {
            if canonical_digest(1, &self.control.operation(*id)?)? != *digest {
                return Err(invalid("completed chain receipt changed"));
            }
        }
        for (id, digest) in &self.record.intents {
            let signed: Signed<Value> =
                self.records.read(&format!("release-successor-{id}.json"))?;
            self.key.verify("release_successor_intent", &signed)?;
            if signed.digest()? != *digest {
                return Err(invalid("completed chain successor authorization changed"));
            }
        }
        let mut targets = BTreeSet::new();
        for reference in &self.record.target_proofs {
            if !targets.insert(&reference.leaf)
                || !matches!(
                    reference.purpose.as_str(),
                    "target_deployment"
                        | "deployment_challenge"
                        | "target_deployment_index"
                        | "external_target_baseline"
                )
                || (reference.purpose == "external_target_baseline"
                    && (self.record.schema_version != 3
                        || !reference.leaf.starts_with("external-baseline-")))
                || reference.leaf.starts_with("target-index-")
            {
                return Err(invalid("completed immutable target reference differs"));
            }
            let signed = read_target_reference(&workspace, self.record.terminal, reference)?;
            self.key.verify(&reference.purpose, &signed)?;
            if signed.digest()? != reference.digest {
                return Err(invalid("completed immutable target proof changed"));
            }
        }
        if expected_files.len() != self.record.effects.len() {
            return Err(invalid("completed step reference inventory is incomplete"));
        }
        let mut files = BTreeSet::new();
        for reference in &self.record.effects {
            if !files.insert(&reference.leaf)
                || !matches!(
                    reference.purpose.as_str(),
                    "release_effect" | "release_no_dispatch"
                )
            {
                return Err(invalid(
                    "completion effect references are duplicate or unsupported",
                ));
            }
            let signed: Signed<Value> = workspace.read(&reference.leaf).map_err(|error| {
                completion_context(
                    error,
                    self.record.terminal,
                    "effect_reference",
                    &reference.purpose,
                )
            })?;
            self.key.verify(&reference.purpose, &signed)?;
            if expected_files.get(&reference.leaf)
                != Some(&(reference.purpose.clone(), reference.digest.clone()))
                || signed.digest()? != reference.digest
            {
                return Err(invalid("completed chain effect record changed"));
            }
        }
        Ok(())
    }
    /// Separate the current outer storage/authentication boundary from missing
    /// references inside a formerly authenticated scope. A missing known seal
    /// is classified by full validate below; malformed/present unauthenticated
    /// storage remains a global queue failure, including before inventory sync.
    pub(crate) fn validate_queue_envelope(&self) -> Result<()> {
        validate_completion_envelope(&self.records, &self.key, &self.record)
    }
    pub(crate) fn validate(&self) -> Result<&BTreeMap<Uuid, domain::ResourceHold>> {
        let signed: Signed<CompletionRecord> = self.records.read(&leaf(self.record.terminal))?;
        self.key.verify(PURPOSE, &signed)?;
        if signed.digest()? != self.signed_digest {
            return Err(invalid("completion reference was replaced"));
        }
        self.validate_references()?;
        Ok(&self.holds)
    }
    pub(crate) fn queue_anchor(&self) -> Result<ReleaseCompletionAnchor> {
        let resources = self
            .terminal
            .plan
            .resources
            .iter()
            .cloned()
            .chain(
                self.holds
                    .values()
                    .flat_map(|hold| hold.resources.iter().cloned()),
            )
            .collect();
        let result = ReleaseCompletionAnchor {
            record: self.record.clone(),
            signed_digest: self.signed_digest.clone(),
            terminal: self.terminal.clone(),
            work: self.work.clone(),
            resources,
            records: self.records.clone(),
            key: self.key.clone(),
        };
        result.queue_scope()?;
        Ok(result)
    }
    /// Flatten only a fully verified completed release's actual immutable
    /// closure. This data is historical provenance, never a success constructor.
    pub(crate) fn carry_references(&self) -> Result<crate::delivery::CarryReferences> {
        self.validate()?;
        use crate::delivery::{CarryNamespace, CarryReferences};
        let mut refs = CarryReferences::default();
        refs.retain_file(
            CarryNamespace::Root,
            &self.records,
            &leaf(self.record.terminal),
            PURPOSE,
            &self.key,
        )?;
        let workspace = kegari_storage::PrivateDir::open(
            &self.records.path().join(self.record.workspace.to_string()),
        )?;
        for (operation, _) in &self.record.attempts {
            refs.retain_file(
                CarryNamespace::Root,
                &self.records,
                &attempt_leaf(*operation),
                "runtime_release_attempt",
                &self.key,
            )?;
        }
        for operation in self.record.intents.keys() {
            refs.retain_file(
                CarryNamespace::Root,
                &self.records,
                &format!("release-successor-{operation}.json"),
                "release_successor_intent",
                &self.key,
            )?;
        }
        for reference in self
            .record
            .effects
            .iter()
            .chain(self.record.origins.values().map(|o| &o.vector))
        {
            refs.retain_file(
                CarryNamespace::Workspace {
                    workspace: self.record.workspace,
                },
                &workspace,
                &reference.leaf,
                &reference.purpose,
                &self.key,
            )?;
        }
        if !self.record.target_proofs.is_empty() {
            let deployment =
                kegari_storage::PrivateDir::open(&workspace.path().join("deployment"))?;
            for reference in &self.record.target_proofs {
                refs.retain_file(
                    CarryNamespace::Deployment {
                        workspace: self.record.workspace,
                    },
                    &deployment,
                    &reference.leaf,
                    &reference.purpose,
                    &self.key,
                )?;
            }
        }
        for origin in self.record.origins.values() {
            if let Some(carried) = &origin.carried {
                refs.extend(carried.references())?;
            }
        }
        for operation in self.record.receipts.keys() {
            refs.retain_receipt(&self.control.operation(*operation)?)?;
        }
        refs.validate(&self.records, &self.control, &self.key)?;
        Ok(refs)
    }
    pub(crate) fn inventory_limit() -> usize {
        MAX_REFERENCES
    }
    pub(crate) fn carry_publications(&self, configuration: &Config) -> Result<(Uuid, Uuid)> {
        self.validate_for_configuration(configuration)?;
        let signed: Signed<AttemptRecord> =
            self.records.read(&attempt_leaf(self.record.terminal))?;
        self.key.verify("runtime_release_attempt", &signed)?;
        if self.record.attempts.last()
            != Some(&(self.record.terminal, canonical_digest(1, &signed.payload)?))
            || signed.payload.attempt.plan != self.terminal.plan
        {
            return Err(invalid(
                "carry origin is not the exact completed terminal attempt",
            ));
        }
        Ok((
            signed.payload.parent_publication.id,
            signed.payload.acceptance_publication.id,
        ))
    }
    pub(crate) fn terminal(&self) -> &ReleaseAttempt {
        &self.terminal
    }
    pub(crate) fn digest(&self) -> &str {
        &self.signed_digest
    }
    pub(crate) fn work(&self) -> &WorkIdentity {
        &self.work
    }
    pub(crate) fn workspace_id(&self) -> Uuid {
        self.record.workspace
    }
    pub(crate) fn dependency_repositories(&self) -> Result<&BTreeSet<RepositoryId>> {
        self.record
            .dependency_binding
            .as_ref()
            .map(|binding| &binding.repositories)
            .ok_or_else(|| {
                Error::new(
                    "dependency_binding_unavailable",
                    "legacy completion does not retain the originally observed provider identities",
                )
            })
    }
    /// Historical dependency credit does not require the old generation or
    /// deployed pointer to remain current. It does require every original
    /// immutable reference and the same configured provider identities.
    pub(crate) fn validate_for_configuration(
        &self,
        config: &Config,
    ) -> Result<domain::SuccessfulRelease> {
        self.validate()?;
        let binding = self.record.dependency_binding.as_ref().ok_or_else(|| {
            Error::new(
                "dependency_binding_unavailable",
                "legacy completion does not retain the originally observed provider identities",
            )
        })?;
        if config.runtime.id != self.record.runtime
            || DependencyBinding::configured(
                config,
                self.record.workspace,
                binding.repositories.clone(),
            )? != *binding
            || self.work.linear_endpoint != binding.linear.endpoint
            || self.work.linear_workspace != binding.linear_workspace
        {
            return Err(invalid(
                "completed release provider or workspace identity changed",
            ));
        }
        let origin = self
            .record
            .origins
            .get(&self.record.terminal)
            .ok_or_else(|| invalid("completed release lacks its original publication identity"))?;
        for id in &origin.receipts {
            let receipt = self.control.operation(*id)?;
            if receipt.expected.get("provider_writer") != Some(&binding.linear.account) {
                return Err(invalid(
                    "completed release publication writer differs from its binding",
                ));
            }
        }
        ReleaseQueue::default().record_success(&self.terminal)
    }
}

/// Read-only presentation. These partial diagnostics are never accepted by
/// queue, baseline, recovery, or permit validation.
pub(in crate::runtime_delivery::release) struct CompletionInspection {
    pub(in crate::runtime_delivery::release) consumed: BTreeMap<Uuid, Uuid>,
    pub(in crate::runtime_delivery::release) errors: BTreeMap<Uuid, Error>,
    pub(in crate::runtime_delivery::release) terminals: BTreeMap<Uuid, VerifiedReleaseCompletion>,
}
impl RuntimeDelivery {
    pub(in crate::runtime_delivery::release) fn inspect_release_completions(
        &self,
        attempts: &BTreeMap<Uuid, AttemptRecord>,
        ownership_errors: &BTreeMap<Uuid, Error>,
    ) -> Result<CompletionInspection> {
        let (cached, mut errors) = {
            let state = self.releases.state.lock().map_err(|_| unavailable())?;
            (state.completions.clone(), state.completion_errors.clone())
        };
        errors.extend(ownership_errors.clone());
        let mut results = BTreeMap::new();
        let mut referenced = BTreeMap::<Uuid, BTreeSet<Uuid>>::new();
        for (terminal, proof) in cached {
            if !attempts.contains_key(&terminal) {
                return Err(invalid(
                    "cached completion lost its authenticated terminal attempt",
                ));
            }
            referenced.insert(
                terminal,
                proof.record.attempts.iter().map(|(id, _)| *id).collect(),
            );
            let result = proof.validate().map(|_| proof.clone());
            results.insert(terminal, result);
        }
        for name in self.records.records_bounded(65536)? {
            let Some(id) = name
                .strip_prefix("release-completion-")
                .and_then(|s| s.strip_suffix(".json"))
            else {
                continue;
            };
            let terminal: Uuid = id
                .parse()
                .map_err(|_| invalid("completion filename identity is invalid"))?;
            if !attempts.contains_key(&terminal) {
                return Err(invalid(
                    "completion lacks its authenticated terminal attempt",
                ));
            }
            if results.contains_key(&terminal) {
                continue;
            }
            let result = self
                .records
                .read_bounded::<Signed<CompletionRecord>>(&name, 1024 * 1024)
                .and_then(|signed| {
                    self.runtime.services.key.verify(PURPOSE, &signed)?;
                    if signed.payload.terminal != terminal
                        || signed.payload.attempts.len() > MAX_RELEASE_LINKS
                    {
                        return Err(invalid("completion diagnostic identity or count differs"));
                    }
                    referenced.insert(
                        terminal,
                        signed.payload.attempts.iter().map(|(id, _)| *id).collect(),
                    );
                    let proof = VerifiedReleaseCompletion::recover(self, signed)?;
                    proof.validate()?;
                    Ok(proof)
                });
            results.insert(terminal, result);
            if results.len() > MAX_REFERENCES {
                return Err(invalid("completion inspection exceeds reference bound"));
            }
        }
        // A missing completion file cannot hide unresolved predecessor holds on
        // a cold inspection. Build the reverse relation once, then bound each
        // diagnostic traversal independently of the full attempt inventory.
        let mut predecessors = BTreeMap::<Uuid, Vec<Uuid>>::new();
        for (id, record) in attempts {
            if let Some(successor) = record.attempt.assigned_successor {
                predecessors.entry(successor).or_default().push(*id);
            }
        }
        for (terminal, record) in attempts {
            if record.attempt.completed() && record.local_retired && !results.contains_key(terminal)
            {
                let mut error = invalid("terminal release has no retained completion reference");
                let mut ids = BTreeSet::new();
                let mut pending = vec![*terminal];
                while let Some(id) = pending.pop() {
                    if !ids.insert(id) {
                        error = invalid("completion diagnostic ancestry is cyclic");
                        break;
                    }
                    if ids.len() > MAX_RELEASE_LINKS {
                        error = invalid("completion diagnostic ancestry exceeds bound");
                        break;
                    }
                    if let Some(prior) = predecessors.get(&id) {
                        if prior.len() > 1 {
                            error = invalid(
                                "completion diagnostic ancestry has ambiguous predecessors",
                            );
                            break;
                        }
                        pending.extend(prior.iter().copied());
                    }
                }
                results.insert(*terminal, Err(error));
                referenced.insert(*terminal, ids);
            }
        }
        let mut valid = BTreeMap::<Uuid, BTreeSet<Uuid>>::new();
        let mut terminals = BTreeMap::new();
        for (terminal, result) in results {
            let result = result.and_then(|proof| {
                for (id, digest) in &proof.record.attempts {
                    if ownership_errors.contains_key(id) {
                        return Err(invalid("completion has an unverified owner reference"));
                    }
                    if attempts
                        .get(id)
                        .map(|r| canonical_digest(1, r))
                        .transpose()?
                        .as_ref()
                        != Some(digest)
                    {
                        return Err(invalid(
                            "attempt inventory changed during completion inspection",
                        ));
                    }
                }
                Ok(proof)
            });
            match result {
                Ok(proof) => {
                    errors.remove(&terminal);
                    valid.insert(terminal, proof.holds.keys().copied().collect());
                    terminals.insert(terminal, proof);
                }
                Err(error) if error.code == "unsafe_storage" => return Err(error),
                Err(error) => {
                    errors.insert(terminal, completion_context(error, terminal, "inspection", PURPOSE)
                        .next("Inspect the retained release and restore or reconcile its exact missing/changed evidence; do not repeat its external effects."));
                }
            }
        }
        let mut claims = BTreeMap::<Uuid, BTreeSet<Uuid>>::new();
        for (terminal, ancestors) in &valid {
            for ancestor in ancestors {
                claims.entry(*ancestor).or_default().insert(*terminal);
            }
        }
        for terminals in claims.values().filter(|terminals| terminals.len() > 1) {
            for terminal in terminals {
                errors.insert(
                    *terminal,
                    completion_context(
                        invalid("ancestor has multiple terminal completions"),
                        *terminal,
                        "inspection",
                        PURPOSE,
                    ),
                );
            }
        }
        let mut consumed = BTreeMap::new();
        for (terminal, ancestors) in valid {
            if !errors.contains_key(&terminal) {
                for ancestor in ancestors {
                    consumed.insert(ancestor, terminal);
                }
            }
        }
        for (terminal, ids) in referenced {
            if let Some(error) = errors.get(&terminal).cloned() {
                for id in ids {
                    if attempts.contains_key(&id) {
                        errors.entry(id).or_insert_with(|| error.clone());
                    }
                }
            }
        }
        terminals.retain(|terminal, _| !errors.contains_key(terminal));
        Ok(CompletionInspection {
            consumed,
            errors,
            terminals,
        })
    }
}

impl RuntimeDelivery {
    /// Run outside both the Control callback and the release-state lock. Cold
    /// restoration of one broken historical fact must not hide unrelated facts.
    pub(in crate::runtime_delivery) fn sync_released_completions(&self) -> Result<()> {
        // Authenticate protective scope, read dependencies and replace both
        // projections under one gate. An older inventory cannot install after
        // another facade has observed a newer fatal failure.
        self.runtime.control.serialize_observation(|_| {
            self.runtime.registration.verify_control_owner()?;
            let result = (|| {
                self.sync_release_completion_queue_in_observation()?;
                let (proofs, errors) = self.released_completion_inventory()?;
                self.runtime
                    .replace_released_completions_in_observation(proofs, errors)
            })();
            if let Err(error) = &result {
                self.runtime.invalidate_released_inventory(error.clone())?;
                self.runtime
                    .replace_released_completions_in_observation(Vec::new(), BTreeMap::new())?;
                let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
                state.completion_inventory_error = Some(error.clone());
                state.queue.invalidate_after_rebuild(error.clone());
            }
            result
        })
    }
    // Caller owns Control and no release/runtime state guard. Failure-only;
    // a later complete successful inventory is required to restore availability.
    fn latch_completion_inventory_error_in_observation(&self, error: &Error) -> Result<()> {
        self.runtime.invalidate_released_inventory(error.clone())?;
        let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
        state.completion_inventory_error = Some(error.clone());
        state.queue.invalidate_after_rebuild(error.clone());
        Ok(())
    }
    fn released_completion_inventory(
        &self,
    ) -> Result<(Vec<VerifiedReleaseCompletion>, BTreeMap<Uuid, Error>)> {
        self.released_completion_inventory_from_names(self.records.records_bounded(65536)?)
    }
    // Production supplies its actual bounded enumeration. Keeping that read
    // separate also makes disappearance between enumeration and open testable.
    pub(in crate::runtime_delivery::release) fn released_completion_inventory_from_names(
        &self,
        names: Vec<String>,
    ) -> Result<(Vec<VerifiedReleaseCompletion>, BTreeMap<Uuid, Error>)> {
        if names.len() > 65536 {
            return Err(invalid("completion storage enumeration exceeds bound"));
        }
        let known: BTreeMap<_, _> = self
            .runtime
            .known_release_completion_anchors()?
            .into_iter()
            .map(|anchor| (anchor.operation(), anchor))
            .collect();
        let cached = {
            let state = self.releases.state.lock().map_err(|_| unavailable())?;
            state.completions.clone()
        };
        let mut records = BTreeMap::new();
        let mut errors = BTreeMap::new();
        for name in names {
            let Some(id) = name
                .strip_prefix("release-completion-")
                .and_then(|s| s.strip_suffix(".json"))
            else {
                continue;
            };
            let id: Uuid = id
                .parse()
                .map_err(|_| invalid("completion filename identity is invalid"))?;
            if leaf(id) != name || records.len() + errors.len() >= MAX_REFERENCES {
                return Err(invalid("completion filename or retained count differs"));
            }
            let signed: Signed<CompletionRecord> =
                match self.records.read_bounded(&name, 1024 * 1024) {
                    Ok(signed) => signed,
                    Err(error) if error.code == "record_missing" => {
                        errors.insert(id, error);
                        continue;
                    }
                    Err(error) => return Err(error),
                };
            self.runtime.services.key.verify(PURPOSE, &signed)?;
            if signed.payload.terminal != id || records.len() + errors.len() >= MAX_REFERENCES {
                return Err(invalid("completion filename or retained count differs"));
            }
            records.insert(id, signed);
        }
        let mut proofs = Vec::new();
        for (id, signed) in records {
            let result = (|| {
                if let Some(prior) = known.get(&id)
                    && prior.digest() != signed.digest()?
                {
                    return Err(invalid(
                        "previously authenticated completion reference was replaced",
                    ));
                }
                if let Some(prior) = cached.get(&id)
                    && prior.digest() != signed.digest()?
                {
                    return Err(invalid("cached completion reference was replaced"));
                }
                let proof = VerifiedReleaseCompletion::recover(self, signed)?;
                proof.validate()?;
                Ok(proof)
            })();
            match result {
                Ok(proof) => proofs.push(proof),
                Err(error) if error.code == "unsafe_storage" => return Err(error),
                Err(error) => {
                    errors.insert(id, error);
                }
            }
        }
        for (id, proof) in cached {
            if !proofs.iter().any(|p| p.terminal().plan.operation_id == id)
                && !errors.contains_key(&id)
            {
                errors.insert(
                    id,
                    proof
                        .validate()
                        .err()
                        .unwrap_or_else(|| invalid("completion disappeared during inventory")),
                );
            }
        }
        for id in known.keys() {
            if !proofs
                .iter()
                .any(|proof| proof.terminal().plan.operation_id == *id)
                && !errors.contains_key(id)
            {
                errors.insert(
                    *id,
                    Error::new(
                        "record_missing",
                        "known release completion is absent from recovered inventory",
                    ),
                );
            }
        }
        if proofs.len() + errors.len() > MAX_REFERENCES {
            return Err(invalid("completion inventory exceeds reference bound"));
        }
        Ok((proofs, errors))
    }
    /// Queue restoration separates authenticated protective scope from usable
    /// completion. No scope can be inferred from a filename, raw attempt, or
    /// caller-supplied error. Unauthenticated/unknown inventory remains global.
    #[cfg(test)]
    pub(in crate::runtime_delivery::release) fn release_completion_queue_inventory(
        &self,
        known: &BTreeMap<Uuid, ReleaseCompletionAnchor>,
    ) -> Result<ReleaseCompletionQueueInventory> {
        self.read_release_completion_queue_inventory(known, |_| Ok(()))
    }
    /// The caller holds Control throughout this bounded local read. Remember
    /// every complete authenticated scope immediately, even if a later entry
    /// makes the complete inventory unavailable. This grants no outcome credit.
    pub(in crate::runtime_delivery::release) fn learn_release_completion_queue_inventory(
        &self,
        known: &BTreeMap<Uuid, ReleaseCompletionAnchor>,
    ) -> Result<ReleaseCompletionQueueInventory> {
        self.read_release_completion_queue_inventory(known, |anchor| {
            #[cfg(test)]
            self.fixture_before_first_completion_anchor(anchor.operation())?;
            self.runtime
                .remember_release_completion_anchors(vec![anchor.clone()])
        })
    }
    fn read_release_completion_queue_inventory(
        &self,
        known: &BTreeMap<Uuid, ReleaseCompletionAnchor>,
        mut remember: impl FnMut(&ReleaseCompletionAnchor) -> Result<()>,
    ) -> Result<ReleaseCompletionQueueInventory> {
        if known.len() > MAX_REFERENCES {
            return Err(invalid("known completion anchors exceed bound"));
        }
        let runtime = self.runtime.configuration.snapshot()?.runtime.id;
        let mut anchors = known.clone();
        for (operation, anchor) in known {
            if *operation != anchor.operation() || anchor.runtime_id() != runtime {
                return Err(invalid("known completion anchor identity differs"));
            }
            anchor.queue_scope()?;
            remember(anchor)?;
        }
        let mut seen = BTreeSet::new();
        let mut proofs = BTreeMap::new();
        let mut errors = BTreeMap::new();
        for name in self.records.records_bounded(65536)? {
            let Some(id) = name
                .strip_prefix("release-completion-")
                .and_then(|s| s.strip_suffix(".json"))
            else {
                continue;
            };
            let operation: Uuid = id
                .parse()
                .map_err(|_| invalid("completion filename identity is invalid"))?;
            if leaf(operation) != name || !seen.insert(operation) || seen.len() > MAX_REFERENCES {
                return Err(invalid("completion filename or count differs"));
            }
            let signed: Signed<CompletionRecord> = match self
                .records
                .read_bounded(&name, 1024 * 1024)
            {
                Ok(signed) => signed,
                Err(error) if error.code == "record_missing" && known.contains_key(&operation) => {
                    errors.insert(operation, error);
                    continue;
                }
                Err(error) => return Err(error),
            };
            // A present unauthenticated seal is not downgraded to a scoped error.
            self.runtime.services.key.verify(PURPOSE, &signed)?;
            if signed.payload.terminal != operation || signed.payload.runtime != runtime {
                return Err(invalid("completion filename/runtime identity differs"));
            }
            if let Some(prior) = known.get(&operation) {
                if prior.digest() != signed.digest()? {
                    errors.insert(
                        operation,
                        invalid("previously authenticated completion seal changed"),
                    );
                    continue;
                }
            } else {
                // Cold restoration needs every signed scope-bearing attempt.
                // Without any one of them there is no safe complete resource map.
                let anchor = ReleaseCompletionAnchor::from_signed(self, &signed)?;
                remember(&anchor)?;
                anchors.insert(operation, anchor);
            }
            let recovered = VerifiedReleaseCompletion::recover(self, signed).and_then(|proof| {
                proof.validate()?;
                if proof.queue_anchor()?.queue_scope()? != anchors[&operation].queue_scope()? {
                    return Err(invalid("completion scope changed during recovery"));
                }
                Ok(proof)
            });
            match recovered {
                Ok(proof) => {
                    proofs.insert(operation, proof);
                }
                Err(error) if error.code == "unsafe_storage" => return Err(error),
                Err(error) => {
                    errors.insert(operation, error);
                }
            }
        }
        for operation in known.keys() {
            if !seen.contains(operation) {
                errors.insert(
                    *operation,
                    Error::new(
                        "record_missing",
                        "previously authenticated completion seal is absent",
                    ),
                );
            }
        }
        if anchors.len() > MAX_REFERENCES {
            return Err(invalid("completion anchor inventory exceeds bound"));
        }
        Ok(ReleaseCompletionQueueInventory {
            proofs,
            anchors,
            errors,
        })
    }
    async fn observe_completion(
        &self,
        workspace: &WorkspaceDelivery,
        record: &AttemptRecord,
    ) -> Result<Option<CompletionRecord>> {
        validate_completed_owner(&self.runtime.control, record)?;
        let origin = workspace
            .coordinator
            .release_origin(
                &record.attempt.plan,
                record.parent_publication.id,
                record.acceptance_publication.id,
            )
            .await?;
        let effects = self.observe_lineage_effects(workspace, record).await?;
        let lineage = self
            .release_lineage_inner(workspace, record, origin.clone(), effects.clone(), true)
            .await?;
        let mut baselines = Vec::new();
        let mut seen = BTreeSet::new();
        for baseline in &origin.manifest().baselines {
            if !seen.insert(&baseline.repository) {
                return Err(invalid("duplicate completion baseline repository"));
            }
            let contract = workspace
                .contracts
                .iter()
                .find(|c| c.repository == baseline.repository)
                .ok_or_else(|| {
                    invalid("completion baseline repository is not currently configured")
                })?;
            let current = workspace.observer.observe_recovery(contract).await?;
            if !current.coherent() {
                return Err(invalid("terminal production inventory is not coherent"));
            }
            baselines.push(current);
        }
        super::super::baseline::verify_known_baseline(&origin, &baselines, &effects)?;
        lineage.revalidate(
            &self.records,
            &self.runtime.control,
            &self.runtime.services.key,
        )?;
        let mut receipts: BTreeMap<_, _> = lineage.receipts.iter().cloned().collect();
        let mut files = Vec::new();
        let mut origins = BTreeMap::new();
        let mut target_proofs = BTreeMap::new();
        for link in &lineage.links {
            for receipt in
                closed_owner_references(&self.runtime.control, &link.record).map_err(|error| {
                    completion_context(
                        error,
                        link.record.attempt.plan.operation_id,
                        "owner_references",
                        "release_recovery",
                    )
                })?
            {
                retain_receipt_reference(&mut receipts, &receipt)?;
            }
            let (publications, vector) = link.origin.retained_references();
            for receipt in publications {
                retain_receipt_reference(&mut receipts, receipt)?;
            }
            origins.insert(
                link.record.attempt.plan.operation_id,
                OriginReference {
                    receipts: publications.iter().map(|r| r.operation_id).collect(),
                    carried: link.origin.carried_reference().cloned(),
                    vector: ProofFile {
                        leaf: vector.0.clone(),
                        purpose: vector.1.clone(),
                        digest: vector.2.clone(),
                    },
                },
            );
            for effect in link.effects.values() {
                if let Some(target) = effect.historical_target() {
                    let contract = workspace
                        .contracts
                        .iter()
                        .find(|contract| contract.repository == target.target().repository)
                        .ok_or_else(|| invalid("historical target contract disappeared"))?;
                    let history = workspace
                        .observer
                        .historical_references(contract, target)
                        .map_err(|error| {
                            completion_context(
                                error,
                                link.record.attempt.plan.operation_id,
                                "target_history",
                                "target_deployment",
                            )
                        })?;
                    for receipt in history.receipts {
                        retain_receipt_reference(&mut receipts, &receipt)?;
                    }
                    for (leaf, purpose, digest) in history.files {
                        let reference = ProofFile {
                            leaf: leaf.clone(),
                            purpose,
                            digest,
                        };
                        if let Some(prior) = target_proofs.get(&leaf)
                            && canonical_digest(1, prior)? != canonical_digest(1, &reference)?
                        {
                            return Err(invalid(
                                "target proof changed during completion observation",
                            ));
                        }
                        target_proofs.insert(leaf, reference);
                    }
                } else if let Some(external) = effect.historical_external() {
                    if !matches!(
                        effect.effect(),
                        Some(domain::ObservedEffect::Verification { .. })
                    ) {
                        return Err(invalid(
                            "external observation cannot establish a deployment effect",
                        ));
                    }
                    let (leaf, purpose, digest) = external.reference().proof_file();
                    let reference = ProofFile {
                        leaf: leaf.clone(),
                        purpose,
                        digest,
                    };
                    if target_proofs
                        .get(&leaf)
                        .is_some_and(|prior: &ProofFile| prior.digest != reference.digest)
                    {
                        return Err(invalid(
                            "external observation proof changed during completion",
                        ));
                    }
                    target_proofs.insert(leaf, reference);
                } else if matches!(
                    effect.effect(),
                    Some(
                        domain::ObservedEffect::Deployment { .. }
                            | domain::ObservedEffect::Verification { .. }
                    )
                ) {
                    return Err(invalid(
                        "successful target lacks its immutable historical proof",
                    ));
                }
            }
            let history = workspace
                .coordinator
                .release_history_references(&link.record.attempt)
                .map_err(|error| {
                    completion_context(
                        error,
                        link.record.attempt.plan.operation_id,
                        "step_history",
                        "release_effect",
                    )
                })?;
            for receipt in history.receipts {
                retain_receipt_reference(&mut receipts, &receipt)?;
            }
            for (leaf, purpose, digest) in history.files {
                files.push(ProofFile {
                    leaf,
                    purpose,
                    digest,
                });
            }
        }
        if receipts.len() > MAX_REFERENCES
            || files.len() > MAX_REFERENCES
            || target_proofs.len() > MAX_REFERENCES
        {
            return Err(invalid("completion effect inventory exceeds bound"));
        }
        let dependency_binding = DependencyBinding::configured(
            &workspace.configuration,
            record.workspace,
            origin
                .manifest()
                .baselines
                .iter()
                .map(|baseline| baseline.repository.clone())
                .collect(),
        )?;
        let record = CompletionRecord {
            schema_version: if target_proofs
                .values()
                .any(|proof| proof.purpose == "external_target_baseline")
            {
                3
            } else {
                2
            },
            runtime: record.runtime,
            workspace: record.workspace,
            config_revision: crate::config::config_revision(&workspace.configuration)?,
            terminal: record.attempt.plan.operation_id,
            attempts: lineage
                .links
                .iter()
                .map(|link| (link.record.attempt.plan.operation_id, link.digest.clone()))
                .collect(),
            receipts,
            intents: lineage.intents.iter().cloned().collect(),
            effects: files,
            origins,
            target_proofs: target_proofs.into_values().collect(),
            inventory_digest: canonical_digest(
                1,
                &baselines
                    .iter()
                    .map(|b| {
                        (
                            b.repository(),
                            b.production(),
                            b.targets(),
                            b.contract_digest(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )?,
            observed_at: Utc::now(),
            dependency_binding: Some(dependency_binding),
        };
        Ok(Some(record))
    }
    pub async fn complete_release_chains(&self, projection: &RuntimeProjection) -> Result<()> {
        self.check_release_completions(projection)
            .await?
            .map_or(Ok(()), Err)
    }
    pub(in crate::runtime_delivery::release) async fn maintain_release_completions(
        &self,
        projection: &RuntimeProjection,
    ) -> Result<()> {
        // Per-chain proof failures stay visible on that historical row. The
        // existing ancestor resource holds still reject overlapping work.
        self.check_release_completions(projection).await.map(|_| ())
    }
    async fn check_release_completions(
        &self,
        projection: &RuntimeProjection,
    ) -> Result<Option<Error>> {
        let candidates = {
            let state = self.releases.state.lock().map_err(|_| unavailable())?;
            state
                .attempts
                .values()
                .filter(|r| {
                    r.attempt.completed()
                        && r.local_retired
                        && !state.completions.contains_key(&r.attempt.plan.operation_id)
                })
                .cloned()
                .collect::<Vec<_>>()
        };
        let mut first = None;
        for record in candidates {
            let result = self.complete_release_chain(projection, &record).await;
            let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
            match result {
                Err(error) => {
                    state
                        .completion_errors
                        .insert(record.attempt.plan.operation_id, error.clone());
                    first.get_or_insert(error);
                }
                Ok(()) => {
                    state
                        .completion_errors
                        .remove(&record.attempt.plan.operation_id);
                }
            }
        }
        self.sync_released_completions()?;
        Ok(first)
    }
    async fn complete_release_chain(
        &self,
        projection: &RuntimeProjection,
        record: &AttemptRecord,
    ) -> Result<()> {
        let existing = self.runtime.control.serialize_observation(|_| {
            self.runtime.registration.verify_control_owner()?;
            let runtime_known = self
                .runtime
                .known_release_completion_anchors()?
                .into_iter()
                .find(|anchor| anchor.operation() == record.attempt.plan.operation_id);
            let known = self
                .releases
                .state
                .lock()
                .map_err(|_| unavailable())?
                .completion_anchors
                .get(&record.attempt.plan.operation_id)
                .cloned();
            if let (Some(local), Some(runtime)) = (&known, &runtime_known)
                && local.queue_scope()? != runtime.queue_scope()?
            {
                return Err(invalid("delivery/runtime completion anchors disagree"));
            }
            let known = known.or(runtime_known);
            let signed = match self
                .records
                .read::<Signed<CompletionRecord>>(&leaf(record.attempt.plan.operation_id))
            {
                Ok(signed) => signed,
                Err(error) if error.code == "record_missing" && known.is_none() => {
                    return Ok(None);
                }
                Err(error) if error.code == "record_missing" => return Err(error),
                Err(error) => {
                    self.latch_completion_inventory_error_in_observation(&error)?;
                    return Err(error);
                }
            };
            let envelope = (|| {
                self.runtime.services.key.verify(PURPOSE, &signed)?;
                if signed.payload.terminal != record.attempt.plan.operation_id
                    || signed.payload.runtime != record.attempt.plan.runtime_id
                {
                    return Err(invalid("completion filename/runtime identity differs"));
                }
                Ok(())
            })();
            if let Err(error) = envelope {
                self.latch_completion_inventory_error_in_observation(&error)?;
                return Err(error);
            }
            let anchor = match known {
                Some(anchor) => {
                    if anchor.digest() != signed.digest()? {
                        return Err(invalid("previously authenticated completion seal changed"));
                    }
                    anchor
                }
                None => match ReleaseCompletionAnchor::from_signed(self, &signed) {
                    Ok(anchor) => anchor,
                    Err(error) => {
                        // Without complete authenticated scope, this first
                        // observed seal cannot be confined to one resource set.
                        self.latch_completion_inventory_error_in_observation(&error)?;
                        return Err(error);
                    }
                },
            };
            self.runtime
                .remember_release_completion_anchors(vec![anchor])?;
            let recovered = VerifiedReleaseCompletion::recover(self, signed).and_then(|proof| {
                proof.validate()?;
                Ok(proof)
            });
            if let Err(error) = &recovered
                && error.code == "unsafe_storage"
            {
                self.latch_completion_inventory_error_in_observation(error)?;
            }
            recovered.map(Some)
        })?;
        if let Some(proof) = existing {
            self.commit_completion(proof, None)?;
            return Ok(());
        }
        let workspace = self.workspace(projection, record.workspace)?;
        let Some(completion) =
            self.observe_completion(&workspace, record)
                .await
                .map_err(|error| {
                    completion_context(
                        error,
                        record.attempt.plan.operation_id,
                        "observe_completion",
                        PURPOSE,
                    )
                })?
        else {
            return Ok(());
        };
        self.persist_completion(completion).map_err(|error| {
            completion_context(
                error,
                record.attempt.plan.operation_id,
                "persist_completion",
                PURPOSE,
            )
        })?;
        Ok(())
    }
    fn persist_completion(&self, record: CompletionRecord) -> Result<()> {
        let signed = self.runtime.services.key.sign(PURPOSE, &record)?;
        let proof = VerifiedReleaseCompletion::recover(self, signed.clone())?;
        self.commit_completion(proof, Some(signed))
    }
    /// Test-only delayed call through the real producer. It captures only an
    /// already authenticated, fully recovered seal; no raw claim can mint a cap.
    #[cfg(test)]
    pub(crate) fn prepare_fixture_completion_commit(
        &self,
        operation: Uuid,
    ) -> Result<impl FnOnce() -> Result<()> + '_> {
        let signed: Signed<CompletionRecord> = self.records.read(&leaf(operation))?;
        if signed.payload.terminal != operation {
            return Err(invalid("fixture completion identity differs"));
        }
        let proof = VerifiedReleaseCompletion::recover(self, signed.clone())?;
        proof.validate()?;
        Ok(move || self.commit_completion(proof, Some(signed)))
    }
    fn commit_completion(
        &self,
        proof: VerifiedReleaseCompletion,
        fresh: Option<Signed<CompletionRecord>>,
    ) -> Result<()> {
        let expected_anchor = proof.queue_anchor()?;
        if let Some(known) = self
            .runtime
            .known_release_completion_anchors()?
            .into_iter()
            .find(|anchor| anchor.operation() == expected_anchor.operation())
            && known.queue_scope()? != expected_anchor.queue_scope()?
        {
            return Err(invalid("known immutable completion anchor changed"));
        }
        #[cfg(test)]
        self.fixture_before_completion_commit(proof.terminal().plan.operation_id)?;
        self.runtime.control.serialize_observation(|_| {
            self.runtime.registration.verify_control_owner()?;
            // Recheck inside the serialized boundary, before any write or
            // release-state lock. A different facade may have learned the same
            // immutable identity after this operation's preflight read.
            let runtime_known = self
                .runtime
                .known_release_completion_anchors()?
                .into_iter()
                .find(|anchor| anchor.operation() == expected_anchor.operation());
            if let Some(known) = &runtime_known
                && known.queue_scope()? != expected_anchor.queue_scope()?
            {
                return Err(invalid("known immutable completion anchor changed"));
            }
            self.runtime.require_released_inventory_available()?;
            self.runtime.require_no_release_inventory_available()?;
            let record = &proof.record;
            if fresh.is_some()
                && (Utc::now() - record.observed_at > Duration::seconds(30)
                    || crate::config::config_revision(&self.runtime.configuration.snapshot()?)?
                        != record.config_revision)
            {
                return Err(invalid(
                    "initial completion observation/configuration changed before persistence",
                ));
            }
            if fresh.is_some() {
                self.runtime
                    .configuration
                    .authority_fences()?
                    .check_workspace(record.workspace)?;
            }
            if let Err(error) = proof.validate_references() {
                if error.code == "unsafe_storage" {
                    self.latch_completion_inventory_error_in_observation(&error)?;
                }
                return Err(error);
            }
            let check_attempts = |state: &ReleaseState| -> Result<()> {
                for (operation, digest) in &record.attempts {
                    if state
                        .attempts
                        .get(operation)
                        .map(|r| canonical_digest(1, r))
                        .transpose()?
                        .as_ref()
                        != Some(digest)
                    {
                        return Err(invalid(
                            "completion attempt changed before serialized commit",
                        ));
                    }
                }
                Ok(())
            };
            let state = self.releases.state.lock().map_err(|_| unavailable())?;
            if fresh.is_some() {
                if let Some(error) = &state.completion_inventory_error {
                    return Err(error.clone());
                }
                state.historical_no_release.require_available()?;
            }
            check_attempts(&state)?;
            let local_known = state.completion_anchors.get(&record.terminal);
            if let Some(known) = local_known
                && known.queue_scope()? != expected_anchor.queue_scope()?
            {
                return Err(invalid("retained immutable completion anchor changed"));
            }
            let already_known = runtime_known.is_some() || local_known.is_some();
            // Control remains held; release the facade guard before a failed
            // protected read may latch shared RuntimeState and local queue state.
            drop(state);
            if let Some(signed) = &fresh {
                match self
                    .records
                    .read::<Signed<CompletionRecord>>(&leaf(record.terminal))
                {
                    Ok(prior) => {
                        let envelope = (|| {
                            self.runtime.services.key.verify(PURPOSE, &prior)?;
                            if prior.payload.terminal != record.terminal
                                || prior.payload.runtime != record.runtime
                                || prior.payload.workspace != record.workspace
                            {
                                return Err(invalid(
                                    "current completion envelope identity changed",
                                ));
                            }
                            Ok(())
                        })();
                        if let Err(error) = envelope {
                            self.latch_completion_inventory_error_in_observation(&error)?;
                            return Err(error);
                        }
                        if prior.digest()? != signed.digest()? {
                            return Err(invalid("immutable completion already differs"));
                        }
                    }
                    Err(error) if error.code == "record_missing" && !already_known => {
                        if let Err(error) = self.records.write(&leaf(record.terminal), signed) {
                            if error.code == "unsafe_storage" {
                                self.latch_completion_inventory_error_in_observation(&error)?;
                            }
                            return Err(error);
                        }
                    }
                    Err(error) if error.code == "record_missing" => return Err(error),
                    Err(error) => {
                        self.latch_completion_inventory_error_in_observation(&error)?;
                        return Err(error);
                    }
                }
            } else {
                if let Err(error) = proof.validate_queue_envelope() {
                    self.latch_completion_inventory_error_in_observation(&error)?;
                    return Err(error);
                }
                if let Err(error) = proof.validate() {
                    if error.code == "unsafe_storage" {
                        self.latch_completion_inventory_error_in_observation(&error)?;
                    }
                    return Err(error);
                }
            }
            // The exact seal now exists or was reread successfully. Preserve
            // its immutable identity before releasing Control. Remembering is
            // not completion credit; repeat exact attempt checks after taking
            // the facade guard again.
            self.runtime
                .remember_release_completion_anchors(vec![expected_anchor.clone()])?;
            let mut state = self.releases.state.lock().map_err(|_| unavailable())?;
            check_attempts(&state)?;

            let mut completions = state.completions.clone();
            completions.insert(record.terminal, proof.clone());
            let anchor = proof.queue_anchor()?;
            if let Some(prior) = state.completion_anchors.get(&record.terminal)
                && prior.queue_scope()? != anchor.queue_scope()?
            {
                return Err(invalid("retained immutable completion anchor changed"));
            }
            let mut anchors = state.completion_anchors.clone();
            anchors.insert(record.terminal, anchor);
            let mut errors = state.completion_errors.clone();
            errors.remove(&record.terminal);
            let queue = match rebuild_queue(
                &state.attempts,
                &state.no_release,
                &completions,
                &anchors,
                &errors,
                state.completion_inventory_error.as_ref(),
                &state.historical_no_release,
            ) {
                Ok(queue) => queue,
                Err(error) => {
                    // This rebuild has already separated scoped reference loss
                    // into quarantine. A remaining global failure must reach
                    // every facade before this serialized commit returns.
                    drop(state);
                    self.latch_completion_inventory_error_in_observation(&error)?;
                    return Err(error);
                }
            };
            state.completion_anchors = anchors;
            state.completion_errors = errors;
            state.completions = completions;
            state.queue = queue;
            Ok(())
        })
    }
}

#[cfg(test)]
impl RuntimeDelivery {
    pub(in crate::runtime_delivery::release) fn fixture_pause_first_completion_anchor(
        &self,
        operation: Uuid,
        observed: std::sync::mpsc::SyncSender<()>,
        resume: std::sync::mpsc::Receiver<()>,
    ) -> Result<()> {
        if self
            .runtime
            .known_release_completion_anchors()?
            .iter()
            .any(|a| a.operation() == operation)
            || self
                .releases
                .state
                .lock()
                .map_err(|_| unavailable())?
                .completion_anchors
                .contains_key(&operation)
        {
            return Err(invalid(
                "fixture reader must begin without the completion anchor",
            ));
        }
        let mut hooks = self
            .releases
            .completion_fixture_hooks
            .lock()
            .map_err(|_| unavailable())?;
        if hooks.first_anchor.is_some() {
            return Err(invalid("fixture first-reader hook is already armed"));
        }
        hooks.first_anchor = Some(FixtureFirstCompletionAnchor {
            operation,
            observed,
            resume,
        });
        Ok(())
    }
    pub(in crate::runtime_delivery::release) fn fixture_signal_completion_commit(
        &self,
        operation: Uuid,
        entered: std::sync::mpsc::SyncSender<()>,
    ) -> Result<()> {
        let mut hooks = self
            .releases
            .completion_fixture_hooks
            .lock()
            .map_err(|_| unavailable())?;
        if hooks.commit_gate.is_some() {
            return Err(invalid("fixture commit hook is already armed"));
        }
        hooks.commit_gate = Some((operation, entered));
        Ok(())
    }
    fn fixture_before_first_completion_anchor(&self, operation: Uuid) -> Result<()> {
        let hook = {
            let mut hooks = self
                .releases
                .completion_fixture_hooks
                .lock()
                .map_err(|_| unavailable())?;
            if hooks
                .first_anchor
                .as_ref()
                .is_some_and(|hook| hook.operation == operation)
            {
                hooks.first_anchor.take()
            } else {
                None
            }
        };
        if let Some(hook) = hook {
            hook.observed
                .send(())
                .map_err(|_| invalid("fixture first-reader observer disconnected"))?;
            hook.resume
                .recv_timeout(std::time::Duration::from_secs(10))
                .map_err(|_| invalid("fixture first-reader release timed out or disconnected"))?;
        }
        Ok(())
    }
    fn fixture_before_completion_commit(&self, operation: Uuid) -> Result<()> {
        let signal = {
            let mut hooks = self
                .releases
                .completion_fixture_hooks
                .lock()
                .map_err(|_| unavailable())?;
            if hooks
                .commit_gate
                .as_ref()
                .is_some_and(|(expected, _)| *expected == operation)
            {
                hooks.commit_gate.take().map(|(_, signal)| signal)
            } else {
                None
            }
        };
        if let Some(signal) = signal {
            signal
                .send(())
                .map_err(|_| invalid("fixture commit observer disconnected"))?;
        }
        Ok(())
    }
}

impl RuntimeDelivery {
    pub(crate) fn retained_carry_completions(
        &self,
        workspace: Uuid,
    ) -> Result<Vec<VerifiedReleaseCompletion>> {
        let state = self.releases.state.lock().map_err(|_| unavailable())?;
        // Clone only; caller validates each immutable proof and current native
        // origin outside Control/release locks before any baseline use.
        Ok(state
            .completions
            .values()
            .filter(|p| p.workspace_id() == workspace)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod reference_tests {
    use super::*;

    #[test]
    fn target_references_use_the_existing_deployment_namespace_only() {
        let root = tempfile::tempdir().unwrap();
        let workspace = kegari_storage::PrivateDir::create(
            &root.path().canonicalize().unwrap().join("workspace"),
        )
        .unwrap();
        let key = kegari_secrets::AttestationKey::new("fixture-key".into(), vec![17; 32]).unwrap();
        let name = "target-proof-fixture.json";
        let expected = key
            .sign(
                "target_deployment",
                &json!({"source":"exact retained target"}),
            )
            .unwrap();
        let misplaced = key
            .sign("target_deployment", &json!({"source":"wrong namespace"}))
            .unwrap();
        let reference = ProofFile {
            leaf: name.into(),
            purpose: "target_deployment".into(),
            digest: expected.digest().unwrap(),
        };
        workspace.write(name, &misplaced).unwrap();
        assert!(read_target_reference(&workspace, Uuid::nil(), &reference).is_err());
        assert!(!workspace.path().join("deployment").exists());
        let deployment = workspace.child("deployment").unwrap();
        deployment.write(name, &expected).unwrap();
        let actual = read_target_reference(&workspace, Uuid::nil(), &reference).unwrap();
        key.verify("target_deployment", &actual).unwrap();
        assert_eq!(actual.digest().unwrap(), reference.digest);
        std::fs::remove_file(deployment.path().join(name)).unwrap();
        let error = read_target_reference(&workspace, Uuid::nil(), &reference).unwrap_err();
        assert_eq!(error.code, "record_missing");
        assert_eq!(error.details["completion_stage"], "target_reference");
        assert_eq!(error.details["reference_purpose"], "target_deployment");
        assert!(workspace.path().join(name).exists());
    }
}
