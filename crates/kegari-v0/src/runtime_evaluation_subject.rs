//! Signed, deny-only reservation of native-shaped evaluation subjects. A saved
//! plan never grants execution or reconstructs a request/source capability.
use super::*;
use crate::evidence::{AttestationKey, Signed};
use serde::{Deserialize, Serialize};

const PURPOSE: &str = "evaluation_subject_plan";
const HISTORY_LIMIT: usize = 65_536;
const MAX_PLAN_TRIALS: usize = 10_000;
const MAX_PLAN_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubjectScope {
    runtime: Uuid,
    workspace: Uuid,
    hostname: String,
    port: u16,
    repository: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SubjectPlan {
    schema_version: u32,
    scope: SubjectScope,
    suite: String,
    first: u64,
    trials: Vec<TrialKey>,
}
/// Immutable observation embedded in the actual metadata result. Not a permit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SubjectReference {
    schema_version: u32,
    plan: String,
    runtime: Uuid,
    workspace: Uuid,
    command: Uuid,
    operation: Uuid,
    trial: String,
    ordinal: u32,
    number: u64,
}
#[derive(Clone)]
pub(super) struct TrialSubject {
    plan: SubjectPlan,
    reference: SubjectReference,
}
impl SubjectPlan {
    fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.scope.runtime.is_nil()
            || self.scope.workspace.is_nil()
            || self.scope.hostname != format!("github.{}.evaluation.invalid", self.scope.workspace)
            || self.scope.port != 443
            || self.scope.repository == 0
            || self.first == 0
            || self.trials.is_empty()
            || self.trials.len() > MAX_PLAN_TRIALS
        {
            return Err(Error::invalid("invalid finite evaluation subject plan"));
        }
        crate::evidence::validate_digest(&self.suite)?;
        self.last()?;
        let mut identities = BTreeSet::new();
        for trial in &self.trials {
            trial.name()?;
            if trial.repetition > 100 || !identities.insert(self.trial_identity(trial)?) {
                return Err(Error::invalid(
                    "invalid or repeated full evaluation TrialKey",
                ));
            }
        }
        Ok(())
    }
    fn digest(&self) -> Result<String> {
        canonical_digest(1, self)
    }
    fn last(&self) -> Result<u64> {
        let offset = u64::try_from(self.trials.len())
            .ok()
            .and_then(|n| n.checked_sub(1))
            .ok_or_else(|| Error::invalid("evaluation plan is empty or exceeds range"))?;
        self.first
            .checked_add(offset)
            .ok_or_else(|| Error::invalid("evaluation native subject range overflows u64"))
    }
    fn slot(&self, trial: &TrialKey) -> Result<(u32, u64)> {
        self.validate()?;
        let ordinal = self.trials.iter().position(|t| t == trial).ok_or_else(|| {
            Error::new(
                "evaluation_fidelity",
                "trial is absent from complete declared plan",
            )
        })?;
        let ordinal =
            u32::try_from(ordinal).map_err(|_| Error::invalid("trial ordinal overflow"))?;
        let number = self
            .first
            .checked_add(u64::from(ordinal))
            .ok_or_else(|| Error::invalid("subject number overflow"))?;
        Ok((ordinal, number))
    }
    fn trial_identity(&self, trial: &TrialKey) -> Result<String> {
        // Exactly api::evaluation_trial_identity; the producer compares the
        // selected slot with that actual validated command before registration.
        canonical_digest(
            1,
            &json!({"runtime":self.scope.runtime,"workspace":self.scope.workspace,"suite":self.suite,"case":trial.case_id,"variant":trial.variant,"profile":trial.profile,"repetition":trial.repetition}),
        )
    }
    fn overlaps(&self, other: &Self) -> Result<bool> {
        // WorkerAuthority::work_key uses HTTPS hostname/port/repository/number.
        // Head/base are intentionally absent: reuse across heads is disallowed.
        Ok(self.scope.runtime == other.scope.runtime
            && self.scope.hostname == other.scope.hostname
            && self.scope.port == other.scope.port
            && self.scope.repository == other.scope.repository
            && self.first <= other.last()?
            && other.first <= self.last()?)
    }
}
impl TrialSubject {
    pub(super) fn derive(
        suite: &crate::evaluation::SuiteManifest,
        binding: &crate::config::EvaluationBinding,
        trial: &TrialKey,
        request: &CommandRequest,
        command: &Receipt,
    ) -> Result<Self> {
        let workspace = evaluation_workspace(request)?;
        let endpoint = url::Url::parse(&crate::config::EvaluationBinding::endpoint(
            workspace, "github",
        ))
        .map_err(|_| Error::invalid("invalid evaluation subject endpoint"))?;
        let plan = SubjectPlan {
            schema_version: 1,
            scope: SubjectScope {
                runtime: request.runtime_id,
                workspace,
                hostname: endpoint
                    .host_str()
                    .ok_or_else(|| Error::invalid("missing evaluation host"))?
                    .into(),
                port: endpoint
                    .port_or_known_default()
                    .ok_or_else(|| Error::invalid("missing evaluation port"))?,
                repository: binding.repository_id,
            },
            suite: suite.digest()?,
            first: binding.pull_request,
            trials: suite.plan()?,
        };
        let (ordinal, number) = plan.slot(trial)?;
        let identity = plan.trial_identity(trial)?;
        if identity != crate::api::evaluation_trial_identity(request)?
            || command.expected.get("evaluation_trial") != Some(&identity)
            || request.request_id != crate::app::command_effect_id(command.operation_id)
            || plan.suite != binding.suite_digest
        {
            return Err(Error::new(
                "evaluation_fidelity",
                "subject differs from actual command/trial provenance",
            ));
        }
        let reference = SubjectReference {
            schema_version: 1,
            plan: plan.digest()?,
            runtime: request.runtime_id,
            workspace,
            command: command.operation_id,
            operation: request.request_id,
            trial: identity,
            ordinal,
            number,
        };
        Ok(Self { plan, reference })
    }
    pub(super) fn number(&self) -> u64 {
        self.reference.number
    }
    pub(super) fn reference(&self) -> &SubjectReference {
        &self.reference
    }
    /// Caller holds actual Control serialization and has just checked the live
    /// local request/owner/config. No gate is entered by this helper.
    pub(super) fn register(&self, runtime: &RuntimeCoordinator) -> Result<()> {
        let records = runtime.registration.root().child("evaluation-subjects")?;
        let receipts = runtime.control.operations()?;
        register_plan(
            &records,
            &runtime.services.key,
            &self.plan,
            &self.reference,
            &receipts,
        )
    }
    pub(super) fn verify(&self, runtime: &RuntimeCoordinator) -> Result<()> {
        let records = runtime.registration.root().child("evaluation-subjects")?;
        let actual = read_plan(
            &records,
            &runtime.services.key,
            &leaf(&self.reference.plan)?,
        )?;
        if actual != self.plan {
            return Err(Error::new(
                "record_corrupt",
                "retained evaluation subject plan changed",
            ));
        }
        verify_reference(&actual, &self.reference)
    }
}
fn leaf(digest: &str) -> Result<String> {
    crate::evidence::validate_digest(digest)?;
    Ok(format!("plan-{}.json", digest.rsplit(':').next().unwrap()))
}
fn read_plan(records: &PrivateDir, key: &AttestationKey, name: &str) -> Result<SubjectPlan> {
    let signed: Signed<SubjectPlan> =
        serde_json::from_slice(&records.read_bytes(name, MAX_PLAN_BYTES)?)
            .map_err(|_| Error::new("record_corrupt", "evaluation subject plan is malformed"))?;
    key.verify(PURPOSE, &signed)?;
    signed.payload.validate()?;
    if leaf(&signed.payload.digest()?)? != name {
        return Err(Error::new(
            "record_corrupt",
            "evaluation plan identity differs from protected filename",
        ));
    }
    Ok(signed.payload)
}
fn verify_reference(plan: &SubjectPlan, reference: &SubjectReference) -> Result<()> {
    let trial = plan
        .trials
        .get(reference.ordinal as usize)
        .ok_or_else(|| Error::new("record_corrupt", "evaluation subject ordinal is absent"))?;
    if reference.schema_version != 1
        || reference.runtime != plan.scope.runtime
        || reference.workspace != plan.scope.workspace
        || reference.command.is_nil()
        || reference.operation != crate::app::command_effect_id(reference.command)
        || reference.plan != plan.digest()?
        || reference.trial != plan.trial_identity(trial)?
        || Some(reference.number) != plan.first.checked_add(u64::from(reference.ordinal))
    {
        return Err(Error::new(
            "record_corrupt",
            "evaluation subject anchor differs from original full plan slot",
        ));
    }
    Ok(())
}
fn register_plan(
    records: &PrivateDir,
    key: &AttestationKey,
    candidate: &SubjectPlan,
    current: &SubjectReference,
    receipts: &[Receipt],
) -> Result<()> {
    candidate.validate()?;
    verify_reference(candidate, current)?;
    if receipts.len() > HISTORY_LIMIT {
        return Err(Error::new(
            "history_limit",
            "evaluation receipt inventory exceeds protected bound",
        ));
    }
    let mut plans = BTreeMap::new();
    let mut commands = BTreeMap::new();
    let mut count = 0usize;
    for name in records.records_bounded(HISTORY_LIMIT)? {
        let plan = read_plan(records, key, &name)?;
        count = count
            .checked_add(plan.trials.len())
            .filter(|n| *n <= HISTORY_LIMIT)
            .ok_or_else(|| {
                Error::new(
                    "history_limit",
                    "reserved evaluation slot history exceeds protected bound",
                )
            })?;
        let digest = plan.digest()?;
        for trial in &plan.trials {
            if commands
                .insert(plan.trial_identity(trial)?, digest.clone())
                .is_some()
            {
                return Err(Error::new(
                    "record_corrupt",
                    "one exact evaluation trial appears in multiple reserved plans",
                ));
            }
        }
        if plan.scope.runtime != candidate.scope.runtime {
            return Err(Error::new(
                "record_corrupt",
                "evaluation plan belongs to another runtime",
            ));
        }
        if digest != current.plan && candidate.overlaps(&plan)? {
            return Err(Error::new(
                "evaluation_subject_conflict",
                "declared evaluation range overlaps a different immutable plan",
            ));
        }
        plans.insert(digest, plan);
    }
    let mut by_operation = BTreeMap::new();
    for receipt in receipts {
        if by_operation.insert(receipt.operation_id, receipt).is_some() {
            return Err(Error::new(
                "record_corrupt",
                "duplicate evaluation receipt operation",
            ));
        }
    }
    let command_target = format!(
        "evaluation:{}:{}",
        candidate.scope.runtime, candidate.scope.workspace
    );
    for receipt in receipts {
        if receipt.kind == "command/EvaluationRun"
            && receipt.canonical_targets.contains(&command_target)
        {
            let identity = receipt.expected.get("evaluation_trial").ok_or_else(|| {
                Error::new(
                    "record_corrupt",
                    "prior evaluation command lacks exact TrialKey",
                )
            })?;
            if receipt.runtime_id != candidate.scope.runtime {
                return Err(Error::new(
                    "record_corrupt",
                    "evaluation command runtime changed",
                ));
            }
            // Only this actual new command may precede its first reservation.
            // Other commands (including failed/pre-metadata commands) require
            // an existing original plan. Absence never reallocates their range.
            if receipt.operation_id != current.command && !commands.contains_key(identity) {
                return Err(Error::new(
                    "record_missing",
                    "prior evaluation command's original subject plan is missing",
                ));
            }
        }
        let Some(observation) = receipt
            .observed_result
            .as_ref()
            .and_then(|v| v.get("metadata_observation"))
        else {
            continue;
        };
        if receipt.kind != "host_metadata"
            || observation["action"] != json!("evaluation_source_read")
        {
            continue;
        }
        let reference: SubjectReference =
            serde_json::from_value(observation.get("subject").cloned().ok_or_else(|| {
                Error::new(
                    "record_missing",
                    "evaluation source metadata lacks original subject plan",
                )
            })?)
            .map_err(|_| {
                Error::new(
                    "record_corrupt",
                    "evaluation subject metadata anchor is invalid",
                )
            })?;
        let plan = plans.get(&reference.plan).ok_or_else(|| {
            Error::new(
                "record_missing",
                "observed evaluation source's original subject plan is missing",
            )
        })?;
        verify_reference(plan, &reference)?;
        if receipt.runtime_id != reference.runtime
            || receipt.operation_id != source::source_operation(reference.operation)?
            || observation["operation"] != json!(reference.operation)
            || by_operation.get(&reference.command).is_none_or(|r| {
                r.kind != "command/EvaluationRun"
                    || r.runtime_id != reference.runtime
                    || r.expected.get("evaluation_trial") != Some(&reference.trial)
            })
        {
            return Err(Error::new(
                "record_corrupt",
                "evaluation source subject lacks its exact actual command/metadata identity",
            ));
        }
    }
    if let Some(existing) = plans.get(&current.plan) {
        if existing != candidate {
            return Err(Error::new(
                "record_corrupt",
                "evaluation subject plan changed",
            ));
        }
        return Ok(());
    }
    if count
        .checked_add(candidate.trials.len())
        .is_none_or(|n| n > HISTORY_LIMIT)
    {
        return Err(Error::new(
            "history_limit",
            "reserved evaluation slot history exceeds protected bound",
        ));
    }
    let signed = key.sign(PURPOSE, candidate)?;
    // The bounded complete plan is written before task registration or source
    // admission. Neither this record nor the metadata anchor is an authority.
    let encoded = serde_json::to_vec(&signed)
        .map_err(|_| Error::invalid("evaluation subject plan encoding failed"))?;
    if encoded.len() as u64 > MAX_PLAN_BYTES {
        return Err(Error::new(
            "history_limit",
            "evaluation subject plan exceeds protected file bound",
        ));
    }
    records.create_bytes_exclusive(&leaf(&current.plan)?, &encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plan(runtime: Uuid, workspace: Uuid) -> SubjectPlan {
        SubjectPlan {
            schema_version: 1,
            scope: SubjectScope {
                runtime,
                workspace,
                hostname: format!("github.{workspace}.evaluation.invalid"),
                port: 443,
                repository: 41,
            },
            suite: bytes_digest(b"complete declared suite"),
            first: 800,
            trials: ["first", "second"]
                .into_iter()
                .flat_map(|variant| {
                    (1..=2).map(move |repetition| TrialKey {
                        case_id: "independent".into(),
                        variant: variant.into(),
                        profile: if variant == "first" {
                            "profile_a"
                        } else {
                            "profile_b"
                        }
                        .into(),
                        repetition,
                    })
                })
                .collect(),
        }
    }
    // These are reservation/receipt DATA, never a request/source/exit capability.
    fn reference(plan: &SubjectPlan, command: Uuid, ordinal: u32) -> SubjectReference {
        let (ordinal, number) = plan.slot(&plan.trials[ordinal as usize]).unwrap();
        SubjectReference {
            schema_version: 1,
            plan: plan.digest().unwrap(),
            runtime: plan.scope.runtime,
            workspace: plan.scope.workspace,
            command,
            operation: crate::app::command_effect_id(command),
            trial: plan.trial_identity(&plan.trials[ordinal as usize]).unwrap(),
            ordinal,
            number,
        }
    }
    fn command(
        control: &crate::control::ControlHost,
        plan: &SubjectPlan,
        ordinal: u32,
    ) -> SubjectReference {
        let reference = reference(plan, Uuid::new_v4(), ordinal);
        control
            .queue_command(
                crate::control::MutationRequest {
                    request_id: reference.command,
                    runtime_id: plan.scope.runtime,
                    actor: Actor {
                        identity: "fixture-operator".into(),
                        principal: Some(Principal::Unix { uid: 500 }),
                        roles: BTreeSet::from(["evaluate".into()]),
                    },
                    kind: "command/EvaluationRun".into(),
                    canonical_targets: vec![format!(
                        "evaluation:{}:{}",
                        plan.scope.runtime, plan.scope.workspace
                    )],
                    expected: BTreeMap::from([(
                        "evaluation_trial".into(),
                        reference.trial.clone(),
                    )]),
                    payload: json!({"fixture":"protected-command-data-only"}),
                },
                "evaluate",
                || Ok(()),
            )
            .unwrap();
        reference
    }
    #[test]
    fn complete_plan_ordinals_cover_variants_profiles_and_repetitions_without_range_reuse() {
        let p = plan(Uuid::new_v4(), Uuid::new_v4());
        p.validate().unwrap();
        assert_eq!(p.last().unwrap(), 803);
        let refs: Vec<_> = (0..4).map(|i| reference(&p, Uuid::new_v4(), i)).collect();
        for r in &refs {
            verify_reference(&p, r).unwrap();
        }
        assert_eq!(
            refs.iter().map(|r| r.number).collect::<BTreeSet<_>>(),
            BTreeSet::from([800, 801, 802, 803])
        );
        assert_eq!(p.trials[0].repetition, p.trials[2].repetition);
        assert_ne!(refs[0].number, refs[2].number);
        let mut changed = p.clone();
        changed.trials.swap(0, 2);
        assert!(verify_reference(&changed, &refs[0]).is_err());
        changed = p.clone();
        changed.first = u64::MAX - 1;
        assert!(changed.validate().is_err());
        changed = p.clone();
        changed.trials[1] = changed.trials[0].clone();
        assert!(changed.validate().is_err());
        changed = p.clone();
        changed.suite = bytes_digest(b"new suite");
        assert!(p.overlaps(&changed).unwrap());
        changed.first = 804;
        assert!(!p.overlaps(&changed).unwrap());
        changed.first = 800;
        changed.scope.workspace = Uuid::new_v4();
        changed.scope.hostname = format!("github.{}.evaluation.invalid", changed.scope.workspace);
        assert!(!p.overlaps(&changed).unwrap());
    }
    #[test]
    fn actual_durable_command_refuses_removed_plan_before_any_metadata_or_reallocation() {
        let temp = tempfile::TempDir::new().unwrap();
        let root =
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("runtime")).unwrap();
        let runtime = Uuid::new_v4();
        let control = crate::control::ControlHost::setup(
            root.child("control").unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        let records = root.child("subjects").unwrap();
        let key = AttestationKey::new("fixture-key".into(), vec![7; 32]).unwrap();
        let p = plan(runtime, Uuid::new_v4());
        let first = command(&control, &p, 0);
        control
            .serialize_observation(|_| {
                register_plan(&records, &key, &p, &first, &control.operations()?)
            })
            .unwrap();
        let original = std::fs::read(records.path().join(leaf(&first.plan).unwrap())).unwrap();
        control
            .complete_command(
                first.command,
                Err(Error::new("fixture", "pre-metadata failure")),
            )
            .unwrap();
        let next = command(&control, &p, 1);
        std::fs::remove_file(records.path().join(leaf(&first.plan).unwrap())).unwrap();
        let before = canonical_digest(1, &control.operations().unwrap()).unwrap();
        assert_eq!(
            control
                .serialize_observation(|_| register_plan(
                    &records,
                    &key,
                    &p,
                    &next,
                    &control.operations()?
                ))
                .unwrap_err()
                .code,
            "record_missing"
        );
        assert!(records.records().unwrap().is_empty());
        assert_eq!(
            canonical_digest(1, &control.operations().unwrap()).unwrap(),
            before
        );
        records
            .create_bytes_exclusive(&leaf(&first.plan).unwrap(), &original)
            .unwrap();
        control
            .serialize_observation(|_| {
                register_plan(&records, &key, &p, &next, &control.operations()?)
            })
            .unwrap();
        let mut changed = p.clone();
        changed.first += 1;
        changed.suite = bytes_digest(b"different suite");
        let changed_ref = reference(&changed, Uuid::new_v4(), 0);
        assert_eq!(
            register_plan(
                &records,
                &key,
                &changed,
                &changed_ref,
                &control.operations().unwrap()
            )
            .unwrap_err()
            .code,
            "evaluation_subject_conflict"
        );
    }
    #[test]
    fn actual_metadata_journal_anchors_original_plan_even_if_another_signed_plan_covers_trial_ids()
    {
        // Actual Control command and host-metadata transitions; observation data
        // is synthetic. No process was launched, and no source/join capability
        // is created or claimed by this reservation test.
        let temp = tempfile::TempDir::new().unwrap();
        let root =
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("runtime")).unwrap();
        let runtime = Uuid::new_v4();
        let control = crate::control::ControlHost::setup(
            root.child("control").unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        let records = root.child("subjects").unwrap();
        let key = AttestationKey::new("fixture-key".into(), vec![7; 32]).unwrap();
        let p = plan(runtime, Uuid::new_v4());
        let first = command(&control, &p, 0);
        control
            .serialize_observation(|_| {
                register_plan(&records, &key, &p, &first, &control.operations()?)
            })
            .unwrap();
        let token = control
            .begin_host_process(
                crate::control::MutationRequest {
                    request_id: source::source_operation(first.operation).unwrap(),
                    runtime_id: runtime,
                    actor: Actor {
                        identity: "fixture-host".into(),
                        principal: None,
                        roles: BTreeSet::new(),
                    },
                    kind: "host_metadata".into(),
                    canonical_targets: vec![format!("evaluation:{runtime}:{}", p.scope.workspace)],
                    expected: BTreeMap::new(),
                    payload: json!({"fixture":"observation-only"}),
                },
                0,
                |_| Ok(()),
            )
            .unwrap();
        control.complete_host_process(&token, || Ok(()), json!({"owned_handles_joined":true,"delivery_evidence":false,"metadata_observation":{"action":"evaluation_source_read","operation":first.operation,"subject":first,"result":{"Err":{"code":"fixture"}}}})).unwrap();
        control
            .complete_command(first.command, Err(Error::new("fixture", "read failed")))
            .unwrap();
        let next = command(&control, &p, 1);
        let original = std::fs::read(records.path().join(leaf(&first.plan).unwrap())).unwrap();
        let mut changed = p.clone();
        changed.first += 100;
        // Even an authentic differently ranged plan cannot replace the original
        // anchor merely because it contains the same command TrialKeys.
        records
            .write_bytes(
                &leaf(&first.plan).unwrap(),
                &serde_json::to_vec(&key.sign(PURPOSE, &changed).unwrap()).unwrap(),
            )
            .unwrap();
        assert_eq!(
            register_plan(&records, &key, &p, &next, &control.operations().unwrap())
                .unwrap_err()
                .code,
            "record_corrupt"
        );
        std::fs::remove_file(records.path().join(leaf(&first.plan).unwrap())).unwrap();
        records
            .write_bytes(
                &leaf(&changed.digest().unwrap()).unwrap(),
                &serde_json::to_vec(&key.sign(PURPOSE, &changed).unwrap()).unwrap(),
            )
            .unwrap();
        assert_eq!(
            register_plan(&records, &key, &p, &next, &control.operations().unwrap())
                .unwrap_err()
                .code,
            "record_missing"
        );
        std::fs::remove_file(
            records
                .path()
                .join(leaf(&changed.digest().unwrap()).unwrap()),
        )
        .unwrap();
        records
            .create_bytes_exclusive(&leaf(&first.plan).unwrap(), &original)
            .unwrap();
        register_plan(&records, &key, &p, &next, &control.operations().unwrap()).unwrap();
    }
}
