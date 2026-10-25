//! Atomic consumption of an unfinished-work trigger. Persisted references are
//! provenance, not authority to recreate the delivery capability or a permit.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunContinuationReference {
    pub(crate) predecessor: Uuid,
    pub(crate) predecessor_receipt_digest: String,
    pub(crate) trigger_digest: String,
    pub(crate) progress_digest: String,
    pub(crate) failure_operation: Uuid,
    pub(crate) review_rounds_consumed: u32,
}

impl RunContinuationReference {
    pub fn from_receipt(receipt: &Receipt) -> Result<Option<Self>> {
        let Some(encoded) = receipt.expected.get("continuation") else {
            return Ok(None);
        };
        let value: Self = serde_json::from_str(encoded)
            .map_err(|_| Error::new("record_corrupt", "invalid continuation reference"))?;
        value.validate()?;
        let run = RunReference::from_receipt(receipt)?.ok_or_else(|| {
            Error::new(
                "record_corrupt",
                "continuation is not an admitted worker session",
            )
        })?;
        if run.attempt < 2
            || receipt.operation_id == value.predecessor
            || receipt.operation_id == value.failure_operation
            || receipt.expected.contains_key("metadata")
            || receipt.expected.contains_key("release_resume")
            || receipt.expected.contains_key("source_advanced")
            || value.review_rounds_consumed > run.max_review_rounds
        {
            return Err(Error::new(
                "record_corrupt",
                "continuation differs from its run admission",
            ));
        }
        Ok(Some(value))
    }
    fn validate(&self) -> Result<()> {
        if self.predecessor.is_nil() || self.failure_operation.is_nil() {
            return Err(Error::new(
                "record_corrupt",
                "continuation operation identity is invalid",
            ));
        }
        for digest in [
            &self.predecessor_receipt_digest,
            &self.trigger_digest,
            &self.progress_digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        Ok(())
    }
}

/// A newly completed implementation can require fresh validation/review while
/// retaining those phases' original budgets. This is not a failed-work claim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceAdvancedReference {
    pub(crate) predecessor: Uuid,
    pub(crate) predecessor_receipt_digest: String,
    pub(crate) source_operation: Uuid,
    pub(crate) source_receipt_digest: String,
    pub(crate) trigger_digest: String,
    pub(crate) progress_digest: String,
    pub(crate) review_rounds_consumed: u32,
}
impl SourceAdvancedReference {
    // Common admission mechanics only. The separate public record and opaque
    // producer preserve the distinct source-advanced versus repair meanings.
    fn mechanics(&self) -> RunContinuationReference {
        RunContinuationReference {
            predecessor: self.predecessor,
            predecessor_receipt_digest: self.predecessor_receipt_digest.clone(),
            failure_operation: self.source_operation,
            trigger_digest: self.trigger_digest.clone(),
            progress_digest: self.progress_digest.clone(),
            review_rounds_consumed: self.review_rounds_consumed,
        }
    }
    fn validate(&self) -> Result<()> {
        self.mechanics().validate()?;
        crate::evidence::validate_digest(&self.source_receipt_digest)?;
        if self.predecessor == self.source_operation {
            return Err(Error::new(
                "record_corrupt",
                "source-advanced phases must have distinct original operations",
            ));
        }
        Ok(())
    }
    pub fn from_receipt(receipt: &Receipt) -> Result<Option<Self>> {
        let Some(encoded) = receipt.expected.get("source_advanced") else {
            return Ok(None);
        };
        let value: Self = serde_json::from_str(encoded)
            .map_err(|_| Error::new("record_corrupt", "invalid source-advanced reference"))?;
        value.validate()?;
        let run = RunReference::from_receipt(receipt)?.ok_or_else(|| {
            Error::new(
                "record_corrupt",
                "source-advanced reference is not an admitted run",
            )
        })?;
        if run.attempt < 2
            || !["child_local_validation", "child_codex_review"].contains(&run.phase.phase.as_str())
            || receipt.operation_id == value.predecessor
            || receipt.operation_id == value.source_operation
            || value.review_rounds_consumed > run.max_review_rounds
            || ["metadata", "release_resume", "continuation"]
                .iter()
                .any(|key| receipt.expected.contains_key(*key))
        {
            return Err(Error::new(
                "record_corrupt",
                "source-advanced reference differs from its run admission",
            ));
        }
        Ok(Some(value))
    }
}

impl ControlHost {
    pub(crate) fn admit_run_source_advanced_observed(
        &self,
        lease: &Lease,
        input: RunAdmission,
        source: SourceAdvancedReference,
        fresh_gate: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        input.reference.validate()?;
        source.validate()?;
        if input.request_id == source.predecessor || input.request_id == source.source_operation {
            return Err(Error::new(
                "ownership_conflict",
                "source-advanced run needs a new operation identity",
            ));
        }
        self.admit_inner(
            lease,
            input.request_id,
            input.work_key,
            input.scope_digest,
            Some(AdmissionReference::SourceAdvancedRun(
                input.reference,
                source,
            )),
            fresh_gate,
        )
    }
    pub(super) fn verify_source_advanced(
        &self,
        snapshot: &ControlSnapshot,
        run: &RunReference,
        source: &SourceAdvancedReference,
        history: &BTreeMap<u32, Receipt>,
    ) -> Result<()> {
        if !["child_local_validation", "child_codex_review"].contains(&run.phase.phase.as_str()) {
            return Err(Error::new(
                "scope_violation",
                "source-advanced trigger is limited to required child validation or review",
            ));
        }
        let receipt = self.receipts.get(source.source_operation)?;
        let original = RunReference::from_receipt(&receipt)?.ok_or_else(|| {
            Error::new(
                "no_work_trigger",
                "source has no actual implementation admission",
            )
        })?;
        if original.phase.phase != "implementation"
            || canonical_digest(1, &receipt)? != source.source_receipt_digest
        {
            return Err(Error::new(
                "stale_result",
                "source no longer binds the exact completed implementation",
            ));
        }
        for receipt in self.receipts.all()? {
            if let Some(later) = RunReference::from_receipt(&receipt)?
                && later.phase == original.phase
            {
                if later.workspace != original.workspace
                    || receipt.runtime_id != snapshot.intent.runtime_id
                {
                    return Err(Error::new(
                        "record_corrupt",
                        "source phase has conflicting workspace or runtime ownership",
                    ));
                }
                if later.attempt >= original.attempt
                    && receipt.operation_id != source.source_operation
                {
                    return Err(Error::new(
                        "stale_result",
                        "a later implementation admission supersedes the source trigger",
                    ));
                }
            }
        }
        self.verify_run_continuation(snapshot, run, &source.mechanics(), history)
    }
    /// The callback must verify the opaque delivery trigger, source and current
    /// authority against this serialized snapshot. It must not reenter control.
    pub(crate) fn admit_run_continued_observed(
        &self,
        lease: &Lease,
        input: RunAdmission,
        continuation: RunContinuationReference,
        fresh_gate: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        input.reference.validate()?;
        continuation.validate()?;
        if input.request_id == continuation.predecessor
            || input.request_id == continuation.failure_operation
        {
            return Err(Error::new(
                "ownership_conflict",
                "continuation requires a new operation identity",
            ));
        }
        self.admit_inner(
            lease,
            input.request_id,
            input.work_key,
            input.scope_digest,
            Some(AdmissionReference::ContinuedRun(
                input.reference,
                continuation,
            )),
            fresh_gate,
        )
    }

    pub(super) fn verify_run_continuation(
        &self,
        snapshot: &ControlSnapshot,
        run: &RunReference,
        continuation: &RunContinuationReference,
        history: &BTreeMap<u32, Receipt>,
    ) -> Result<()> {
        let (_, predecessor) = history.last_key_value().ok_or_else(|| {
            Error::new("no_work_trigger", "continuation has no actual predecessor")
        })?;
        if predecessor.operation_id != continuation.predecessor
            || canonical_digest(1, predecessor)? != continuation.predecessor_receipt_digest
        {
            return Err(Error::new(
                "stale_result",
                "continuation predecessor is no longer the exact latest session",
            ));
        }
        let mut max_attempts = run.max_attempts;
        let mut max_turns = run.max_turns;
        let mut max_review_rounds = run.max_review_rounds;
        let mut prior_review_rounds = 0;
        for receipt in history.values() {
            let prior = RunReference::from_receipt(receipt)?.ok_or_else(|| {
                Error::new(
                    "record_corrupt",
                    "continuation history lacks a run reference",
                )
            })?;
            max_attempts = max_attempts.min(prior.max_attempts);
            max_turns = max_turns.min(prior.max_turns);
            max_review_rounds = max_review_rounds.min(prior.max_review_rounds);
            let consumed = RunContinuationReference::from_receipt(receipt)?
                .or(SourceAdvancedReference::from_receipt(receipt)?
                    .map(|source| source.mechanics()));
            if let Some(consumed) = consumed {
                if consumed.trigger_digest == continuation.trigger_digest
                    || consumed.progress_digest == continuation.progress_digest
                {
                    return Err(Error::new(
                        "no_progress",
                        "unfinished trigger or material progress was already consumed",
                    ));
                }
                if consumed.review_rounds_consumed < prior_review_rounds {
                    return Err(Error::new(
                        "record_corrupt",
                        "continuation review history regresses",
                    ));
                }
                prior_review_rounds = consumed.review_rounds_consumed;
            }
        }
        if run.max_attempts > max_attempts
            || run.max_turns > max_turns
            || run.max_review_rounds > max_review_rounds
            || continuation.review_rounds_consumed < prior_review_rounds
            || continuation.review_rounds_consumed > max_review_rounds
        {
            return Err(Error::new(
                "attempts_exhausted",
                "continuation would reset or exceed a retained phase or review bound",
            ));
        }
        let failure = self.receipts.get(continuation.failure_operation)?;
        let failure_run = RunReference::from_receipt(&failure)?.ok_or_else(|| {
            Error::new(
                "no_work_trigger",
                "unfinished obligation lacks an actual worker admission",
            )
        })?;
        if failure_run.workspace != run.workspace
            || failure_run.phase.work != run.phase.work
            || failure_run.phase.generation != run.phase.generation
        {
            return Err(Error::new(
                "scope_violation",
                "unfinished obligation belongs to another workspace or work generation",
            ));
        }
        for receipt in [predecessor, &failure] {
            let result = receipt.observed_result.as_ref();
            if receipt.runtime_id != snapshot.intent.runtime_id
                || receipt.phase != ReceiptPhase::Reconciled
                || result
                    .and_then(|v| v.get("local_execution"))
                    .and_then(Value::as_str)
                    != Some("exited")
                || result
                    .and_then(|v| v.get("unresolved_effects"))
                    .and_then(Value::as_array)
                    .is_none_or(|v| !v.is_empty())
                || snapshot
                    .active_operations
                    .iter()
                    .any(|p| p.operation_id == receipt.operation_id)
                || snapshot.unresolved_operations.values().any(|targets| {
                    targets
                        .iter()
                        .any(|target| receipt.canonical_targets.contains(target))
                })
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "continuation requires joined predecessor and failure sessions with reconciled effects",
                ));
            }
            if Utc::now().signed_duration_since(receipt.updated_at) < chrono::Duration::seconds(1) {
                return Err(Error::new(
                    "continuation_wait",
                    "unfinished-work continuation requires at least one second after reconciliation",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn digest(value: &str) -> String {
        canonical_digest(1, &value).unwrap()
    }
    fn setup() -> (
        tempfile::TempDir,
        Arc<ControlHost>,
        Lease,
        RunReference,
        Receipt,
        Receipt,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let host = Arc::new(
            ControlHost::setup(
                PrivateDir::create(&temp.path().canonicalize().unwrap().join("state")).unwrap(),
                runtime,
                Uuid::new_v4(),
            )
            .unwrap(),
        );
        host.lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: runtime,
                actor: Actor {
                    identity: "fixture-operator".into(),
                    roles: ["lifecycle".into()].into(),
                    principal: None,
                },
                kind: "lifecycle".into(),
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
            account: "fixture-account".into(),
            normalized_state: None,
            phase: crate::scheduler::PhaseKey {
                work: "child".into(),
                generation: digest("generation"),
                phase: "implementation".into(),
                retry_grant: None,
            },
            profile_digest: digest("profile"),
            instruction_digest: digest("instructions"),
            max_attempts: 3,
            max_turns: 20,
            max_review_rounds: 3,
            attempt: 1,
        };
        let permit = host
            .admit_run(&lease, admission(run.clone()), || Ok(()))
            .unwrap();
        let predecessor = host
            .retire_local(permit.operation_id, json!({"finished":"implementation"}))
            .unwrap();
        let mut validation = run.clone();
        validation.phase.phase = "child_local_validation".into();
        let check = host
            .admit_run(&lease, admission(validation), || Ok(()))
            .unwrap();
        let failure = host
            .retire_local(check.operation_id, json!({"finished":"validation"}))
            .unwrap();
        (temp, host, lease, run, predecessor, failure)
    }
    fn admission(reference: RunReference) -> RunAdmission {
        RunAdmission {
            request_id: Uuid::new_v4(),
            work_key: "exact-child-work".into(),
            scope_digest: digest("contract"),
            reference,
        }
    }
    fn continuation(predecessor: &Receipt, failure: &Receipt) -> RunContinuationReference {
        RunContinuationReference {
            predecessor: predecessor.operation_id,
            predecessor_receipt_digest: canonical_digest(1, predecessor).unwrap(),
            trigger_digest: digest("actual-trigger"),
            progress_digest: digest("source-and-obligation"),
            failure_operation: failure.operation_id,
            review_rounds_consumed: 0,
        }
    }
    fn spacing() {
        std::thread::sleep(std::time::Duration::from_millis(1050));
    }

    #[test]
    fn continuation_is_atomic_and_retained_across_restart() {
        let (temp, host, lease, mut run, predecessor, failure) = setup();
        let reference = continuation(&predecessor, &failure);
        run.attempt = 2;
        assert_eq!(
            host.admit_run_continued_observed(
                &lease,
                admission(run.clone()),
                reference.clone(),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "continuation_wait"
        );
        spacing();
        let before = host.operations().unwrap().len();
        assert_eq!(
            host.admit_run_continued_observed(
                &lease,
                admission(run.clone()),
                reference.clone(),
                |_| Err(Error::new("stale_authority", "fixture changed"))
            )
            .unwrap_err()
            .code,
            "stale_authority"
        );
        assert_eq!(host.operations().unwrap().len(), before);
        let permit = host
            .admit_run_continued_observed(
                &lease,
                admission(run.clone()),
                reference.clone(),
                |snapshot| {
                    assert!(snapshot.active_operations.is_empty());
                    Ok(())
                },
            )
            .unwrap();
        let actual = host.operation(permit.operation_id).unwrap();
        assert_eq!(
            RunReference::from_receipt(&actual)
                .unwrap()
                .unwrap()
                .attempt,
            2
        );
        assert_eq!(
            RunContinuationReference::from_receipt(&actual).unwrap(),
            Some(reference.clone())
        );
        let predecessor = host
            .retire_local(permit.operation_id, json!({"joined":true}))
            .unwrap();
        drop(host);
        let host = ControlHost::recover(
            PrivateDir::open(&temp.path().canonicalize().unwrap().join("state")).unwrap(),
            lease.runtime_id,
        )
        .unwrap();
        let lease = Lease {
            owner_generation: 2,
            owner_id: Uuid::new_v4(),
            ..lease
        };
        host.attach_reconciled(lease.clone()).unwrap();
        let mut repeated = continuation(&predecessor, &failure);
        run.attempt = 3;
        repeated.trigger_digest = digest("new-trigger-same-source-obligation");
        assert_eq!(
            host.admit_run_continued_observed(
                &lease,
                admission(run.clone()),
                repeated.clone(),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "no_progress"
        );
        repeated.trigger_digest = reference.trigger_digest;
        repeated.progress_digest = digest("different-source");
        assert_eq!(
            host.admit_run_continued_observed(&lease, admission(run), repeated, |_| Ok(()))
                .unwrap_err()
                .code,
            "no_progress"
        );
    }

    #[test]
    fn continuation_cannot_fork_or_change_its_latest_predecessor() {
        let (_temp, host, lease, mut run, predecessor, failure) = setup();
        run.attempt = 2;
        let mut reference = continuation(&predecessor, &failure);
        reference.predecessor_receipt_digest = digest("wrong-whole-receipt");
        assert_eq!(
            host.admit_run_continued_observed(
                &lease,
                admission(run.clone()),
                reference,
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "stale_result"
        );
        spacing();
        let reference = continuation(&predecessor, &failure);
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let threads = (0..2)
            .map(|_| {
                let (host, lease, run, reference, barrier) = (
                    host.clone(),
                    lease.clone(),
                    run.clone(),
                    reference.clone(),
                    barrier.clone(),
                );
                std::thread::spawn(move || {
                    barrier.wait();
                    host.admit_run_continued_observed(&lease, admission(run), reference, |_| Ok(()))
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results.iter().find_map(|r| r.as_ref().err()).unwrap().code,
            "ownership_conflict"
        );
        let permit = results.into_iter().find_map(Result::ok).unwrap();
        host.retire_local(permit.operation_id, json!({"joined":true}))
            .unwrap();
        run.attempt = 3;
        assert_eq!(
            host.admit_run_continued_observed(&lease, admission(run), reference, |_| Ok(()))
                .unwrap_err()
                .code,
            "stale_result"
        );
    }

    #[test]
    fn continuation_cannot_raise_budgets_or_reset_review_rounds() {
        let (_temp, host, lease, mut run, predecessor, failure) = setup();
        run.attempt = 2;
        let mut reference = continuation(&predecessor, &failure);
        reference.review_rounds_consumed = 2;
        for field in ["attempts", "turns", "reviews"] {
            let mut changed = run.clone();
            match field {
                "attempts" => changed.max_attempts = 4,
                "turns" => changed.max_turns = 21,
                _ => changed.max_review_rounds = 4,
            }
            assert_eq!(
                host.admit_run_continued_observed(
                    &lease,
                    admission(changed),
                    reference.clone(),
                    |_| Ok(())
                )
                .unwrap_err()
                .code,
                "attempts_exhausted"
            );
        }
        spacing();
        let permit = host
            .admit_run_continued_observed(&lease, admission(run.clone()), reference, |_| Ok(()))
            .unwrap();
        let predecessor = host
            .retire_local(permit.operation_id, json!({"joined":true}))
            .unwrap();
        let mut next = continuation(&predecessor, &failure);
        next.trigger_digest = digest("later-trigger");
        next.progress_digest = digest("later-progress");
        next.review_rounds_consumed = 1;
        run.attempt = 3;
        assert_eq!(
            host.admit_run_continued_observed(
                &lease,
                admission(run.clone()),
                next.clone(),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "attempts_exhausted"
        );
        next.review_rounds_consumed = 4;
        assert_eq!(
            host.admit_run_continued_observed(&lease, admission(run), next, |_| Ok(()))
                .unwrap_err()
                .code,
            "attempts_exhausted"
        );
    }

    #[test]
    fn continuation_preserves_unknown_effect_holds() {
        let (_temp, host, lease, mut run, predecessor, _) = setup();
        let mut validation = run.clone();
        validation.phase.phase = "second_validation".into();
        let permit = host
            .admit_run(&lease, admission(validation), || Ok(()))
            .unwrap();
        let request = MutationRequest {
            request_id: Uuid::new_v4(),
            runtime_id: lease.runtime_id,
            actor: Actor {
                identity: "fixture-worker".into(),
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: "codex/turn/start".into(),
            canonical_targets: vec![format!("codex-session:{}", permit.operation_id)],
            expected: BTreeMap::new(),
            payload: Value::Null,
        };
        let unknown = request.request_id;
        let (_ticket, ()) = host
            .begin_protocol_effect(request, &permit, || Ok(()), |_| Ok(()))
            .unwrap();
        let failure = host
            .retire_local(permit.operation_id, json!({"joined":true}))
            .unwrap();
        run.attempt = 2;
        assert_eq!(
            host.admit_run_continued_observed(
                &lease,
                admission(run),
                continuation(&predecessor, &failure),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "reconciliation_required"
        );
        assert!(
            host.snapshot()
                .unwrap()
                .unresolved_operations
                .contains_key(&unknown)
        );
        assert_eq!(host.operation(unknown).unwrap().phase, ReceiptPhase::Sent);
    }

    #[test]
    fn source_followup_keeps_validation_budget_and_consumes_exact_new_source_once() {
        let (_temp, host, lease, mut implementation, _, validation) = setup();
        implementation.attempt = 2;
        let permit = host
            .admit_run(&lease, admission(implementation.clone()), || Ok(()))
            .unwrap();
        let source = host
            .retire_local(permit.operation_id, json!({"completed":"repaired-source"}))
            .unwrap();
        let mut run = RunReference::from_receipt(&validation).unwrap().unwrap();
        run.attempt = 2;
        let mut reference = SourceAdvancedReference {
            predecessor: validation.operation_id,
            predecessor_receipt_digest: canonical_digest(1, &validation).unwrap(),
            source_operation: source.operation_id,
            source_receipt_digest: canonical_digest(1, &source).unwrap(),
            trigger_digest: digest("new-scoped-source"),
            progress_digest: digest("required-validation-new-tree"),
            review_rounds_consumed: 1,
        };
        let mut wrong = reference.clone();
        wrong.source_receipt_digest = digest("altered-source-receipt");
        assert_eq!(
            host.admit_run_source_advanced_observed(&lease, admission(run.clone()), wrong, |_| Ok(
                ()
            ))
            .unwrap_err()
            .code,
            "stale_result"
        );
        let mut expanded = run.clone();
        expanded.max_attempts = 4;
        assert_eq!(
            host.admit_run_source_advanced_observed(
                &lease,
                admission(expanded),
                reference.clone(),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "attempts_exhausted"
        );
        spacing();
        let permit = host
            .admit_run_source_advanced_observed(
                &lease,
                admission(run.clone()),
                reference.clone(),
                |_| Ok(()),
            )
            .unwrap();
        let receipt = host.operation(permit.operation_id).unwrap();
        assert_eq!(
            SourceAdvancedReference::from_receipt(&receipt).unwrap(),
            Some(reference.clone())
        );
        assert!(
            RunContinuationReference::from_receipt(&receipt)
                .unwrap()
                .is_none()
        );
        let receipt = host
            .retire_local(permit.operation_id, json!({"completed":"fresh-validation"}))
            .unwrap();
        reference.predecessor = receipt.operation_id;
        reference.predecessor_receipt_digest = canonical_digest(1, &receipt).unwrap();
        run.attempt = 3;
        reference.trigger_digest = digest("different-notification-same-source");
        assert_eq!(
            host.admit_run_source_advanced_observed(
                &lease,
                admission(run.clone()),
                reference.clone(),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "no_progress"
        );
        implementation.attempt = 3;
        let latest = host
            .admit_run(&lease, admission(implementation), || Ok(()))
            .unwrap();
        host.retire_local(latest.operation_id, json!({"completed":"later-source"}))
            .unwrap();
        reference.progress_digest = digest("different-claimed-progress");
        assert_eq!(
            host.admit_run_source_advanced_observed(&lease, admission(run), reference, |_| Ok(()))
                .unwrap_err()
                .code,
            "stale_result"
        );
    }
}
