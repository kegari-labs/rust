//! Actual Control receipt/preparation/admission tests only. Provider publication
//! and terminal model observations below are synthetic component inputs. They
//! neither create human acceptance nor any opaque reassembly/joined capability.
use super::*;
use serde_json::json;
fn digest(label: &str) -> String {
    canonical_digest(1, &label).unwrap()
}
fn exact(receipt: &Receipt) -> ReviewReceiptReference {
    ReviewReceiptReference {
        operation: receipt.operation_id,
        receipt_digest: canonical_digest(1, receipt).unwrap(),
    }
}
fn finished() -> Value {
    json!({"outcome":"completed","local_exit_confirmed":true,"hook_error":null,
        "session":{"terminal":"completed","input_required":false,"intervention":null}})
}
struct Fixture {
    _directory: tempfile::TempDir,
    host: ControlHost,
    lease: Lease,
    reference: ParentReassemblyReference,
    run: RunReference,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let host = ControlHost::setup(
            PrivateDir::create(&directory.path().canonicalize().unwrap().join("control")).unwrap(),
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
        let work = WorkIdentity {
            linear_endpoint: "https://linear.invalid/graphql".into(),
            linear_workspace: Uuid::new_v4(),
            parent: Uuid::new_v4(),
            child: None,
            generation: crate::evidence::Generation {
                authorization: digest("authorization"),
                definition: digest("definition"),
                correction: digest("correction"),
                scope: digest("scope"),
            },
        };
        let run = RunReference {
            workspace: Uuid::new_v4(),
            account: "actual component account".into(),
            normalized_state: None,
            phase: PhaseKey {
                work: work_key(&work).unwrap(),
                generation: canonical_digest(1, &work.generation).unwrap(),
                phase: "parent_evidence_review".into(),
                retry_grant: None,
            },
            profile_digest: digest("profile"),
            instruction_digest: digest("instruction"),
            max_attempts: 2,
            max_turns: 3,
            max_review_rounds: 2,
            attempt: 1,
        };
        let previous = host
            .admit_run(
                &lease,
                RunAdmission {
                    request_id: Uuid::new_v4(),
                    work_key: run.phase.work.clone(),
                    scope_digest: digest("old source"),
                    reference: run.clone(),
                },
                || Ok(()),
            )
            .unwrap();
        let previous = host
            .retire_local(previous.operation_id, finished())
            .unwrap();
        let source = host
            .admit(
                &lease,
                Uuid::new_v4(),
                run.phase.work.clone(),
                digest("assembly"),
                || Ok(()),
            )
            .unwrap();
        let native = format!(
            "linear:{}:{}:{}",
            work.linear_endpoint, work.linear_workspace, work.parent
        );
        let publication = |label: &str| {
            host.mutate(
                MutationRequest {
                    request_id: Uuid::new_v4(),
                    runtime_id: runtime,
                    actor: Actor {
                        identity: format!("owner:{}", lease.owner_id),
                        roles: BTreeSet::new(),
                        principal: None,
                    },
                    kind: "linear_comment".into(),
                    canonical_targets: vec![native.clone()],
                    expected: BTreeMap::from([("provider_body_digest".into(), digest(label))]),
                    payload: json!({"component":"synthetic native comment; no human capability"}),
                },
                Some(&source),
                None,
                || Ok(()),
                |_| {
                    Ok((
                        BTreeMap::from([("comment_id".into(), Uuid::new_v4().to_string())]),
                        json!({"acknowledged":true}),
                    ))
                },
            )
            .unwrap()
        };
        let original_parent = publication("parent body");
        let original_acceptance = publication("acceptance body");
        let source = host.retire_local(source.operation_id,json!({
            "parent_reassembly_vector":digest("vector"),"parent_reassembly_baseline":digest("baseline"),"original_reviewed_token":digest("original token")
        })).unwrap();
        let mut reference = ParentReassemblyReference {
            grants: Vec::new(),
            retry_of: None,
            transient_of: None,
            history: BTreeMap::new(),
            schema_version: 1,
            runtime,
            workspace: run.workspace,
            account: run.account.clone(),
            operation: Uuid::new_v4(),
            parent_work: work.clone(),
            worker_work: work,
            phase: run.phase.clone(),
            predecessor: exact(&previous),
            source: exact(&source),
            original_parent: exact(&original_parent),
            original_acceptance: exact(&original_acceptance),
            original_token: digest("original token"),
            vector_digest: digest("vector"),
            baseline_digest: digest("baseline"),
            assignment_digest: digest("assignment"),
            progress_digest: digest("progress"),
            inventory_digest: digest("placeholder"),
            attempt: 2,
            max_attempts: 2,
            max_turns: 3,
            max_review_rounds: 2,
        };
        reference.inventory_digest =
            parent_reassembly_inventory(&host.operations().unwrap(), &reference).unwrap();
        Self {
            _directory: directory,
            host,
            lease,
            reference,
            run,
        }
    }
    fn spaced(&self) {
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }
    fn input(&self) -> RunAdmission {
        RunAdmission {
            request_id: self.reference.operation,
            work_key: self.run.phase.work.clone(),
            scope_digest: digest("new source"),
            reference: RunReference {
                attempt: 2,
                ..self.run.clone()
            },
        }
    }
    fn prepare(&self) -> ReviewReceiptReference {
        self.prepare_with(json!({"phase":"worker_preparation","result":"prepared"}))
    }
    fn prepare_with(&self, observation: Value) -> ReviewReceiptReference {
        let reference = PreparationReference {
            worker_operation: self.reference.operation,
            workspace: self.reference.workspace,
            account: self.reference.account.clone(),
            phase: self.reference.phase.clone(),
        };
        let permit = self
            .host
            .admit_parent_reassembly_preparation_observed(
                &self.lease,
                PreparationAdmission {
                    request_id: reference.operation_id().unwrap(),
                    work_key: reference.phase.work.clone(),
                    scope_digest: digest("preparation"),
                    reference,
                },
                self.reference.clone(),
                |_| Ok(()),
            )
            .unwrap();
        exact(
            &self
                .host
                .retire_local(permit.operation_id, observation)
                .unwrap(),
        )
    }
}
#[test]
fn actual_parent_preparation_is_required_and_final_callback_leaves_no_receipt() {
    let f = Fixture::new();
    f.spaced();
    let preparation = f.prepare();
    let before = f.host.operations().unwrap().len();
    let mut wrong = preparation.clone();
    wrong.receipt_digest = digest("substituted prep");
    let error = f
        .host
        .admit_parent_reassembly_run_observed(
            &f.lease,
            f.input(),
            f.reference.clone(),
            wrong,
            |_| Ok(()),
        )
        .unwrap_err();
    assert_eq!(error.code, "parent_reassembly");
    assert_eq!(f.host.operations().unwrap().len(), before);
    let error = f
        .host
        .admit_parent_reassembly_run_observed(
            &f.lease,
            f.input(),
            f.reference.clone(),
            preparation.clone(),
            |_| {
                Err(Error::new(
                    "stale_result",
                    "actual component current gate denied",
                ))
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "stale_result");
    assert_eq!(f.host.operations().unwrap().len(), before);
    let permit = f
        .host
        .admit_parent_reassembly_run_observed(
            &f.lease,
            f.input(),
            f.reference.clone(),
            preparation.clone(),
            |_| Ok(()),
        )
        .unwrap();
    let receipt = f.host.operation(permit.operation_id).unwrap();
    assert_eq!(
        ParentReassemblyReference::from_receipt(&receipt).unwrap(),
        Some(f.reference.clone())
    );
    assert_eq!(
        serde_json::from_str::<ReviewReceiptReference>(&receipt.expected[PREPARATION]).unwrap(),
        preparation
    );
    let actual = RunReference::from_receipt(&receipt).unwrap().unwrap();
    assert_eq!(
        (
            actual.attempt,
            actual.max_attempts,
            actual.max_turns,
            actual.max_review_rounds
        ),
        (2, 2, 3, 2)
    );
    f.host
        .retire_local(permit.operation_id, finished())
        .unwrap();
    let mut next = f.input();
    next.request_id = Uuid::new_v4();
    next.reference.attempt = 3;
    let error = f.host.admit_run(&f.lease, next, || Ok(())).unwrap_err();
    assert_eq!(error.code, "record_corrupt");
    let prep = PreparationReference {
        worker_operation: Uuid::new_v4(),
        workspace: f.reference.workspace,
        account: f.reference.account.clone(),
        phase: f.reference.phase.clone(),
    };
    let error = f
        .host
        .admit_preparation(
            &f.lease,
            PreparationAdmission {
                request_id: prep.operation_id().unwrap(),
                work_key: prep.phase.work.clone(),
                scope_digest: digest("unmarked preparation"),
                reference: prep,
            },
            || Ok(()),
        )
        .unwrap_err();
    assert_eq!(error.code, "parent_reassembly_required");
}
#[test]
fn exact_inventory_caps_and_unknown_exit_cannot_be_relabelled() {
    let f = Fixture::new();
    f.spaced();
    let receipts = f.host.operations().unwrap();
    let snapshot = f.host.snapshot().unwrap();
    verify_parent_reassembly_reference(&snapshot, &f.reference, &receipts).unwrap();
    let mut raised = f.reference.clone();
    raised.max_attempts = 3;
    assert_eq!(
        verify_parent_reassembly_reference(&snapshot, &raised, &receipts)
            .unwrap_err()
            .code,
        "attempts_exhausted"
    );
    let mut changed = receipts.clone();
    let previous = changed
        .iter_mut()
        .find(|r| r.operation_id == f.reference.predecessor.operation)
        .unwrap();
    previous.observed_result.as_mut().unwrap()["observation"]["local_exit_confirmed"] =
        json!(false);
    let mut linked = f.reference.clone();
    linked.predecessor = exact(previous);
    linked.inventory_digest = parent_reassembly_inventory(&changed, &linked).unwrap();
    assert_eq!(
        verify_parent_reassembly_reference(&snapshot, &linked, &changed)
            .unwrap_err()
            .code,
        "parent_reassembly"
    );
    let mut mixed = receipts.clone();
    let previous = mixed
        .iter_mut()
        .find(|r| r.operation_id == f.reference.predecessor.operation)
        .unwrap();
    let mut run = RunReference::from_receipt(previous).unwrap().unwrap();
    run.phase.retry_grant = Some(Uuid::new_v4());
    previous
        .expected
        .insert("run".into(), serde_json::to_string(&run).unwrap());
    assert_eq!(
        parent_reassembly_inventory(&mixed, &f.reference)
            .unwrap_err()
            .code,
        "parent_reassembly"
    );
}
#[test]
fn strict_reference_rejects_wrong_namespace_and_unbounded_input() {
    let f = Fixture::new();
    let mut wrong = f.reference.clone();
    wrong.worker_work.child = Some(Uuid::new_v4());
    assert_eq!(wrong.validate().unwrap_err().code, "parent_reassembly");
    let mut wrong = f.reference.clone();
    wrong.phase.retry_grant = Some(Uuid::new_v4());
    assert_eq!(wrong.validate().unwrap_err().code, "parent_reassembly");
    let mut wrong = f.reference.clone();
    wrong.attempt = 1;
    assert_eq!(wrong.validate().unwrap_err().code, "parent_reassembly");
    let mut value = serde_json::to_value(&f.reference).unwrap();
    value["phase"]["unexpected"] = json!(true);
    assert!(serde_json::from_value::<ParentReassemblyReference>(value).is_err());
}

#[test]
fn actual_closed_failed_parent_preparation_cannot_authorize_model_admission() {
    use std::sync::atomic::{AtomicBool, Ordering};
    for observation in [
        json!({"phase":"worker_preparation","result":"failed"}),
        json!({"phase":"worker_preparation","result":"blocked"}),
        json!({"phase":"another_phase","result":"prepared"}),
        json!({"result":"prepared"}),
    ] {
        let f = Fixture::new();
        f.spaced();
        let preparation = f.prepare_with(observation);
        let actual = f.host.operation(preparation.operation).unwrap();
        assert_eq!(actual.phase, ReceiptPhase::Reconciled);
        assert_eq!(
            actual.observed_result.as_ref().unwrap()["local_execution"],
            "exited"
        );
        // Exact authentic hash/owner/marker is deliberately valid; the result
        // shape alone must prevent this model admission and final callback.
        let before = f.host.operations().unwrap().len();
        let callback = AtomicBool::new(false);
        let error = f
            .host
            .admit_parent_reassembly_run_observed(
                &f.lease,
                f.input(),
                f.reference.clone(),
                preparation,
                |_| {
                    callback.store(true, Ordering::SeqCst);
                    Ok(())
                },
            )
            .unwrap_err();
        assert_eq!(error.code, "parent_reassembly");
        assert_eq!(
            error.message,
            "parent reassembly requires an actually successful closed preparation"
        );
        assert!(!callback.load(Ordering::SeqCst));
        assert_eq!(f.host.operations().unwrap().len(), before);
    }
}
#[test]
fn actual_marked_parent_history_cannot_be_dropped_in_a_new_grant_namespace() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let f = Fixture::new();
    f.spaced();
    let preparation = f.prepare();
    let marked = f
        .host
        .admit_parent_reassembly_run_observed(
            &f.lease,
            f.input(),
            f.reference.clone(),
            preparation,
            |_| Ok(()),
        )
        .unwrap();
    f.host.retire_local(marked.operation_id,json!({"outcome":"failed","local_exit_confirmed":true,"hook_error":null,"error":{"code":"provider_transport","message":"component terminal input","retryable":true,"details":null,"next_action":null},"session":{"terminal":"failed","input_required":false,"intervention":null}})).unwrap();
    let mut phase = f.run.phase.clone();
    phase.retry_grant = Some(Uuid::new_v4());
    // The UUID is synthetic namespace data, not a VerifiedRetryGrant. Raw
    // Control and generic planning must reject it before any caller can supply
    // a current grant callback; this does not claim native grant qualification.
    let input = RunAdmission {
        request_id: Uuid::new_v4(),
        work_key: phase.work.clone(),
        scope_digest: digest("new budget"),
        reference: RunReference {
            phase: phase.clone(),
            attempt: 1,
            ..f.run.clone()
        },
    };
    let before = f.host.operations().unwrap().len();
    let callback = AtomicBool::new(false);
    let error = f
        .host
        .admit_run(&f.lease, input, || {
            callback.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.code, "parent_reassembly_required");
    assert!(!callback.load(Ordering::SeqCst));
    assert_eq!(f.host.operations().unwrap().len(), before);
    let prep = PreparationReference {
        worker_operation: Uuid::new_v4(),
        workspace: f.run.workspace,
        account: f.run.account.clone(),
        phase: phase.clone(),
    };
    let error = f
        .host
        .admit_preparation(
            &f.lease,
            PreparationAdmission {
                request_id: prep.operation_id().unwrap(),
                work_key: phase.work.clone(),
                scope_digest: digest("new grant preparation"),
                reference: prep,
            },
            || {
                callback.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap_err();
    assert_eq!(error.code, "parent_reassembly_required");
    assert!(!callback.load(Ordering::SeqCst));
    assert_eq!(f.host.operations().unwrap().len(), before);
    let runtime: kegari_config::RuntimeConfig = serde_json::from_value(
        json!({"id":f.lease.runtime_id,"host_id":"component","max_agents":1}),
    )
    .unwrap();
    assert_eq!(
        crate::runtime_schedule::plan(
            &runtime,
            &phase,
            f.run.workspace,
            &f.host.operations().unwrap(),
            chrono::Utc::now()
        )
        .unwrap_err()
        .code,
        "parent_reassembly_required"
    );
    for field in ["work", "generation", "phase"] {
        let mut other = phase.clone();
        match field {
            "work" => other.work = digest("otherwork"),
            "generation" => other.generation = digest("othergeneration"),
            _ => other.phase = "otherphase".into(),
        };
        reject_unmarked_parent_phase(&other, &f.host.operations().unwrap()).unwrap();
    }
}

// This is data-gate evidence only. Native grants are exercised independently
// through DecisionManager/provider-wire; no opaque parent worker is fabricated.
#[test]
fn marked_failed_parent_grant_preserves_source_and_renews_only_exact_budget() {
    let mut f = Fixture::new();
    f.spaced();
    let preparation = f.prepare();
    let permit = f
        .host
        .admit_parent_reassembly_run_observed(
            &f.lease,
            f.input(),
            f.reference.clone(),
            preparation,
            |_| Ok(()),
        )
        .unwrap();
    let old_reference = f.reference.clone();
    let failed = f
        .host
        .retire_local(
            permit.operation_id,
            json!({
                "outcome":"failed","local_exit_confirmed":true,"hook_error":null,
                "session":{"terminal":"failed","input_required":false,"intervention":null}
            }),
        )
        .unwrap();
    let owner = f
        .host
        .admit(
            &f.lease,
            Uuid::new_v4(),
            "synthetic grant publication".into(),
            digest("grant scope"),
            || Ok(()),
        )
        .unwrap();
    let command_id = Uuid::new_v4();
    let command = f
        .host
        .mutate(
            MutationRequest {
                request_id: command_id,
                runtime_id: f.reference.runtime,
                actor: Actor {
                    identity: "synthetic component command".into(),
                    roles: BTreeSet::new(),
                    principal: None,
                },
                kind: "command/Retry".into(),
                canonical_targets: vec![f.reference.phase.work.clone()],
                expected: BTreeMap::new(),
                payload: json!({"synthetic":"not an operator grant capability"}),
            },
            Some(&owner),
            None,
            || Ok(()),
            |_| Ok((BTreeMap::new(), json!({"complete":true}))),
        )
        .unwrap();
    let comment_id = Uuid::new_v4().to_string();
    let target = format!(
        "linear:{}:{}:{}",
        f.reference.worker_work.linear_endpoint,
        f.reference.worker_work.linear_workspace,
        f.reference.worker_work.parent
    );
    let publication = f
        .host
        .mutate(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: f.reference.runtime,
                actor: Actor {
                    identity: "synthetic component command".into(),
                    roles: BTreeSet::new(),
                    principal: None,
                },
                kind: "linear_comment".into(),
                canonical_targets: vec![target.clone()],
                expected: BTreeMap::from([
                    ("provider_writer".into(), "component writer".into()),
                    ("provider_body_digest".into(), digest("grant body")),
                    ("parent_command_id".into(), command_id.to_string()),
                    (
                        "parent_payload_digest".into(),
                        command.payload_digest.clone(),
                    ),
                ]),
                payload: json!({"synthetic":"grant publication data"}),
            },
            Some(&owner),
            None,
            || Ok(()),
            |_| {
                Ok((
                    BTreeMap::from([("comment_id".into(), comment_id.clone())]),
                    json!({"acknowledged":true}),
                ))
            },
        )
        .unwrap();
    f.host
        .retire_local(owner.operation_id, json!({"component":"closed"}))
        .unwrap();
    let receipts = f.host.operations().unwrap();
    let prior_admissions = receipts
        .iter()
        .filter_map(|r| {
            RunReference::from_receipt(r)
                .unwrap()
                .filter(|run| run.phase == old_reference.phase)
                .map(|_| (r.operation_id, r.payload_digest.clone()))
        })
        .collect();
    f.reference.grants = vec![ParentGrantReference {
        previous_phase: old_reference.phase.clone(),
        publication: exact(&publication),
        grant_digest: digest("synthetic grant identity"),
        native_target: target,
        comment_id,
        provider_writer: "component writer".into(),
        body_digest: digest("grant body"),
        command: command_id,
        command_payload_digest: command.payload_digest,
        command_actor: command.actor,
        prior_admissions,
        max_attempts: 3,
        max_turns: 2,
        max_review_rounds: 1,
    }];
    f.reference.phase = f.reference.grants[0].phase();
    f.reference.operation = Uuid::new_v4();
    f.reference.predecessor = exact(&failed);
    f.reference.retry_of = Some(old_reference.digest().unwrap());
    f.reference.attempt = 1;
    f.reference.max_attempts = 3;
    f.reference.max_turns = 2;
    f.reference.max_review_rounds = 1;
    f.reference.history = receipts
        .iter()
        .filter(|r| r.kind == "admission")
        .map(|r| (r.operation_id, canonical_digest(1, r).unwrap()))
        .collect();
    for retained in [
        &old_reference.original_parent,
        &old_reference.original_acceptance,
    ] {
        f.reference
            .history
            .insert(retained.operation, retained.receipt_digest.clone());
    }
    f.reference.inventory_digest = parent_reassembly_inventory(&receipts, &f.reference).unwrap();
    let mut reordered = receipts.clone();
    reordered.reverse();
    assert_eq!(
        parent_reassembly_inventory(&reordered, &f.reference).unwrap(),
        f.reference.inventory_digest
    );
    f.run.phase = f.reference.phase.clone();
    f.run.max_attempts = 3;
    f.run.max_turns = 2;
    f.run.max_review_rounds = 1;
    f.spaced();
    let snapshot = f.host.snapshot().unwrap();
    verify_parent_reassembly_reference(&snapshot, &f.reference, &f.host.operations().unwrap())
        .unwrap();
    let mut changed = f.reference.clone();
    changed.assignment_digest = digest("new assignment");
    assert_eq!(
        verify_parent_reassembly_reference(&snapshot, &changed, &f.host.operations().unwrap())
            .unwrap_err()
            .code,
        "parent_reassembly"
    );
    let mut changed = f.reference.clone();
    changed.grants[0]
        .prior_admissions
        .remove(&old_reference.predecessor.operation);
    assert_eq!(
        verify_parent_reassembly_reference(&snapshot, &changed, &f.host.operations().unwrap())
            .unwrap_err()
            .code,
        "parent_reassembly"
    );
    let mut changed = f.reference.clone();
    changed.history.remove(&old_reference.source.operation);
    assert_eq!(
        verify_parent_reassembly_reference(&snapshot, &changed, &f.host.operations().unwrap())
            .unwrap_err()
            .code,
        "parent_reassembly"
    );
    let preparation = f.prepare();
    let input = RunAdmission {
        request_id: f.reference.operation,
        work_key: f.reference.phase.work.clone(),
        scope_digest: digest("same exact source"),
        reference: RunReference {
            attempt: 1,
            ..f.run.clone()
        },
    };
    let admitted = f
        .host
        .admit_parent_reassembly_run_observed(
            &f.lease,
            input,
            f.reference.clone(),
            preparation,
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(
        RunReference::from_receipt(&f.host.operation(admitted.operation_id).unwrap())
            .unwrap()
            .unwrap()
            .attempt,
        1
    );
    f.host
        .retire_local(admitted.operation_id, finished())
        .unwrap();
    assert_eq!(
        f.host
            .operation(failed.operation_id)
            .unwrap()
            .payload_digest,
        failed.payload_digest
    );
    assert!(
        reject_unmarked_parent_phase(&f.reference.phase, &f.host.operations().unwrap()).is_err()
    );
    // A later actual baseline assembly continues G at ordinal two; G is not
    // renewed and the old failed admission/grant remain immutable ancestry.
    let previous = f.host.operation(admitted.operation_id).unwrap();
    let source = f
        .host
        .admit(
            &f.lease,
            Uuid::new_v4(),
            f.reference.phase.work.clone(),
            digest("next actual assembly"),
            || Ok(()),
        )
        .unwrap();
    let source=f.host.retire_local(source.operation_id,json!({
        "parent_reassembly_vector":digest("next vector"),"parent_reassembly_baseline":digest("next baseline"),
        "original_reviewed_token":f.reference.original_token
    })).unwrap();
    f.reference.operation = Uuid::new_v4();
    f.reference.predecessor = exact(&previous);
    f.reference.source = exact(&source);
    f.reference.retry_of = None;
    f.reference.vector_digest = digest("next vector");
    f.reference.baseline_digest = digest("next baseline");
    f.reference.assignment_digest = digest("next scoped assignment");
    f.reference.progress_digest = digest("next actual progress");
    f.reference.attempt = 2;
    for receipt in f
        .host
        .operations()
        .unwrap()
        .iter()
        .filter(|r| r.kind == "admission")
    {
        f.reference
            .history
            .insert(receipt.operation_id, canonical_digest(1, receipt).unwrap());
    }
    f.reference.inventory_digest =
        parent_reassembly_inventory(&f.host.operations().unwrap(), &f.reference).unwrap();
    f.spaced();
    let prep = f.prepare();
    let next = f
        .host
        .admit_parent_reassembly_run_observed(
            &f.lease,
            f.input(),
            f.reference.clone(),
            prep,
            |_| Ok(()),
        )
        .unwrap();
    let run = RunReference::from_receipt(&f.host.operation(next.operation_id).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            run.phase.retry_grant,
            run.attempt,
            run.max_attempts,
            run.max_turns,
            run.max_review_rounds
        ),
        (Some(publication.operation_id), 2, 3, 2, 1)
    );
    let failed = f
        .host
        .retire_local(
            next.operation_id,
            json!({
                "outcome":"failed","local_exit_confirmed":true,"hook_error":null,
                "error":Error::new("provider_transport","synthetic known closed transport failure"),
                "session":{"terminal":"failed","input_required":false,"intervention":null}
            }),
        )
        .unwrap();
    let previous = f.reference.clone();
    f.reference.operation = Uuid::new_v4();
    f.reference.predecessor = exact(&failed);
    f.reference.transient_of = Some(previous.digest().unwrap());
    f.reference.attempt = 3;
    for receipt in f
        .host
        .operations()
        .unwrap()
        .iter()
        .filter(|r| r.kind == "admission")
    {
        f.reference
            .history
            .insert(receipt.operation_id, canonical_digest(1, receipt).unwrap());
    }
    let receipts = f.host.operations().unwrap();
    f.reference.inventory_digest = parent_reassembly_inventory(&receipts, &f.reference).unwrap();
    f.spaced();
    verify_parent_reassembly_reference(&f.host.snapshot().unwrap(), &f.reference, &receipts)
        .unwrap();
    let runtime: kegari_config::RuntimeConfig = serde_json::from_value(json!({
        "id":f.reference.runtime,"host_id":"component","max_agents":1,"retry_cap_ms":40000
    }))
    .unwrap();
    let waiting = crate::runtime_schedule::plan_parent_reassembly(
        &runtime,
        &f.reference,
        &receipts,
        failed.updated_at + chrono::Duration::seconds(9),
    )
    .unwrap_err();
    assert_eq!(waiting.code, "continuation_wait");
    let ready = crate::runtime_schedule::plan_parent_reassembly(
        &runtime,
        &f.reference,
        &receipts,
        failed.updated_at + chrono::Duration::seconds(10),
    )
    .unwrap();
    assert_eq!(
        (
            ready.attempt,
            ready.max_attempts,
            ready.max_turns,
            ready.max_review_rounds
        ),
        (3, 3, 2, 1)
    );
    assert_eq!(
        ready.not_before,
        failed.updated_at + chrono::Duration::seconds(10)
    );
    assert_eq!(f.reference.phase, previous.phase);
    assert_eq!(f.reference.source, previous.source);
    assert_eq!(f.reference.assignment_digest, previous.assignment_digest);
    // A flag/message cannot turn auth/policy/unknown failures into automatic
    // work. These copied observations never enter the protected journal.
    for code in [
        "authentication_required",
        "unsupported_codex_version",
        "scope_conflict",
        "provider_intervention",
        "unrecognized",
    ] {
        let mut denied = failed.clone();
        let mut error = Error::new(code, "explicit nontransient data");
        error.retryable = true;
        denied.observed_result.as_mut().unwrap()["observation"]["error"] =
            serde_json::to_value(error).unwrap();
        assert_eq!(
            parent_transient_failure(&denied).unwrap_err().code,
            "persistent_blocker"
        );
    }
    let mut retry_after = failed.clone();
    retry_after.observed_result.as_mut().unwrap()["observation"]["error"] = serde_json::to_value(
        Error::new("provider_transport", "native longer retry-after")
            .detail(json!({"retry_after_ms":30000})),
    )
    .unwrap();
    assert_eq!(parent_transient_failure(&retry_after).unwrap(), Some(30000));
    let mut delayed_receipts = receipts.clone();
    *delayed_receipts
        .iter_mut()
        .find(|r| r.operation_id == retry_after.operation_id)
        .unwrap() = retry_after.clone();
    let mut delayed = f.reference.clone();
    delayed.predecessor = exact(&retry_after);
    delayed.history.insert(
        retry_after.operation_id,
        canonical_digest(1, &retry_after).unwrap(),
    );
    delayed.inventory_digest = parent_reassembly_inventory(&delayed_receipts, &delayed).unwrap();
    assert_eq!(
        crate::runtime_schedule::plan_parent_reassembly(
            &runtime,
            &delayed,
            &delayed_receipts,
            failed.updated_at + chrono::Duration::seconds(29)
        )
        .unwrap_err()
        .code,
        "continuation_wait"
    );
    assert_eq!(
        crate::runtime_schedule::plan_parent_reassembly(
            &runtime,
            &delayed,
            &delayed_receipts,
            failed.updated_at + chrono::Duration::seconds(30)
        )
        .unwrap()
        .not_before,
        failed.updated_at + chrono::Duration::seconds(30)
    );
    assert_eq!(
        canonical_digest(1, &f.host.operation(failed.operation_id).unwrap()).unwrap(),
        canonical_digest(1, &failed).unwrap()
    );

    let mut exceeded = f.reference.clone();
    exceeded.attempt = 4;
    assert_eq!(exceeded.validate().unwrap_err().code, "parent_reassembly");
}

#[test]
fn absent_parent_grant_fields_preserve_legacy_reference_bytes() {
    let f = Fixture::new();
    let legacy_inventory: BTreeMap<_, _> = f
        .host
        .operations()
        .unwrap()
        .iter()
        .filter_map(|receipt| {
            RunReference::from_receipt(receipt)
                .unwrap()
                .filter(|run| run.phase == f.reference.phase)
                .map(|run| {
                    (
                        run.attempt,
                        (receipt.operation_id, canonical_digest(1, receipt).unwrap()),
                    )
                })
        })
        .collect();
    assert_eq!(
        f.reference.inventory_digest,
        canonical_digest(1, &legacy_inventory).unwrap()
    );
    let value = serde_json::to_value(&f.reference).unwrap();
    assert!(value.get("grants").is_none());
    assert!(value.get("retry_of").is_none());
    assert!(value.get("transient_of").is_none());
    assert!(value.get("history").is_none());
    let decoded: ParentReassemblyReference = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
}
