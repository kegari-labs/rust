//! Admission-declared Control effect membership. This records reservations, not
//! delivery success, process exit, worker joining, or cold execution authority.
use super::*;

#[path = "control_parent_effect_history.rs"]
mod parent_history;
pub(crate) use parent_history::{ParentEffectHistoryBinding, ParentEffectHistoryObservation};

pub(super) const KEY: &str = "owner_effect_index";
const VERSION: &str = "1";
const PREFIX: &str = "owner-effect-index-";
const MAX_RECORDS: usize = 65_536;
const MAX_BYTES: usize = 256 * 1024;

fn invalid(message: &str) -> Error {
    Error::new("effect_index", message)
}

pub(super) fn eligible(reference: Option<&AdmissionReference>) -> bool {
    let phase = match reference {
        Some(
            AdmissionReference::Preparation(v)
            | AdmissionReference::ParentPreparation(v, _)
            | AdmissionReference::RoundPreparation(v, _),
        ) => &v.phase.phase,
        Some(
            AdmissionReference::Run(v)
            | AdmissionReference::ParentRun(v, _, _)
            | AdmissionReference::RoundRun(v, _)
            | AdmissionReference::ContinuedRun(v, _)
            | AdmissionReference::SourceAdvancedRun(v, _)
            | AdmissionReference::ResumedRun(v, _)
            | AdmissionReference::RoundResumedRun(v, _, _),
        ) => &v.phase.phase,
        _ => return false,
    };
    matches!(
        phase.as_str(),
        "parent_integrated_validation" | "parent_evidence_review"
    )
}

fn required(receipt: &Receipt) -> Result<bool> {
    let Some(version) = receipt.expected.get(KEY) else {
        if receipt.remote_ids.contains_key(KEY) {
            return Err(invalid(
                "frozen owner reference lost its required index declaration",
            ));
        }
        return Ok(false);
    };
    if version != VERSION || receipt.kind != "admission" {
        return Err(invalid("unsupported owner effect index declaration"));
    }
    let phase = match (
        PreparationReference::from_receipt(receipt)?,
        RunReference::from_receipt(receipt)?,
    ) {
        (Some(v), None) => v.phase,
        (None, Some(v)) => v.phase,
        _ => {
            return Err(invalid(
                "indexed owner has no unique typed parent admission",
            ));
        }
    };
    if !matches!(
        phase.phase.as_str(),
        "parent_integrated_validation" | "parent_evidence_review"
    ) {
        return Err(invalid("indexed owner is not an eligible parent phase"));
    }
    Ok(true)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Root {
    schema_version: u32,
    owner: Uuid,
    runtime: Uuid,
    generation: u64,
    admission_digest: String,
    work: String,
    scope: String,
}
impl Root {
    fn from_admission(receipt: &Receipt) -> Result<Self> {
        if !required(receipt)?
            || receipt.operation_id != receipt.request_id
            || receipt.canonical_targets.len() != 1
        {
            return Err(invalid(
                "effect index requires exact declared owner admission",
            ));
        }
        Ok(Self {
            schema_version: 1,
            owner: receipt.operation_id,
            runtime: receipt.runtime_id,
            generation: receipt.owner_generation,
            admission_digest: receipt_request_digest(receipt)?,
            work: receipt.canonical_targets[0].clone(),
            scope: receipt
                .expected
                .get("scope")
                .ok_or_else(|| invalid("indexed owner scope missing"))?
                .clone(),
        })
    }
    fn verify_effect(&self, receipt: &Receipt) -> Result<()> {
        if receipt.operation_id != receipt.request_id
            || receipt.runtime_id != self.runtime
            || receipt.owner_generation != self.generation
            || receipt.expected.get("admission_operation") != Some(&self.owner.to_string())
            || receipt.expected.get("admitted_work") != Some(&self.work)
            || receipt.expected.get("admitted_scope") != Some(&self.scope)
        {
            return Err(invalid("effect receipt differs from indexed owner"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    root: String,
    count: u32,
    tip: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    root: String,
    ordinal: u32,
    previous: String,
    operation: Uuid,
    receipt: String,
}

/// Never reconstructed from disk. Kept after freeze/retirement as this host's
/// high-water anchor; uncertain writes fence it and never mint a replacement.
pub(super) struct LiveIndex {
    root: Root,
    cursor: Cursor,
    frozen: bool,
    failed: bool,
    retirement: Option<Receipt>,
    retirement_request: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Membership {
    owner: Uuid,
    identity: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Anchor {
    root: Root,
    cursor: Cursor,
    frozen: bool,
}
/// Read-only membership learned from actual validated storage or the live
/// owner's pre-write high-water. This never grants a writer, permit or join.
#[derive(Default, Debug)]
pub(super) struct History {
    owners: BTreeMap<Uuid, Anchor>,
    members: BTreeMap<Uuid, Membership>,
}
impl History {
    fn retain(&mut self, root: &Root, cursor: &Cursor, frozen: bool) {
        self.owners.insert(
            root.owner,
            Anchor {
                root: root.clone(),
                cursor: cursor.clone(),
                frozen,
            },
        );
    }
}

struct Inventory {
    root: Root,
    cursor: Cursor,
    frozen: Option<Cursor>,
    members: BTreeMap<Uuid, String>,
}
fn name(owner: Uuid, suffix: &str) -> String {
    format!("{PREFIX}{owner}-{suffix}.json")
}
fn entry_name(owner: Uuid, ordinal: u32) -> String {
    name(owner, &format!("entry-{ordinal:010}"))
}
fn create<T: Serialize>(root: &PrivateDir, leaf: &str, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(&serde_json::json!({"schema_version":1,"digest":canonical_digest(1,value)?,"payload":value}))
        .map_err(|_| invalid("effect index encoding failed"))?;
    if bytes.len() > MAX_BYTES {
        return Err(invalid(
            "effect index record exceeds protected record bound",
        ));
    }
    root.create_bytes_exclusive(leaf, &bytes)
}
fn receipts(store: &ReceiptStore) -> Result<BTreeMap<Uuid, Receipt>> {
    let mut result = BTreeMap::new();
    for leaf in store.directory.records_bounded(MAX_RECORDS)? {
        let receipt: Receipt = store.directory.read(&leaf)?;
        if leaf != format!("{}.json", receipt.request_id)
            || receipt.operation_id != receipt.request_id
            || result.insert(receipt.operation_id, receipt).is_some()
        {
            return Err(invalid(
                "effect index receipt inventory has an alias or duplicate",
            ));
        }
    }
    Ok(result)
}
fn index_names(root: &PrivateDir) -> Result<BTreeSet<String>> {
    Ok(root
        .records_bounded(MAX_RECORDS)?
        .into_iter()
        .filter(|n| n.starts_with(PREFIX))
        .collect())
}
fn read(
    root: &PrivateDir,
    owner: &Receipt,
    all: &BTreeMap<Uuid, Receipt>,
    names: &BTreeSet<String>,
) -> Result<Inventory> {
    let owned = all
        .values()
        .filter(|v| v.expected.get("admission_operation") == Some(&owner.operation_id.to_string()))
        .map(|v| v.operation_id)
        .collect();
    read_owned(root, owner, all, names, &owned)
}

fn read_owned(
    root: &PrivateDir,
    owner: &Receipt,
    all: &BTreeMap<Uuid, Receipt>,
    names: &BTreeSet<String>,
    owned: &BTreeSet<Uuid>,
) -> Result<Inventory> {
    let expected_root = Root::from_admission(owner)?;
    let actual: Root = root.read(&name(owner.operation_id, "root"))?;
    if actual != expected_root {
        return Err(invalid("owner effect index root changed"));
    }
    let cursor: Cursor = root.read(&name(actual.owner, "head"))?;
    let root_digest = canonical_digest(1, &actual)?;
    if cursor.root != root_digest || cursor.count as usize > MAX_RECORDS {
        return Err(invalid(
            "owner effect index head differs from root or bound",
        ));
    }
    let mut expected_names: BTreeSet<_> =
        [name(actual.owner, "root"), name(actual.owner, "head")].into();
    let mut previous = root_digest.clone();
    let mut members = BTreeMap::new();
    for ordinal in 1..=cursor.count {
        let leaf = entry_name(actual.owner, ordinal);
        let entry: Entry = root.read(&leaf)?;
        if entry.root != root_digest
            || entry.ordinal != ordinal
            || entry.previous != previous
            || members
                .insert(entry.operation, entry.receipt.clone())
                .is_some()
        {
            return Err(invalid("owner effect index has a gap, fork, or duplicate"));
        }
        let receipt = all
            .get(&entry.operation)
            .ok_or_else(|| Error::new("record_missing", "indexed effect receipt is missing"))?;
        actual.verify_effect(receipt)?;
        if receipt_request_digest(receipt)? != entry.receipt {
            return Err(invalid("indexed effect immutable identity changed"));
        }
        previous = canonical_digest(1, &entry)?;
        expected_names.insert(leaf);
    }
    if previous != cursor.tip {
        return Err(invalid("owner effect index terminal tip changed"));
    }
    let frozen_name = name(actual.owner, "frozen");
    let frozen = if names.contains(&frozen_name) {
        let frozen: Cursor = root.read(&frozen_name)?;
        if frozen != cursor {
            return Err(invalid("frozen owner membership differs from head"));
        }
        expected_names.insert(frozen_name);
        Some(frozen)
    } else {
        None
    };
    let prefix = format!("{PREFIX}{}-", actual.owner);
    if names
        .range(prefix.clone()..)
        .take_while(|v| v.starts_with(&prefix))
        .cloned()
        .collect::<BTreeSet<_>>()
        != expected_names
    {
        return Err(invalid(
            "owner effect index has uncommitted or unexpected records",
        ));
    }
    for operation in owned {
        let receipt = all
            .get(operation)
            .ok_or_else(|| invalid("owner receipt inventory changed"))?;
        actual.verify_effect(receipt)?;
        if !members.contains_key(&receipt.operation_id) {
            return Err(invalid(
                "owner receipt is absent from reserved membership index",
            ));
        }
    }
    if matches!(
        owner.phase,
        ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
    ) {
        let frozen = frozen
            .as_ref()
            .ok_or_else(|| invalid("closed indexed owner has no frozen membership"))?;
        if owner.remote_ids.get(KEY) != Some(&canonical_digest(1, frozen)?) {
            return Err(invalid(
                "closed owner lost its exact frozen membership reference",
            ));
        }
    } else if owner.remote_ids.contains_key(KEY) {
        return Err(invalid("open owner has a terminal membership reference"));
    }
    Ok(Inventory {
        root: actual,
        cursor,
        frozen,
        members,
    })
}

pub(super) fn recover(root: &PrivateDir, store: &ReceiptStore) -> Result<History> {
    let all = receipts(store)?;
    let names = index_names(root)?;
    let mut owners = BTreeSet::new();
    let mut history = History::default();
    let mut owned: BTreeMap<Uuid, BTreeSet<Uuid>> = BTreeMap::new();
    for receipt in all.values() {
        if let Some(owner) = receipt.expected.get("admission_operation") {
            let owner =
                Uuid::parse_str(owner).map_err(|_| invalid("effect owner identity is invalid"))?;
            owned.entry(owner).or_default().insert(receipt.operation_id);
        }
    }
    let empty = BTreeSet::new();
    for receipt in all.values() {
        if required(receipt)? {
            let inventory = read_owned(
                root,
                receipt,
                &all,
                &names,
                owned.get(&receipt.operation_id).unwrap_or(&empty),
            )?;
            history.retain(
                &inventory.root,
                &inventory.cursor,
                inventory.frozen.is_some(),
            );
            for (operation, identity) in inventory.members {
                if history
                    .members
                    .insert(
                        operation,
                        Membership {
                            owner: receipt.operation_id,
                            identity,
                        },
                    )
                    .is_some()
                {
                    return Err(invalid("effect belongs to multiple indexed owners"));
                }
            }
            owners.insert(receipt.operation_id);
        }
    }
    for leaf in names {
        let owner = leaf
            .strip_prefix(PREFIX)
            .and_then(|s| s.get(..36))
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| invalid("effect index filename is not canonical"))?;
        if !owners.contains(&owner) {
            return Err(invalid("effect index has no declared retained owner"));
        }
    }
    Ok(history)
}

/// Failure can add conservative scope, never erase a resource held by an
/// already transmitted effect. Admission closure does not fence other owners.
fn retain_effect_hold(
    state: &mut State,
    operation: Uuid,
    targets: impl IntoIterator<Item = String>,
) {
    let retained = state.holds.entry(operation).or_default();
    for target in targets {
        if !retained.contains(&target) {
            retained.push(target);
        }
    }
}

impl ControlHost {
    fn index_failure<T>(&self, state: &mut State, owner: Uuid, result: Result<T>) -> Result<T> {
        result.inspect_err(|error| {
            state.open = false;
            state.error = Some(error.code.clone());
            state.fenced.insert(owner);
            if let Some(live) = state.effect_indexes.get_mut(&owner) {
                live.failed = true;
            }
        })
    }
    /// A missing known indexed identity is never a fresh request. Check before
    /// ReceiptStore::prepare can create bytes, including requests of other kinds.
    /// Intact identities retain the existing conflict/replay semantics.
    pub(super) fn guard_indexed_request_id(
        &self,
        state: &mut State,
        operation: Uuid,
    ) -> Result<()> {
        let owner = if state.effect_history.owners.contains_key(&operation)
            || state.effect_indexes.contains_key(&operation)
        {
            Some(operation)
        } else {
            state
                .effect_history
                .members
                .get(&operation)
                .map(|m| m.owner)
        };
        let Some(owner) = owner else { return Ok(()) };
        let result = (|| {
            let receipt = self.receipts.get(operation)?;
            if receipt.operation_id != operation || receipt.request_id != operation {
                return Err(invalid(
                    "known indexed request changed its operation identity",
                ));
            }
            self.verify_indexed_receipt(state, &receipt)
        })();
        if result.is_err()
            && let Some(anchor) = state.effect_history.owners.get(&owner)
        {
            let work = anchor.root.work.clone();
            retain_effect_hold(state, operation, [work]);
        }
        self.index_failure(state, owner, result)
    }
    pub(super) fn create_effect_index(&self, state: &mut State, admission: &Receipt) -> Result<()> {
        if state.effect_indexes.contains_key(&admission.operation_id)
            || state
                .effect_history
                .owners
                .contains_key(&admission.operation_id)
            || state
                .effect_history
                .members
                .contains_key(&admission.operation_id)
        {
            return self.index_failure(
                state,
                admission.operation_id,
                Err(invalid(
                    "new owner cannot replace retained indexed identity",
                )),
            );
        }
        if !required(admission)? {
            return Ok(());
        }
        let result = (|| {
            let root = Root::from_admission(admission)?;
            let digest = canonical_digest(1, &root)?;
            let cursor = Cursor {
                root: digest.clone(),
                count: 0,
                tip: digest,
            };
            // Register the high-water identity before any uncertain persistent write.
            state.effect_indexes.insert(
                root.owner,
                LiveIndex {
                    root: root.clone(),
                    cursor: cursor.clone(),
                    frozen: false,
                    failed: false,
                    retirement: None,
                    retirement_request: None,
                },
            );
            state.effect_history.retain(&root, &cursor, false);
            if state.effect_history.owners.len() >= MAX_RECORDS
                || self.root.records_bounded(MAX_RECORDS)?.len() > MAX_RECORDS - 3
            {
                return Err(invalid("owner effect index storage limit reached"));
            }
            create(&self.root, &name(root.owner, "root"), &root)?;
            self.root.write(&name(root.owner, "head"), &cursor)
        })();
        self.index_failure(state, admission.operation_id, result)
    }
    fn index_inventory(&self, state: &State, owner: Uuid) -> Result<Option<Inventory>> {
        let admission = self.receipts.get(owner)?;
        if !required(&admission)? {
            if state.effect_history.owners.contains_key(&owner)
                || state.effect_indexes.contains_key(&owner)
                || index_names(&self.root)?
                    .iter()
                    .any(|n| n.starts_with(&format!("{PREFIX}{owner}-")))
            {
                return Err(invalid("known owner lost its index declaration"));
            }
            return Ok(None);
        }
        let inventory = read(
            &self.root,
            &admission,
            &receipts(&self.receipts)?,
            &index_names(&self.root)?,
        )?;
        if state
            .effect_indexes
            .get(&owner)
            .is_some_and(|live| live.frozen)
            && inventory.frozen.is_none()
        {
            return Err(Error::new(
                "record_missing",
                "known frozen owner membership is missing",
            ));
        }
        if let Some(live) = state.effect_indexes.get(&owner)
            && (live.root != inventory.root
                || live.cursor != inventory.cursor
                || (!live.frozen && inventory.frozen.is_some()))
        {
            return Err(invalid(
                "owner effect index differs from retained high-water state",
            ));
        }
        if let Some(anchor) = state.effect_history.owners.get(&owner)
            && (anchor.root != inventory.root
                || anchor.cursor != inventory.cursor
                || anchor.frozen != inventory.frozen.is_some())
        {
            return Err(invalid(
                "owner effect index differs from retained historical membership",
            ));
        }
        Ok(Some(inventory))
    }
    pub(super) fn register_indexed_effect(
        &self,
        state: &mut State,
        permit: Option<&Permit>,
        receipt: &Receipt,
        fresh: bool,
    ) -> Result<()> {
        let Some(permit) = permit else { return Ok(()) };
        let result = (|| {
            // The fresh Prepared receipt is not a member yet. Exclude exactly it
            // for this before-dispatch transition; never permit an old backfill.
            let admission = self.receipts.get(permit.operation_id)?;
            if !required(&admission)? {
                self.index_inventory(state, permit.operation_id)?;
                return Ok(());
            }
            let mut all = receipts(&self.receipts)?;
            if fresh {
                all.remove(&receipt.operation_id);
            }
            let inventory = read(&self.root, &admission, &all, &index_names(&self.root)?)?;
            let live = state
                .effect_indexes
                .get_mut(&permit.operation_id)
                .ok_or_else(|| invalid("no actual live index writer"))?;
            if live.failed
                || live.frozen
                || inventory.frozen.is_some()
                || live.root != inventory.root
                || live.cursor != inventory.cursor
            {
                return Err(invalid("effect index writer is fenced, frozen, or changed"));
            }
            live.root.verify_effect(receipt)?;
            if !fresh {
                if inventory.members.get(&receipt.operation_id)
                    != Some(&receipt_request_digest(receipt)?)
                {
                    return Err(invalid("existing effect has no exact index membership"));
                }
                return Ok(());
            }
            if live.cursor.count as usize >= MAX_RECORDS - 3
                || state.effect_history.members.len() >= MAX_RECORDS
                || self.root.records_bounded(MAX_RECORDS)?.len() > MAX_RECORDS - 2
            {
                return Err(invalid("owner effect membership limit reached"));
            }
            if state
                .effect_history
                .owners
                .contains_key(&receipt.operation_id)
                || state
                    .effect_history
                    .members
                    .contains_key(&receipt.operation_id)
            {
                return Err(invalid(
                    "effect operation already has retained indexed membership",
                ));
            }
            let entry = Entry {
                root: live.cursor.root.clone(),
                ordinal: live.cursor.count + 1,
                previous: live.cursor.tip.clone(),
                operation: receipt.operation_id,
                receipt: receipt_request_digest(receipt)?,
            };
            live.cursor.count = entry.ordinal;
            live.cursor.tip = canonical_digest(1, &entry)?;
            state.effect_history.retain(&live.root, &live.cursor, false);
            state.effect_history.members.insert(
                entry.operation,
                Membership {
                    owner: permit.operation_id,
                    identity: entry.receipt.clone(),
                },
            );
            // A write failure leaves the retained high-water advanced/fenced.
            create(
                &self.root,
                &entry_name(permit.operation_id, entry.ordinal),
                &entry,
            )?;
            #[cfg(test)]
            let fault = state.effect_index_fault.take();
            self.root.write_observed(
                &name(permit.operation_id, "head"),
                &live.cursor,
                |_boundary| {
                    #[cfg(test)]
                    if fault == Some(_boundary) {
                        return Err(invalid("injected index head persistence failure"));
                    }
                    Ok(())
                },
            )
        })();
        let result = self.index_failure(state, permit.operation_id, result);
        if result.is_err() {
            let mut targets = receipt.canonical_targets.clone();
            targets.push(permit.work_key.clone());
            retain_effect_hold(state, receipt.operation_id, targets);
        }
        result
    }
    pub(super) fn prepare_indexed_effect(
        &self,
        state: &mut State,
        request: &MutationRequest,
        permit: Option<&Permit>,
        generation: u64,
    ) -> Result<(Receipt, bool)> {
        self.guard_indexed_request_id(state, request.request_id)?;
        let result = self.receipts.prepare(request, generation);
        if result.is_err()
            && let Some(permit) = permit
            && state.effect_indexes.contains_key(&permit.operation_id)
        {
            let mut targets = request.canonical_targets.clone();
            targets.push(permit.work_key.clone());
            retain_effect_hold(state, request.request_id, targets);
            return self.index_failure(state, permit.operation_id, result);
        }
        result
    }
    pub(super) fn index_effect_result<T>(
        &self,
        state: &mut State,
        receipt: &Receipt,
        result: Result<T>,
    ) -> Result<T> {
        let owner = state
            .effect_history
            .members
            .get(&receipt.operation_id)
            .map(|m| m.owner)
            .or_else(|| {
                receipt
                    .expected
                    .get("admission_operation")
                    .and_then(|v| Uuid::parse_str(v).ok())
            });
        if let Some(owner) = owner
            && state.effect_indexes.contains_key(&owner)
            && result.is_err()
        {
            let mut targets = receipt.canonical_targets.clone();
            if let Some(work) = receipt.expected.get("admitted_work") {
                targets.push(work.clone());
            }
            retain_effect_hold(state, receipt.operation_id, targets);
            return self.index_failure(state, owner, result);
        }
        result
    }
    pub(super) fn save_indexed_receipt(&self, state: &mut State, receipt: &Receipt) -> Result<()> {
        self.verify_indexed_receipt(state, receipt)?;
        self.index_effect_result(state, receipt, self.receipts.save(receipt))
    }
    pub(super) fn send_indexed_receipt(
        &self,
        state: &mut State,
        receipt: &mut Receipt,
    ) -> Result<()> {
        self.verify_indexed_receipt(state, receipt)?;
        let result = self.receipts.mark_sent(receipt);
        self.index_effect_result(state, receipt, result)
    }
    pub(super) fn acknowledge_indexed_receipt(
        &self,
        state: &mut State,
        receipt: &mut Receipt,
        ids: BTreeMap<String, String>,
        observed: Value,
    ) -> Result<()> {
        self.verify_indexed_receipt(state, receipt)?;
        let result = self.receipts.acknowledge(receipt, ids, observed);
        self.index_effect_result(state, receipt, result)
    }
    /// Resolve by original indexed operation, never by the current receipt's
    /// optional owner field. Cold validated history is read-only membership.
    fn indexed_member(&self, state: &State, operation: Uuid) -> Result<Option<Membership>> {
        if let Some(member) = state.effect_history.members.get(&operation) {
            return Ok(Some(member.clone()));
        }
        // Before classifying an operation as genuinely unindexed, validate the
        // bounded stored closure, including orphan entries and lost policy.
        Ok(recover(&self.root, &self.receipts)?
            .members
            .get(&operation)
            .cloned())
    }
    pub(super) fn verify_indexed_receipt(
        &self,
        state: &mut State,
        receipt: &Receipt,
    ) -> Result<()> {
        if let Some(anchor) = state.effect_history.owners.get(&receipt.operation_id) {
            let owner = receipt.operation_id;
            let expected = anchor.root.admission_digest.clone();
            let checked = (|| {
                if receipt.kind != "admission" || receipt_request_digest(receipt)? != expected {
                    return Err(invalid("owner differs from its retained indexed identity"));
                }
                self.index_inventory(state, owner)?
                    .ok_or_else(|| invalid("known owner lost its index declaration"))?;
                Ok(())
            })();
            return self.index_failure(state, owner, checked);
        }
        let member = self.indexed_member(state, receipt.operation_id)?;
        let declared = receipt
            .expected
            .get("admission_operation")
            .map(|v| Uuid::parse_str(v).map_err(|_| invalid("effect owner identity is invalid")))
            .transpose()?;
        let owner = member.as_ref().map(|m| m.owner).or(declared);
        let Some(owner) = owner else { return Ok(()) };
        let result = (|| {
            if let Some(member) = member {
                if declared != Some(member.owner)
                    || receipt_request_digest(receipt)? != member.identity
                {
                    return Err(invalid("effect differs from its retained indexed identity"));
                }
                let inventory = self
                    .index_inventory(state, member.owner)?
                    .ok_or_else(|| invalid("indexed effect owner lost its declaration"))?;
                if inventory.members.get(&receipt.operation_id) != Some(&member.identity) {
                    return Err(invalid("effect has no exact indexed membership"));
                }
            } else if self.index_inventory(state, owner)?.is_some() {
                return Err(invalid("effect has no retained indexed membership"));
            }
            Ok(())
        })();
        self.index_failure(state, owner, result)
    }
    pub(super) fn verify_effect_identity(
        &self,
        state: &mut State,
        receipt: &Receipt,
        operation: Uuid,
        identity: &str,
        owner: Option<Uuid>,
    ) -> Result<()> {
        let checked = (|| {
            if receipt.operation_id != operation
                || receipt.request_id != operation
                || receipt_request_digest(receipt)? != identity
            {
                return Err(invalid(
                    "effect receipt differs from its actual reserved identity",
                ));
            }
            self.verify_indexed_receipt(state, receipt)
        })();
        if let Some(owner) = owner {
            self.index_failure(state, owner, checked)
        } else {
            checked
        }
    }
    pub(super) fn freeze_effect_index(
        &self,
        state: &mut State,
        receipt: &mut Receipt,
        kind: &str,
        caller_observation: &Value,
    ) -> Result<()> {
        let owner = receipt.operation_id;
        let result = (|| {
            let Some(inventory) = self.index_inventory(state, owner)? else {
                return Ok(());
            };
            let live = state
                .effect_indexes
                .get_mut(&owner)
                .ok_or_else(|| invalid("cold records cannot freeze an owner index"))?;
            live.frozen = true;
            let digest = canonical_digest(1, &live.cursor)?;
            receipt.remote_ids.insert(KEY.into(), digest);
            let request = canonical_digest(1, &(kind, caller_observation))?;
            if let Some(previous) = &live.retirement {
                if live.retirement_request.as_ref() != Some(&request)
                    || receipt_request_digest(previous)? != receipt_request_digest(receipt)?
                {
                    return Err(invalid(
                        "owner retirement retry changed its original observation",
                    ));
                }
                // A late actual ACK may settle current holds after the first
                // persistence failure. Preserve the original terminal snapshot;
                // never turn that progress into a different retirement request.
                *receipt = previous.clone();
            } else {
                live.retirement_request = Some(request);
                live.retirement = Some(receipt.clone());
            }
            state.effect_history.retain(&live.root, &live.cursor, true);
            if inventory.frozen.is_none() {
                create(&self.root, &name(owner, "frozen"), &live.cursor)?;
            }
            Ok(())
        })();
        self.index_failure(state, owner, result)
    }
    pub(super) fn save_indexed_owner(&self, state: &mut State, receipt: &Receipt) -> Result<()> {
        if !required(receipt)? {
            return self.receipts.save(receipt);
        }
        #[cfg(test)]
        let fault = state.effect_index_fault.take();
        let result = self.receipts.directory.write_observed(
            &format!("{}.json", receipt.request_id),
            receipt,
            |_boundary| {
                #[cfg(test)]
                if fault == Some(_boundary) {
                    return Err(invalid("injected owner retirement persistence failure"));
                }
                Ok(())
            },
        );
        self.index_failure(state, receipt.operation_id, result)
    }
    pub(super) fn indexed_reconciliation(
        &self,
        state: &mut State,
        receipt: &Receipt,
        observation: Reconciliation,
    ) -> Result<Reconciliation> {
        self.verify_indexed_receipt(state, receipt)?;
        if receipt.kind != "admission" {
            return Ok(observation);
        }
        if state.permits.contains_key(&receipt.operation_id)
            && (state.effect_indexes.contains_key(&receipt.operation_id) || required(receipt)?)
        {
            return Err(Error::new(
                "operation_active",
                "indexed owner must retire before reconciliation",
            ));
        }
        let result = (|| {
            let Some(inventory) = self.index_inventory(state, receipt.operation_id)? else {
                return Ok(observation);
            };
            let frozen = inventory
                .frozen
                .ok_or_else(|| invalid("owner reconciliation cannot invent frozen membership"))?;
            match observation {
                Reconciliation::UniqueVerified {
                    mut remote_ids,
                    result,
                    payload_digest,
                    actor,
                } => {
                    let digest = canonical_digest(1, &frozen)?;
                    if remote_ids.get(KEY).is_some_and(|v| v != &digest) {
                        return Err(invalid(
                            "reconciliation substitutes frozen owner membership",
                        ));
                    }
                    remote_ids.insert(KEY.into(), digest);
                    Ok(Reconciliation::UniqueVerified {
                        remote_ids,
                        result,
                        payload_digest,
                        actor,
                    })
                }
                other => Ok(other),
            }
        })();
        self.index_failure(state, receipt.operation_id, result)
    }
}

#[cfg(test)]
#[path = "control_effect_index_tests.rs"]
mod tests;
