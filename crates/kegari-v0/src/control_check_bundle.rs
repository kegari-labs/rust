//! One owned validation effect, with a finite immutable command membership.
//! Plans and journal references are data. Only the original live token can
//! advance a step; cold readers never recreate launch or settlement authority.
use super::*;
use crate::process_records::{CheckBundleMember, CheckMemberKind};

pub(super) const PLAN_KEY: &str = "check_bundle";
const PERMIT_KEY: &str = "check_bundle_permit";
const CLOSURE_KEY: &str = "check_bundle_closed";
const DIRECTORY: &str = "check-bundles";
const MAX_BYTES: u64 = 16_384;
const MAX_STEPS: usize = 17;

fn invalid(message: &str) -> Error {
    Error::new("check_bundle_identity", message)
}

/// Input from the executor's actual prepared check. This is not authority.
pub(crate) struct CheckBundleStepSpec {
    pub process_id: Uuid,
    pub kind: CheckMemberKind,
    pub service: Option<String>,
    pub launch_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct CheckBundleStep {
    ordinal: u32,
    process_id: Uuid,
    kind: CheckMemberKind,
    service: Option<String>,
    launch_digest: String,
}
impl CheckBundleStep {
    pub(crate) fn ordinal(&self) -> u32 {
        self.ordinal
    }
    pub(crate) fn process_id(&self) -> Uuid {
        self.process_id
    }
    pub(crate) fn kind(&self) -> &CheckMemberKind {
        &self.kind
    }
    pub(crate) fn service(&self) -> Option<&str> {
        self.service.as_deref()
    }
    pub(crate) fn launch_digest(&self) -> &str {
        &self.launch_digest
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanRecord {
    schema_version: u32,
    effect_id: Uuid,
    owner_operation: Uuid,
    runtime_id: Uuid,
    binding_digest: String,
    isolation_digest: String,
    steps: Vec<CheckBundleStep>,
}
#[derive(Clone, Debug)]
pub(crate) struct CheckBundlePlan {
    record: PlanRecord,
    digest: String,
}
impl CheckBundlePlan {
    pub(crate) fn new(
        effect_id: Uuid,
        owner_operation: Uuid,
        runtime_id: Uuid,
        binding_digest: String,
        isolation_digest: String,
        specs: Vec<CheckBundleStepSpec>,
    ) -> Result<Self> {
        Self::from_record(PlanRecord {
            schema_version: 1,
            effect_id,
            owner_operation,
            runtime_id,
            binding_digest,
            isolation_digest,
            steps: specs
                .into_iter()
                .enumerate()
                .map(|(ordinal, step)| CheckBundleStep {
                    ordinal: ordinal as u32,
                    process_id: step.process_id,
                    kind: step.kind,
                    service: step.service,
                    launch_digest: step.launch_digest,
                })
                .collect(),
        })
    }
    fn from_record(record: PlanRecord) -> Result<Self> {
        if record.schema_version != 1
            || record.effect_id.is_nil()
            || record.owner_operation.is_nil()
            || record.runtime_id.is_nil()
            || record.effect_id == record.owner_operation
            || record.steps.len() < 3
            || record.steps.len() > MAX_STEPS
            || record.steps.len().is_multiple_of(2)
        {
            return Err(invalid("invalid finite check bundle identity"));
        }
        crate::evidence::validate_digest(&record.binding_digest)?;
        crate::evidence::validate_digest(&record.isolation_digest)?;
        let mut processes = BTreeSet::new();
        let mut names = BTreeSet::new();
        for (index, step) in record.steps.iter().enumerate() {
            crate::evidence::validate_digest(&step.launch_digest)?;
            if step.ordinal as usize != index
                || step.process_id.is_nil()
                || step.process_id == record.effect_id
                || step.process_id == record.owner_operation
                || !processes.insert(step.process_id)
            {
                return Err(invalid(
                    "check bundle member identity is not unique and ordered",
                ));
            }
            if index + 1 == record.steps.len() {
                if step.kind != CheckMemberKind::Check || step.service.is_some() {
                    return Err(invalid("check bundle must end with its single check"));
                }
            } else {
                let name = step
                    .service
                    .as_deref()
                    .ok_or_else(|| invalid("service member has no name"))?;
                if name.is_empty()
                    || name.len() > 64
                    || !name.as_bytes()[0].is_ascii_alphanumeric()
                    || !name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
                {
                    return Err(invalid("invalid check bundle service name"));
                }
                if index.is_multiple_of(2) {
                    if step.kind != CheckMemberKind::Service || !names.insert(name) {
                        return Err(invalid(
                            "service starts must be unique and precede readiness",
                        ));
                    }
                } else if step.kind != CheckMemberKind::Readiness
                    || step.service != record.steps[index - 1].service
                {
                    return Err(invalid("readiness must follow its exact service"));
                }
            }
        }
        let encoded =
            serde_json::to_vec(&record).map_err(|_| invalid("encode check bundle plan"))?;
        if encoded.len() as u64 > MAX_BYTES {
            return Err(invalid("check bundle plan exceeds its bound"));
        }
        let digest = canonical_digest(1, &record)?;
        Ok(Self { record, digest })
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
    pub(crate) fn effect_id(&self) -> Uuid {
        self.record.effect_id
    }
    pub(crate) fn owner_operation(&self) -> Uuid {
        self.record.owner_operation
    }
    pub(crate) fn isolation_digest(&self) -> &str {
        &self.record.isolation_digest
    }
    pub(crate) fn steps(&self) -> &[CheckBundleStep] {
        &self.record.steps
    }
    fn member(&self, step: &CheckBundleStep) -> CheckBundleMember {
        CheckBundleMember {
            bundle_effect: self.effect_id(),
            plan_digest: self.digest.clone(),
            ordinal: step.ordinal,
            kind: step.kind.clone(),
            service: step.service.clone(),
        }
    }
    fn encoded(&self) -> Result<String> {
        serde_json::to_string(&self.record).map_err(|_| invalid("encode check bundle plan"))
    }
    fn from_receipt(receipt: &Receipt) -> Result<Self> {
        let encoded = receipt
            .expected
            .get(PLAN_KEY)
            .ok_or_else(|| invalid("missing admitted check bundle"))?;
        if encoded.len() as u64 > MAX_BYTES {
            return Err(invalid("oversized admitted check bundle"));
        }
        let record = serde_json::from_str(encoded)
            .map_err(|_| invalid("malformed admitted check bundle"))?;
        let plan = Self::from_record(record)?;
        if receipt.kind != "worker_validation"
            || receipt.operation_id != plan.effect_id()
            || receipt.request_id != plan.effect_id()
            || receipt.runtime_id != plan.record.runtime_id
            || receipt.expected.get("admission_operation")
                != Some(&plan.owner_operation().to_string())
        {
            return Err(invalid("check bundle differs from its original admission"));
        }
        Ok(plan)
    }
}

/// Made only inside the original Control gate after durable membership write.
/// The native boundary consumes it; serialized member data cannot replace it.
pub struct CheckBundleLaunch {
    permit: Permit,
    plan_digest: String,
    step: CheckBundleStep,
    member: CheckBundleMember,
}
impl CheckBundleLaunch {
    pub(crate) fn permit(&self) -> &Permit {
        &self.permit
    }
    pub(crate) fn plan_digest(&self) -> &str {
        &self.plan_digest
    }
    pub(crate) fn process_id(&self) -> Uuid {
        self.step.process_id
    }
    pub(crate) fn ordinal(&self) -> u32 {
        self.step.ordinal
    }
    pub(crate) fn member(&self) -> &CheckBundleMember {
        &self.member
    }
    pub(crate) fn launch_digest(&self) -> &str {
        self.step.launch_digest()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Attempt {
    schema_version: u32,
    effect: Uuid,
    owner: Uuid,
    permit_digest: String,
    receipt_identity: String,
    plan_digest: String,
    step: CheckBundleStep,
    previous: Option<String>,
}
impl Attempt {
    fn leaf(&self) -> String {
        format!("{}-{}.json", self.effect, self.step.ordinal)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Closure {
    schema_version: u32,
    plan_digest: String,
    attempted: usize,
    tip: Option<String>,
}

/// The original pre-Sent gate has rejected this effect before invoking any
/// registration/launch callback. This cannot settle an opaque callback error.
pub(super) fn record_never_started(receipt: &mut Receipt) -> Result<()> {
    if receipt.phase != ReceiptPhase::Prepared || !receipt.remote_ids.is_empty() {
        return Err(invalid(
            "only an original untransmitted bundle can be never-started",
        ));
    }
    let plan = CheckBundlePlan::from_receipt(receipt)?;
    let closed = serde_json::to_string(&Closure {
        schema_version: 1,
        plan_digest: plan.digest().into(),
        attempted: 0,
        tip: None,
    })
    .map_err(|_| invalid("encode never-started bundle"))?;
    receipt.remote_ids.insert(CLOSURE_KEY.into(), closed);
    Ok(())
}

pub(crate) struct InFlightCheckBundle {
    effect: InFlightEffect,
    plan: CheckBundlePlan,
    directory: PrivateDir,
    attempts: Vec<Attempt>,
    poisoned: bool,
}
impl InFlightCheckBundle {
    fn verify_prefix(&self) -> Result<()> {
        for attempt in &self.attempts {
            let bytes = self.directory.read_bytes(&attempt.leaf(), MAX_BYTES)?;
            let actual: Attempt = serde_json::from_slice(&bytes)
                .map_err(|_| invalid("malformed attempted bundle step"))?;
            if &actual != attempt {
                return Err(invalid("original attempted bundle step changed"));
            }
        }
        Ok(())
    }
}

impl ControlHost {
    pub(crate) fn begin_check_bundle<T>(
        &self,
        mut request: MutationRequest,
        permit: &Permit,
        plan: CheckBundlePlan,
        fresh_gate: impl FnOnce() -> Result<()>,
        register_owner: impl FnOnce(&Receipt) -> Result<T>,
    ) -> Result<(InFlightCheckBundle, T)> {
        if request.kind != "worker_validation"
            || request.request_id != plan.effect_id()
            || permit.operation_id != plan.owner_operation()
            || request.runtime_id != plan.record.runtime_id
            || permit.lease.runtime_id != plan.record.runtime_id
            || request.expected.contains_key(PLAN_KEY)
            || request.expected.contains_key(PERMIT_KEY)
        {
            return Err(invalid(
                "check bundle is not this exact worker validation request",
            ));
        }
        request.expected.insert(PLAN_KEY.into(), plan.encoded()?);
        request
            .expected
            .insert(PERMIT_KEY.into(), canonical_digest(1, permit)?);
        let directory = self.root.child(DIRECTORY)?;
        let (effect, owner) =
            self.begin_owned_effect(request, permit, fresh_gate, register_owner, false)?;
        Ok((
            InFlightCheckBundle {
                effect,
                plan,
                directory,
                attempts: Vec::new(),
                poisoned: false,
            },
            owner,
        ))
    }

    fn verify_check_bundle(
        &self,
        state: &mut State,
        bundle: &InFlightCheckBundle,
        permit: &Permit,
    ) -> Result<Receipt> {
        if canonical_digest(1, permit)? != canonical_digest(1, &bundle.effect.permit)? {
            return Err(invalid(
                "check bundle permit differs from its original owner",
            ));
        }
        let receipt = self.receipts.get(bundle.effect.request_id)?;
        self.verify_effect_identity(
            state,
            &receipt,
            bundle.effect.request_id,
            &bundle.effect.identity,
            Some(permit.operation_id),
        )?;
        if receipt.phase != ReceiptPhase::Sent
            || CheckBundlePlan::from_receipt(&receipt)?.digest() != bundle.plan.digest()
        {
            return Err(invalid(
                "check bundle is no longer its original sent effect",
            ));
        }
        if !state.holds.get(&receipt.operation_id).is_some_and(|held| {
            held.contains(&permit.work_key)
                && receipt
                    .canonical_targets
                    .iter()
                    .all(|target| held.contains(target))
        }) {
            return Err(invalid(
                "check bundle lost its original work or resource hold",
            ));
        }
        bundle.verify_prefix()?;
        Ok(receipt)
    }

    pub(crate) fn advance_check_bundle<T>(
        &self,
        bundle: &mut InFlightCheckBundle,
        permit: &Permit,
        ordinal: u32,
        fresh_gate: impl FnOnce() -> Result<()>,
        launch: impl FnOnce(CheckBundleLaunch) -> Result<T>,
    ) -> Result<T> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        if bundle.poisoned || ordinal as usize != bundle.attempts.len() {
            return Err(invalid(
                "bundle step was already attempted or is out of order",
            ));
        }
        let step = bundle
            .plan
            .steps()
            .get(ordinal as usize)
            .cloned()
            .ok_or_else(|| invalid("bundle step is outside the admitted plan"))?;
        // Any failure from here closes this token to further dispatch. Actual
        // settled-prefix cleanup may still report a known failed bundle.
        bundle.poisoned = true;
        let receipt = self.verify_check_bundle(&mut state, bundle, permit)?;
        validate_live_permit(&state, permit)?;
        if state.service_stopping || state.error.is_some() {
            return Err(Error::new(
                "access_denied",
                "check bundle host is stopping or uncertain",
            ));
        }
        if state.holds.iter().any(|(id, targets)| {
            *id != receipt.operation_id
                && targets.iter().any(|target| {
                    receipt.canonical_targets.contains(target)
                        || target == &permit.work_key
                        || target == &state.intent.runtime_id.to_string()
                })
        }) {
            return Err(Error::new(
                "reconciliation_required",
                "another unresolved effect holds this check bundle",
            ));
        }
        fresh_gate()?;
        let attempt = Attempt {
            schema_version: 1,
            effect: receipt.operation_id,
            owner: permit.operation_id,
            permit_digest: canonical_digest(1, permit)?,
            receipt_identity: bundle.effect.identity.clone(),
            plan_digest: bundle.plan.digest().into(),
            step: step.clone(),
            previous: bundle
                .attempts
                .last()
                .map(|last| canonical_digest(1, last))
                .transpose()?,
        };
        let bytes = serde_json::to_vec(&attempt).map_err(|_| invalid("encode bundle step"))?;
        // Never adopt a preexisting record or retry an uncertain write.
        bundle
            .directory
            .create_bytes_exclusive(&attempt.leaf(), &bytes)?;
        bundle.attempts.push(attempt);
        let result = launch(CheckBundleLaunch {
            permit: permit.clone(),
            plan_digest: bundle.plan.digest().into(),
            member: bundle.plan.member(&step),
            step,
        });
        if result.is_ok() {
            bundle.poisoned = false;
        }
        result
    }

    pub(crate) fn complete_check_bundle(
        &self,
        bundle: InFlightCheckBundle,
        permit: &Permit,
        owner: &crate::execution::CheckBundleOwner,
        fresh_gate: impl FnOnce() -> Result<()>,
        observed: Value,
    ) -> Result<Receipt> {
        if bundle.poisoned || bundle.attempts.len() != bundle.plan.steps().len() {
            return Err(invalid(
                "successful check settlement requires the full original bundle",
            ));
        }
        self.settle_check_bundle(bundle, permit, owner, fresh_gate, observed)
    }

    pub(crate) fn complete_check_bundle_failure(
        &self,
        bundle: InFlightCheckBundle,
        permit: &Permit,
        owner: &crate::execution::CheckBundleOwner,
        fresh_gate: impl FnOnce() -> Result<()>,
        error: &Error,
    ) -> Result<Receipt> {
        let observed = serde_json::json!({"check_bundle_failed": {
            "plan_digest": bundle.plan.digest(), "error": error,
            "attempted_members": bundle.attempts.len(), "repair_evidence": false }});
        self.settle_check_bundle(bundle, permit, owner, fresh_gate, observed)
    }

    fn settle_check_bundle(
        &self,
        bundle: InFlightCheckBundle,
        permit: &Permit,
        owner: &crate::execution::CheckBundleOwner,
        fresh_gate: impl FnOnce() -> Result<()>,
        observed: Value,
    ) -> Result<Receipt> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::new("control_failed", "control serialization failed"))?;
        let mut receipt = self.verify_check_bundle(&mut state, &bundle, permit)?;
        owner.verify_settled(&bundle.plan)?;
        if owner.attempted_prefix() != bundle.attempts.len() {
            return Err(invalid(
                "settled owner differs from durable attempted membership",
            ));
        }
        let closure = Closure {
            schema_version: 1,
            plan_digest: bundle.plan.digest().into(),
            attempted: bundle.attempts.len(),
            tip: bundle
                .attempts
                .last()
                .map(|last| canonical_digest(1, last))
                .transpose()?,
        };
        let closed =
            serde_json::to_string(&closure).map_err(|_| invalid("encode closed bundle"))?;
        let current = validate_live_permit(&state, permit).and_then(|()| fresh_gate());
        let value = match &current {
            Ok(()) => observed,
            Err(error) => {
                serde_json::json!({"effect":"exited_evidence_rejected","error":error,"observed":observed})
            }
        };
        self.acknowledge_indexed_receipt(
            &mut state,
            &mut receipt,
            BTreeMap::from([(CLOSURE_KEY.into(), closed)]),
            value,
        )?;
        state.holds.remove(&receipt.operation_id);
        current?;
        Ok(receipt)
    }

    /// Read-only closure data, safe inside an existing Control observation gate.
    /// A pending/missing closure never returns an empty successful inventory.
    /// This does not make a writer, permit, joined handle or native capability.
    pub(crate) fn check_bundle_attempted_members(
        &self,
        receipt: &Receipt,
    ) -> Result<Option<Vec<(Uuid, CheckBundleMember)>>> {
        if !receipt.expected.contains_key(PLAN_KEY) {
            if receipt.expected.contains_key(PERMIT_KEY)
                || receipt.remote_ids.contains_key(CLOSURE_KEY)
            {
                return Err(invalid("check bundle closure lost its required plan"));
            }
            return Ok(None);
        }
        let plan = CheckBundlePlan::from_receipt(receipt)?;
        let encoded = receipt.remote_ids.get(CLOSURE_KEY).ok_or_else(|| {
            Error::new(
                "operation_uncertain",
                "check bundle has no actual closed membership",
            )
        })?;
        if encoded.len() as u64 > MAX_BYTES {
            return Err(invalid("oversized closed check bundle"));
        }
        let closure: Closure =
            serde_json::from_str(encoded).map_err(|_| invalid("malformed closed check bundle"))?;
        if !matches!(
            receipt.phase,
            ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
        ) || closure.schema_version != 1
            || closure.plan_digest != plan.digest()
            || closure.attempted > plan.steps().len()
        {
            return Err(invalid("invalid terminal check bundle membership"));
        }
        self.read_bundle_prefix(receipt, &plan, closure.attempted, Some(&closure.tip))
    }

    fn read_bundle_prefix(
        &self,
        receipt: &Receipt,
        plan: &CheckBundlePlan,
        attempted: usize,
        expected_tip: Option<&Option<String>>,
    ) -> Result<Option<Vec<(Uuid, CheckBundleMember)>>> {
        if attempted > plan.steps().len() {
            return Err(invalid("attempted bundle prefix exceeds its admitted plan"));
        }
        let permit_digest = receipt
            .expected
            .get(PERMIT_KEY)
            .ok_or_else(|| invalid("bundle lost original permit identity"))?;
        crate::evidence::validate_digest(permit_digest)?;
        let directory = PrivateDir::open(&self.root.path().join(DIRECTORY))?;
        let identity = receipt_request_digest(receipt)?;
        let mut previous = None;
        let mut members = Vec::new();
        for step in plan.steps().iter().take(attempted) {
            let leaf = format!("{}-{}.json", plan.effect_id(), step.ordinal);
            let bytes = directory.read_bytes(&leaf, MAX_BYTES)?;
            let attempt: Attempt = serde_json::from_slice(&bytes)
                .map_err(|_| invalid("malformed retained bundle step"))?;
            if attempt.schema_version != 1
                || attempt.effect != plan.effect_id()
                || attempt.owner != plan.owner_operation()
                || attempt.permit_digest != *permit_digest
                || attempt.receipt_identity != identity
                || attempt.plan_digest != plan.digest()
                || attempt.step != *step
                || attempt.previous != previous
            {
                return Err(invalid(
                    "retained attempted member differs from exact closed bundle",
                ));
            }
            previous = Some(canonical_digest(1, &attempt)?);
            members.push((step.process_id, plan.member(step)));
        }
        if expected_tip.is_some_and(|tip| &previous != tip) {
            return Err(invalid("closed bundle membership tip changed"));
        }
        // A recorded attempt beyond the retained closed prefix is a conflict,
        // not an unattempted suffix. Reads never create or adopt these records.
        for step in plan.steps().iter().skip(attempted) {
            match directory.read_bytes(
                &format!("{}-{}.json", plan.effect_id(), step.ordinal),
                MAX_BYTES,
            ) {
                Err(error) if error.code == "record_missing" => (),
                Err(error) => return Err(error),
                Ok(_) => {
                    return Err(invalid(
                        "check bundle has an attempt outside its closed prefix",
                    ));
                }
            }
        }
        Ok(Some(members))
    }

    fn verify_bundle_process_receipt(
        &self,
        binding: &crate::process_records::SpawnBinding,
        receipt: &Receipt,
    ) -> Result<()> {
        // The complete batch already checked exact member identity and read
        // its immutable prefix once. This finishes the original owner binding.
        if receipt.expected.get("admission_operation") != Some(&binding.operation_id.to_string())
            || receipt.expected.get("admitted_scope") != Some(&binding.scope_digest)
            || receipt.expected.get("admitted_work") != Some(&binding.work_key)
            || binding.control_owner.is_some()
        {
            return Err(invalid(
                "process differs from its exact admitted bundle member",
            ));
        }
        let lease = binding
            .lease
            .as_ref()
            .ok_or_else(|| invalid("bundle process has no original worker lease"))?;
        let admission = self.receipts.get(binding.operation_id)?;
        if lease.runtime_id != receipt.runtime_id
            || lease.owner_generation != receipt.owner_generation
            || admission.kind != "admission"
            || admission.runtime_id != lease.runtime_id
            || admission.owner_generation != lease.owner_generation
            || admission.actor != format!("owner:{}", lease.owner_id)
            || admission.expected.get("scope") != Some(&binding.scope_digest)
            || admission.canonical_targets != vec![binding.work_key.clone()]
        {
            return Err(invalid(
                "bundle process differs from its original worker admission",
            ));
        }
        Ok(())
    }

    /// Cleanup-only terminal observation for the retained original aggregate.
    /// It does not dispatch or convert a historical record to a live capability.
    pub(crate) fn check_bundle_owner_closed(
        &self,
        receipt: &Receipt,
        owner: &crate::execution::CheckBundleOwner,
    ) -> Result<bool> {
        if matches!(receipt.phase, ReceiptPhase::Prepared | ReceiptPhase::Sent) {
            return Ok(false);
        }
        let plan = CheckBundlePlan::from_receipt(receipt)?;
        if owner.plan().digest() != plan.digest() {
            return Err(invalid(
                "terminal receipt differs from retained bundle owner",
            ));
        }
        let current = self.receipts.get(receipt.operation_id)?;
        if canonical_digest(1, &current)? != canonical_digest(1, receipt)? {
            return Err(invalid(
                "terminal bundle receipt changed during observation",
            ));
        }
        let members = self
            .check_bundle_attempted_members(receipt)?
            .ok_or_else(|| invalid("terminal owner lost its bundle membership"))?;
        if members.len() != owner.attempted_prefix() {
            return Err(invalid(
                "terminal bundle prefix differs from retained owner",
            ));
        }
        owner.verify_settled(&plan)?;
        Ok(true)
    }

    /// Exact finite inventory join for an already selected scope. Callers pass
    /// their existing receipt batch and actual process bindings; no global
    /// discovery, nested Control gate or adoption occurs here.
    pub(crate) fn verify_check_bundle_inventory(
        &self,
        receipts: &[Receipt],
        bindings: &[crate::process_records::SpawnBinding],
    ) -> Result<()> {
        self.verify_live_check_bundle_inventory(receipts, bindings, |_, _| Ok(None))
    }

    /// Warm maintenance only: caller holds the original Control observation
    /// gate across receipt/process reads and this join, so member creation cannot
    /// race the inventory. Only exact retained aggregate ownership admits a
    /// pending prefix. Cold records and terminal records remain strict.
    pub(crate) fn verify_live_check_bundle_inventory(
        &self,
        receipts: &[Receipt],
        bindings: &[crate::process_records::SpawnBinding],
        retained: impl Fn(
            Uuid,
            Uuid,
        )
            -> Result<Option<std::sync::Arc<dyn crate::execution::CommandCancellation>>>,
    ) -> Result<()> {
        if receipts.len() > 65_536 || bindings.len() > 65_536 {
            return Err(Error::new(
                "record_limit",
                "check bundle inventory exceeds its bound",
            ));
        }
        let mut bundles = BTreeMap::new();
        let mut expected = BTreeMap::new();
        for receipt in receipts {
            let members =
                if receipt.phase == ReceiptPhase::Sent && receipt.expected.contains_key(PLAN_KEY) {
                    if receipt.remote_ids.contains_key(CLOSURE_KEY) {
                        return Err(invalid(
                            "pending bundle unexpectedly carries terminal closure",
                        ));
                    }
                    let plan = CheckBundlePlan::from_receipt(receipt)?;
                    let handle =
                        retained(plan.owner_operation(), plan.effect_id())?.ok_or_else(|| {
                            Error::new(
                                "operation_uncertain",
                                "pending check bundle has no original retained owner",
                            )
                        })?;
                    let owner = handle.check_bundle_owner().ok_or_else(|| {
                        Error::new(
                            "operation_uncertain",
                            "pending check bundle has no typed aggregate owner",
                        )
                    })?;
                    let attempted = owner.verify_live_prefix(&plan)?;
                    self.read_bundle_prefix(receipt, &plan, attempted, None)?
                } else {
                    self.check_bundle_attempted_members(receipt)?
                };
            if let Some(members) = members {
                if bundles.insert(receipt.operation_id, receipt).is_some() {
                    return Err(invalid(
                        "duplicate bundle effect in selected receipt inventory",
                    ));
                }
                for (process, member) in members {
                    if expected.len() >= 65_536 {
                        return Err(Error::new(
                            "record_limit",
                            "bundle members exceed the whole inventory bound",
                        ));
                    }
                    if expected.insert(process, member).is_some() {
                        return Err(invalid("one process is claimed by multiple bundle members"));
                    }
                }
            }
        }
        let mut observed = BTreeSet::new();
        for binding in bindings {
            match (&binding.check_bundle, expected.get(&binding.effect_id)) {
                (None, None) => (),
                (Some(member), Some(original)) if member == original => {
                    if !observed.insert(binding.effect_id) {
                        return Err(invalid("duplicate original bundle process binding"));
                    }
                    let receipt = bundles.get(&member.bundle_effect).ok_or_else(|| {
                        invalid("bundle process lies outside selected receipt scope")
                    })?;
                    self.verify_bundle_process_receipt(binding, receipt)?;
                }
                _ => {
                    return Err(invalid(
                        "bundle process membership was removed or substituted",
                    ));
                }
            }
        }
        if observed.len() != expected.len() {
            return Err(invalid("an attempted bundle process record is missing"));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "control_check_bundle_tests.rs"]
mod tests;
