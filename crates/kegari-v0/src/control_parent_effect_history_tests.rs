use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

// Actual Control/private storage component only. Retirement observations below
// are labeled data: no native worker join or process/check capability is created.
struct Fixture {
    _root: tempfile::TempDir,
    host: ControlHost,
    lease: Lease,
    run: RunReference,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let host = ControlHost::setup(
            PrivateDir::create(&root.path().canonicalize().unwrap().join("control")).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        host.lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: runtime,
                actor: Actor {
                    identity: "component operator".into(),
                    roles: ["lifecycle".into()].into(),
                    principal: None,
                },
                kind: "start".into(),
                canonical_targets: vec![runtime.to_string()],
                expected: BTreeMap::new(),
                payload: Value::Null,
            },
            Lifecycle::Start,
            0,
        )
        .unwrap();
        let lease = Lease {
            runtime_id: runtime,
            owner_generation: 1,
            owner_id: Uuid::new_v4(),
        };
        host.attach_reconciled(lease.clone()).unwrap();
        let run = RunReference {
            workspace: Uuid::new_v4(),
            account: "component".into(),
            normalized_state: None,
            phase: crate::scheduler::PhaseKey {
                work: format!("component-parent:{}", Uuid::new_v4()),
                generation: digest("generation"),
                phase: "parent_evidence_review".into(),
                retry_grant: None,
            },
            profile_digest: digest("profile"),
            instruction_digest: digest("instructions"),
            max_attempts: 2,
            max_turns: 3,
            max_review_rounds: 2,
            attempt: 1,
        };
        Self {
            _root: root,
            host,
            lease,
            run,
        }
    }
    fn prep_reference(&self, model: Uuid) -> PreparationReference {
        PreparationReference {
            worker_operation: model,
            workspace: self.run.workspace,
            account: self.run.account.clone(),
            phase: self.run.phase.clone(),
        }
    }
    fn prepare(&self, model: Uuid) -> Receipt {
        let reference = self.prep_reference(model);
        let permit = self
            .host
            .admit_preparation(
                &self.lease,
                PreparationAdmission {
                    request_id: reference.operation_id().unwrap(),
                    work_key: self.run.phase.work.clone(),
                    scope_digest: digest("preparation scope"),
                    reference,
                },
                || Ok(()),
            )
            .unwrap();
        self.host
            .retire_local(
                permit.operation_id,
                json!({"phase":"worker_preparation","result":"prepared","component_only":true}),
            )
            .unwrap()
    }
    fn model(&self, model: Uuid) -> Permit {
        self.host
            .admit_run(
                &self.lease,
                RunAdmission {
                    request_id: model,
                    work_key: self.run.phase.work.clone(),
                    scope_digest: digest("model scope"),
                    reference: self.run.clone(),
                },
                || Ok(()),
            )
            .unwrap()
    }
    fn request(&self, label: &str) -> MutationRequest {
        MutationRequest {
            request_id: Uuid::new_v4(),
            runtime_id: self.lease.runtime_id,
            actor: Actor {
                identity: format!("owner:{}", self.lease.owner_id),
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: label.into(),
            canonical_targets: vec![format!("component:{label}")],
            expected: BTreeMap::new(),
            payload: json!({"component":label}),
        }
    }
    fn read(&self, prep: Uuid, model: Uuid) -> Result<ParentEffectHistoryBinding> {
        self.host
            .with_parent_effect_history(prep, model, |_, history| Ok(history.binding().clone()))
    }
    /// Explicit legacy receipt DATA compatibility, not a legacy admission or
    /// worker capability. The actual protected reader must label it incomplete.
    fn legacy_owner_data(&self, model: Uuid, preparation: bool) -> Receipt {
        let mut expected = BTreeMap::from([("scope".into(), digest("legacy scope"))]);
        let operation = if preparation {
            let reference = self.prep_reference(model);
            expected.insert(
                "worker_preparation".into(),
                serde_json::to_string(&reference).unwrap(),
            );
            reference.operation_id().unwrap()
        } else {
            expected.insert("run".into(), serde_json::to_string(&self.run).unwrap());
            model
        };
        let (mut receipt, fresh) = self
            .host
            .receipts
            .prepare(
                &MutationRequest {
                    request_id: operation,
                    runtime_id: self.lease.runtime_id,
                    actor: Actor {
                        identity: format!("owner:{}", self.lease.owner_id),
                        roles: BTreeSet::new(),
                        principal: None,
                    },
                    kind: "admission".into(),
                    canonical_targets: vec![self.run.phase.work.clone()],
                    expected,
                    payload: json!({"fixture":"legacy historical receipt data, no permit"}),
                },
                self.lease.owner_generation,
            )
            .unwrap();
        assert!(fresh);
        receipt.phase = ReceiptPhase::Reconciled;
        receipt.observed_result = Some(
            json!({"local_execution":"exited","unresolved_effects":[],"observation":{"phase":"worker_preparation","result":"prepared","component_only":true}}),
        );
        self.host.receipts.save(&receipt).unwrap();
        receipt
    }
}
fn digest(value: &str) -> String {
    canonical_digest(1, &value).unwrap()
}

#[test]
fn independent_integrated_review_preserves_distinct_preparation_and_model_history() {
    let mut f = Fixture::new();
    f.run.phase.phase = "parent_integrated_review".into();
    let operation = Uuid::new_v4();
    let preparation = f.prepare(operation);
    let model = f.model(operation);
    assert!(f.read(preparation.operation_id, operation).is_err());
    f.host
        .retire_local(model.operation_id, json!({"component_only":true}))
        .unwrap();
    let original = f.read(preparation.operation_id, operation).unwrap();
    assert!(original.has_indexed_owner());
    // A different synthesis owner cannot replace the actual read-only review.
    f.run.phase.phase = "parent_evidence_review".into();
    let unrelated = f.model(Uuid::new_v4());
    f.host
        .retire_local(unrelated.operation_id, json!({"component_only":true}))
        .unwrap();
    assert!(
        f.read(preparation.operation_id, unrelated.operation_id)
            .is_err()
    );
    assert_eq!(
        f.read(preparation.operation_id, operation).unwrap(),
        original
    );
}

#[test]
fn real_control_pair_retains_all_reserved_outcomes_without_reader_writes() {
    let f = Fixture::new();
    let model = Uuid::new_v4();
    let prep = f.prepare(model);
    let permit = f.model(model);
    let mut expected = BTreeMap::new();
    let local_request = f.request("local");
    let (local, ()) = f
        .host
        .begin_effect(local_request, &permit, || Ok(()), |_| Ok(()))
        .unwrap();
    let receipt = f
        .host
        .complete_effect(local, &permit, || Ok(()), json!({"component_exit":true}))
        .unwrap();
    expected.insert(receipt.operation_id, canonical_digest(1, &receipt).unwrap());
    let receipt = f
        .host
        .mutate(
            f.request("sync"),
            Some(&permit),
            None,
            || Ok(()),
            |_| Ok((BTreeMap::new(), json!({"error":{"code":"known_failure"}}))),
        )
        .unwrap();
    expected.insert(receipt.operation_id, canonical_digest(1, &receipt).unwrap());
    let RemoteReservation::Prepared(reservation) = f
        .host
        .reserve_remote(f.request("remote"), Some(permit.clone()), None, |_| Ok(()))
        .unwrap()
    else {
        panic!()
    };
    let (remote, ()) = f
        .host
        .launch_remote(reservation, Ok(()), |_| Ok(()), |_| {})
        .unwrap();
    let receipt = f
        .host
        .complete_remote(remote, Ok((BTreeMap::new(), json!({"component_ack":true}))))
        .unwrap();
    expected.insert(receipt.operation_id, canonical_digest(1, &receipt).unwrap());
    let rejected = f.request("not_sent");
    assert_eq!(
        f.host
            .reserve_remote(rejected.clone(), Some(permit.clone()), None, |_| Err(
                Error::new("component_refusal", "known before send")
            ))
            .err()
            .unwrap()
            .code,
        "component_refusal"
    );
    let receipt = f.host.operation(rejected.request_id).unwrap();
    expected.insert(receipt.operation_id, canonical_digest(1, &receipt).unwrap());
    f.host
        .retire_local(model, json!({"component_only":true}))
        .unwrap();
    let before = canonical_digest(1, &f.host.operations().unwrap()).unwrap();
    f.host
        .with_parent_effect_history(prep.operation_id, model, |snapshot, history| {
            assert!(snapshot.active_operations.is_empty());
            assert_eq!(history.preparation().operation_id, prep.operation_id);
            assert_eq!(history.model().operation_id, model);
            assert_eq!(
                history
                    .effects()
                    .iter()
                    .map(|r| Ok((r.operation_id, canonical_digest(1, r)?)))
                    .collect::<Result<BTreeMap<_, _>>>()?,
                expected
            );
            assert!(matches!(
                history.binding.preparation,
                OwnerEffectHistory::FrozenControlIndexV1 { count: 0, .. }
            ));
            assert!(matches!(
                history.binding.model,
                OwnerEffectHistory::FrozenControlIndexV1 { count: 4, .. }
            ));
            Ok(())
        })
        .unwrap();
    assert_eq!(
        canonical_digest(1, &f.host.operations().unwrap()).unwrap(),
        before
    );
}

#[test]
fn missing_or_substituted_indexed_receipt_blocks_before_first_consumer_callback() {
    let f = Fixture::new();
    let model = Uuid::new_v4();
    let prep = f.prepare(model);
    let permit = f.model(model);
    let receipt = f
        .host
        .mutate(
            f.request("sync"),
            Some(&permit),
            None,
            || Ok(()),
            |_| Ok((BTreeMap::new(), json!({"component":"sync"}))),
        )
        .unwrap();
    f.host
        .retire_local(model, json!({"component_only":true}))
        .unwrap();
    let leaf = format!("{}.json", receipt.operation_id);
    let bytes = f
        .host
        .receipts
        .directory
        .read_bytes(&leaf, MAX_BYTES as u64)
        .unwrap();
    std::fs::remove_file(f.host.receipts.directory.path().join(&leaf)).unwrap();
    let callbacks = AtomicUsize::new(0);
    assert_eq!(
        f.host
            .with_parent_effect_history(prep.operation_id, model, |_, _| {
                callbacks.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap_err()
            .code,
        "record_missing"
    );
    assert_eq!(callbacks.load(Ordering::SeqCst), 0);
    f.host
        .receipts
        .directory
        .write_bytes(&leaf, &bytes)
        .unwrap();
    let original = f.read(prep.operation_id, model).unwrap();
    for field in ["admission_operation", "admitted_work", "admitted_scope"] {
        let mut changed = receipt.clone();
        changed.expected.remove(field);
        f.host.receipts.save(&changed).unwrap();
        assert_eq!(
            f.read(prep.operation_id, model).unwrap_err().code,
            "effect_index"
        );
        f.host
            .receipts
            .directory
            .write_bytes(&leaf, &bytes)
            .unwrap();
    }
    assert_eq!(f.read(prep.operation_id, model).unwrap(), original);
}

#[test]
fn frozen_zero_requires_all_original_files_and_known_identity_never_becomes_legacy() {
    let f = Fixture::new();
    let model = Uuid::new_v4();
    let prep = f.prepare(model);
    let _permit = f.model(model);
    let owner = f
        .host
        .retire_local(model, json!({"component_only":true}))
        .unwrap();
    let binding = f.read(prep.operation_id, model).unwrap();
    for suffix in ["root", "head", "frozen"] {
        let leaf = name(model, suffix);
        let bytes = f.host.root.read_bytes(&leaf, MAX_BYTES as u64).unwrap();
        std::fs::remove_file(f.host.root.path().join(&leaf)).unwrap();
        let error = f.read(prep.operation_id, model).unwrap_err();
        assert_eq!(
            error.code,
            if suffix == "frozen" {
                "effect_index"
            } else {
                "record_missing"
            }
        );
        f.host.root.create_bytes_exclusive(&leaf, &bytes).unwrap();
    }
    assert_eq!(f.read(prep.operation_id, model).unwrap(), binding);
    let mut downgraded = owner;
    downgraded.expected.remove(KEY);
    downgraded.remote_ids.remove(KEY);
    f.host.receipts.save(&downgraded).unwrap();
    assert_eq!(
        f.read(prep.operation_id, model).unwrap_err().message,
        "known indexed parent owner cannot become legacy history"
    );
    for suffix in ["root", "head", "frozen"] {
        std::fs::remove_file(f.host.root.path().join(name(model, suffix))).unwrap();
    }
    assert_eq!(
        f.read(prep.operation_id, model).unwrap_err().message,
        "known indexed parent owner cannot become legacy history"
    );
}

#[test]
fn late_actual_ack_preserves_membership_but_does_not_rewrite_original_unresolved_owner() {
    let f = Fixture::new();
    let model = Uuid::new_v4();
    let prep = f.prepare(model);
    let permit = f.model(model);
    let RemoteReservation::Prepared(reservation) = f
        .host
        .reserve_remote(f.request("pending"), Some(permit.clone()), None, |_| Ok(()))
        .unwrap()
    else {
        panic!()
    };
    let (remote, ()) = f
        .host
        .launch_remote(reservation, Ok(()), |_| Ok(()), |_| {})
        .unwrap();
    let owner = f
        .host
        .retire_local(model, json!({"component_only":true}))
        .unwrap();
    let frozen = f
        .host
        .root
        .read_bytes(&name(model, "frozen"), MAX_BYTES as u64)
        .unwrap();
    assert_eq!(
        f.read(prep.operation_id, model).unwrap_err().message,
        "parent effect history requires closed owner observations"
    );
    f.host
        .complete_remote(
            remote,
            Ok((BTreeMap::new(), json!({"actual_component_ack":true}))),
        )
        .unwrap();
    assert_eq!(
        f.host
            .root
            .read_bytes(&name(model, "frozen"), MAX_BYTES as u64)
            .unwrap(),
        frozen
    );
    assert_eq!(
        canonical_digest(1, &f.host.operation(model).unwrap()).unwrap(),
        canonical_digest(1, &owner).unwrap()
    );
    assert_eq!(
        f.read(prep.operation_id, model).unwrap_err().message,
        "parent effect history requires closed owner observations"
    );
}

#[test]
fn partial_legacy_data_never_invents_the_other_owners_index() {
    for (legacy_preparation, legacy_model) in [(true, true), (true, false), (false, true)] {
        let f = Fixture::new();
        let model = Uuid::new_v4();
        let prep = if legacy_preparation {
            f.legacy_owner_data(model, true)
        } else {
            f.prepare(model)
        };
        if legacy_model {
            f.legacy_owner_data(model, false);
        } else {
            let _permit = f.model(model);
            f.host
                .retire_local(model, json!({"component_only":true}))
                .unwrap();
        }
        let binding = f.read(prep.operation_id, model).unwrap();
        assert_eq!(
            matches!(binding.preparation, OwnerEffectHistory::LegacyRetainedScan),
            legacy_preparation
        );
        assert_eq!(
            matches!(binding.model, OwnerEffectHistory::LegacyRetainedScan),
            legacy_model
        );
        assert_eq!(
            binding.has_indexed_owner(),
            !legacy_preparation || !legacy_model
        );
        assert_eq!(
            serde_json::from_value::<ParentEffectHistoryBinding>(
                serde_json::to_value(&binding).unwrap()
            )
            .unwrap(),
            binding
        );
    }
}

#[test]
fn cold_reader_restores_comparison_history_without_a_live_index_writer() {
    let f = Fixture::new();
    let model = Uuid::new_v4();
    let prep = f.prepare(model);
    let _permit = f.model(model);
    f.host
        .retire_local(model, json!({"component_only":true}))
        .unwrap();
    let before = f.read(prep.operation_id, model).unwrap();
    let path = f.host.root.path().to_path_buf();
    let runtime = f.lease.runtime_id;
    let Fixture {
        _root: root, host, ..
    } = f;
    drop(host);
    let host = ControlHost::recover(PrivateDir::open(&path).unwrap(), runtime).unwrap();
    assert!(host.state.lock().unwrap().effect_indexes.is_empty());
    let after = host
        .with_parent_effect_history(prep.operation_id, model, |snapshot, history| {
            assert!(snapshot.active_operations.is_empty());
            Ok(history.binding().clone())
        })
        .unwrap();
    assert_eq!(after, before);
    drop(host);
    drop(root);
}
