//! Actual retained delivery assignments used by the existing session recovery
//! machinery. A map entry is neither a process handle nor a completed worker.
use super::*;
use crate::delivery::{DeliveryWorkerAssignment, ReplacementWork};

fn lock_error() -> Error {
    Error::new("runtime_unavailable", "replacement registry lock failed")
}

impl RuntimeCoordinator {
    pub(crate) fn retain_replacement_work(
        &self,
        operation: Uuid,
        work: Arc<ReplacementWork>,
    ) -> Result<()> {
        work.revalidate_retained()?;
        let mut state = self.state.lock().map_err(|_| lock_error())?;
        if let Some(prior) = state.replacement_work.get(&operation) {
            if prior.digest() != work.digest() {
                return Err(Error::new(
                    "operation_conflict",
                    "replacement changed its original assignment",
                ));
            }
            return Ok(());
        }
        if state.replacement_work.len() >= 65_536 {
            return Err(Error::new(
                "record_limit",
                "replacement inventory exceeds its bound",
            ));
        }
        state.replacement_work.insert(operation, work);
        Ok(())
    }

    pub(crate) fn retained_replacement_work(
        &self,
        operation: Uuid,
    ) -> Result<Option<Arc<ReplacementWork>>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| lock_error())?
            .replacement_work
            .get(&operation)
            .cloned())
    }

    /// Cache-only comparison also used under the control admission mutex.
    /// Actual source revalidation stays in restoration and worker preparation.
    pub(crate) fn replacement_resume_assignment(
        &self,
        operation: Uuid,
    ) -> Result<Option<DeliveryWorkerAssignment>> {
        let Some(work) = self.retained_replacement_work(operation)? else {
            return Ok(None);
        };
        let original = WorkerAuthority::Authorized(work.authority().clone());
        original.compare(&self.current_admission_authority(&original)?)?;
        let projection = self.observations.admission_snapshot()?;
        let parent = projection.parent(
            work.authority().workspace_id(),
            work.authority().work().parent,
        )?;
        let child = work
            .authority()
            .work()
            .child
            .ok_or_else(|| Error::new("scope_violation", "replacement needs a child"))?;
        let delivery = self.current_delivery(&parent.key)?;
        let current = delivery.verified_children().get(&child).ok_or_else(|| {
            Error::new(
                "stale_result",
                "replacement's original child evidence is unavailable",
            )
        })?;
        work.verify_current_child(current)?;
        DeliveryWorkerAssignment::for_replacement(&work).map(Some)
    }

    pub(crate) fn retain_replacement_resume_successor(
        &self,
        predecessor: Uuid,
        successor: Uuid,
    ) -> Result<()> {
        let Some(work) = self.retained_replacement_work(predecessor)? else {
            return Err(Error::new(
                "resume_incompatible",
                "replacement lost its original assignment",
            ));
        };
        let receipt = self.control.operation(successor)?;
        let resumed =
            crate::control::RunResumeReference::from_receipt(&receipt)?.ok_or_else(|| {
                Error::new(
                    "resume_incompatible",
                    "replacement successor lacks actual resume admission",
                )
            })?;
        let run = crate::control::run_reference_from_receipt(&receipt)?.ok_or_else(|| {
            Error::new(
                "resume_incompatible",
                "replacement successor lacks run admission",
            )
        })?;
        if resumed.predecessor() != predecessor
            || run.phase.phase != "child_replacement"
            || run.phase.work != work.authority().work_key()?
            || run.workspace != work.authority().workspace_id()
        {
            return Err(Error::new(
                "resume_incompatible",
                "replacement successor changed original work",
            ));
        }
        self.retain_replacement_work(successor, work.clone())?;
        let records = self.registration.root().child("delivery-runtime")?;
        let checkpoint = work.checkpoint(self.runtime_id, successor)?;
        let leaf = format!("replacement-work-{successor}.json");
        if records.path().join(&leaf).exists() {
            let existing: kegari_secrets::Signed<crate::delivery::ReplacementCheckpoint> =
                records.read(&leaf)?;
            self.services
                .key
                .verify("runtime_replacement_work", &existing)?;
            if canonical_digest(1, &existing.payload)? != canonical_digest(1, &checkpoint)? {
                return Err(Error::new(
                    "operation_conflict",
                    "replacement successor checkpoint changed",
                ));
            }
        } else {
            records.write(
                &leaf,
                &self
                    .services
                    .key
                    .sign("runtime_replacement_work", &checkpoint)?,
            )?;
        }
        Ok(())
    }

    /// Follow only actual sequential resume admissions. A second successor or
    /// cross-work link is a conflict, never permission to choose one arbitrarily.
    pub(crate) fn latest_replacement_operation(&self, original: Uuid) -> Result<Uuid> {
        Ok(*self
            .replacement_chain(original)?
            .last()
            .expect("original is retained"))
    }

    pub(crate) fn retain_replacement_successor_chain(&self, original: Uuid) -> Result<()> {
        for pair in self.replacement_chain(original)?.windows(2) {
            self.retain_replacement_resume_successor(pair[0], pair[1])?;
        }
        Ok(())
    }

    fn replacement_chain(&self, original: Uuid) -> Result<Vec<Uuid>> {
        replacement_chain(original, &self.control.operations()?)
    }
}

fn replacement_chain(original: Uuid, receipts: &[crate::control::Receipt]) -> Result<Vec<Uuid>> {
    let mut current = original;
    let mut seen = BTreeSet::new();
    let mut chain = Vec::new();
    loop {
        if !seen.insert(current) {
            return Err(Error::new(
                "record_corrupt",
                "replacement resume ancestry contains a cycle",
            ));
        }
        chain.push(current);
        let Some(prior) = receipts
            .iter()
            .find(|receipt| receipt.operation_id == current)
        else {
            // A retained assignment can precede its first worker admission.
            if current == original {
                return Ok(chain);
            }
            return Err(Error::new(
                "record_missing",
                "replacement resume predecessor is missing",
            ));
        };
        let prior_run = crate::control::run_reference_from_receipt(prior)?
            .ok_or_else(|| Error::new("record_corrupt", "replacement predecessor is not a run"))?;
        let mut next = None;
        for receipt in receipts {
            let Some(resume) = crate::control::RunResumeReference::from_receipt(receipt)? else {
                continue;
            };
            if resume.predecessor() != current {
                continue;
            }
            let run = crate::control::run_reference_from_receipt(receipt)?.ok_or_else(|| {
                Error::new("record_corrupt", "replacement successor is not a run")
            })?;
            if prior_run.phase.phase != "child_replacement"
                || receipt.runtime_id != prior.runtime_id
                || resume.predecessor_receipt_digest != canonical_digest(1, prior)?
                || run.max_attempts > prior_run.max_attempts
                || run.max_turns > prior_run.max_turns
                || run.max_review_rounds > prior_run.max_review_rounds
                || run.phase != prior_run.phase
                || run.phase.phase != "child_replacement"
                || run.attempt
                    != prior_run.attempt.checked_add(1).ok_or_else(|| {
                        Error::new("attempts_exhausted", "replacement attempt overflow")
                    })?
                || run.workspace != prior_run.workspace
                || run.account != prior_run.account
                || next.replace(receipt.operation_id).is_some()
            {
                return Err(Error::new(
                    "operation_conflict",
                    "replacement resume ancestry is not a unique original-work chain",
                ));
            }
        }
        match next {
            Some(next) => current = next,
            None => return Ok(chain),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{Receipt, ReceiptPhase, RunResumeReference};

    fn receipt(id: u128, prior: Option<&Receipt>) -> Receipt {
        let digest = bytes_digest(b"replacement fixture");
        let run = RunReference {
            workspace: Uuid::from_u128(10),
            account: "fixture".into(),
            normalized_state: None,
            phase: kegari_types::run::PhaseKey {
                work: "child".into(),
                generation: digest.clone(),
                phase: "child_replacement".into(),
                retry_grant: None,
            },
            profile_digest: digest.clone(),
            instruction_digest: digest.clone(),
            max_attempts: 4,
            max_turns: 4,
            max_review_rounds: 4,
            attempt: prior
                .map(|r| {
                    crate::control::run_reference_from_receipt(r)
                        .unwrap()
                        .unwrap()
                        .attempt
                        + 1
                })
                .unwrap_or(1),
        };
        let mut expected = BTreeMap::from([("run".into(), serde_json::to_string(&run).unwrap())]);
        if let Some(prior) = prior {
            expected.insert(
                "session_resume".into(),
                serde_json::to_string(&RunResumeReference {
                    predecessor: prior.operation_id,
                    predecessor_receipt_digest: canonical_digest(1, prior).unwrap(),
                    recovery_digest: digest.clone(),
                    instruction_compatibility_digest: digest.clone(),
                    directory_reuse_digest: digest.clone(),
                    review_rounds_consumed: 0,
                })
                .unwrap(),
            );
        }
        Receipt {
            operation_id: Uuid::from_u128(id),
            request_id: Uuid::from_u128(id),
            runtime_id: Uuid::from_u128(9),
            owner_generation: 1,
            actor: "runtime".into(),
            kind: "admission".into(),
            canonical_targets: vec!["child".into()],
            payload_digest: digest,
            expected,
            phase: ReceiptPhase::Reconciled,
            remote_ids: BTreeMap::new(),
            observed_result: None,
            cancellation: "fixture".into(),
            updated_at: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        }
    }

    #[test]
    fn replacement_resume_chain_preserves_exact_receipts_and_refuses_forks() {
        let first = receipt(1, None);
        let second = receipt(2, Some(&first));
        let third = receipt(3, Some(&second));
        assert_eq!(
            replacement_chain(
                first.operation_id,
                &[third.clone(), first.clone(), second.clone()]
            )
            .unwrap(),
            vec![first.operation_id, second.operation_id, third.operation_id]
        );
        let fork = receipt(4, Some(&first));
        assert_eq!(
            replacement_chain(first.operation_id, &[first.clone(), second.clone(), fork])
                .unwrap_err()
                .code,
            "operation_conflict"
        );
        let mut changed = first.clone();
        changed.observed_result = Some(json!({"other":"receipt"}));
        assert_eq!(
            replacement_chain(first.operation_id, &[changed, second.clone()])
                .unwrap_err()
                .code,
            "operation_conflict"
        );
        let mut changed = second;
        let mut run = crate::control::run_reference_from_receipt(&changed)
            .unwrap()
            .unwrap();
        run.phase.work = "another-child".into();
        changed
            .expected
            .insert("run".into(), serde_json::to_string(&run).unwrap());
        assert_eq!(
            replacement_chain(first.operation_id, &[first, changed])
                .unwrap_err()
                .code,
            "operation_conflict"
        );
    }
}
