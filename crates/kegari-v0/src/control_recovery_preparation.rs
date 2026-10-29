//! Finite metadata-only recovery preparation. This is neither a worker
//! preparation nor a model admission; opaque recovery still precedes resume.
use super::*;
#[cfg(test)]
use kegari_types::run::RunReference;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRecoveryPreparationReference {
    pub(crate) predecessor: Uuid,
    pub(crate) predecessor_receipt_digest: String,
    pub(crate) candidate_digest: String,
    pub(crate) workspace: Uuid,
    pub(crate) account: String,
    pub(crate) phase: kegari_types::run::PhaseKey,
    pub(crate) attempt: u32,
    pub(crate) max_attempts: u32,
}
impl SessionRecoveryPreparationReference {
    pub fn predecessor(&self) -> Uuid {
        self.predecessor
    }
    pub fn candidate_digest(&self) -> &str {
        &self.candidate_digest
    }
    pub fn phase(&self) -> &kegari_types::run::PhaseKey {
        &self.phase
    }
    pub fn account(&self) -> &str {
        &self.account
    }
    pub fn workspace(&self) -> Uuid {
        self.workspace
    }
    pub fn operation_id(&self) -> Result<Uuid> {
        self.validate()?;
        let digest = canonical_digest(
            1,
            &serde_json::json!({"purpose":"session_recovery_preparation","predecessor":self.predecessor,"candidate":self.candidate_digest,"attempt":self.attempt}),
        )?;
        let bytes = hex::decode(digest.rsplit(':').next().unwrap_or_default())
            .map_err(|_| Error::invalid("recovery preparation digest"))?;
        let mut id: [u8; 16] = bytes[..16]
            .try_into()
            .map_err(|_| Error::invalid("recovery preparation identity"))?;
        id[6] = (id[6] & 0x0f) | 0x80;
        id[8] = (id[8] & 0x3f) | 0x80;
        Ok(Uuid::from_bytes(id))
    }
    pub(super) fn validate(&self) -> Result<()> {
        if self.predecessor.is_nil()
            || self.workspace.is_nil()
            || self.account.trim().is_empty()
            || self.account.len() > 512
            || self.phase.work.is_empty()
            || self.phase.work.len() > 512
            || self.phase.phase.is_empty()
            || self.phase.phase.len() > 64
            || !self
                .phase
                .phase
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
            || self.phase.retry_grant.is_some_and(|id| id.is_nil())
            || self.attempt == 0
            || self.attempt > self.max_attempts
        {
            return Err(Error::new(
                "record_corrupt",
                "invalid finite session recovery preparation",
            ));
        }
        for digest in [
            &self.predecessor_receipt_digest,
            &self.candidate_digest,
            &self.phase.generation,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        Ok(())
    }
    pub fn from_receipt(receipt: &Receipt) -> Result<Option<Self>> {
        let paired = ReviewRoundRecoveryReference::from_metadata_receipt(receipt)?;
        let Some(encoded) = receipt.expected.get("session_recovery_preparation") else {
            return Ok(None);
        };
        let value: Self = serde_json::from_str(encoded)
            .map_err(|_| Error::new("record_corrupt", "invalid recovery preparation reference"))?;
        value.validate()?;
        if paired.is_some() {
            return Ok(Some(value));
        }
        if receipt.kind != "admission"
            || receipt.operation_id != value.operation_id()?
            || receipt.request_id != receipt.operation_id
            || receipt.canonical_targets != [value.phase.work.clone()]
            || [
                "review_round_recovery",
                "review_round_resume",
                "review_round",
                "review_round_preparation",
                "run",
                "metadata",
                "worker_preparation",
                "continuation",
                "source_advanced",
                "release_resume",
                "session_resume",
            ]
            .iter()
            .any(|key| receipt.expected.contains_key(*key))
        {
            return Err(Error::new(
                "record_corrupt",
                "recovery metadata is attached to another admission kind",
            ));
        }
        crate::evidence::validate_digest(
            receipt
                .expected
                .get("scope")
                .ok_or_else(|| Error::new("record_corrupt", "recovery metadata has no scope"))?,
        )?;
        Ok(Some(value))
    }
}
/// A completed read is history, not its lost live proof. A fresh bounded
/// metadata admission may observe it again after restart without replaying a
/// model turn or adopting the old provider process.
pub(crate) fn metadata_readback_refreshable(receipt: &Receipt) -> bool {
    receipt.phase == ReceiptPhase::Reconciled
        && receipt.observed_result.as_ref().is_some_and(|value| {
            value["local_execution"] == "exited"
                && value["unresolved_effects"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
                && value["observation"]["metadata_recovery"] == "verified"
                && value["observation"]["local_exit_confirmed"] == true
                && value["observation"]["proof_digest"]
                    .as_str()
                    .is_some_and(|digest| crate::evidence::validate_digest(digest).is_ok())
        })
}

impl ControlHost {
    pub(crate) fn admit_session_recovery_preparation(
        &self,
        lease: &Lease,
        reference: SessionRecoveryPreparationReference,
        scope_digest: String,
        fresh: impl FnOnce(&ControlSnapshot) -> Result<()>,
    ) -> Result<Permit> {
        reference.validate()?;
        crate::evidence::validate_digest(&scope_digest)?;
        self.admit_inner(
            lease,
            reference.operation_id()?,
            reference.phase.work.clone(),
            scope_digest,
            Some(AdmissionReference::SessionRecovery(reference)),
            fresh,
        )
    }
    pub(super) fn verify_session_recovery_preparation(
        &self,
        snapshot: &ControlSnapshot,
        reference: &SessionRecoveryPreparationReference,
    ) -> Result<()> {
        self.verify_session_recovery_preparation_accounting(snapshot, reference, None)
    }
    pub(super) fn verify_session_recovery_preparation_accounting(
        &self,
        snapshot: &ControlSnapshot,
        reference: &SessionRecoveryPreparationReference,
        paired: Option<&RoundRecoveryContext>,
    ) -> Result<()> {
        let receipts = self.receipts.all()?;
        let mut runs = BTreeMap::new();
        let mut preparations = BTreeMap::new();
        let mut cap = reference.max_attempts;
        let mut model_cap = u32::MAX;
        for receipt in &receipts {
            if let Some(run) = crate::control::run_reference_from_receipt(receipt)?
                && run.phase == reference.phase
            {
                if run.workspace != reference.workspace
                    || run.account != reference.account
                    || receipt.runtime_id != snapshot.intent.runtime_id
                    || runs.insert(run.attempt, (receipt, run.clone())).is_some()
                {
                    return Err(Error::new(
                        "identity_mismatch",
                        "recovery predecessor history differs from account or phase",
                    ));
                }
                cap = cap.min(run.max_attempts);
                model_cap = model_cap.min(run.max_attempts);
            }
            if let Some(old) = SessionRecoveryPreparationReference::from_receipt(receipt)?
                && old.predecessor == reference.predecessor
            {
                if old.phase != reference.phase
                    || old.workspace != reference.workspace
                    || old.account != reference.account
                    || receipt.runtime_id != snapshot.intent.runtime_id
                    || preparations.insert(old.attempt, receipt).is_some()
                {
                    return Err(Error::new(
                        "record_corrupt",
                        "recovery preparation history changed identity",
                    ));
                }
                if let Some(paired) = paired {
                    paired.verify_metadata(receipt)?;
                }
                cap = cap.min(old.max_attempts);
            }
        }
        let (_, (prior, run)) = runs
            .last_key_value()
            .ok_or_else(|| Error::new("no_work_trigger", "session recovery has no original run"))?;
        if runs.keys().copied().ne(1..=runs.len() as u32)
            || prior.operation_id != reference.predecessor
            || canonical_digest(1, prior)? != reference.predecessor_receipt_digest
        {
            return Err(Error::new(
                "stale_result",
                "session recovery predecessor is not the exact latest phase receipt",
            ));
        }
        if paired.is_none()
            && runs
                .values()
                .any(|(receipt, _)| receipt.expected.contains_key("review_round"))
        {
            return Err(Error::new(
                "review_round_required",
                "metadata session recovery cannot omit retained round provenance",
            ));
        }
        if run.attempt >= model_cap
            || reference.max_attempts > cap
            || reference.attempt != preparations.len() as u32 + 1
            || preparations
                .keys()
                .copied()
                .ne(1..=preparations.len() as u32)
            || reference.attempt > cap
        {
            return Err(Error::new(
                "attempts_exhausted",
                "session recovery cannot reset metadata or model budgets",
            ));
        }
        let result = prior.observed_result.as_ref();
        if prior.phase != ReceiptPhase::Reconciled
            || result.is_none_or(|v| {
                v["local_execution"] != "exited"
                    || v["unresolved_effects"]
                        .as_array()
                        .is_none_or(|v| !v.is_empty())
                    || v["observation"]["outcome"] != "failed"
                    || v["observation"]["local_exit_confirmed"] != true
                    || !v["observation"]["hook_error"].is_null()
                    || v["observation"]["session"]["terminal"] != "failed"
                    || v["observation"]["session"]["input_required"] != false
                    || !v["observation"]["session"]["intervention"].is_null()
                    || !v["observation"]["session"]["turn_id"].is_null()
                    || !v["observation"]["session"]["unresolved_turn"].is_null()
            })
        {
            return Err(Error::new(
                "reconciliation_required",
                "metadata recovery requires a fully closed terminal failure",
            ));
        }
        let mut latest = prior.updated_at;
        for previous in preparations.values() {
            let result = previous.observed_result.as_ref();
            if previous.phase != ReceiptPhase::Reconciled
                || result.is_none_or(|v| {
                    v["local_execution"] != "exited"
                        || v["unresolved_effects"]
                            .as_array()
                            .is_none_or(|v| !v.is_empty())
                        || !(v["observation"]["metadata_recovery"] == "failed_no_effect"
                            && v["observation"]["retryable"] == true
                            || paired.is_none() && metadata_readback_refreshable(previous))
                })
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "prior metadata recovery is neither retryable nor a confirmed closed ordinary readback",
                ));
            }
            latest = latest.max(previous.updated_at);
        }
        if Utc::now().signed_duration_since(latest) < chrono::Duration::seconds(1) {
            return Err(Error::new(
                "continuation_wait",
                "metadata recovery requires one second after reconciliation",
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
    fn digest(v: &str) -> String {
        canonical_digest(1, &v).unwrap()
    }
    fn setup() -> (
        tempfile::TempDir,
        Arc<ControlHost>,
        Lease,
        SessionRecoveryPreparationReference,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let host = Arc::new(
            ControlHost::setup(
                PrivateDir::create(&temp.path().canonicalize().unwrap().join("control")).unwrap(),
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
            account: "fixture".into(),
            normalized_state: None,
            phase: kegari_types::run::PhaseKey {
                work: "fixture-work".into(),
                generation: digest("generation"),
                phase: "implementation".into(),
                retry_grant: None,
            },
            profile_digest: digest("profile"),
            instruction_digest: digest("instructions"),
            max_attempts: 3,
            max_turns: 3,
            max_review_rounds: 2,
            attempt: 1,
        };
        let permit = host
            .admit_run(
                &lease,
                RunAdmission {
                    request_id: Uuid::new_v4(),
                    work_key: run.phase.work.clone(),
                    scope_digest: digest("worker scope"),
                    reference: run.clone(),
                },
                || Ok(()),
            )
            .unwrap();
        // Control component fixture only, no native recovery capability.
        let receipt=host.retire_local(permit.operation_id,json!({"outcome":"failed","local_exit_confirmed":true,"hook_error":null,"session":{"terminal":"failed","input_required":false,"intervention":null,"turn_id":null,"unresolved_turn":null}})).unwrap();
        let reference = SessionRecoveryPreparationReference {
            predecessor: permit.operation_id,
            predecessor_receipt_digest: canonical_digest(1, &receipt).unwrap(),
            candidate_digest: digest("component candidate"),
            workspace: run.workspace,
            account: run.account,
            phase: run.phase,
            attempt: 1,
            max_attempts: 2,
        };
        (temp, host, lease, reference)
    }
    #[test]
    fn metadata_preparation_is_exact_atomic_and_not_a_worker_admission() {
        let (_temp, host, lease, reference) = setup();
        assert_eq!(
            host.admit_session_recovery_preparation(
                &lease,
                reference.clone(),
                digest("metadata scope"),
                |_| Ok(())
            )
            .unwrap_err()
            .code,
            "continuation_wait"
        );
        std::thread::sleep(std::time::Duration::from_millis(1050));
        let barrier = Arc::new(Barrier::new(3));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let (host, lease, reference, barrier) = (
                    host.clone(),
                    lease.clone(),
                    reference.clone(),
                    barrier.clone(),
                );
                std::thread::spawn(move || {
                    barrier.wait();
                    host.admit_session_recovery_preparation(
                        &lease,
                        reference,
                        digest("metadata scope"),
                        |_| Ok(()),
                    )
                })
            })
            .collect();
        barrier.wait();
        let permits: Vec<_> = threads
            .into_iter()
            .filter_map(|thread| thread.join().unwrap().ok())
            .collect();
        assert_eq!(permits.len(), 1);
        let receipt = host.operation(permits[0].operation_id).unwrap();
        assert_eq!(
            SessionRecoveryPreparationReference::from_receipt(&receipt).unwrap(),
            Some(reference)
        );
        assert!(
            crate::control::run_reference_from_receipt(&receipt)
                .unwrap()
                .is_none()
        );
        assert!(
            PreparationReference::from_receipt(&receipt)
                .unwrap()
                .is_none()
        );
        for key in [
            "run",
            "metadata",
            "worker_preparation",
            "continuation",
            "source_advanced",
            "release_resume",
            "session_resume",
        ] {
            let mut mixed = receipt.clone();
            mixed.expected.insert(key.into(), "{}".into());
            assert_eq!(
                SessionRecoveryPreparationReference::from_receipt(&mixed)
                    .unwrap_err()
                    .code,
                "record_corrupt"
            );
        }
    }
    #[test]
    fn metadata_preparation_cannot_reset_caps_change_candidate_to_replay_or_cross_unknown_effects()
    {
        let (_temp, host, lease, reference) = setup();
        std::thread::sleep(std::time::Duration::from_millis(1050));
        for case in ["cap", "predecessor", "predicate"] {
            let mut r = reference.clone();
            if case == "cap" {
                r.max_attempts = 4;
            }
            if case == "predecessor" {
                r.predecessor_receipt_digest = digest("other");
            }
            let id = r.operation_id().unwrap();
            let result =
                host.admit_session_recovery_preparation(&lease, r, digest("scope"), |_| {
                    if case == "predicate" {
                        Err(Error::new(
                            "fixture_refusal",
                            "changed current native authority",
                        ))
                    } else {
                        Ok(())
                    }
                });
            assert!(result.is_err(), "{case}");
            assert!(host.operation(id).is_err());
        }
        let permit = host
            .admit_session_recovery_preparation(&lease, reference.clone(), digest("scope"), |_| {
                Ok(())
            })
            .unwrap();
        let request = MutationRequest {
            request_id: Uuid::new_v4(),
            runtime_id: lease.runtime_id,
            actor: Actor {
                identity: "fixture".into(),
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: "fixture_unknown".into(),
            canonical_targets: vec![permit.work_key.clone()],
            expected: BTreeMap::new(),
            payload: Value::Null,
        };
        let effect = request.request_id;
        let _ = host
            .begin_effect(request, &permit, || Ok(()), |_| Ok(()))
            .unwrap();
        host.retire_local(
            permit.operation_id,
            json!({"metadata_recovery":"failed_no_effect","retryable":true}),
        )
        .unwrap();
        let mut next = reference;
        next.attempt = 2;
        next.candidate_digest = digest("different candidate cannot reset original budget");
        assert_eq!(
            host.admit_session_recovery_preparation(&lease, next, digest("scope"), |_| Ok(()))
                .unwrap_err()
                .code,
            "reconciliation_required"
        );
        assert_eq!(host.operation(effect).unwrap().phase, ReceiptPhase::Sent);
    }
    #[test]
    fn verified_metadata_readback_can_refresh_within_original_budget() {
        let (_temp, host, lease, mut reference) = setup();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let first = host
            .admit_session_recovery_preparation(&lease, reference.clone(), digest("scope"), |_| {
                Ok(())
            })
            .unwrap();
        let prior = host.retire_local(first.operation_id, json!({
            "metadata_recovery":"verified", "proof_digest":digest("old in-memory proof"), "local_exit_confirmed":true
        })).unwrap();
        assert!(metadata_readback_refreshable(&prior));
        let mut unknown = prior.clone();
        unknown.observed_result.as_mut().unwrap()["observation"]["local_exit_confirmed"] =
            json!(false);
        assert!(!metadata_readback_refreshable(&unknown));
        reference.attempt = 2;
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let next = host
            .admit_session_recovery_preparation(
                &lease,
                reference.clone(),
                digest("fresh scope"),
                |_| Ok(()),
            )
            .unwrap();
        assert_ne!(next.operation_id, first.operation_id);
        assert!(
            !host
                .operation(next.operation_id)
                .unwrap()
                .expected
                .contains_key("run")
        );
        assert_eq!(
            canonical_digest(1, &prior).unwrap(),
            canonical_digest(1, &host.operation(first.operation_id).unwrap()).unwrap()
        );
        host.retire_local(next.operation_id, json!({"metadata_recovery":"verified", "proof_digest":digest("new actual readback"), "local_exit_confirmed":true})).unwrap();
        reference.attempt = 3;
        reference.max_attempts = 3;
        assert_eq!(
            host.admit_session_recovery_preparation(&lease, reference, digest("scope"), |_| Ok(()))
                .unwrap_err()
                .code,
            "attempts_exhausted"
        );
    }
}
