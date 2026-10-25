//! Deterministic fixtures use real disposable Git repositories. Provider facts
//! below are explicitly simulated complete snapshots, not live qualification.
use chrono::{Duration, Utc};
use kegari_v0::digest::{bytes_digest, canonical_digest};
use kegari_v0::error::{Error, Result};
use kegari_v0::evidence::*;
use kegari_v0::git::*;
use kegari_v0::release::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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

struct Fixture {
    _root: TempDir,
    source: PathBuf,
    runs: PathBuf,
    git: Git,
    repository: RepositoryId,
    parent: Uuid,
    child: Uuid,
    production: String,
    base: String,
    head: String,
    squash: String,
}

impl Fixture {
    fn new(dependent_on_b: bool) -> Self {
        let root = TempDir::new().unwrap();
        let source = root.path().join("operator-checkout");
        let runs = root.path().join("owned-runs");
        private_dir(&source);
        private_dir(&runs);
        git_run(&source, &["init", "-b", "main"]);
        fs::write(source.join("a.txt"), "first\nsecond\nthird\nfourth\n").unwrap();
        let production = commit(&source, "feat: establish production baseline");
        git_run(&source, &["switch", "-c", "staging"]);
        fs::write(
            source.join("unapproved-b.txt"),
            "B must never enter A's release\n",
        )
        .unwrap();
        if dependent_on_b {
            fs::write(
                source.join("a.txt"),
                "first\nB-only context\nthird\nfourth\n",
            )
            .unwrap();
        }
        let base = commit(&source, "feat: stage unapproved parent B");
        git_run(&source, &["switch", "-c", "CHILD-1-Full-Parent-A-Title"]);
        fs::write(
            source.join("a.txt"),
            if dependent_on_b {
                "first\nB-only context\nthird\nA fourth\n"
            } else {
                "first\nsecond\nthird\nA fourth\n"
            },
        )
        .unwrap();
        let head = commit(&source, "feat: implement parent A");
        git_run(&source, &["switch", "staging"]);
        git_run(
            &source,
            &["merge", "--squash", "CHILD-1-Full-Parent-A-Title"],
        );
        git_run(
            &source,
            &[
                "commit",
                "--signoff",
                "-m",
                "feat: squash parent A into staging",
            ],
        );
        let squash = git_run(&source, &["rev-parse", "HEAD"]);
        Self {
            _root: root,
            source,
            runs,
            git: Git::new(git_binary()).unwrap(),
            repository: RepositoryId {
                hostname: "github.com".into(),
                id: "101".into(),
            },
            parent: Uuid::new_v4(),
            child: Uuid::new_v4(),
            production,
            base,
            head,
            squash,
        }
    }

    fn delta(&self) -> DeltaArtifact {
        self.git
            .capture_delta(
                &self.source,
                CaptureDelta {
                    repository: self.repository.clone(),
                    parent_id: self.parent,
                    child_id: self.child,
                    work_generation: &d("definition"),
                    correction_generation: &d("correction"),
                    scope: WriteScope {
                        paths: vec!["a.txt".into()],
                    },
                    base: &self.base,
                    head: &self.head,
                    staging_parent: &self.base,
                    staging_commit: &self.squash,
                },
            )
            .unwrap()
    }

    fn candidate(&self, operation: Uuid, deltas: &[DeltaArtifact]) -> Result<Candidate> {
        self.git.assemble_candidate(
            &self.source,
            &self.runs,
            CandidateRequest {
                operation_id: operation,
                runtime_id: Uuid::new_v4(),
                repository: self.repository.clone(),
                parent_id: self.parent,
                work_generation: &d("definition"),
                correction_generation: &d("correction"),
                production_commit: &self.production,
                deltas,
                child_dependencies: &BTreeMap::new(),
                satisfied_children: &BTreeSet::new(),
                identity: CommitIdentity {
                    name: "Disposable Fixture Human".into(),
                    email: "fixture@example.test".into(),
                },
            },
        )
    }
}

fn d(value: &str) -> String {
    canonical_digest(1, &value).unwrap()
}

#[test]
fn v0_t16_real_candidate_excludes_staged_b_and_preserves_operator_partial_staging() {
    let f = Fixture::new(false);
    let delta = f.delta();
    fs::write(f.source.join("a.txt"), "operator staged change\n").unwrap();
    git_run(&f.source, &["add", "a.txt"]);
    fs::write(f.source.join("a.txt"), "operator unstaged change\n").unwrap();
    fs::write(
        f.source.join("private-note.txt"),
        "unfinished operator work\n",
    )
    .unwrap();
    let status_before = git_run(&f.source, &["status", "--porcelain=v1"]);
    let staged_before = git_run(&f.source, &["diff", "--cached", "--binary"]);
    let candidate = f.candidate(Uuid::new_v4(), &[delta]).unwrap();
    assert_eq!(
        fs::read_to_string(candidate.path().join("a.txt")).unwrap(),
        "first\nsecond\nthird\nA fourth\n"
    );
    assert!(!candidate.path().join("unapproved-b.txt").exists());
    assert!(!candidate.path().join("private-note.txt").exists());
    assert_eq!(
        git_run(&f.source, &["status", "--porcelain=v1"]),
        status_before
    );
    assert_eq!(
        git_run(&f.source, &["diff", "--cached", "--binary"]),
        staged_before
    );
    assert_eq!(
        fs::read_to_string(f.source.join("a.txt")).unwrap(),
        "operator unstaged change\n"
    );
    assert_eq!(
        git_run(candidate.path(), &["rev-parse", "HEAD^"]),
        f.production
    );
    assert!(
        git_run(candidate.path(), &["log", "-1", "--format=%B"])
            .contains("Signed-off-by: Disposable Fixture Human <fixture@example.test>")
    );
    let inspected = f
        .git
        .inspect_candidate(&f.runs, candidate.manifest.operation_id)
        .unwrap();
    assert_eq!(inspected.manifest, candidate.manifest);
}

#[test]
fn v0_t16_missing_b_context_blocks_without_fuzz_or_import_and_preserves_partial_candidate() {
    let f = Fixture::new(true);
    let operation = Uuid::new_v4();
    let error = f.candidate(operation, &[f.delta()]).unwrap_err();
    assert_eq!(error.code, "isolated_patch_conflict");
    let path = f.runs.join(format!("candidate-{operation}"));
    assert!(path.join(".git/kegari-candidate.json").exists());
    assert!(!path.join("unapproved-b.txt").exists());
    assert_eq!(
        fs::read_to_string(path.join("a.txt")).unwrap(),
        "first\nsecond\nthird\nfourth\n"
    );
    assert_eq!(
        f.git
            .inspect_candidate(&f.runs, operation)
            .unwrap_err()
            .code,
        "unfinished_candidate"
    );
    assert_eq!(
        f.candidate(operation, &[f.delta()]).unwrap_err().code,
        "ownership_conflict"
    );
}

#[test]
fn v0_t15_squash_delta_scope_and_exact_artifact_are_revalidated() {
    let f = Fixture::new(false);
    let mut delta = f.delta();
    delta.patch.extend_from_slice(b"forged patch data\n");
    assert_eq!(
        f.git.verify_delta(&f.source, &delta).unwrap_err().code,
        "provenance_mismatch"
    );
    let result = f.git.capture_delta(
        &f.source,
        CaptureDelta {
            repository: f.repository.clone(),
            parent_id: f.parent,
            child_id: f.child,
            work_generation: &d("definition"),
            correction_generation: &d("correction"),
            scope: WriteScope {
                paths: vec!["elsewhere/".into()],
            },
            base: &f.base,
            head: &f.head,
            staging_parent: &f.base,
            staging_commit: &f.squash,
        },
    );
    assert_eq!(result.unwrap_err().code, "scope_violation");
    fs::write(f.source.join("a.txt"), "unreviewed squash content\n").unwrap();
    let wrong = commit(&f.source, "fix: change staging after review");
    let result = f.git.capture_delta(
        &f.source,
        CaptureDelta {
            repository: f.repository.clone(),
            parent_id: f.parent,
            child_id: f.child,
            work_generation: &d("definition"),
            correction_generation: &d("correction"),
            scope: WriteScope {
                paths: vec!["a.txt".into()],
            },
            base: &f.base,
            head: &f.head,
            staging_parent: &f.base,
            staging_commit: &wrong,
        },
    );
    assert_eq!(result.unwrap_err().code, "provenance_mismatch");
}

#[test]
fn full_unicode_branch_title_is_preserved_and_length_is_never_truncated() {
    let git = Git::new(git_binary()).unwrap();
    let branch = git
        .branch_name(
            "CHILD-12",
            "Full Ångström / 日本語 Title @{Draft} .. Final.lock",
            255,
        )
        .unwrap();
    assert_eq!(
        branch.name,
        "CHILD-12-Full-Ångström-日本語-Title-@-Draft}-Final-lock"
    );
    assert_eq!(
        git.branch_name("CHILD-12", "Long complete parent title", 15)
            .unwrap_err()
            .code,
        "invalid_branch_name"
    );
    assert!(title_suffix(".. / \n ~").is_err());
    assert!(git.branch_name("../escaped", "Parent", 255).is_err());
}

#[test]
fn v0_t17_binary_modes_and_symlink_targets_survive_exact_patch_application() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new(false);
    git_run(&f.source, &["switch", "-c", "BINARY-child", &f.production]);
    fs::write(f.source.join("asset.bin"), [0, 255, 0, 1, 2, 3]).unwrap();
    fs::write(f.source.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(f.source.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    symlink("a.txt", f.source.join("link")).unwrap();
    let head = commit(&f.source, "feat: add binary executable and link");
    git_run(
        &f.source,
        &["switch", "-c", "binary-staging", &f.production],
    );
    git_run(&f.source, &["merge", "--squash", "BINARY-child"]);
    git_run(
        &f.source,
        &["commit", "--signoff", "-m", "feat: squash binary delta"],
    );
    let squash = git_run(&f.source, &["rev-parse", "HEAD"]);
    let delta = f
        .git
        .capture_delta(
            &f.source,
            CaptureDelta {
                repository: f.repository.clone(),
                parent_id: f.parent,
                child_id: f.child,
                work_generation: &d("definition"),
                correction_generation: &d("correction"),
                scope: WriteScope {
                    paths: vec!["asset.bin".into(), "run.sh".into(), "link".into()],
                },
                base: &f.production,
                head: &head,
                staging_parent: &f.production,
                staging_commit: &squash,
            },
        )
        .unwrap();
    assert!(String::from_utf8_lossy(&delta.patch).contains("GIT binary patch"));
    let candidate = f.candidate(Uuid::new_v4(), &[delta]).unwrap();
    assert_eq!(
        fs::read(candidate.path().join("asset.bin")).unwrap(),
        [0, 255, 0, 1, 2, 3]
    );
    assert_ne!(
        fs::metadata(candidate.path().join("run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    assert_eq!(
        fs::read_link(candidate.path().join("link")).unwrap(),
        PathBuf::from("a.txt")
    );
}

fn work(parent: Uuid, child: Option<Uuid>) -> WorkIdentity {
    WorkIdentity {
        linear_endpoint: "https://api.linear.app/graphql".into(),
        linear_workspace: Uuid::from_u128(42),
        parent,
        child,
        generation: Generation {
            authorization: d("authorization"),
            definition: d("definition"),
            correction: d("correction"),
            scope: d("scope"),
        },
    }
}

fn publication<T: serde::Serialize>(payload: &T, provider: &str) -> Publication {
    Publication {
        provider: provider.into(),
        record_id: "observed-record-123".into(),
        writer_account_id: "selected-user".into(),
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
        writer_account_id: "selected-user".into(),
        attestation_record_id: record.record_id.clone(),
    });
    (signed, record, status)
}

#[test]
fn v0_t15_forged_review_status_and_stale_head_do_not_pass() {
    let f = Fixture::new(false);
    let delta = f.delta();
    let w = work(f.parent, Some(f.child));
    let revision = RevisionBinding {
        repository: f.repository.clone(),
        head: f.head.clone(),
        base: f.base.clone(),
        tested_commit: f.head.clone(),
        tested_tree: delta.manifest.validated_head.tree.clone(),
    };
    let key = AttestationKey::new("key-v1".into(), vec![7; 32]).unwrap();
    let (mut signed, mut record, status) = review(
        &w,
        &revision,
        &delta.manifest.digest().unwrap(),
        Some(1),
        &key,
    );
    verify_review(
        &signed,
        &record,
        status.as_ref(),
        &key,
        ReviewRequirements {
            work: &w,
            revision: &revision,
            delta_digest: &delta.manifest.digest().unwrap(),
            writer: "selected-user",
        },
    )
    .unwrap();
    signed.payload.revision.head = f.production.clone();
    record.payload_digest = signed.digest().unwrap();
    assert_eq!(
        verify_review(
            &signed,
            &record,
            status.as_ref(),
            &key,
            ReviewRequirements {
                work: &w,
                revision: &revision,
                delta_digest: &delta.manifest.digest().unwrap(),
                writer: "selected-user"
            }
        )
        .unwrap_err()
        .code,
        "invalid_attestation"
    );
    let (signed, record, mut status) = review(
        &w,
        &revision,
        &delta.manifest.digest().unwrap(),
        Some(1),
        &key,
    );
    status.as_mut().unwrap().writer_account_id = "untrusted-writer".into();
    assert_eq!(
        verify_review(
            &signed,
            &record,
            status.as_ref(),
            &key,
            ReviewRequirements {
                work: &w,
                revision: &revision,
                delta_digest: &delta.manifest.digest().unwrap(),
                writer: "selected-user"
            }
        )
        .unwrap_err()
        .code,
        "review_gate"
    );
    assert!(
        verify_review(
            &signed,
            &record,
            None,
            &key,
            ReviewRequirements {
                work: &w,
                revision: &revision,
                delta_digest: &delta.manifest.digest().unwrap(),
                writer: "selected-user"
            }
        )
        .is_err()
    );
}

#[test]
fn local_success_cannot_bypass_missing_skipped_stale_or_self_hosted_ci() {
    let f = Fixture::new(false);
    let revision = RevisionBinding {
        repository: f.repository.clone(),
        head: f.head.clone(),
        base: f.base.clone(),
        tested_commit: f.head.clone(),
        tested_tree: f.delta().manifest.validated_head.tree,
    };
    let w = work(f.parent, Some(f.child));
    let original = checks(&w, &revision);
    verify_checks(&check_requirements(), &w, &revision, &original).unwrap();
    for conclusion in [
        Conclusion::Skipped,
        Conclusion::Neutral,
        Conclusion::Failure,
        Conclusion::Cancelled,
        Conclusion::Pending,
        Conclusion::Unknown,
    ] {
        let mut snapshot = original.clone();
        snapshot.runs[1].conclusion = conclusion;
        assert_eq!(
            verify_checks(&check_requirements(), &w, &revision, &snapshot)
                .unwrap_err()
                .code,
            "validation_gate"
        );
    }
    let mut snapshot = original.clone();
    snapshot.runs[1].runner = Runner::SelfHosted;
    assert!(verify_checks(&check_requirements(), &w, &revision, &snapshot).is_err());
    let mut snapshot = original.clone();
    snapshot.runs[1].revision.base = f.production.clone();
    assert!(verify_checks(&check_requirements(), &w, &revision, &snapshot).is_err());
    let mut snapshot = original.clone();
    snapshot.pagination_complete = false;
    assert_eq!(
        verify_checks(&check_requirements(), &w, &revision, &snapshot)
            .unwrap_err()
            .code,
        "provider_pagination"
    );
    let mut snapshot = original;
    snapshot.runs.pop();
    assert!(verify_checks(&check_requirements(), &w, &revision, &snapshot).is_err());
}

fn synthetic_plan(parent: Uuid, repository: &RepositoryId, suffix: char) -> ReleasePlan {
    let base = Revision {
        commit: "1".repeat(40),
        tree: "2".repeat(40),
    };
    let candidate = Revision {
        commit: "3".repeat(40),
        tree: "4".repeat(40),
    };
    ReleasePlan {
        schema_version: 1,
        runtime_id: Uuid::new_v4(),
        operation_id: Uuid::new_v4(),
        parent_id: parent,
        linear_workspace: Uuid::from_u128(42),
        generation: work(parent, None).generation,
        evidence_token: d("evidence"),
        acceptance_digest: d("acceptance"),
        carried_acceptance: None,
        resources: BTreeSet::from([ResourceKey::Repository {
            repository: repository.clone(),
        }]),
        dependencies: vec![],
        spacing_seconds: 0,
        deployment_free: BTreeSet::from([repository.clone()]),
        steps: vec![ReleaseStep {
            id: format!("merge-{suffix}"),
            depends_on: BTreeSet::new(),
            kind: StepKind::Merge {
                repository: repository.clone(),
                pull_request: 1,
                production_base: base,
                candidate,
                selected_delta_digests: vec![d("delta")],
            },
        }],
    }
}

fn restored_attempt(plan: ReleasePlan) -> ReleaseAttempt {
    ReleaseAttempt {
        steps: plan
            .steps
            .iter()
            .map(|s| (s.id.clone(), StepState::NotStarted))
            .collect(),
        plan,
        admitted_at: Utc::now(),
        control_generation: 1,
        owner_generation: 1,
        step_history: BTreeMap::new(),
        cancellations: BTreeMap::new(),
        hold_reason: None,
        abandoned: false,
        resolved: false,
        assigned_successor: None,
        recovery_records: vec![],
    }
}

fn persist(path: &Path, attempt: &ReleaseAttempt) -> Result<()> {
    let bytes = serde_json::to_vec(attempt).unwrap();
    fs::write(path, &bytes).map_err(|e| Error::io("fixture persisted release receipt", e))
}

#[test]
fn v0_t09_sent_unknown_is_durable_before_dispatch_and_never_blindly_replayed() {
    let repo = RepositoryId {
        hostname: "github.com".into(),
        id: "101".into(),
    };
    let plan = synthetic_plan(Uuid::new_v4(), &repo, 'a');
    let mut attempt = restored_attempt(plan);
    let generation = attempt.plan.generation.clone();
    let evidence = attempt.plan.evidence_token.clone();
    let revisions = BTreeMap::from([(
        repo,
        Revision {
            commit: "1".repeat(40),
            tree: "2".repeat(40),
        },
    )]);
    let facts = FreshReleaseFacts {
        generation: &generation,
        evidence_token: &evidence,
        owner_generation: 1,
        authority_current: true,
        pending_correction: false,
        provider_reads_complete: true,
        resource_lock_owned: true,
        credentials_usable: true,
        repository_policy_current: true,
        force_stopped: false,
        current_revisions: &revisions,
    };
    let dir = TempDir::new().unwrap();
    let receipt = dir.path().join("release.json");
    let payload = attempt
        .prepare_step("merge-a", &facts, |a| persist(&receipt, a))
        .unwrap();
    let dispatch = attempt
        .mark_sent(&payload, &facts, |a| persist(&receipt, a))
        .unwrap();
    let recovered: ReleaseAttempt = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
    assert!(matches!(
        recovered.steps["merge-a"],
        StepState::SentUnknown { .. }
    ));
    assert_eq!(dispatch.payload().kind, payload.kind);
    assert_eq!(
        attempt
            .mark_sent(&payload, &facts, |_| Ok(()))
            .unwrap_err()
            .code,
        "operation_uncertain"
    );
    assert!(attempt.prepare_step("merge-a", &facts, |_| Ok(())).is_err());
    attempt
        .observe_success(
            "merge-a",
            ObservedEffect::Merge {
                repository: revisions.keys().next().unwrap().clone(),
                pull_request: 1,
                commit: Revision {
                    commit: "5".repeat(40),
                    tree: "4".repeat(40),
                },
                parent_commit: "1".repeat(40),
            },
            Utc::now(),
            |a| persist(&receipt, a),
        )
        .unwrap();
    assert!(attempt.completed());
    let mut queue = ReleaseQueue::default();
    queue.record_success(&attempt).unwrap();
}

#[test]
fn v0_t20_correction_fences_remaining_step_and_persistence_failure_grants_no_dispatch() {
    let repo = RepositoryId {
        hostname: "github.com".into(),
        id: "101".into(),
    };
    let mut attempt = restored_attempt(synthetic_plan(Uuid::new_v4(), &repo, 'a'));
    let generation = attempt.plan.generation.clone();
    let evidence = attempt.plan.evidence_token.clone();
    let revisions = BTreeMap::from([(
        repo,
        Revision {
            commit: "1".repeat(40),
            tree: "2".repeat(40),
        },
    )]);
    let mut facts = FreshReleaseFacts {
        generation: &generation,
        evidence_token: &evidence,
        owner_generation: 1,
        authority_current: true,
        pending_correction: false,
        provider_reads_complete: true,
        resource_lock_owned: true,
        credentials_usable: true,
        repository_policy_current: true,
        force_stopped: false,
        current_revisions: &revisions,
    };
    let error = attempt
        .prepare_step("merge-a", &facts, |_| {
            Err(Error::new("disk_full", "fixture write failed"))
        })
        .unwrap_err();
    assert_eq!(error.code, "disk_full");
    assert_eq!(attempt.steps["merge-a"], StepState::NotStarted);
    let payload = attempt.prepare_step("merge-a", &facts, |_| Ok(())).unwrap();
    facts.pending_correction = true;
    assert_eq!(
        attempt
            .mark_sent(&payload, &facts, |_| Ok(()))
            .unwrap_err()
            .code,
        "release_fenced"
    );
    assert!(matches!(
        attempt.steps["merge-a"],
        StepState::Prepared { .. }
    ));
}

#[test]
fn v0_t20_human_rerun_cannot_be_cancelled_as_owned() {
    let owned = OwnedRun {
        repository: RepositoryId {
            hostname: "github.com".into(),
            id: "101".into(),
        },
        run_id: 10,
        attempt: 1,
        operation_id: Uuid::new_v4(),
        actor_account_id: "selected-user".into(),
        origin_digest: d("owned-dispatch"),
    };
    let mut rerun = owned.clone();
    rerun.attempt = 2;
    assert!(owned.may_cancel(&owned));
    assert!(!owned.may_cancel(&rerun));
}

#[test]
fn v0_t30_partial_hold_blocks_overlap_but_not_disjoint_release_and_resolve_never_claims_success() {
    let repo = RepositoryId {
        hostname: "github.com".into(),
        id: "101".into(),
    };
    let unrelated = RepositoryId {
        hostname: "github.com".into(),
        id: "202".into(),
    };
    let mut attempt = restored_attempt(synthetic_plan(Uuid::new_v4(), &repo, 'a'));
    attempt.steps.insert(
        "merge-a".into(),
        StepState::Failed {
            reason: "provider terminal failure".into(),
            remote_id: Some("remote-1".into()),
            observed_at: Utc::now(),
        },
    );
    attempt.hold_reason = Some("partial failure".into());
    let mut queue = ReleaseQueue::default();
    queue.restore_hold(&attempt).unwrap();
    assert_eq!(
        queue
            .eligible(&synthetic_plan(Uuid::new_v4(), &repo, 'b'), Utc::now())
            .unwrap_err()
            .code,
        "release_hold"
    );
    queue
        .eligible(&synthetic_plan(Uuid::new_v4(), &unrelated, 'c'), Utc::now())
        .unwrap();
    let baseline = ReconciledBaseline {
        provider_reads_complete: true,
        coherent: true,
        observed_at: Utc::now(),
        observations: vec![BaselineObservation {
            resource: ResourceKey::Repository { repository: repo },
            production_commit: "1".repeat(40),
            deployed_commit: None,
            artifact_digest: None,
            provider_record: "observed-github-ref".into(),
        }],
    };
    attempt
        .resolve(
            "authenticated-human",
            "retain known failed history",
            &baseline,
            Utc::now(),
            |_| Ok(()),
        )
        .unwrap();
    assert!(attempt.resolved);
    assert!(!attempt.completed());
    assert!(queue.record_success(&attempt).is_err());
}

#[test]
fn v0_t11_accepted_staging_or_generation_prose_is_not_gate_evidence() {
    // HMAC authenticates canonical bytes and purpose, not a same-named status or
    // a text claim. This also guards cross-purpose acceptance/review substitution.
    let key = AttestationKey::new("key-v1".into(), vec![9; 32]).unwrap();
    let signed = key
        .sign("codex_review", &serde_json::json!({"a":2,"z":1}))
        .unwrap();
    key.verify("codex_review", &signed).unwrap();
    assert!(key.verify("human_acceptance", &signed).is_err());
    assert_ne!(bytes_digest(b"same patch"), bytes_digest(b"same patch\n"));
}

#[test]
fn conflicting_recovery_preserves_all_existing_resource_ownership() {
    let first = RepositoryId {
        hostname: "github.com".into(),
        id: "101".into(),
    };
    let second = RepositoryId {
        hostname: "github.com".into(),
        id: "202".into(),
    };
    let first_plan = synthetic_plan(Uuid::new_v4(), &first, 'a');
    let second_plan = synthetic_plan(Uuid::new_v4(), &second, 'b');
    let mut queue = ReleaseQueue::default();
    queue.reserve(&first_plan, Utc::now()).unwrap();
    queue.reserve(&second_plan, Utc::now()).unwrap();
    let mut changed = first_plan.clone();
    changed.resources.extend(second_plan.resources.clone());
    changed
        .deployment_free
        .extend(second_plan.deployment_free.clone());
    changed.steps.extend(second_plan.steps.clone());
    let recovered = restored_attempt(changed);
    assert_eq!(
        queue.reconcile_attempt(&recovered).unwrap_err().code,
        "ownership_conflict"
    );
    assert!(queue.owns_all(&first_plan));
    assert!(queue.owns_all(&second_plan));
    assert_eq!(
        queue
            .eligible(&synthetic_plan(Uuid::new_v4(), &first, 'c'), Utc::now())
            .unwrap_err()
            .code,
        "release_locked"
    );
}

struct GateFixture {
    _git_fixtures: Vec<Fixture>,
    key: AttestationKey,
    parent: VerifiedParent,
    children: Vec<VerifiedChild>,
    production: Vec<VerifiedProduction>,
    acceptance: VerifiedAcceptance,
    plan: ReleasePlan,
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
        } => (delta.digest().unwrap(), Some(*pull_request)),
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
            writer: "selected-user",
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
        "selected-user",
    )
    .unwrap();
    (child, review)
}

fn verify_fixture_parent(
    manifest: &ParentEvidence,
    children: &[VerifiedChild],
) -> Result<VerifiedParent> {
    verify_parent(
        manifest,
        ParentRequirements {
            work: &manifest.work,
            required_children: &manifest.required_children,
            e2e_child: manifest.e2e_child,
            criteria: &BTreeSet::from(["prepared-outcome".into()]),
            integrated_commands: &BTreeMap::from([("integrated".into(), vec!["./e2e.sh".into()])]),
            test_resources: &BTreeSet::new(),
            production_targets: &BTreeSet::new(),
            visual: false,
        },
        children,
        &publication(manifest, "linear"),
        "selected-user",
    )
}

impl GateFixture {
    fn new(repo_count: usize) -> Self {
        let key = AttestationKey::new("fixture-key-v1".into(), vec![17; 32]).unwrap();
        let parent_id = Uuid::new_v4();
        let parent_work = work(parent_id, None);
        let mut fixtures = vec![];
        let mut children = vec![];
        let mut reviews = vec![];
        let mut candidates = vec![];
        let mut baselines = vec![];
        for index in 0..repo_count {
            let mut fixture = Fixture::new(false);
            fixture.parent = parent_id;
            fixture.repository.id = (100 + index).to_string();
            let delta = fixture.delta();
            let revision = RevisionBinding {
                repository: fixture.repository.clone(),
                head: fixture.head.clone(),
                base: fixture.base.clone(),
                tested_commit: fixture.head.clone(),
                tested_tree: delta.manifest.validated_head.tree.clone(),
            };
            let (child, review) = verified_child(
                work(parent_id, Some(fixture.child)),
                revision,
                ChildOutcome::Changed {
                    pull_request: 100 + index as u64,
                    delta: Box::new(delta.manifest.clone()),
                },
                &key,
            );
            let candidate = fixture.candidate(Uuid::new_v4(), &[delta]).unwrap();
            children.push(child);
            reviews.push(review);
            baselines.push(ProductionBaseline {
                repository: fixture.repository.clone(),
                production: candidate.manifest.production.clone(),
                deployed_revision: None,
                deployed_artifact: None,
                deployment_free: true,
                successor_recovery_operation: None,
            });
            candidates.push(candidate.manifest);
            fixtures.push(fixture);
        }
        let e2e_child = Uuid::new_v4();
        let candidate = &candidates[0];
        let result = candidate.result.as_ref().unwrap();
        let revision = RevisionBinding {
            repository: candidate.repository.clone(),
            head: result.commit.clone(),
            base: candidate.production.commit.clone(),
            tested_commit: result.commit.clone(),
            tested_tree: result.tree.clone(),
        };
        let (e2e, _) = verified_child(
            work(parent_id, Some(e2e_child)),
            revision,
            ChildOutcome::NoChange {
                reason: "read-only E2E validation fulfilled its prepared responsibility".into(),
                change_gates_inapplicable: BTreeSet::from([
                    "branch".into(),
                    "pull_request".into(),
                    "staging_merge".into(),
                ]),
                exact_sha_validation: "observed exact SHA workflow".into(),
            },
            &key,
        );
        children.push(e2e);
        let mut manifest = ParentEvidence {
            schema_version: 1,
            work: parent_work.clone(),
            required_children: children
                .iter()
                .map(|c| c.evidence().work.child.unwrap())
                .collect(),
            e2e_child,
            child_results: children
                .iter()
                .map(|c| (c.evidence().work.child.unwrap(), c.digest().into()))
                .collect(),
            executive_summary:
                "Prepared outcome delivered by scoped children; isolated E2E checks passed.".into(),
            touched_components: vec!["a.txt".into()],
            fulfilled_criteria: BTreeSet::from(["prepared-outcome".into()]),
            caveats: vec!["deterministic provider fixture".into()],
            go: true,
            baselines,
            candidates,
            dependency_snapshot_digest: d("no-parent-dependencies"),
            production_targets: BTreeSet::new(),
            integrated_results: vec![],
            visual: false,
            video: None,
            publication_operation: Uuid::new_v4(),
            created_at: Utc::now(),
            supersedes: None,
        };
        manifest.integrated_results.push(IntegratedRun {
            id: "integrated".into(),
            operation_id: Uuid::new_v4(),
            e2e_child,
            candidate_vector_digest: manifest.candidate_vector_digest().unwrap(),
            command: vec!["./e2e.sh".into()],
            test_resources: BTreeSet::new(),
            started_at: manifest.created_at - Duration::seconds(2),
            completed_at: manifest.created_at - Duration::seconds(1),
            conclusion: Conclusion::Success,
            after_staging_integration: true,
            artifacts: vec![artifact_bytes(
                "fixture://integrated-output",
                b"observed integration result",
            )],
        });
        let parent = verify_fixture_parent(&manifest, &children).unwrap();
        let acceptors = BTreeSet::from(["human-linear-user".into()]);
        let signed = prepare_acceptance(
            &parent,
            AcceptanceRequest {
                supplied_token: parent.token(),
                current_work: &parent_work,
                pending_correction: false,
                actor: AcceptanceActor {
                    control_actor: "os-user-501",
                    linear_actor: "human-linear-user",
                    provider_writer: "selected-user",
                    has_accept_role: true,
                    permitted_acceptors: &acceptors,
                },
                runtime_id: Uuid::new_v4(),
                operation_id: Uuid::new_v4(),
                now: Utc::now(),
            },
            &key,
        )
        .unwrap();
        let acceptance = verify_acceptance(
            &signed,
            &publication(&signed, "linear"),
            &parent,
            &parent_work,
            false,
            &acceptors,
            &key,
        )
        .unwrap();
        let mut production = vec![];
        let mut steps = vec![];
        for (index, candidate) in parent.manifest().candidates.iter().enumerate() {
            let result = candidate.result.as_ref().unwrap();
            let revision = RevisionBinding {
                repository: candidate.repository.clone(),
                head: result.commit.clone(),
                base: candidate.production.commit.clone(),
                tested_commit: result.commit.clone(),
                tested_tree: result.tree.clone(),
            };
            let derived = derive_review(
                &parent,
                candidate,
                revision.clone(),
                200 + index as u64,
                &reviews,
                HostOperation {
                    runtime_id: Uuid::new_v4(),
                    owner_generation: 1,
                    operation_id: Uuid::new_v4(),
                },
            )
            .unwrap();
            let signed = key.sign("codex_review", &derived).unwrap();
            let published = publication(&signed, "github");
            let status = ReviewStatus {
                repository: revision.repository.clone(),
                head: revision.head.clone(),
                context: REVIEW_CONTEXT.into(),
                state: "success".into(),
                writer_account_id: "selected-user".into(),
                attestation_record_id: published.record_id.clone(),
            };
            let verified = verify_review(
                &signed,
                &published,
                Some(&status),
                &key,
                ReviewRequirements {
                    work: &parent_work,
                    revision: &revision,
                    delta_digest: &derived.delta_digest,
                    writer: "selected-user",
                },
            )
            .unwrap();
            let requirements = vec![
                CheckRequirement {
                    id: "unit".into(),
                    kind: CheckKind::ProductionCi,
                    workflow_id: Some(400),
                    command: vec![],
                },
                CheckRequirement {
                    id: "e2e".into(),
                    kind: CheckKind::ProductionCi,
                    workflow_id: Some(401),
                    command: vec![],
                },
            ];
            let mut snapshot = checks(&parent_work, &revision);
            snapshot.runs = requirements
                .iter()
                .map(|r| {
                    let mut run = snapshot.runs[1].clone();
                    run.id = r.id.clone();
                    run.kind = CheckKind::ProductionCi;
                    run.workflow_id = r.workflow_id;
                    run
                })
                .collect();
            production.push(
                verify_production(
                    ProductionRequirements {
                        parent: &parent,
                        revision: &revision,
                        checks: &requirements,
                        required_contexts: &BTreeSet::from([
                            "unit".into(),
                            "e2e".into(),
                            REVIEW_CONTEXT.into(),
                        ]),
                    },
                    &snapshot,
                    &verified,
                    &protection(&revision, "main"),
                )
                .unwrap(),
            );
            steps.push(ReleaseStep {
                id: format!("merge-{index}"),
                depends_on: BTreeSet::new(),
                kind: StepKind::Merge {
                    repository: candidate.repository.clone(),
                    pull_request: 200 + index as u64,
                    production_base: candidate.production.clone(),
                    candidate: result.clone(),
                    selected_delta_digests: candidate.ordered_delta_digests.clone(),
                },
            });
        }
        let plan = ReleasePlan {
            schema_version: 1,
            runtime_id: Uuid::new_v4(),
            operation_id: Uuid::new_v4(),
            parent_id,
            linear_workspace: parent_work.linear_workspace,
            generation: parent_work.generation,
            evidence_token: parent.token().into(),
            acceptance_digest: acceptance.digest().into(),
            carried_acceptance: None,
            resources: parent
                .manifest()
                .candidates
                .iter()
                .map(|c| ResourceKey::Repository {
                    repository: c.repository.clone(),
                })
                .collect(),
            dependencies: vec![],
            spacing_seconds: 0,
            deployment_free: parent
                .manifest()
                .candidates
                .iter()
                .map(|c| c.repository.clone())
                .collect(),
            steps,
        };
        Self {
            _git_fixtures: fixtures,
            key,
            parent,
            children,
            production,
            acceptance,
            plan,
        }
    }

    fn facts(&self) -> AdmissionFacts {
        AdmissionFacts {
            running: true,
            included: true,
            authorized: true,
            no_pending_correction: true,
            preparation_complete: true,
            provider_reads_complete: true,
            repository_policy_usable: true,
            credentials_usable: true,
            manual_mode: true,
            explicit_manual_request: Some(Uuid::new_v4()),
            actor_may_release: true,
            owner_generation: 1,
            control_generation: 1,
            required_step_ids: self.plan.steps.iter().map(|s| s.id.clone()).collect(),
        }
    }

    fn evidence(&self) -> DeliveryEvidence<'_> {
        DeliveryEvidence {
            parent: &self.parent,
            acceptance: &self.acceptance,
            carried: None,
            production: &self.production,
        }
    }
}

#[test]
fn review_regression_release_cannot_drop_an_accepted_repository_or_required_step() {
    let f = GateFixture::new(2);
    ReleaseAttempt::admit(f.plan.clone(), f.evidence(), f.facts(), Utc::now()).unwrap();
    let mut partial = f.plan.clone();
    let removed = partial.steps.pop().unwrap();
    let repo = removed.kind.repository().clone();
    partial.resources.remove(&ResourceKey::Repository {
        repository: repo.clone(),
    });
    partial.deployment_free.remove(&repo);
    assert_eq!(
        ReleaseAttempt::admit(partial.clone(), f.evidence(), f.facts(), Utc::now())
            .unwrap_err()
            .code,
        "release_incomplete"
    );
    let mut bad_contract = f.facts();
    bad_contract.required_step_ids = partial.steps.iter().map(|s| s.id.clone()).collect();
    assert_eq!(
        ReleaseAttempt::admit(partial, f.evidence(), bad_contract, Utc::now())
            .unwrap_err()
            .code,
        "scope_violation"
    );
    let mut missing_tests = f.evidence();
    missing_tests.production = &f.production[..1];
    assert!(ReleaseAttempt::admit(f.plan.clone(), missing_tests, f.facts(), Utc::now()).is_err());
}

#[test]
fn review_regression_parent_cannot_add_an_unaccounted_patch_or_omit_nochange_e2e_child() {
    let f = GateFixture::new(1);
    let mut forged = f.parent.manifest().clone();
    forged.candidates[0]
        .ordered_delta_digests
        .push(d("unapproved-extra-delta"));
    forged.candidates[0]
        .ordered_patch_digests
        .push(bytes_digest(b"unapproved extra code"));
    assert_eq!(
        verify_fixture_parent(&forged, &f.children)
            .unwrap_err()
            .code,
        "candidate_provenance"
    );
    assert_eq!(
        verify_fixture_parent(f.parent.manifest(), &f.children[..1])
            .unwrap_err()
            .code,
        "parent_incomplete"
    );
}

#[test]
fn review_regression_public_carry_record_is_not_a_verified_carry_capability() {
    let f = GateFixture::new(1);
    let mut manifest = f.parent.manifest().clone();
    manifest.created_at = Utc::now();
    manifest.caveats.push("new evidence publication".into());
    let newer = verify_fixture_parent(&manifest, &f.children).unwrap();
    let mut plan = f.plan.clone();
    plan.evidence_token = newer.token().into();
    plan.carried_acceptance = Some(CarriedAcceptance {
        schema_version: 1,
        original_reviewed_token: f.parent.token().into(),
        original_acceptance_digest: f.acceptance.digest().into(),
        current_evidence_token: newer.token().into(),
        unchanged_patch_proof: d("fabricated-proof"),
        production_validation_digest: d("fabricated-tests"),
        created_at: Utc::now(),
    });
    let evidence = DeliveryEvidence {
        parent: &newer,
        acceptance: &f.acceptance,
        carried: None,
        production: &f.production,
    };
    assert_eq!(
        ReleaseAttempt::admit(plan, evidence, f.facts(), Utc::now())
            .unwrap_err()
            .code,
        "stale_result"
    );
    assert!(
        carry_acceptance(
            &f.parent,
            &newer,
            &f.acceptance,
            &artifact_bytes("fixture://known-released-baseline", b"proof"),
            &[],
            Utc::now()
        )
        .is_err()
    );
}

#[test]
fn review_regression_resume_retains_failed_remote_step_history() {
    let f = GateFixture::new(1);
    let mut attempt =
        ReleaseAttempt::admit(f.plan.clone(), f.evidence(), f.facts(), Utc::now()).unwrap();
    let failed = StepState::Failed {
        reason: "observed provider terminal failure".into(),
        remote_id: Some("remote-action-123".into()),
        observed_at: Utc::now(),
    };
    attempt.steps.insert("merge-0".into(), failed.clone());
    attempt.hold_reason = Some("known failure".into());
    attempt
        .resume(
            "authenticated-operator",
            "provider prerequisite repaired",
            &f.parent,
            true,
            Utc::now(),
            |_| Ok(()),
        )
        .unwrap();
    assert_eq!(attempt.steps["merge-0"], StepState::NotStarted);
    assert!(attempt.step_history["merge-0"].contains(&failed));
    let persisted = serde_json::to_vec(&attempt).unwrap();
    let recovered: ReleaseAttempt = serde_json::from_slice(&persisted).unwrap();
    assert!(recovered.step_history["merge-0"].contains(&failed));
}

#[test]
fn v0_t19_stale_token_pending_correction_and_wrong_acceptor_cannot_approve() {
    let f = GateFixture::new(1);
    let acceptors = BTreeSet::from(["human-linear-user".into()]);
    for (token, pending, actor) in [
        ("stale", false, "human-linear-user"),
        (f.parent.token(), true, "human-linear-user"),
        (f.parent.token(), false, "not-allowed"),
    ] {
        assert!(
            prepare_acceptance(
                &f.parent,
                AcceptanceRequest {
                    supplied_token: token,
                    current_work: &f.parent.manifest().work,
                    pending_correction: pending,
                    actor: AcceptanceActor {
                        control_actor: "os-user-501",
                        linear_actor: actor,
                        provider_writer: "selected-user",
                        has_accept_role: true,
                        permitted_acceptors: &acceptors
                    },
                    runtime_id: Uuid::new_v4(),
                    operation_id: Uuid::new_v4(),
                    now: Utc::now()
                },
                &f.key
            )
            .is_err()
        );
    }
    let mut facts = f.facts();
    facts.running = false;
    assert_eq!(
        ReleaseAttempt::admit(f.plan.clone(), f.evidence(), facts, Utc::now())
            .unwrap_err()
            .code,
        "release_ineligible"
    );
}
