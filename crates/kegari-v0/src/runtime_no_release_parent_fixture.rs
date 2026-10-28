//! Synthetic provider/check facts passed through production evidence validators.
//! Actual no-change candidate and baseline come from the separately admitted
//! Git/vector fixture; this helper supplies no execution or joined capability.
use crate::evidence::*;
use chrono::{Duration, Utc};
use kegari_digest::canonical_digest;
use kegari_git::CandidateManifest;
use kegari_secrets::{AttestationKey, Signed};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
fn d(value: &str) -> String {
    canonical_digest(1, &value).unwrap()
}
fn publication<T: serde::Serialize>(payload: &T, provider: &str) -> Publication {
    Publication {
        provider: provider.into(),
        record_id: "observed-record-123".into(),
        writer_account_id: "00000000-0000-0000-0000-000000000005".into(),
        payload_digest: canonical_digest(1, payload).unwrap(),
        observed_at: Utc::now(),
        url: "https://example.test/evidence/123".into(),
    }
}

fn check_requirements() -> Vec<CheckRequirement> {
    vec![
        CheckRequirement {
            id: "local-full".into(),
            kind: CheckKind::Local,
            workflow_id: None,
            command: vec!["./test.sh".into()],
        },
        CheckRequirement {
            id: "ci-full".into(),
            kind: CheckKind::HostedCi,
            workflow_id: Some(99),
            command: vec![],
        },
    ]
}

fn checks(w: &WorkIdentity, revision: &RevisionBinding) -> CheckSnapshot {
    let now = Utc::now();
    CheckSnapshot {
        pagination_complete: true,
        observed_at: now,
        runs: check_requirements()
            .into_iter()
            .map(|r| CheckRun {
                id: r.id,
                kind: r.kind.clone(),
                operation_id: Uuid::new_v4(),
                work_digest: w.digest().unwrap(),
                revision: revision.clone(),
                workflow_id: r.workflow_id,
                run_id: r.workflow_id.map(|_| 123),
                attempt: 1,
                runner: if r.kind == CheckKind::Local {
                    Runner::LocalSandbox
                } else {
                    Runner::GitHubHosted
                },
                command: r.command,
                environment_identity: "fixture-environment-v1".into(),
                started_at: Some(now - Duration::seconds(3)),
                completed_at: Some(now - Duration::seconds(1)),
                conclusion: Conclusion::Success,
                artifacts: vec![artifact_bytes("fixture://actual-output", b"exit 0")],
            })
            .collect(),
    }
}

fn review(
    w: &WorkIdentity,
    revision: &RevisionBinding,
    delta: &str,
    pr: Option<u64>,
    key: &AttestationKey,
) -> (Signed<ReviewAttestation>, Publication, Option<ReviewStatus>) {
    let signed = key
        .sign(
            "codex_review",
            &ReviewAttestation {
                schema_version: 1,
                runtime_id: Uuid::new_v4(),
                owner_generation: 1,
                operation_id: Uuid::new_v4(),
                work: w.clone(),
                revision: revision.clone(),
                pull_request: pr,
                delta_digest: delta.into(),
                verdict: Verdict::Pass,
                unresolved_findings: vec![],
                supporting_links: vec!["https://example.test/actual-review".into()],
                review: ReviewKind::Agent {
                    session_id: "fixture-thread-fixture-turn".into(),
                    instruction_manifest: d("instructions"),
                },
            },
        )
        .unwrap();
    let record = publication(&signed, if pr.is_some() { "github" } else { "linear" });
    let status = pr.map(|_| ReviewStatus {
        repository: revision.repository.clone(),
        head: revision.head.clone(),
        context: REVIEW_CONTEXT.into(),
        state: "success".into(),
        writer_account_id: "00000000-0000-0000-0000-000000000005".into(),
        attestation_record_id: record.record_id.clone(),
    });
    (signed, record, status)
}

fn protection(revision: &RevisionBinding, branch: &str) -> RepositoryProtection {
    RepositoryProtection {
        repository: revision.repository.clone(),
        branch: branch.into(),
        observed_base: revision.base.clone(),
        required_contexts: BTreeSet::from([REVIEW_CONTEXT.into(), "unit".into(), "e2e".into()]),
        strict_current_base: true,
        squash_allowed: true,
        extra_approvals_satisfied: true,
        code_owner_reviews_satisfied: true,
        admin_bypass: false,
        observed_at: Utc::now(),
    }
}

fn verified_child(
    w: WorkIdentity,
    revision: RevisionBinding,
    outcome: ChildOutcome,
    key: &AttestationKey,
) -> (VerifiedChild, VerifiedReview) {
    let criteria = BTreeSet::from(["prepared-outcome".into()]);
    let (delta_digest, pr) = match &outcome {
        ChildOutcome::Changed {
            pull_request,
            delta,
        } => (delta.review_digest().unwrap(), Some(*pull_request)),
        ChildOutcome::Replacement { delta, .. } => (delta.review_digest().unwrap(), None),
        ChildOutcome::NoChange { reason, .. } => (
            no_change_digest(&w, &revision, &criteria, reason).unwrap(),
            None,
        ),
    };
    let (signed, record, status) = review(&w, &revision, &delta_digest, pr, key);
    let review = verify_review(
        &signed,
        &record,
        status.as_ref(),
        key,
        ReviewRequirements {
            work: &w,
            revision: &revision,
            delta_digest: &delta_digest,
            writer: "00000000-0000-0000-0000-000000000005",
        },
    )
    .unwrap();
    let evidence = ChildEvidence {
        validation_selected: None,
        schema_version: 1,
        work: w.clone(),
        revision: revision.clone(),
        fulfilled_criteria: criteria.clone(),
        outcome,
        checks: checks(&w, &revision),
        review_digest: review.digest().into(),
        results_for_dependents: vec![artifact_bytes("fixture://child-result", b"observed result")],
    };
    let observed = publication(&evidence, "linear");
    let requirements = ChildRequirements {
        work: &w,
        criteria: &criteria,
        revision: &revision,
        checks: &check_requirements(),
        required_contexts: &BTreeSet::from([REVIEW_CONTEXT.into()]),
    };
    let child = verify_child(
        &evidence,
        requirements,
        &review,
        pr.map(|_| protection(&revision, "staging")).as_ref(),
        &observed,
        "00000000-0000-0000-0000-000000000005",
    )
    .unwrap();
    (child, review)
}

pub fn parent(
    parent_work: &WorkIdentity,
    child_work: &WorkIdentity,
    key: &AttestationKey,
    candidate: CandidateManifest,
    baselines: Vec<ProductionBaseline>,
    dependency_snapshot_digest: String,
) -> (VerifiedParent, VerifiedChild) {
    assert!(candidate.ordered_delta_digests.is_empty());
    assert_eq!(candidate.result.as_ref(), Some(&candidate.production));
    assert_eq!(baselines.len(), 1);
    assert_eq!(baselines[0].production, candidate.production);
    let production_targets: BTreeSet<_> = baselines
        .iter()
        .flat_map(|baseline| baseline.target_artifacts.keys().cloned())
        .collect();
    let repository = candidate.repository.clone();
    let revision = candidate.production.clone();
    let (child, _) = verified_child(
        child_work.clone(),
        RevisionBinding {
            repository: repository.clone(),
            head: revision.commit.clone(),
            base: revision.commit.clone(),
            tested_commit: revision.commit.clone(),
            tested_tree: revision.tree.clone(),
        },
        ChildOutcome::NoChange {
            reason: "Synthetic complete no-change validation against exact fixture production"
                .into(),
            change_gates_inapplicable: ["branch", "pull_request", "staging_merge"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
            exact_sha_validation: "synthetic exact revision hosted observation".into(),
        },
        key,
    );
    let child_id = child_work.child.unwrap();
    let now = Utc::now();
    let mut manifest = ParentEvidence {
        archived_integrated_validation: None,
        integrated_validation: None,
        schema_version: 1,
        work: parent_work.clone(),
        required_children: BTreeSet::from([child_id]),
        e2e_child: child_id,
        child_results: BTreeMap::from([(child_id, child.digest().into())]),
        required_contributions: BTreeMap::new(),
        executive_summary:
            "Complete prepared fixture responsibility against isolated production candidate".into(),
        touched_components: vec!["behavior.txt".into()],
        fulfilled_criteria: BTreeSet::from(["prepared-outcome".into()]),
        caveats: vec!["Synthetic provider/check observations; no live qualification".into()],
        go: true,
        baselines,
        candidates: vec![candidate],
        dependency_snapshot_digest,
        production_targets: production_targets.clone(),
        integrated_results: vec![],
        visual: false,
        video: None,
        publication_operation: Uuid::new_v4(),
        created_at: now,
        supersedes: None,
    };
    manifest.integrated_results.push(IntegratedRun {
        id: "e2e".into(),
        operation_id: Uuid::new_v4(),
        e2e_child: child_id,
        candidate_vector_digest: manifest.candidate_vector_digest().unwrap(),
        command: vec!["/usr/bin/true".into()],
        test_resources: BTreeSet::new(),
        started_at: now - Duration::seconds(2),
        completed_at: now - Duration::seconds(1),
        conclusion: Conclusion::Success,
        after_staging_integration: true,
        artifacts: vec![artifact_bytes(
            "fixture://synthetic-integrated-result",
            b"synthetic complete result",
        )],
    });
    let verified = verify_parent(
        &manifest,
        ParentRequirements {
            work: parent_work,
            required_children: &manifest.required_children,
            e2e_child: child_id,
            criteria: &manifest.fulfilled_criteria,
            integrated_commands: &BTreeMap::from([("e2e".into(), vec!["/usr/bin/true".into()])]),
            test_resources: &BTreeSet::new(),
            production_targets: &production_targets,
            visual: false,
            required_contributions: &BTreeMap::new(),
            production_integrations: &[],
        },
        std::slice::from_ref(&child),
        &publication(&manifest, "linear"),
        "00000000-0000-0000-0000-000000000005",
    )
    .unwrap();
    (verified, child)
}
