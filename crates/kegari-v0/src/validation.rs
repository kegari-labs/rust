//! Host-enrolled validation obligations. Serialized records describe history;
//! only an actual worker's observed reads and current admission enroll a plan.
use crate::digest::canonical_digest;
use crate::error::{Error, Result};
use crate::evidence::{CheckKind, validate_digest};
use crate::execution::{CorrectionExecutionProvenance, ExecutionBinding};
use crate::workflow::{CheckCommand, CheckService};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;
use uuid::Uuid;

pub const MAX_VALIDATION_HISTORY: usize = 32;
pub const MAX_VALIDATION_PLAN_BYTES: usize = 192 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryJournalReference {
    pub leaf: String,
    pub digest: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DiscoveryReadContent {
    Source {
        requested_path: String,
        path: PathBuf,
        digest: String,
        bytes: u64,
    },
    Context {
        key: String,
        digest: String,
        bytes: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryReadReference {
    pub schema_version: u32,
    pub operation_id: Uuid,
    pub call_id: Uuid,
    pub turn: String,
    pub instruction_digest: String,
    pub correction_input_digest: String,
    pub result_digest: String,
    pub journal: DiscoveryJournalReference,
    pub content: DiscoveryReadContent,
}
impl DiscoveryReadReference {
    pub fn digest(&self) -> Result<String> {
        self.validate_shape()?;
        canonical_digest(1, self)
    }
    pub(crate) fn validate_shape(&self) -> Result<()> {
        if self.schema_version != 1 || self.operation_id.is_nil() || self.call_id.is_nil() {
            return Err(invalid(
                "read reference requires actual call and operation identities",
            ));
        }
        text(&self.turn, 256)?;
        text(&self.journal.leaf, 256)?;
        if self.journal.leaf.contains(['/', '\\'])
            || matches!(self.journal.leaf.as_str(), "." | "..")
            || self.journal.bytes == 0
            || self.journal.bytes > 256 * 1024
        {
            return Err(invalid(
                "read reference must name one bounded protected journal entry",
            ));
        }
        for digest in [
            &self.instruction_digest,
            &self.correction_input_digest,
            &self.result_digest,
            &self.journal.digest,
        ] {
            validate_digest(digest)?;
        }
        let (digest, bytes) = match &self.content {
            DiscoveryReadContent::Source {
                requested_path,
                path,
                digest,
                bytes,
            } => {
                text(requested_path, 4096)?;
                if !path.is_absolute()
                    || path
                        .to_str()
                        .is_none_or(|p| p.len() > 4096 || p.contains('\0'))
                {
                    return Err(invalid("source read requires its observed absolute path"));
                }
                (digest, bytes)
            }
            DiscoveryReadContent::Context { key, digest, bytes } => {
                text(key, 256)?;
                (digest, bytes)
            }
        };
        validate_digest(digest)?;
        if *bytes > 16 * 1024 * 1024 {
            return Err(invalid("read content exceeds its finite evidence bound"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationRequirement {
    pub id: String,
    pub statement: String,
    pub rationale: String,
    pub argv: Vec<String>,
    pub cwd: String,
    #[serde(default)]
    pub services: Vec<CheckService>,
    #[serde(default)]
    pub artifacts: Vec<String>,
}
impl ValidationRequirement {
    pub(crate) fn validate_shape(&self) -> Result<()> {
        text(&self.id, 128)?;
        text(&self.statement, 4096)?;
        text(&self.rationale, 4096)?;
        text(&self.cwd, 4096)?;
        if self.argv.is_empty()
            || self.argv.len() > 128
            || self.argv[0].trim().is_empty()
            || self.argv.iter().any(|s| s.len() > 4096 || s.contains('\0'))
            || self.argv.iter().map(String::len).sum::<usize>() > 65_536
        {
            return Err(invalid("validation commands require bounded literal argv"));
        }
        if self.artifacts.len() > 16 {
            return Err(invalid("validation artifact count exceeds its bound"));
        }
        let mut artifacts = BTreeSet::new();
        for artifact in &self.artifacts {
            text(artifact, 4096)?;
            if !artifacts.insert(artifact) {
                return Err(invalid("duplicate validation artifact"));
            }
        }
        CheckCommand {
            argv: self.argv.clone(),
            result_artifact: String::new(),
            services: self.services.clone(),
        }
        .validate_services()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationProposal {
    pub previous: Option<String>,
    pub reason: String,
    pub supporting_reads: Vec<String>,
    pub requirements: Vec<ValidationRequirement>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidationPlanRecord {
    pub schema_version: u32,
    pub original_binding: ExecutionBinding,
    pub binding: ExecutionBinding,
    pub correction: Option<CorrectionExecutionProvenance>,
    pub kind: CheckKind,
    pub workflow_digest: String,
    pub instruction_digest: String,
    pub tool_contract_digest: String,
    /// Actual effective provider input, including any admitted correction.
    pub context_digest: String,
    pub source_digest: String,
    pub predecessor: Option<String>,
    pub ordinal: u32,
    pub reason: String,
    pub requirements: Vec<ValidationRequirement>,
    pub supporting_reads: Vec<DiscoveryReadReference>,
    pub created_at: DateTime<Utc>,
}
impl ValidationPlanRecord {
    pub(crate) fn validate_shape(&self) -> Result<()> {
        if self.schema_version != 1
            || self.kind != CheckKind::Local
            || self.ordinal == 0
            || self.ordinal as usize > MAX_VALIDATION_HISTORY
            || (self.ordinal == 1) != self.predecessor.is_none()
        {
            return Err(invalid(
                "unsupported validation plan version, kind or history position",
            ));
        }
        for binding in [&self.original_binding, &self.binding] {
            binding.work.validate()?;
            binding.repository.validate()?;
            binding.revision.validate()?;
            validate_digest(&binding.isolation_digest)?;
            if binding.runtime_id.is_nil()
                || binding.operation_id.is_nil()
                || binding.work.child.is_none()
                || binding.candidate_vector_digest.is_some()
            {
                return Err(invalid(
                    "discovered plan requires an actual child-local binding",
                ));
            }
        }
        let mut unchanged = self.binding.clone();
        unchanged.work = self.original_binding.work.clone();
        if unchanged != self.original_binding {
            return Err(invalid(
                "validation discovery cannot change original execution ownership",
            ));
        }
        match &self.correction {
            None if self.binding != self.original_binding => {
                return Err(invalid(
                    "changed validation work requires actual correction provenance",
                ));
            }
            Some(correction) => {
                validate_digest(&correction.transition_digest)?;
                validate_digest(&correction.input_digest)?;
                let mut work = self.binding.work.clone();
                work.generation.correction =
                    self.original_binding.work.generation.correction.clone();
                if correction.original_binding_digest
                    != canonical_digest(1, &self.original_binding)?
                    || correction.watermark == 0
                    || correction.input_digest != self.context_digest
                    || work != self.original_binding.work
                {
                    return Err(invalid(
                        "correction cannot replace validation scope or original ownership",
                    ));
                }
            }
            None => (),
        }
        for digest in [
            &self.workflow_digest,
            &self.instruction_digest,
            &self.tool_contract_digest,
            &self.context_digest,
            &self.source_digest,
        ] {
            validate_digest(digest)?;
        }
        if let Some(previous) = &self.predecessor {
            validate_digest(previous)?;
        }
        text(&self.reason, 4096)?;
        if self.requirements.is_empty()
            || self.requirements.len() > 64
            || self.supporting_reads.is_empty()
            || self.supporting_reads.len() > 64
        {
            return Err(invalid(
                "validation needs bounded obligations and observed supporting reads",
            ));
        }
        let mut ids = BTreeSet::new();
        for requirement in &self.requirements {
            requirement.validate_shape()?;
            if !ids.insert(&requirement.id) {
                return Err(invalid("duplicate validation obligation"));
            }
        }
        let mut reads = BTreeSet::new();
        for reference in &self.supporting_reads {
            reference.validate_shape()?;
            if reference.operation_id != self.binding.operation_id
                || reference.instruction_digest != self.instruction_digest
                || reference.correction_input_digest != self.context_digest
                || !reads.insert(reference.digest()?)
            {
                return Err(invalid(
                    "supporting read is duplicate or belongs to another effective admission",
                ));
            }
        }
        if serde_json::to_vec(self)
            .map_err(|_| invalid("encode validation plan"))?
            .len()
            > MAX_VALIDATION_PLAN_BYTES
        {
            return Err(invalid("validation plan exceeds its finite journal bound"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ValidationEnrollment {
    pub original_binding: ExecutionBinding,
    pub binding: ExecutionBinding,
    pub correction: Option<CorrectionExecutionProvenance>,
    pub workflow_digest: String,
    pub instruction_digest: String,
    pub tool_contract_digest: String,
    pub context_digest: String,
    pub source_digest: String,
}

/// No Deserialize implementation: records cannot recreate host enrollment.
#[derive(Clone, Debug)]
pub struct ValidationPlan {
    history: Vec<ValidationPlanRecord>,
    digest: String,
}
impl ValidationPlan {
    pub fn record(&self) -> &ValidationPlanRecord {
        self.history
            .last()
            .expect("enrollment has a current record")
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn history(&self) -> &[ValidationPlanRecord] {
        &self.history
    }
    pub fn requirement(&self, id: &str) -> Result<&ValidationRequirement> {
        self.record()
            .requirements
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| invalid("obligation is absent from the current enrolled plan"))
    }
    pub(crate) fn enroll(
        enrollment: ValidationEnrollment,
        proposal: ValidationProposal,
        supporting_refs: Vec<DiscoveryReadReference>,
        previous: Option<&ValidationPlan>,
    ) -> Result<Self> {
        if proposal.previous.as_deref() != previous.map(Self::digest) {
            return Err(invalid(
                "proposal does not continue the current validation plan",
            ));
        }
        let references = supporting_refs
            .iter()
            .map(DiscoveryReadReference::digest)
            .collect::<Result<Vec<_>>>()?;
        if references != proposal.supporting_reads {
            return Err(invalid(
                "proposal supporting reads differ from actual retained observations",
            ));
        }
        let mut history = previous.map(|p| p.history.clone()).unwrap_or_default();
        if let Some(prior) = previous {
            if prior.record().original_binding != enrollment.original_binding
                || prior.record().workflow_digest != enrollment.workflow_digest
                || prior.record().instruction_digest != enrollment.instruction_digest
                || prior.record().tool_contract_digest != enrollment.tool_contract_digest
            {
                return Err(invalid(
                    "validation history cannot transfer between worker admissions",
                ));
            }
            let ids: BTreeSet<_> = proposal.requirements.iter().map(|r| &r.id).collect();
            if prior
                .record()
                .requirements
                .iter()
                .any(|r| !ids.contains(&r.id))
            {
                return Err(invalid(
                    "validation discovery cannot discard an enrolled obligation",
                ));
            }
            if prior
                .record()
                .correction
                .as_ref()
                .map(|c| c.watermark)
                .unwrap_or(0)
                > enrollment
                    .correction
                    .as_ref()
                    .map(|c| c.watermark)
                    .unwrap_or(0)
            {
                return Err(invalid(
                    "validation history cannot roll back its correction watermark",
                ));
            }
        }
        let record = ValidationPlanRecord {
            schema_version: 1,
            original_binding: enrollment.original_binding,
            binding: enrollment.binding,
            correction: enrollment.correction,
            kind: CheckKind::Local,
            workflow_digest: enrollment.workflow_digest,
            instruction_digest: enrollment.instruction_digest,
            tool_contract_digest: enrollment.tool_contract_digest,
            context_digest: enrollment.context_digest,
            source_digest: enrollment.source_digest,
            predecessor: proposal.previous,
            ordinal: (history.len() + 1) as u32,
            reason: proposal.reason,
            requirements: proposal.requirements,
            supporting_reads: supporting_refs,
            created_at: Utc::now(),
        };
        record.validate_shape()?;
        let digest = canonical_digest(1, &record)?;
        history.push(record);
        Ok(Self { history, digest })
    }
}

fn invalid(message: &str) -> Error {
    Error::new("validation_plan", message)
}
fn text(value: &str, maximum: usize) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > maximum
        || value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        Err(invalid(
            "validation text is empty, oversized or contains control characters",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::bytes_digest;
    use crate::evidence::{Generation, WorkIdentity};
    use crate::git::{RepositoryId, Revision};

    // Deliberate data fixtures for enrollment invariants, not observed reads or
    // native execution evidence. The worker tests exercise the actual producer.
    fn fixture() -> (
        ValidationEnrollment,
        ValidationProposal,
        Vec<DiscoveryReadReference>,
    ) {
        let digest = bytes_digest(b"descriptive enrollment fixture");
        let binding = ExecutionBinding {
            runtime_id: Uuid::from_u128(1),
            operation_id: Uuid::from_u128(2),
            work: WorkIdentity {
                linear_endpoint: "https://api.linear.app/".into(),
                linear_workspace: Uuid::from_u128(3),
                parent: Uuid::from_u128(4),
                child: Some(Uuid::from_u128(5)),
                generation: Generation {
                    authorization: digest.clone(),
                    definition: digest.clone(),
                    correction: digest.clone(),
                    scope: digest.clone(),
                },
            },
            repository: RepositoryId {
                hostname: "github.com".into(),
                id: "fixture".into(),
            },
            revision: Revision {
                commit: "a".repeat(40),
                tree: "b".repeat(40),
            },
            candidate_vector_digest: None,
            isolation_digest: digest.clone(),
        };
        let read = DiscoveryReadReference {
            schema_version: 1,
            operation_id: binding.operation_id,
            call_id: Uuid::from_u128(6),
            turn: "fixture-turn".into(),
            instruction_digest: digest.clone(),
            correction_input_digest: digest.clone(),
            result_digest: digest.clone(),
            journal: DiscoveryJournalReference {
                leaf: "000001.json".into(),
                digest: digest.clone(),
                bytes: 1,
            },
            content: DiscoveryReadContent::Context {
                key: "task".into(),
                digest: digest.clone(),
                bytes: 1,
            },
        };
        let proposal = ValidationProposal {
            previous: None,
            reason: "check the task's behavior".into(),
            supporting_reads: vec![read.digest().unwrap()],
            requirements: vec![ValidationRequirement {
                id: "behavior".into(),
                statement: "specified behavior holds".into(),
                rationale: "the task requires it".into(),
                argv: vec!["unfamiliar-test-tool".into()],
                cwd: "/fixture/repository".into(),
                services: vec![],
                artifacts: vec![],
            }],
        };
        let enrollment = ValidationEnrollment {
            original_binding: binding.clone(),
            binding,
            correction: None,
            workflow_digest: digest.clone(),
            instruction_digest: digest.clone(),
            tool_contract_digest: digest.clone(),
            context_digest: digest.clone(),
            source_digest: digest,
        };
        (enrollment, proposal, vec![read])
    }

    #[test]
    fn revisions_keep_obligations_and_original_history_without_catalog_or_artifact() {
        let (enrollment, mut proposal, reads) = fixture();
        let first =
            ValidationPlan::enroll(enrollment.clone(), proposal.clone(), reads.clone(), None)
                .unwrap();
        let original = first.record().clone();
        proposal.previous = Some(first.digest().into());
        proposal.reason = "repository evidence identifies the correct test invocation".into();
        proposal.requirements[0]
            .argv
            .push("--exercise-behavior".into());
        let second = ValidationPlan::enroll(
            enrollment.clone(),
            proposal.clone(),
            reads.clone(),
            Some(&first),
        )
        .unwrap();
        assert_eq!(second.history().len(), 2);
        assert_eq!(second.history()[0], original);
        assert_ne!(second.digest(), first.digest());
        assert_eq!(second.record().ordinal, 2);
        proposal.previous = Some(second.digest().into());
        proposal.requirements[0].id = "easier-replacement".into();
        assert!(ValidationPlan::enroll(enrollment, proposal, reads, Some(&second)).is_err());
        assert_eq!(first.record(), &original);
    }

    #[test]
    fn enrollment_rejects_forged_reads_scope_and_history() {
        let (enrollment, proposal, reads) = fixture();
        let first =
            ValidationPlan::enroll(enrollment.clone(), proposal.clone(), reads.clone(), None)
                .unwrap();
        for case in 0..7 {
            let mut owner = enrollment.clone();
            let mut draft = proposal.clone();
            let mut references = reads.clone();
            match case {
                0 => draft.supporting_reads.clear(),
                1 => references[0].operation_id = Uuid::from_u128(100),
                2 => owner.binding.work.generation.scope = bytes_digest(b"widened"),
                3 => draft.previous = Some(first.digest().into()),
                4 => draft.requirements.clear(),
                5 => draft.requirements.push(draft.requirements[0].clone()),
                _ => draft.reason.clear(),
            }
            assert!(
                ValidationPlan::enroll(owner, draft, references, None).is_err(),
                "case {case}"
            );
        }
        let mut changed = enrollment;
        changed.original_binding.operation_id = Uuid::from_u128(100);
        changed.binding.operation_id = Uuid::from_u128(100);
        let mut next = proposal;
        next.previous = Some(first.digest().into());
        assert!(ValidationPlan::enroll(changed, next, reads, Some(&first)).is_err());
    }

    #[test]
    fn plan_history_is_finite_and_record_roundtrip_does_not_enroll_it() {
        let (enrollment, mut proposal, reads) = fixture();
        let mut plan =
            ValidationPlan::enroll(enrollment.clone(), proposal.clone(), reads.clone(), None)
                .unwrap();
        for _ in 1..MAX_VALIDATION_HISTORY {
            proposal.previous = Some(plan.digest().into());
            plan = ValidationPlan::enroll(
                enrollment.clone(),
                proposal.clone(),
                reads.clone(),
                Some(&plan),
            )
            .unwrap();
        }
        let bytes = serde_json::to_vec(plan.record()).unwrap();
        let decoded: ValidationPlanRecord = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, *plan.record());
        assert_eq!(canonical_digest(1, &decoded).unwrap(), plan.digest());
        proposal.previous = Some(plan.digest().into());
        assert!(ValidationPlan::enroll(enrollment, proposal, reads, Some(&plan)).is_err());
    }
}
