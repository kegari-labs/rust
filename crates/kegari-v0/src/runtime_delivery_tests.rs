use super::*;
use crate::app::ConfigurationControl;
use crate::corrections::wire_fixture as wire;
use crate::digest::canonical_digest;
use crate::platform::{ProcessIdentity, ProcessObservation, ProcessProbe};
use crate::reconcile::DependencyOutcomes;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
// This deterministic process identity fixture does not qualify native ownership
// or permit any child process launch. These tests exercise read-only composition.
struct IdentityFixture(ProcessIdentity);
impl ProcessProbe for IdentityFixture {
    fn current(&self) -> Result<ProcessIdentity> {
        Ok(self.0.clone())
    }
    fn observe(&self, expected: &ProcessIdentity) -> Result<ProcessObservation> {
        Ok(if expected == &self.0 {
            ProcessObservation::Running
        } else {
            ProcessObservation::DifferentIdentity
        })
    }
    fn matching_executable(&self, path: &Path) -> Result<Vec<ProcessIdentity>> {
        Ok(if path == self.0.executable {
            vec![self.0.clone()]
        } else {
            vec![]
        })
    }
}
pub(super) async fn fixture() -> (
    tempfile::TempDir,
    crate::runtime_fixture::RuntimeFixture,
    Arc<RuntimeDelivery>,
) {
    fixture_authorization(false).await
}
pub(super) async fn fixture_authorization(
    authorized: bool,
) -> (
    tempfile::TempDir,
    crate::runtime_fixture::RuntimeFixture,
    Arc<RuntimeDelivery>,
) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let executable = resolve_git().unwrap();
    let tools = root.join("tools");
    fs::create_dir(&tools).unwrap();
    fs::set_permissions(&tools, fs::Permissions::from_mode(0o700)).unwrap();
    let git = tools.join("git");
    fs::copy(executable, &git).unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o555)).unwrap();
    let source = root.join("source");
    fs::create_dir(&source).unwrap();
    fs::create_dir(source.join(".git")).unwrap();
    let workflow = root.join("WORKFLOW.md");
    fs::write(&workflow,"---\nchecks:\n  local:\n    local:\n      argv: ['/usr/bin/true']\n      result_artifact: '{{ paths.scratch }}/local.json'\n  integrated:\n    e2e:\n      argv: ['/usr/bin/true']\n      result_artifact: '{{ paths.scratch }}/e2e.json'\n---\nExplicit synthetic provider fixture\n").unwrap();
    let mut configuration = wire::CacheConfiguration(wire::config()).snapshot().unwrap();
    let selected = configuration.workspaces.get_mut("example").unwrap();
    selected.workflow = workflow.to_str().unwrap().into();
    selected.repositories.get_mut("42").unwrap().source = source.to_str().unwrap().into();
    let data = wire::Fixture::new();
    let identity = ProcessIdentity {
        pid: std::process::id(),
        parent_pid: unsafe { libc::getppid() } as u32,
        process_group: unsafe { libc::getpgrp() } as u32,
        uid: unsafe { libc::geteuid() },
        boot_id: Uuid::new_v4(),
        started_seconds: 1,
        started_microseconds: 0,
        executable: std::env::current_exe().unwrap().canonicalize().unwrap(),
    };
    let built = crate::runtime_fixture::build(crate::runtime_fixture::FixtureInputs {
        root: root.clone(),
        configuration,
        inputs: Arc::new(wire::CacheInputs(data.clone())),
        secrets: Arc::new(wire::CacheSecrets),
        key: Arc::new(crate::evidence::AttestationKey::new("fixture".into(), vec![7; 32]).unwrap()),
        probe: Arc::new(IdentityFixture(identity)),
        git: git.clone(),
    })
    .await
    .unwrap();
    let projection = built.runtime.observations.refresh().await.unwrap();
    if authorized {
        let parent = projection.parent(wire::id(2), wire::id(100)).unwrap();
        let signed = built
            .runtime
            .services
            .key
            .sign(
                "parent_authorization",
                parent.authorization_candidate.as_ref().unwrap(),
            )
            .unwrap();
        let body =
            crate::evidence::encode_signed_publication("parent_authorization", &signed).unwrap();
        data.lock().unwrap().comments.entry(wire::id(100)).or_default().push(json!({"id":wire::id(950),"body":body,"createdAt":"2026-01-04T00:00:00Z","updatedAt":"2026-01-04T00:00:00Z","editedAt":null,"user":{"id":wire::id(5)},"onBehalfOf":null,"botActor":null,"issue":{"id":wire::id(100)},"url":"https://linear.invalid/comment/950"}));
        built.runtime.observations.refresh().await.unwrap();
    }
    let delivery =
        RuntimeDelivery::with_fixture(built.runtime.clone(), built.processes.clone(), git).unwrap();
    (temp, built, delivery)
}
#[tokio::test]
async fn canonical_repository_id_builds_current_workspace_without_any_process_or_model_launch() {
    let (_temp, fixture, delivery) = fixture().await;
    let projection = fixture.runtime.observations.snapshot().unwrap();
    let workspace = delivery.workspace(&projection, wire::id(2)).unwrap();
    assert_eq!(workspace.contracts[0].repository.id, "42");
    assert_eq!(workspace.contracts[0].provider.owner, "fixture");
    assert!(
        fixture
            .runtime
            .control
            .snapshot()
            .unwrap()
            .active_operations
            .is_empty()
    );
    assert!(
        fixture
            .runtime
            .control
            .snapshot()
            .unwrap()
            .pending_local_processes
            .is_empty()
    );
    assert_eq!(
        fixture
            .configuration
            .snapshot()
            .unwrap()
            .revision()
            .unwrap(),
        projection.config_revision
    );
}
#[tokio::test]
async fn unprepared_or_revoked_parent_refresh_never_admits_workers_or_preserves_old_readiness() {
    let (_temp, fixture, delivery) = fixture().await;
    let projection = fixture.runtime.observations.snapshot().unwrap();
    let parent = projection.parent(wire::id(2), wire::id(100)).unwrap();
    delivery.refresh(&projection).await.unwrap();
    assert!(
        fixture
            .runtime
            .current_delivery(&parent.key)
            .unwrap()
            .verified_parent()
            .is_none()
    );
    assert!(fixture.runtime.control.operations().unwrap().is_empty());
    fixture
        .configuration
        .fences
        .lock()
        .unwrap()
        .workspaces
        .insert(wire::id(2));
    assert_eq!(
        delivery
            .workspace(&projection, wire::id(2))
            .err()
            .unwrap()
            .code,
        "permission_fenced"
    );
    delivery.refresh(&projection).await.unwrap();
    assert!(
        fixture
            .runtime
            .current_delivery(&parent.key)
            .unwrap()
            .verified_acceptance()
            .is_none()
    );
    assert!(
        fixture
            .runtime
            .control
            .snapshot()
            .unwrap()
            .active_operations
            .is_empty()
    );
}

#[test]
fn phase_uuid_uses_versioned_digest_payload_and_is_domain_separated() {
    let runtime = Uuid::new_v4();
    let source = Uuid::new_v4();
    let first = delivery_id(runtime, source, "manual-release").unwrap();
    assert_eq!(
        first,
        delivery_id(runtime, source, "manual-release").unwrap()
    );
    assert_ne!(
        first,
        delivery_id(runtime, source, "another-phase").unwrap()
    );
    assert_ne!(
        first,
        delivery_id(Uuid::new_v4(), source, "manual-release").unwrap()
    );
    assert_eq!(first.get_version_num(), 8);
}

pub(super) fn start_control(runtime: &RuntimeCoordinator) -> crate::control::Lease {
    let id = runtime.configuration.snapshot().unwrap().runtime.id;
    let state = runtime.control.snapshot().unwrap();
    runtime
        .control
        .lifecycle(
            crate::control::MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: id,
                actor: crate::control::Actor {
                    identity: "fixture-operator".into(),
                    principal: None,
                    roles: ["lifecycle".into()].into(),
                },
                kind: "lifecycle".into(),
                canonical_targets: vec![id.to_string()],
                expected: BTreeMap::new(),
                payload: json!(null),
            },
            crate::control::Lifecycle::Start,
            state.intent.generation,
        )
        .unwrap();
    let lease = crate::control::Lease {
        runtime_id: id,
        owner_generation: 1,
        owner_id: Uuid::new_v4(),
    };
    runtime.control.attach_reconciled(lease.clone()).unwrap();
    lease
}
fn fixture_run(workspace: Uuid, phase: crate::scheduler::PhaseKey) -> crate::control::RunReference {
    crate::control::RunReference {
        workspace,
        account: "codex-fixture".into(),
        normalized_state: Some("in progress".into()),
        phase,
        profile_digest: canonical_digest(1, &"fixture-profile").unwrap(),
        instruction_digest: canonical_digest(1, &"fixture-instructions").unwrap(),
        max_attempts: 3,
        max_turns: 4,
        max_review_rounds: 2,
        attempt: 1,
    }
}
#[tokio::test]
async fn rejected_launch_intent_retries_only_when_preparation_and_run_have_no_effects() {
    let (_temp, fixture, service) = fixture().await;
    fixture
        .configuration
        .config
        .lock()
        .unwrap()
        .runtime
        .max_agents = 1;
    let lease = start_control(&fixture.runtime);
    let phase = crate::scheduler::PhaseKey {
        work: "fixture-occupied-work".into(),
        generation: canonical_digest(1, &"generation").unwrap(),
        phase: "implementation".into(),
        retry_grant: None,
    };
    let occupied = Uuid::new_v4();
    fixture
        .runtime
        .control
        .admit_run(
            &lease,
            crate::control::RunAdmission {
                request_id: occupied,
                work_key: phase.work.clone(),
                scope_digest: canonical_digest(1, &"occupied").unwrap(),
                reference: fixture_run(wire::id(2), phase.clone()),
            },
            || Ok(()),
        )
        .unwrap();
    let worker = Uuid::new_v4();
    service
        .records
        .write("fixture-launch-intent.json", &json!({"operation":worker}))
        .unwrap();
    let next = crate::scheduler::PhaseKey {
        work: "fixture-next-work".into(),
        phase: "child_validation".into(),
        ..phase
    };
    let before = fixture.runtime.control.operations().unwrap().len();
    assert_eq!(
        fixture
            .runtime
            .check_worker_capacity(wire::id(2), "codex-fixture", &next, "in progress")
            .unwrap_err()
            .code,
        "capacity_wait"
    );
    assert!(!service.worker_launch_admitted(worker).unwrap());
    assert_eq!(fixture.runtime.control.operations().unwrap().len(), before);
    assert!(fixture.processes.operations().unwrap().is_empty());
    fixture
        .runtime
        .control
        .retire_local(occupied, json!({"fixture":"no process was launched"}))
        .unwrap();
    fixture
        .runtime
        .check_worker_capacity(wire::id(2), "codex-fixture", &next, "in progress")
        .unwrap();
    // The same intent can now enter exactly one actual durable admission. This
    // fixture deliberately does not run a model or manufacture a completion.
    fixture
        .runtime
        .control
        .admit_run(
            &lease,
            crate::control::RunAdmission {
                request_id: worker,
                work_key: next.work.clone(),
                scope_digest: canonical_digest(1, &"next").unwrap(),
                reference: fixture_run(wire::id(2), next),
            },
            || Ok(()),
        )
        .unwrap();
    assert!(service.worker_launch_admitted(worker).unwrap());
    assert_eq!(
        fixture
            .runtime
            .control
            .operations()
            .unwrap()
            .iter()
            .filter(|r| r.operation_id == worker)
            .count(),
        1
    );
    let unknown = Uuid::new_v4();
    let preparation = crate::runtime_workers::preparation_operation(unknown).unwrap();
    fixture
        .runtime
        .control
        .admit(
            &lease,
            preparation,
            "fixture-other-work".into(),
            canonical_digest(1, &"prep").unwrap(),
            || Ok(()),
        )
        .unwrap();
    assert_eq!(
        service.worker_launch_admitted(unknown).unwrap_err().code,
        "reconciliation_required"
    );
    assert_eq!(
        preparation,
        crate::runtime_workers::preparation_operation(unknown).unwrap()
    );
    assert_ne!(preparation, worker);
}

#[tokio::test]
async fn admitted_delivery_gate_survives_harmless_reload_but_not_permission_revocation() {
    let (_temp, fixture, service) = fixture_authorization(true).await;
    let projection = fixture.runtime.observations.snapshot().unwrap();
    let parent = projection.parent(wire::id(2), wire::id(100)).unwrap();
    let expected = parent
        .fresh_gate(
            Some(wire::id(101)),
            GatePurpose::NewOperation,
            &DependencyOutcomes::default(),
            Utc::now(),
            Duration::seconds(30),
        )
        .unwrap();
    let _active = fixture
        .runtime
        .observations
        .capture_active(
            expected.workspace_id(),
            expected.config_revision(),
            expected.workflow_digest(),
        )
        .unwrap();
    fixture
        .configuration
        .config
        .lock()
        .unwrap()
        .runtime
        .max_agents = 3;
    let workflow = fixture.configuration.snapshot().unwrap().workspaces["example"]
        .workflow
        .clone();
    let mut contents = fs::read_to_string(&workflow).unwrap();
    contents.push_str("\nNew instructions apply only to new admissions.\n");
    fs::write(workflow, contents).unwrap();
    fixture.runtime.observations.refresh().await.unwrap();
    assert_eq!(
        service.current_gate()(&expected).unwrap_err().code,
        "stale_config"
    );
    service.retained_gate()(&expected).unwrap();
    fixture
        .configuration
        .fences
        .lock()
        .unwrap()
        .workspaces
        .insert(wire::id(2));
    assert_eq!(
        service.retained_gate()(&expected).unwrap_err().code,
        "permission_fenced"
    );
    assert!(fixture.runtime.control.operations().unwrap().is_empty());
}
