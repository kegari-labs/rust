//! Descriptive schema/predicate tests only. These values do not construct a
//! ValidationPlan, VerifiedCheckExecution, worker outcome or native capability.
use super::*;
use crate::execution::{
    ArtifactProof, DiscoveredCheckProvenance, ExecutionBinding, SourceSnapshot,
};
use crate::git::{RepositoryId, Revision};
use crate::isolation::{CommandObservation, CommandOutcome};
use crate::platform::BinaryIdentity;
use crate::validation::{
    DiscoveryJournalReference, DiscoveryReadContent, DiscoveryReadReference, ValidationRequirement,
};
use kegari_digest::bytes_digest;
use std::path::PathBuf;

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

fn evidence() -> (ValidationEvidence, RevisionBinding) {
    let digest = bytes_digest(b"descriptive fixture");
    let repository = RepositoryId {
        hostname: "github.com".into(),
        id: "42".into(),
    };
    let revision = Revision {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    let work = WorkIdentity {
        linear_endpoint: "https://linear.invalid/graphql".into(),
        linear_workspace: Uuid::from_u128(1),
        parent: Uuid::from_u128(2),
        child: Some(Uuid::from_u128(3)),
        generation: Generation {
            authorization: digest.clone(),
            definition: digest.clone(),
            correction: digest.clone(),
            scope: digest.clone(),
        },
    };
    let binding = ExecutionBinding {
        runtime_id: Uuid::from_u128(4),
        operation_id: Uuid::from_u128(5),
        work,
        repository: repository.clone(),
        revision: revision.clone(),
        candidate_vector_digest: None,
        isolation_digest: digest.clone(),
    };
    let source = SourceSnapshot {
        revision: revision.clone(),
        files: BTreeMap::new(),
    };
    let plan = ValidationPlanRecord {
        schema_version: 1,
        original_binding: binding.clone(),
        binding: binding.clone(),
        correction: None,
        kind: CheckKind::Local,
        workflow_digest: digest.clone(),
        instruction_digest: digest.clone(),
        tool_contract_digest: digest.clone(),
        context_digest: digest.clone(),
        source_digest: source.digest().unwrap(),
        predecessor: None,
        ordinal: 1,
        reason: "actual task requires the declared check".into(),
        requirements: vec![ValidationRequirement {
            id: "behavior".into(),
            statement: "requested behavior".into(),
            rationale: "repository evidence".into(),
            argv: vec!["unfamiliar-tool".into(), "verify".into()],
            cwd: ".".into(),
            services: vec![],
            artifacts: vec![],
        }],
        supporting_reads: vec![DiscoveryReadReference {
            schema_version: 1,
            operation_id: binding.operation_id,
            call_id: Uuid::from_u128(6),
            turn: "turn".into(),
            instruction_digest: digest.clone(),
            correction_input_digest: digest.clone(),
            result_digest: digest.clone(),
            journal: DiscoveryJournalReference {
                leaf: "event.json".into(),
                digest: digest.clone(),
                bytes: 10,
            },
            content: DiscoveryReadContent::Context {
                key: "task".into(),
                digest: digest.clone(),
                bytes: 10,
            },
        }],
        created_at: at(1),
    };
    let invocation = Uuid::from_u128(7);
    let transcript = PathBuf::from(format!(
        "/owned/scratch/validation-transcript-{invocation}.json"
    ));
    let binary = BinaryIdentity {
        path: "/usr/bin/fixture".into(),
        digest: digest.clone(),
        bytes: 1,
    };
    let observation = CommandObservation {
        schema_version: 1,
        requested_argv: plan.requirements[0].argv.clone(),
        selected_executable: binary.path.clone(),
        canonical_executable: binary.clone(),
        launcher: binary,
        cwd: "/owned/repository".into(),
        environment: BTreeMap::from([("HOME".into(), "/owned/scratch".into())]),
        original_isolation_digest: digest.clone(),
        isolation_digest: digest.clone(),
        launch_digest: digest.clone(),
        backend_policy_digest: format!("sha256:v5:{}", "a".repeat(64)),
    };
    let plan_digest = canonical_digest(1, &plan).unwrap();
    let attempt = CheckExecutionRecord {
        schema_version: 4,
        discovered: Some(DiscoveredCheckProvenance {
            schema_version: 1,
            plan: plan.clone(),
            plan_digest: plan_digest.clone(),
            invocation_id: invocation,
            observation: observation.clone(),
            service_observations: vec![],
            prepared_at: at(2),
            deadline_at: at(12),
            command_timeout_ms: 10_000,
            output_bytes: 1024,
            transcript: transcript.clone(),
            artifacts: vec![transcript.clone()],
        }),
        correction: None,
        bundle: None,
        binding: binding.clone(),
        workflow_digest: digest.clone(),
        check_name: "behavior".into(),
        kind: CheckKind::Local,
        before: source.clone(),
        after: source,
        artifacts: vec![ArtifactProof {
            path: transcript,
            digest: digest.clone(),
            bytes: 10,
            validation_operation: binding.operation_id,
            candidate_vector_digest: None,
        }],
        command: CommandOutcome {
            schema_version: 1,
            argv: observation.requested_argv,
            cwd: observation.cwd.clone(),
            isolation_digest: digest,
            exit_code: Some(0),
            termination_signal: None,
            timed_out: false,
            stdout: "observed output".into(),
            stderr: String::new(),
            output_truncated: false,
            elapsed_ms: 1,
        },
        started_at: at(2),
        completed_at: at(3),
        evidence_class: ExecutionEvidenceClass::ConfinedExecution,
        successful: true,
        blocker: None,
    };
    let revision = RevisionBinding {
        repository,
        base: "c".repeat(40),
        head: revision.commit.clone(),
        tested_commit: revision.commit,
        tested_tree: revision.tree,
    };
    (
        ValidationEvidence {
            schema_version: 1,
            plan_digest,
            original_cwd: "/owned/repository".into(),
            history: vec![plan],
            attempts: vec![attempt],
        },
        revision,
    )
}

fn assessment(evidence: &ValidationEvidence) -> ValidationAssessment {
    ValidationAssessment {
        evidence_digest: evidence.digest().unwrap(),
        sufficient: true,
        rationale: "The actual task, discovery history and retained attempts support this scope"
            .into(),
        requirements: BTreeMap::from([(
            "behavior".into(),
            ValidationRequirementAssessment {
                sufficient: true,
                rationale: "The observed result addresses this obligation".into(),
                evidence_digests: vec![canonical_digest(1, &evidence.attempts[0]).unwrap()],
            },
        )]),
    }
}

#[test]
fn changed_current_plan_cannot_reuse_prior_completed_plan() {
    let (mut evidence, revision) = evidence();
    let work = evidence.history[0].binding.work.clone();
    evidence.validate(&work, &revision).unwrap();
    let mut next = evidence.history[0].clone();
    next.predecessor = Some(evidence.plan_digest.clone());
    next.ordinal = 2;
    next.created_at = at(4);
    next.reason = "new discovery adds an assertion".into();
    next.requirements[0].argv.push("--extra".into());
    evidence.plan_digest = canonical_digest(1, &next).unwrap();
    evidence.history.push(next);
    assert_eq!(
        evidence.validate(&work, &revision).unwrap_err().code,
        "validation_obligations_pending"
    );
}

#[test]
fn later_timeout_without_repair_capability_blocks_old_success() {
    let (mut evidence, revision) = evidence();
    let work = evidence.history[0].binding.work.clone();
    let mut timeout = evidence.attempts[0].clone();
    timeout.successful = false;
    timeout.blocker = Some("timeout".into());
    timeout.command.timed_out = true;
    timeout.command.exit_code = None;
    timeout.started_at = at(4);
    timeout.completed_at = at(14);
    let provenance = timeout.discovered.as_mut().unwrap();
    provenance.invocation_id = Uuid::from_u128(8);
    provenance.prepared_at = at(4);
    provenance.deadline_at = at(14);
    provenance.transcript =
        "/owned/scratch/validation-transcript-00000000-0000-0000-0000-000000000008.json".into();
    provenance.artifacts = vec![provenance.transcript.clone()];
    timeout.artifacts[0].path = provenance.transcript.clone();
    evidence.attempts.push(timeout);
    assert_eq!(
        evidence.validate(&work, &revision).unwrap_err().code,
        "validation_failed"
    );
    assert_eq!(evidence.attempts.len(), 2);
}

#[test]
fn adequacy_requires_complete_identity_current_attempt_and_explicit_sufficiency() {
    let (evidence, revision) = evidence();
    evidence
        .validate(&evidence.history[0].binding.work, &revision)
        .unwrap();
    let valid = assessment(&evidence);
    evidence.verify_assessment(&valid, true).unwrap();
    let references = evidence.attempt_references().unwrap();
    assert_eq!(
        references[0]["record_digest"],
        canonical_digest(1, &evidence.attempts[0]).unwrap()
    );
    assert_eq!(references[0]["plan_digest"], evidence.plan_digest);
    assert_eq!(references[0]["obligation_id"], "behavior");
    assert_eq!(references[0]["selected"], true);
    for mut changed in [valid.clone(), valid.clone(), valid.clone(), valid.clone()]
        .into_iter()
        .enumerate()
    {
        match changed.0 {
            0 => changed.1.evidence_digest = bytes_digest(b"other history"),
            1 => changed.1.requirements.clear(),
            2 => changed
                .1
                .requirements
                .get_mut("behavior")
                .unwrap()
                .evidence_digests
                .clear(),
            _ => changed.1.sufficient = false,
        }
        assert_eq!(
            evidence
                .verify_assessment(&changed.1, true)
                .unwrap_err()
                .code,
            "validation_adequacy"
        );
    }
    let mut findings = valid;
    findings.sufficient = false;
    evidence.verify_assessment(&findings, false).unwrap();
}

#[test]
fn duplicate_substituted_and_fixture_attempts_cannot_form_production_evidence() {
    let (valid, revision) = evidence();
    let work = valid.history[0].binding.work.clone();
    let mut duplicate = valid.clone();
    duplicate.attempts.push(duplicate.attempts[0].clone());
    assert!(duplicate.validate(&work, &revision).is_err());
    let mut changed = valid.clone();
    changed.attempts[0].discovered.as_mut().unwrap().plan_digest = bytes_digest(b"other plan");
    assert!(changed.validate(&work, &revision).is_err());
    let mut fixture = valid;
    fixture.attempts[0].evidence_class = ExecutionEvidenceClass::DeterministicFixture;
    assert_eq!(
        fixture.validate(&work, &revision).unwrap_err().code,
        "validation_gate"
    );
}

#[test]
fn configured_obligation_is_preserved_and_empty_configuration_is_not_a_blocker() {
    let (evidence, _) = evidence();
    let workflow = crate::workflow::Workflow::parse("---\nchecks:\n  local:\n    behavior:\n      argv: [unfamiliar-tool, verify]\n      result_artifact: result.json\n---\nTask\n", "/descriptive/workflow.md").unwrap();
    evidence.verify_configured(&[], &workflow).unwrap();
    evidence
        .verify_configured(&["behavior".into()], &workflow)
        .unwrap();
    let mut changed = evidence.clone();
    changed.history[0].requirements[0]
        .argv
        .push("--narrow".into());
    assert_eq!(
        changed
            .verify_configured(&["behavior".into()], &workflow)
            .unwrap_err()
            .code,
        "validation_gate"
    );
    let mut cwd = evidence;
    cwd.attempts[0].command.cwd = "/owned/repository/subset".into();
    assert_eq!(
        cwd.verify_configured(&["behavior".into()], &workflow)
            .unwrap_err()
            .code,
        "validation_gate"
    );
}

#[test]
fn signed_review_schema_cannot_implicitly_promote_legacy_or_insufficient_assessment() {
    let (validation, revision) = evidence();
    let key = AttestationKey::new("predicate-test-key".into(), vec![7; 32]).unwrap();
    let work = validation.history[0].binding.work.clone();
    let delta = bytes_digest(b"descriptive no-change delta");
    let mut record = ReviewAttestation {
        schema_version: 2,
        runtime_id: Uuid::from_u128(4),
        owner_generation: 1,
        operation_id: Uuid::from_u128(20),
        work: work.clone(),
        revision: revision.clone(),
        pull_request: None,
        delta_digest: delta.clone(),
        verdict: Verdict::Pass,
        unresolved_findings: vec![],
        supporting_links: vec!["https://example.invalid/evidence".into()],
        review: ReviewKind::AgentValidation {
            session_id: "original-review-session".into(),
            instruction_manifest: bytes_digest(b"review instructions"),
            assessment: assessment(&validation),
            validation: Box::new(validation),
        },
    };
    let verify = |record: &ReviewAttestation| {
        let signed = key.sign("codex_review", record).unwrap();
        let publication = Publication {
            provider: "linear".into(),
            record_id: "record".into(),
            writer_account_id: "writer".into(),
            payload_digest: canonical_digest(1, &signed).unwrap(),
            observed_at: at(20),
            url: "https://example.invalid/record".into(),
        };
        verify_review(
            &signed,
            &publication,
            None,
            &key,
            ReviewRequirements {
                work: &work,
                revision: &revision,
                delta_digest: &delta,
                writer: "writer",
            },
        )
    };
    verify(&record).unwrap();
    record.schema_version = 1;
    assert_eq!(verify(&record).unwrap_err().code, "review_gate");
    record.schema_version = 2;
    if let ReviewKind::AgentValidation { assessment, .. } = &mut record.review {
        assessment.sufficient = false;
    }
    assert_eq!(verify(&record).unwrap_err().code, "validation_adequacy");
    record.review = ReviewKind::Agent {
        session_id: "old-session".into(),
        instruction_manifest: bytes_digest(b"old instructions"),
    };
    assert_eq!(verify(&record).unwrap_err().code, "review_gate");
    record.schema_version = 1;
    verify(&record).unwrap(); // Historical legacy semantics stay explicit.
}

fn integrated_description() -> (
    ReviewedIntegratedValidation,
    WorkIdentity,
    String,
    crate::git::CandidateManifest,
    RevisionBinding,
) {
    let (mut validation, revision) = evidence();
    let vector = bytes_digest(b"entire descriptive candidate vector");
    validation.schema_version = 2;
    let plan = &mut validation.history[0];
    plan.kind = CheckKind::Integrated;
    plan.original_binding.candidate_vector_digest = Some(vector.clone());
    plan.binding.candidate_vector_digest = Some(vector.clone());
    validation.plan_digest = canonical_digest(1, plan).unwrap();
    let attempt = &mut validation.attempts[0];
    attempt.kind = CheckKind::Integrated;
    attempt.binding = plan.binding.clone();
    attempt.artifacts[0].candidate_vector_digest = Some(vector.clone());
    let provenance = attempt.discovered.as_mut().unwrap();
    provenance.plan = plan.clone();
    provenance.plan_digest = validation.plan_digest.clone();
    let candidate = crate::git::CandidateManifest {
        schema_version: 1,
        operation_id: Uuid::from_u128(30),
        runtime_id: plan.binding.runtime_id,
        repository: revision.repository.clone(),
        parent_id: plan.binding.work.parent,
        work_generation: bytes_digest(b"work"),
        correction_generation: bytes_digest(b"correction"),
        git_version: "descriptive".into(),
        production: Revision {
            commit: revision.base.clone(),
            tree: "c".repeat(40),
        },
        ordered_delta_digests: vec![],
        ordered_patch_digests: vec![],
        selected_contributions: BTreeMap::new(),
        production_integrations: vec![],
        result: Some(plan.binding.revision.clone()),
        combined_patch_digest: None,
    };
    let review = IntegratedAdequacyReview {
        schema_version: 1,
        operation_id: Uuid::from_u128(31),
        session_id: "separate review".into(),
        work: plan.binding.work.clone(),
        candidate_vector_digest: vector.clone(),
        instruction_manifest: bytes_digest(b"review instructions"),
        tool_contract_digest: bytes_digest(b"review tools"),
        context_digest: bytes_digest(b"actual review context"),
        completed_at: at(4),
        assessment: assessment(&validation),
    };
    let mut parent = review.work.clone();
    parent.child = None;
    (
        ReviewedIntegratedValidation { validation, review },
        parent,
        vector,
        candidate,
        revision,
    )
}

#[test]
fn integrated_description_requires_separate_sufficient_exact_vector_review() {
    let (reviewed, parent, vector, candidate, revision) = integrated_description();
    let e2e = reviewed.review.work.child.unwrap();
    reviewed
        .verify(&parent, e2e, &vector, std::slice::from_ref(&candidate))
        .unwrap();
    assert!(
        reviewed
            .validation
            .validate(&reviewed.review.work, &revision)
            .is_err()
    );
    assert!(
        reviewed
            .verify(
                &parent,
                e2e,
                &bytes_digest(b"other vector"),
                std::slice::from_ref(&candidate)
            )
            .is_err()
    );
    assert!(reviewed.verify(&parent, e2e, &vector, &[]).is_err());
    let mut same_owner = reviewed.clone();
    same_owner.review.operation_id = same_owner.validation.history[0].binding.operation_id;
    assert_eq!(
        same_owner
            .verify(&parent, e2e, &vector, std::slice::from_ref(&candidate))
            .unwrap_err()
            .code,
        "validation_adequacy"
    );
    let mut insufficient = reviewed.clone();
    insufficient.review.assessment.sufficient = false;
    assert_eq!(
        insufficient
            .verify(&parent, e2e, &vector, std::slice::from_ref(&candidate))
            .unwrap_err()
            .code,
        "validation_adequacy"
    );
    let mut substituted = reviewed;
    substituted.review.assessment.evidence_digest = bytes_digest(b"other inventory");
    assert_eq!(
        substituted
            .verify(&parent, e2e, &vector, &[candidate])
            .unwrap_err()
            .code,
        "validation_adequacy"
    );
}

#[test]
fn integrated_configuration_is_optional_but_selected_constraints_remain_exact() {
    let (reviewed, _, _, _, _) = integrated_description();
    let workflow = crate::workflow::Workflow::parse("---\nchecks:\n  integrated:\n    behavior:\n      argv: [unfamiliar-tool, verify]\n      result_artifact: result.json\n---\nTask\n", "/descriptive/workflow.md").unwrap();
    reviewed
        .validation
        .verify_configured(&[], &workflow)
        .unwrap();
    reviewed
        .validation
        .verify_configured(&["behavior".into()], &workflow)
        .unwrap();
    assert!(
        reviewed
            .validation
            .verify_configured(&["missing".into()], &workflow)
            .is_err()
    );
    let mut changed = reviewed.validation;
    changed.history[0].requirements[0]
        .services
        .push(crate::workflow::CheckService {
            name: "extra".into(),
            argv: vec!["service".into()],
            readiness: crate::workflow::ServiceReadiness {
                argv: vec!["ready".into()],
                timeout_ms: 1,
            },
            max_lifetime_ms: 1,
        });
    assert!(
        changed
            .verify_configured(&["behavior".into()], &workflow)
            .is_err()
    );
}

#[test]
fn compact_review_resolves_large_complete_history_before_child_predicates() {
    use crate::validation_archive::{self, tests::descriptive_fixture};
    let temp = tempfile::tempdir().unwrap();
    let records =
        crate::storage::PrivateDir::create(&temp.path().canonicalize().unwrap().join("records"))
            .unwrap();
    let key = AttestationKey::new("compact-test".into(), vec![43; 32]).unwrap();
    let (namespace, mut history, artifacts) = descriptive_fixture();
    // Larger than both the former remote publication and local leaf bounds.
    history.attempts[0].command.stdout = "retained failed output\n".repeat(16_000);
    history.attempts[0]
        .discovered
        .as_mut()
        .unwrap()
        .output_bytes = 1024 * 1024;
    assert!(serde_json::to_vec(&history).unwrap().len() > 256 * 1024);
    let archive = validation_archive::retain(&records, &key, namespace, &history, |proof| {
        Ok(artifacts[&canonical_digest(1, proof)?].clone())
    })
    .unwrap();
    let current = history.current().unwrap();
    let work = current.binding.work.clone();
    let revision = RevisionBinding {
        repository: current.binding.repository.clone(),
        base: current.binding.revision.commit.clone(),
        head: current.binding.revision.commit.clone(),
        tested_commit: current.binding.revision.commit.clone(),
        tested_tree: current.binding.revision.tree.clone(),
    };
    let criteria = BTreeSet::from(["behavior".into()]);
    let reason = "descriptive no-change fixture";
    let delta = no_change_digest(&work, &revision, &criteria, reason).unwrap();
    let mut adequacy = assessment(&history);
    adequacy
        .requirements
        .get_mut("behavior")
        .unwrap()
        .evidence_digests = history.selected_digests().unwrap();
    let signed = key
        .sign(
            "codex_review",
            &ReviewAttestation {
                schema_version: 3,
                runtime_id: current.binding.runtime_id,
                owner_generation: 1,
                operation_id: Uuid::from_u128(41),
                work: work.clone(),
                revision: revision.clone(),
                pull_request: None,
                delta_digest: delta.clone(),
                verdict: Verdict::Pass,
                unresolved_findings: vec![],
                supporting_links: vec!["https://review.invalid/context".into()],
                review: ReviewKind::AgentValidationArchive {
                    session_id: "original review".into(),
                    instruction_manifest: bytes_digest(b"instructions"),
                    archive: archive.reference().clone(),
                    assessment: adequacy,
                },
            },
        )
        .unwrap();
    assert!(serde_json::to_vec(&signed).unwrap().len() < 60 * 1024);
    let publication = Publication {
        provider: "linear".into(),
        record_id: "review".into(),
        writer_account_id: "writer".into(),
        payload_digest: canonical_digest(1, &signed).unwrap(),
        observed_at: at(10),
        url: "https://review.invalid/record".into(),
    };
    let requirements = || ReviewRequirements {
        work: &work,
        revision: &revision,
        delta_digest: &delta,
        writer: "writer",
    };
    assert_eq!(
        verify_review(&signed, &publication, None, &key, requirements())
            .unwrap_err()
            .code,
        "validation_archive"
    );
    let review = verify_review_resolved(
        &signed,
        &publication,
        None,
        &key,
        requirements(),
        Some(archive.clone()),
    )
    .unwrap();
    assert_eq!(review.validation().unwrap(), &history);
    assert!(!review.validation().unwrap().attempts[0].successful);
    let local = history.runs(&revision).unwrap();
    let mut hosted = local[0].clone();
    hosted.id = "ci".into();
    hosted.kind = CheckKind::HostedCi;
    hosted.runner = Runner::GitHubHosted;
    hosted.workflow_id = Some(1);
    hosted.run_id = Some(1);
    let child = ChildEvidence {
        schema_version: 3,
        validation_selected: Some(history.selected_digests().unwrap()),
        work: work.clone(),
        revision: revision.clone(),
        fulfilled_criteria: criteria.clone(),
        outcome: ChildOutcome::NoChange {
            reason: reason.into(),
            change_gates_inapplicable: BTreeSet::from([
                "branch".into(),
                "pull_request".into(),
                "staging_merge".into(),
            ]),
            exact_sha_validation: "actual descriptive check".into(),
        },
        checks: CheckSnapshot {
            pagination_complete: true,
            observed_at: at(10),
            runs: vec![hosted],
        },
        review_digest: review.digest.clone(),
        results_for_dependents: local[0].artifacts.clone(),
    };
    let mut checks = history.requirements().unwrap();
    checks.push(CheckRequirement {
        id: "ci".into(),
        kind: CheckKind::HostedCi,
        workflow_id: Some(1),
        command: vec![],
    });
    let contexts = BTreeSet::new();
    let required = || ChildRequirements {
        work: &work,
        criteria: &criteria,
        revision: &revision,
        checks: &checks,
        required_contexts: &contexts,
    };
    validate_child_candidate(&child, required(), &review, None).unwrap();
    let mut changed = child.clone();
    changed.validation_selected.as_mut().unwrap().clear();
    assert!(validate_child_candidate(&changed, required(), &review, None).is_err());
    let mut changed = child.clone();
    changed.checks.runs.extend(local);
    assert!(validate_child_candidate(&changed, required(), &review, None).is_err());
    let mut changed = child;
    changed.checks.runs.clear();
    assert_eq!(
        validate_child_candidate(&changed, required(), &review, None)
            .unwrap_err()
            .code,
        "validation_gate"
    );
}

#[test]
fn compact_integrated_selection_resolves_original_records_and_rejects_substitution() {
    use crate::validation_archive::{self, ValidationArchiveNamespace};
    let temp = tempfile::tempdir().unwrap();
    let records =
        crate::storage::PrivateDir::create(&temp.path().canonicalize().unwrap().join("records"))
            .unwrap();
    let key = AttestationKey::new("integrated-compact".into(), vec![44; 32]).unwrap();
    let (mut original, parent, vector, candidate, _) = integrated_description();
    let bytes = b"descriptive fixture";
    original.validation.attempts[0].artifacts[0].bytes = bytes.len() as u64;
    original.review.assessment = assessment(&original.validation);
    let archive = validation_archive::retain(
        &records,
        &key,
        ValidationArchiveNamespace {
            workspace_id: Uuid::from_u128(9),
            runtime_id: original.validation.history[0].binding.runtime_id,
        },
        &original.validation,
        |_| Ok(bytes.to_vec()),
    )
    .unwrap();
    let compact = ArchivedIntegratedValidation {
        archive: archive.reference().clone(),
        selected: original.validation.selected_digests().unwrap(),
        test_resources: BTreeSet::from(["test-db".into()]),
        review: original.review.clone(),
    };
    let runs = compact.runs(&archive).unwrap();
    assert_eq!(
        runs[0].command,
        original.validation.attempts[0].command.argv
    );
    assert_eq!(runs[0].test_resources, compact.test_resources);
    assert_eq!(runs[0].artifacts[0].digest, bytes_digest(bytes));
    let resolved = ReviewedIntegratedValidation {
        validation: archive.evidence().clone(),
        review: compact.review.clone(),
    };
    resolved
        .verify(
            &parent,
            original.review.work.child.unwrap(),
            &vector,
            std::slice::from_ref(&candidate),
        )
        .unwrap();
    let mut changed = compact.clone();
    changed.selected[0] = bytes_digest(b"substituted record");
    assert!(changed.runs(&archive).is_err());
    let mut changed = compact;
    changed.archive.evidence_digest = bytes_digest(b"substituted full history");
    assert!(changed.runs(&archive).is_err());
}
