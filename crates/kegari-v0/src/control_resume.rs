//! Atomic budget provenance for one verified provider-session recovery. Records
//! are not execution capabilities; the runtime must supply the opaque provider,
//! instruction and directory proofs again inside the final admission predicate.
use super::*;
use kegari_types::run::RunReference;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunResumeReference {
    pub(crate) predecessor: Uuid,
    pub(crate) predecessor_receipt_digest: String,
    pub(crate) recovery_digest: String,
    pub(crate) instruction_compatibility_digest: String,
    pub(crate) directory_reuse_digest: String,
    pub(crate) review_rounds_consumed: u32,
}

impl RunResumeReference {
    pub fn predecessor(&self) -> Uuid {
        self.predecessor
    }
    pub fn directory_reuse_digest(&self) -> &str {
        &self.directory_reuse_digest
    }
    pub fn recovery_digest(&self) -> &str {
        &self.recovery_digest
    }
    pub(super) fn validate(&self) -> Result<()> {
        if self.predecessor.is_nil() {
            return Err(Error::new(
                "record_corrupt",
                "resumed session has no predecessor",
            ));
        }
        for digest in [
            &self.predecessor_receipt_digest,
            &self.recovery_digest,
            &self.instruction_compatibility_digest,
            &self.directory_reuse_digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        Ok(())
    }
    pub fn from_receipt(receipt: &Receipt) -> Result<Option<Self>> {
        let paired = ReviewRoundResumeReference::from_model_receipt(receipt)?;
        let Some(encoded) = receipt.expected.get("session_resume") else {
            return Ok(None);
        };
        let value: Self = serde_json::from_str(encoded)
            .map_err(|_| Error::new("record_corrupt", "invalid session resume reference"))?;
        value.validate()?;
        if paired.is_some() {
            return Ok(Some(value));
        }
        let run = crate::control::run_reference_from_receipt(receipt)?
            .ok_or_else(|| Error::new("record_corrupt", "session resume is not an admitted run"))?;
        if run.attempt < 2
            || receipt.operation_id == value.predecessor
            || value.review_rounds_consumed > run.max_review_rounds
            || [
                "review_round_recovery",
                "review_round_resume",
                "review_round",
                "review_round_preparation",
                "metadata",
                "worker_preparation",
                "session_recovery_preparation",
                "continuation",
                "source_advanced",
                "release_resume",
            ]
            .iter()
            .any(|key| receipt.expected.contains_key(*key))
        {
            return Err(Error::new(
                "record_corrupt",
                "session resume differs from its exact run admission",
            ));
        }
        Ok(Some(value))
    }
}

impl ControlHost {
    /// The fresh predicate must verify actual joined completion, current native
    /// recovery, exact assembly compatibility and the current directory family.
    /// It must not reenter ControlHost or perform network I/O.
    pub(crate) fn admit_run_resumed_observed(
        &self,
        lease: &Lease,
        input: RunAdmission,
        resume: RunResumeReference,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        crate::control::validate_run_reference(&input.reference)?;
        resume.validate()?;
        if input.request_id == resume.predecessor || input.work_key != input.reference.phase.work {
            return Err(Error::new(
                "scope_violation",
                "session resume needs a new exact-work operation",
            ));
        }
        self.admit_inner(
            lease,
            input.request_id,
            input.work_key,
            input.scope_digest,
            Some(AdmissionReference::ResumedRun(input.reference, resume)),
            fresh,
        )
    }

    pub(super) fn verify_run_resume(
        &self,
        snapshot: &ControlSnapshot,
        run: &RunReference,
        resume: &RunResumeReference,
        history: &BTreeMap<u32, Receipt>,
    ) -> Result<()> {
        self.verify_run_resume_accounting(snapshot, run, resume, history, None)
    }
    pub(super) fn verify_run_resume_accounting(
        &self,
        snapshot: &ControlSnapshot,
        run: &RunReference,
        resume: &RunResumeReference,
        history: &BTreeMap<u32, Receipt>,
        paired: Option<&RoundRecoveryContext>,
    ) -> Result<()> {
        if paired.is_none()
            && history
                .values()
                .any(|receipt| receipt.expected.contains_key("review_round"))
        {
            return Err(Error::new(
                "review_round_required",
                "provider resume cannot discard round provenance",
            ));
        }
        let (_, predecessor) = history.last_key_value().ok_or_else(|| {
            Error::new(
                "no_work_trigger",
                "session resume has no admitted predecessor",
            )
        })?;
        if predecessor.operation_id != resume.predecessor
            || canonical_digest(1, predecessor)? != resume.predecessor_receipt_digest
        {
            return Err(Error::new(
                "stale_result",
                "session resume predecessor is not the exact latest receipt",
            ));
        }
        let mut caps = (run.max_attempts, run.max_turns, run.max_review_rounds);
        let mut consumed_rounds = 0;
        for receipt in history.values() {
            let prior = crate::control::run_reference_from_receipt(receipt)?
                .ok_or_else(|| Error::new("record_corrupt", "resume history lacks its run"))?;
            if receipt.runtime_id != snapshot.intent.runtime_id
                || prior.workspace != run.workspace
                || prior.account != run.account
                || prior.profile_digest != run.profile_digest
                || receipt.canonical_targets != [run.phase.work.clone()]
            {
                return Err(Error::new(
                    "identity_mismatch",
                    "resumed session changed runtime, workspace, account or profile",
                ));
            }
            caps.0 = caps.0.min(prior.max_attempts);
            caps.1 = caps.1.min(prior.max_turns);
            caps.2 = caps.2.min(prior.max_review_rounds);
            if let Some(old) = RunResumeReference::from_receipt(receipt)? {
                if old.recovery_digest == resume.recovery_digest {
                    return Err(Error::new(
                        "no_progress",
                        "provider recovery was already consumed",
                    ));
                }
                consumed_rounds = consumed_rounds.max(old.review_rounds_consumed);
            }
            if let Some(old) = RunContinuationReference::from_receipt(receipt)? {
                consumed_rounds = consumed_rounds.max(old.review_rounds_consumed);
            }
            if let Some(old) = SourceAdvancedReference::from_receipt(receipt)? {
                consumed_rounds = consumed_rounds.max(old.review_rounds_consumed);
            }
        }
        if run.max_attempts > caps.0
            || run.max_turns > caps.1
            || run.max_review_rounds > caps.2
            || resume.review_rounds_consumed < consumed_rounds
            || (paired.is_none() && resume.review_rounds_consumed > caps.2)
        {
            return Err(Error::new(
                "attempts_exhausted",
                "session resume would reset a retained phase budget",
            ));
        }
        let result = predecessor.observed_result.as_ref();
        let observation = result.and_then(|value| value.get("observation"));
        if predecessor.phase != ReceiptPhase::Reconciled
            || result.is_none_or(|value| {
                value["local_execution"] != "exited"
                    || value["unresolved_effects"]
                        .as_array()
                        .is_none_or(|v| !v.is_empty())
            })
            || observation.is_none_or(|value| {
                value["outcome"] != "failed"
                    || value["local_exit_confirmed"] != true
                    || !value["hook_error"].is_null()
                    || value["session"]["terminal"] != "failed"
                    || value["session"]["input_required"] != false
                    || !value["session"]["intervention"].is_null()
                    || !value["session"]["turn_id"].is_null()
                    || !value["session"]["unresolved_turn"].is_null()
            })
            || snapshot
                .active_operations
                .iter()
                .any(|p| p.operation_id == predecessor.operation_id)
            || snapshot.unresolved_operations.values().any(|targets| {
                targets
                    .iter()
                    .any(|target| predecessor.canonical_targets.contains(target))
            })
        {
            return Err(Error::new(
                "reconciliation_required",
                "resume requires an observed failed terminal session with joined execution and reconciled effects",
            ));
        }
        if Utc::now().signed_duration_since(predecessor.updated_at) < chrono::Duration::seconds(1) {
            return Err(Error::new(
                "continuation_wait",
                "session recovery requires one second after reconciliation",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Barrier};

    fn digest(value: &str) -> String {
        canonical_digest(1, &value).unwrap()
    }
    fn input(reference: RunReference) -> RunAdmission {
        RunAdmission {
            request_id: Uuid::new_v4(),
            work_key: reference.phase.work.clone(),
            scope_digest: digest("scope"),
            reference,
        }
    }
    fn failed_observation() -> Value {
        // Deliberately synthetic terminal metadata exercises only the Control
        // journal. It never issues a provider/directory recovery capability.
        json!({"outcome":"failed","local_exit_confirmed":true,"hook_error":null,
            "session":{"terminal":"failed","input_required":false,"turn_id":null,"intervention":null,"unresolved_turn":null}})
    }
    fn setup() -> (
        tempfile::TempDir,
        Arc<ControlHost>,
        Lease,
        RunReference,
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
                    identity: "fixture".into(),
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
            phase: kegari_types::run::PhaseKey {
                work: "work".into(),
                generation: digest("generation"),
                phase: "implementation".into(),
                retry_grant: None,
            },
            profile_digest: digest("profile"),
            instruction_digest: digest("instructions"),
            max_attempts: 4,
            max_turns: 8,
            max_review_rounds: 3,
            attempt: 1,
        };
        let permit = host
            .admit_run(&lease, input(run.clone()), || Ok(()))
            .unwrap();
        let receipt = host
            .retire_local(permit.operation_id, failed_observation())
            .unwrap();
        (temp, host, lease, run, receipt)
    }
    fn reference(receipt: &Receipt) -> RunResumeReference {
        RunResumeReference {
            predecessor: receipt.operation_id,
            predecessor_receipt_digest: canonical_digest(1, receipt).unwrap(),
            recovery_digest: digest("fixture recovery"),
            instruction_compatibility_digest: digest("fixture compatibility"),
            directory_reuse_digest: digest("fixture directory"),
            review_rounds_consumed: 2,
        }
    }
    fn spacing() {
        std::thread::sleep(std::time::Duration::from_millis(1050));
    }

    #[test]
    fn exact_resume_admission_consumes_once_and_preserves_successor_unknown_effects() {
        let (_temp, host, lease, mut run, old) = setup();
        spacing();
        run.attempt = 2;
        let barrier = Arc::new(Barrier::new(3));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let (host, lease, run, old, barrier) = (
                    host.clone(),
                    lease.clone(),
                    run.clone(),
                    old.clone(),
                    barrier.clone(),
                );
                std::thread::spawn(move || {
                    barrier.wait();
                    host.admit_run_resumed_observed(&lease, input(run), reference(&old), |_| Ok(()))
                })
            })
            .collect();
        barrier.wait();
        let mut admitted = vec![];
        for thread in threads {
            if let Ok(permit) = thread.join().unwrap() {
                admitted.push(permit);
            }
        }
        assert_eq!(admitted.len(), 1);
        let permit = &admitted[0];
        let receipt = host.operation(permit.operation_id).unwrap();
        assert_eq!(
            RunResumeReference::from_receipt(&receipt).unwrap(),
            Some(reference(&old))
        );
        for key in [
            "metadata",
            "worker_preparation",
            "session_recovery_preparation",
            "continuation",
            "source_advanced",
            "release_resume",
        ] {
            let mut mixed = receipt.clone();
            mixed.expected.insert(key.into(), "{}".into());
            assert_eq!(
                RunResumeReference::from_receipt(&mixed).unwrap_err().code,
                "record_corrupt"
            );
        }
        let effect = MutationRequest {
            request_id: Uuid::new_v4(),
            runtime_id: lease.runtime_id,
            actor: Actor {
                identity: "fixture".into(),
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: "fixture_effect".into(),
            canonical_targets: vec![run.phase.work.clone()],
            expected: BTreeMap::new(),
            payload: Value::Null,
        };
        let effect_id = effect.request_id;
        let (_pending, ()) = host
            .begin_effect(effect, permit, || Ok(()), |_| Ok(()))
            .unwrap();
        let latest = host
            .retire_local(permit.operation_id, failed_observation())
            .unwrap();
        run.attempt = 3;
        assert_eq!(
            host.admit_run_resumed_observed(&lease, input(run), reference(&latest), |_| Ok(()))
                .unwrap_err()
                .code,
            "reconciliation_required"
        );
        assert_eq!(host.operation(effect_id).unwrap().phase, ReceiptPhase::Sent);
        assert!(
            host.snapshot()
                .unwrap()
                .unresolved_operations
                .contains_key(&effect_id)
        );
        assert_eq!(
            canonical_digest(1, &host.operation(old.operation_id).unwrap()).unwrap(),
            canonical_digest(1, &old).unwrap()
        );
    }

    #[test]
    fn resume_rechecks_spacing_original_caps_identity_and_final_predicate_without_receipts() {
        let (_temp, host, lease, mut run, old) = setup();
        run.attempt = 2;
        assert_eq!(
            host.admit_run_resumed_observed(
                &lease,
                input(run.clone()),
                reference(&old),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "continuation_wait"
        );
        spacing();
        for case in [
            "turns",
            "reviews",
            "account",
            "profile",
            "predecessor",
            "predicate",
        ] {
            let mut candidate = run.clone();
            let mut proof = reference(&old);
            let expected = match case {
                "turns" => {
                    candidate.max_turns += 1;
                    "attempts_exhausted"
                }
                "reviews" => {
                    candidate.max_review_rounds += 1;
                    "attempts_exhausted"
                }
                "account" => {
                    candidate.account = "other".into();
                    "identity_mismatch"
                }
                "profile" => {
                    candidate.profile_digest = digest("other");
                    "identity_mismatch"
                }
                "predecessor" => {
                    proof.predecessor_receipt_digest = digest("other");
                    "stale_result"
                }
                _ => "fixture_refusal",
            };
            let admission = input(candidate);
            let id = admission.request_id;
            assert_eq!(
                host.admit_run_resumed_observed(&lease, admission, proof, |_| {
                    if case == "predicate" {
                        Err(Error::new(
                            "fixture_refusal",
                            "actual producer proof absent",
                        ))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err()
                .code,
                expected
            );
            assert_eq!(host.operation(id).unwrap_err().code, "record_missing");
        }
        assert!(host.snapshot().unwrap().active_operations.is_empty());
    }

    #[test]
    fn recovery_trigger_and_consumed_review_rounds_cannot_reset_after_reconciliation() {
        let (_temp, host, lease, mut run, old) = setup();
        spacing();
        run.attempt = 2;
        let permit = host
            .admit_run_resumed_observed(&lease, input(run.clone()), reference(&old), |_| Ok(()))
            .unwrap();
        let latest = host
            .retire_local(permit.operation_id, failed_observation())
            .unwrap();
        spacing();
        run.attempt = 3;
        let mut proof = reference(&latest);
        assert_eq!(
            host.admit_run_resumed_observed(&lease, input(run.clone()), proof.clone(), |_| Ok(()))
                .unwrap_err()
                .code,
            "no_progress"
        );
        proof.recovery_digest = digest("next recovery");
        proof.review_rounds_consumed = 1;
        assert_eq!(
            host.admit_run_resumed_observed(&lease, input(run.clone()), proof.clone(), |_| Ok(()))
                .unwrap_err()
                .code,
            "attempts_exhausted"
        );
        proof.review_rounds_consumed = 2;
        let permit = host
            .admit_run_resumed_observed(&lease, input(run), proof, |_| Ok(()))
            .unwrap();
        assert_eq!(
            crate::control::run_reference_from_receipt(
                &host.operation(permit.operation_id).unwrap()
            )
            .unwrap()
            .unwrap()
            .attempt,
            3
        );
    }
}
