//! Authenticated historical validation retained at an actual joined checkpoint.
//! No decoded plan, attempt or process record becomes a live execution capability.
use super::*;
use crate::control::Receipt;
use crate::validation::ValidationPrefix;
use crate::validation_archive::{
    ResolvedValidationArchive, ValidationArchiveNamespace, ValidationArchiveReference,
};
use kegari_secrets::{AttestationKey, Signed};
use kegari_storage::PrivateDir;

const PURPOSE: &str = "worker_cold_validation_history";
const LEAF: &str = "validation-recovery.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    schema_version: u32,
    runtime: Uuid,
    operation: Uuid,
    scope: String,
    instruction: String,
    session: String,
    journal: String,
    plan: String,
    attempts: usize,
    verified: usize,
    failed: usize,
    archive: ValidationArchiveReference,
}
#[derive(Clone, Debug)]
pub(crate) struct ColdValidationCheckpoint {
    signed: Signed<Checkpoint>,
    archive: ResolvedValidationArchive,
}
impl ColdValidationCheckpoint {
    pub(crate) fn retain(
        records: &PrivateDir,
        key: &AttestationKey,
        runtime: Uuid,
        completion: &RunCompletion,
    ) -> Result<Option<Self>> {
        if completion.validation_plan.is_none() {
            return Ok(None);
        }
        if !completion.local_exit_confirmed
            || completion
                .error
                .as_ref()
                .is_none_or(|error| error.code != "turn_incomplete")
            || completion.hook_error.is_some()
            || completion.validation_error.is_some()
            || completion.state.turn_id.is_some()
            || completion.state.unresolved_turn.is_some()
            || completion.state.intervention.is_some()
            || completion.state.input_required
        {
            return Ok(None);
        }
        let contract = completion
            .contract
            .as_ref()
            .ok_or_else(|| invalid("original contract is missing"))?;
        if !matches!(
            contract.instructions().manifest.role,
            AgentRole::Implementation | AgentRole::Documentation | AgentRole::Testing
        ) || contract.workflow().front_matter.hooks.before_run.is_some()
            || contract.workflow().front_matter.hooks.after_run.is_some()
        {
            return Ok(None);
        }
        let prefix = ValidationPrefix::from_cold_completion(completion)?
            .ok_or_else(|| invalid("original enrollment is missing"))?;
        let evidence = prefix.archive_evidence()?;
        let archive = crate::validation_archive::retain_history(
            &records.child("validation-recovery-archive")?,
            key,
            ValidationArchiveNamespace {
                workspace_id: contract.instructions().manifest.workspace_id,
                runtime_id: runtime,
            },
            &evidence,
            |proof| prefix.read_artifact(proof),
        )?;
        let payload = Checkpoint {
            schema_version: 1,
            runtime,
            operation: completion.operation_id,
            scope: contract.scope_digest().into(),
            instruction: contract.instructions().manifest_digest.clone(),
            session: canonical_digest(1, &completion.state)?,
            journal: journal_identity(records)?,
            plan: evidence.plan_digest.clone(),
            attempts: completion.validation_attempts.len(),
            verified: completion.checks.len(),
            failed: completion.failed_checks.len(),
            archive: archive.reference().clone(),
        };
        let signed = key.sign(PURPOSE, &payload)?;
        let bytes = serde_json::to_vec(&signed)
            .map_err(|_| Error::invalid("encode validation checkpoint"))?;
        records.create_bytes_exclusive(LEAF, &bytes)?;
        Ok(Some(Self { signed, archive }))
    }
    pub(crate) fn read(
        records: &PrivateDir,
        key: &AttestationKey,
        receipt: &Receipt,
    ) -> Result<Option<Self>> {
        let observation = receipt
            .observed_result
            .as_ref()
            .and_then(|value| value.get("observation"))
            .ok_or_else(|| invalid("retired observation is missing"))?;
        let Some(expected) = observation
            .get("validation_checkpoint")
            .and_then(Value::as_str)
        else {
            if observation
                .get("validation_plan_digest")
                .is_some_and(|value| !value.is_null())
            {
                return Err(invalid(
                    "validation history has no joined authenticated checkpoint",
                ));
            }
            return Ok(None);
        };
        let signed: Signed<Checkpoint> = records.read(LEAF)?;
        key.verify(PURPOSE, &signed)?;
        let value = &signed.payload;
        let reference = crate::control::run_reference_from_receipt(receipt)?
            .ok_or_else(|| invalid("actual run admission is missing"))?;
        if signed.digest()? != expected
            || value.schema_version != 1
            || value.runtime != receipt.runtime_id
            || value.operation != receipt.operation_id
            || receipt.expected.get("scope") != Some(&value.scope)
            || value.instruction != reference.instruction_digest
            || value.session != canonical_digest(1, &observation["session"])?
            || observation["validation_plan_digest"].as_str() != Some(&value.plan)
            || observation["validation_attempt_count"].as_u64() != Some(value.attempts as u64)
            || observation["verified_check_count"].as_u64() != Some(value.verified as u64)
            || observation["failed_check_count"].as_u64() != Some(value.failed as u64)
        {
            return Err(invalid(
                "validation checkpoint differs from its actual retired admission",
            ));
        }
        let archive = crate::validation_archive::read_history(
            &PrivateDir::open(&records.path().join("validation-recovery-archive"))?,
            key,
            &ValidationArchiveNamespace {
                workspace_id: reference.workspace,
                runtime_id: receipt.runtime_id,
            },
            &value.archive,
        )?;
        let result = Self { signed, archive };
        result.verify_journal(records, receipt)?;
        Ok(Some(result))
    }
    pub(crate) fn verify_journal(&self, records: &PrivateDir, receipt: &Receipt) -> Result<()> {
        let value = &self.signed.payload;
        let current = self.archive.evidence().current()?;
        if value.operation != receipt.operation_id
            || value.runtime != receipt.runtime_id
            || value.journal != journal_identity(records)?
            || current.binding.operation_id != receipt.operation_id
            || current.instruction_digest != value.instruction
            || self.archive.evidence().plan_digest != value.plan
            || self.archive.evidence().attempts.len() != value.attempts
        {
            return Err(invalid(
                "original validation journal or complete archive changed",
            ));
        }
        Ok(())
    }
    pub(crate) fn prefix(&self) -> Result<ValidationPrefix> {
        ValidationPrefix::from_archive(self.archive.clone())
    }
    pub(crate) fn digest(&self) -> Result<String> {
        self.signed.digest()
    }
}
fn journal_identity(records: &PrivateDir) -> Result<String> {
    let contract: Value = records.read("contract.json")?;
    let journal = PrivateDir::open(&records.path().join("journal"))?;
    let mut digest = canonical_digest(1, &contract)?;
    let mut bytes = 0usize;
    for leaf in journal.records_bounded(65_537)? {
        let value: Value = journal.read(&leaf)?;
        bytes = bytes
            .checked_add(
                serde_json::to_vec(&value)
                    .map_err(|_| Error::invalid("encode validation journal"))?
                    .len(),
            )
            .ok_or_else(|| invalid("journal size overflow"))?;
        if bytes > 64 * 1024 * 1024 {
            return Err(invalid("journal exceeds recovery bound"));
        }
        digest = canonical_digest(1, &json!({"previous":digest,"leaf":leaf,"value":value}))?;
    }
    Ok(digest)
}
fn invalid(message: &str) -> Error {
    Error::new("validation_recovery_pending", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::ValidationProposal;
    use kegari_types::run::{PhaseKey, RunReference};

    #[test]
    fn cold_validation_checkpoint_preserves_history_and_refuses_substitution() {
        // This fixture authenticates historical data. It never creates a live
        // ValidationPlan, execution result or native ownership capability.
        let (namespace, prior, artifacts) = crate::validation_archive::tests::descriptive_fixture();
        let mut current = prior.clone();
        current.history = vec![prior.current().unwrap().clone()];
        current.attempts.clear();
        let plan = &mut current.history[0];
        plan.binding.operation_id = Uuid::from_u128(101);
        plan.original_binding.operation_id = plan.binding.operation_id;
        plan.ordinal = 1;
        plan.predecessor = None;
        plan.created_at += chrono::Duration::seconds(10);
        for read in &mut plan.supporting_reads {
            read.operation_id = plan.binding.operation_id;
        }
        current.plan_digest = canonical_digest(1, plan).unwrap();
        current.schema_version = 5;
        current.prior_history = vec![prior.clone()];
        current.validate_observed_history().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let records = PrivateDir::create(&root.join("records")).unwrap();
        records
            .write("contract.json", &json!({"scope":"fixture"}))
            .unwrap();
        records
            .child("journal")
            .unwrap()
            .write("binding.json", &json!({"scope":"fixture"}))
            .unwrap();
        let key = AttestationKey::new("cold-validation-fixture".into(), vec![49; 32]).unwrap();
        let archive = crate::validation_archive::retain_history(
            &records.child("validation-recovery-archive").unwrap(),
            &key,
            namespace.clone(),
            &current,
            |proof| {
                artifacts
                    .get(&canonical_digest(1, proof)?)
                    .cloned()
                    .ok_or_else(|| invalid("fixture artifact missing"))
            },
        )
        .unwrap();
        assert!(
            crate::validation_archive::read(
                &records.child("validation-recovery-archive").unwrap(),
                &key,
                &namespace,
                archive.reference()
            )
            .is_err()
        );
        let prefix = ValidationPrefix::from_archive(archive.clone()).unwrap();
        assert_eq!(prefix.archive_evidence().unwrap(), current);
        assert_eq!(prefix.history()[0], prior);
        assert_eq!(prefix.history()[1].schema_version, 3);
        assert!(!prefix.history()[0].attempts[0].successful);
        for proof in current.all_attempts().flat_map(|record| &record.artifacts) {
            assert_eq!(
                prefix.read_artifact(proof).unwrap(),
                artifacts[&canonical_digest(1, proof).unwrap()]
            );
        }
        let mut proposal = ValidationProposal {
            previous: None,
            reason: "fresh source read".into(),
            supporting_reads: vec![],
            requirements: current.current().unwrap().requirements.clone(),
        };
        prefix.require_obligations(&proposal).unwrap();
        proposal.requirements.clear();
        assert_eq!(
            prefix.require_obligations(&proposal).unwrap_err().code,
            "validation_obligations_pending"
        );
        let plan = current.current().unwrap();
        let session = json!({"historical_session":"fixture"});
        let payload = Checkpoint {
            schema_version: 1,
            runtime: namespace.runtime_id,
            operation: plan.binding.operation_id,
            scope: canonical_digest(1, &"scope").unwrap(),
            instruction: plan.instruction_digest.clone(),
            session: canonical_digest(1, &session).unwrap(),
            journal: journal_identity(&records).unwrap(),
            plan: current.plan_digest.clone(),
            attempts: 0,
            verified: 0,
            failed: 0,
            archive: archive.reference().clone(),
        };
        let signed = key.sign(PURPOSE, &payload).unwrap();
        records.write(LEAF, &signed).unwrap();
        let run = RunReference {
            workspace: namespace.workspace_id,
            account: "fixture".into(),
            normalized_state: Some("running".into()),
            phase: PhaseKey {
                work: "fixture-work".into(),
                generation: canonical_digest(1, &"generation").unwrap(),
                phase: "implementation".into(),
                retry_grant: None,
            },
            profile_digest: canonical_digest(1, &"profile").unwrap(),
            instruction_digest: payload.instruction.clone(),
            max_attempts: 3,
            max_turns: 4,
            max_review_rounds: 2,
            attempt: 1,
        };
        let mut receipt = Receipt {
            operation_id: payload.operation,
            request_id: payload.operation,
            runtime_id: payload.runtime,
            owner_generation: 1,
            actor: "fixture".into(),
            kind: "admission".into(),
            canonical_targets: vec![run.phase.work.clone()],
            payload_digest: canonical_digest(1, &"payload").unwrap(),
            expected: BTreeMap::from([
                ("scope".into(), payload.scope.clone()),
                ("run".into(), serde_json::to_string(&run).unwrap()),
            ]),
            phase: crate::control::ReceiptPhase::Reconciled,
            remote_ids: BTreeMap::new(),
            observed_result: Some(
                json!({"observation":{"session":session,"validation_checkpoint":signed.digest().unwrap(),"validation_plan_digest":current.plan_digest,"validation_attempt_count":0,"verified_check_count":0,"failed_check_count":0}}),
            ),
            cancellation: "none".into(),
            updated_at: chrono::Utc::now(),
        };
        let restored = ColdValidationCheckpoint::read(&records, &key, &receipt)
            .unwrap()
            .unwrap();
        assert_eq!(restored.prefix().unwrap().digest(), prefix.digest());
        receipt.observed_result.as_mut().unwrap()["observation"]["validation_attempt_count"] =
            json!(1);
        assert!(ColdValidationCheckpoint::read(&records, &key, &receipt).is_err());
        receipt.observed_result.as_mut().unwrap()["observation"]["validation_attempt_count"] =
            json!(0);
        records
            .child("journal")
            .unwrap()
            .write("event-00000000.json", &json!({"kind":"unexpected"}))
            .unwrap();
        assert!(ColdValidationCheckpoint::read(&records, &key, &receipt).is_err());
    }
}
