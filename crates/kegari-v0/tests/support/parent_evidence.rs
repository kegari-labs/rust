//! Synthetic provider/check facts passed through production evidence validators.
//! Actual Git candidate bytes are created only in an owned disposable repository.
use crate::pinned_git;
use chrono::{Duration, Utc};
use kegari_v0::{digest::canonical_digest, evidence::*, git::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;
use uuid::Uuid;
fn git_binary() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join("git"))
        .find(|p| p.is_file())
        .expect("Git is required")
}

fn git_run(path: &Path, args: &[&str]) -> String {
    let output = Command::new(git_binary())
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Disposable Fixture Human")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.test")
        .env("GIT_COMMITTER_NAME", "Disposable Fixture Human")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.test")
        .current_dir(path)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgSign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_string()
}

fn commit(path: &Path, message: &str) -> String {
    git_run(path, &["add", "-A"]);
    git_run(path, &["commit", "--signoff", "-m", message]);
    git_run(path, &["rev-parse", "HEAD"])
}

fn private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
}

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
) -> (TempDir, VerifiedParent) {
    let tmp = TempDir::new().unwrap();
    let source = tmp.path().join("source");
    let runs = tmp.path().join("runs");
    private_dir(&source);
    private_dir(&runs);
    git_run(&source, &["init", "-b", "main"]);
    fs::write(source.join("behavior.txt"), "known production fixture\n").unwrap();
    let production = commit(&source, "test: create owned production fixture");
    let git = Git::new(pinned_git::prepare(tmp.path())).unwrap();
    let repository = RepositoryId {
        hostname: "github.invalid".into(),
        id: "42".into(),
    };
    let candidate = git
        .assemble_candidate(
            &source,
            &runs,
            CandidateRequest {
                operation_id: Uuid::new_v4(),
                runtime_id: Uuid::from_u128(1),
                repository: repository.clone(),
                parent_id: parent_work.parent,
                work_generation: &parent_work.generation.definition,
                correction_generation: &parent_work.generation.correction,
                production_commit: &production,
                deltas: &[],
                child_dependencies: &BTreeMap::new(),
                satisfied_children: &BTreeSet::new(),
                production_integrations: &[],
                identity: CommitIdentity {
                    name: "Disposable Fixture Human".into(),
                    email: "fixture@example.test".into(),
                },
            },
        )
        .unwrap();
    let revision = candidate.manifest.production.clone();
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
        baselines: vec![ProductionBaseline {
            target_artifacts: BTreeMap::new(),
            repository,
            production: revision,
            deployed_revision: None,
            deployed_artifact: None,
            deployment_free: true,
            successor_recovery_operation: None,
        }],
        candidates: vec![candidate.manifest],
        dependency_snapshot_digest: d("fixture no dependencies"),
        production_targets: BTreeSet::new(),
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
            production_targets: &BTreeSet::new(),
            visual: false,
            required_contributions: &BTreeMap::new(),
            production_integrations: &[],
        },
        &[child],
        &publication(&manifest, "linear"),
        "00000000-0000-0000-0000-000000000005",
    )
    .unwrap();
    (tmp, verified)
}
