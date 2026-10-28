//! Deadline, output and journal integration stays with the native command owner.
use super::*;
use kegari_digest::bytes_digest;
use kegari_execution::{BoundManagedExecution, ExecutionRole, IsolationPlan, PathGrant};
use kegari_types::error::{Error, Result};
use std::path::Path;
use std::{collections::BTreeMap, fs};

struct Fixture {
    _temp: tempfile::TempDir,
    isolation: ValidatedIsolation,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let execution_root = root.join("owned");
        let repository = execution_root.join("repository");
        let scratch = execution_root.join("scratch");
        let external = root.join("external");
        let resource = root.join("resource");
        for directory in [&repository, &scratch, &external, &resource] {
            fs::create_dir_all(directory).unwrap();
        }
        fs::create_dir_all(repository.join("nested/deeper")).unwrap();
        fs::write(resource.join("input"), b"original").unwrap();
        let mut plan = IsolationPlan {
            schema_version: 1,
            role: ExecutionRole::Child,
            execution_root,
            cwd: repository.clone(),
            repository,
            scratch,
            read_roots: vec![PathGrant {
                path: resource,
                directory: true,
            }],
            source_writes: vec![],
            resource_writes: vec![],
            forbidden_roots: vec![],
            managed_execution: None,
        };
        plan.managed_execution =
            Some(BoundManagedExecution::automatic(&plan, crate::BUILD_ID).unwrap());
        Self {
            _temp: temp,
            isolation: plan.validate(crate::BUILD_ID).unwrap(),
        }
    }
}

#[test]
fn expired_original_deadline_refuses_before_probe_journal_or_unqualified_start() {
    use crate::control::ControlHost;
    use crate::control::Lifecycle;
    use crate::control::MutationRequest;
    use crate::process_records::ProcessJournal;
    use kegari_control_api::identity::Actor;
    use kegari_platform::{ProcessIdentity, ProcessObservation, ProcessProbe};
    use kegari_storage::PrivateDir;
    use kegari_types::control::Lease;
    use std::{
        collections::BTreeSet,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Instant,
    };
    use uuid::Uuid;
    struct NoProbe(Arc<AtomicUsize>);
    impl ProcessProbe for NoProbe {
        fn current(&self) -> Result<ProcessIdentity> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(Error::invalid("component forbids process probes"))
        }
        fn observe(&self, _: &ProcessIdentity) -> Result<ProcessObservation> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(Error::invalid("component forbids process probes"))
        }
        fn matching_executable(&self, _: &Path) -> Result<Vec<ProcessIdentity>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(Error::invalid("component forbids process probes"))
        }
    }
    let fixture = Fixture::new();
    let root = fixture._temp.path().canonicalize().unwrap();
    let runtime = Uuid::new_v4();
    let control = ControlHost::setup(
        PrivateDir::create(&root.join("control")).unwrap(),
        runtime,
        Uuid::new_v4(),
    )
    .unwrap();
    control
        .lifecycle(
            MutationRequest {
                request_id: Uuid::new_v4(),
                runtime_id: runtime,
                actor: Actor {
                    identity: "component operator".into(),
                    roles: BTreeSet::from(["lifecycle".into()]),
                    principal: None,
                },
                kind: "start".into(),
                canonical_targets: vec![runtime.to_string()],
                expected: BTreeMap::new(),
                payload: serde_json::Value::Null,
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
    control.attach_reconciled(lease.clone()).unwrap();
    let permit = control
        .admit(
            &lease,
            Uuid::new_v4(),
            "component invocation".into(),
            bytes_digest(b"component scope"),
            || Ok(()),
        )
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let journal_path = root.join("processes");
    let journal = Arc::new(
        ProcessJournal::with_probe(
            PrivateDir::create(&journal_path).unwrap(),
            runtime,
            Arc::new(NoProbe(calls.clone())),
        )
        .unwrap(),
    );
    let deadline = Instant::now();
    let result = start_journaled_command_until(
        &fixture.isolation,
        &["never-launched".into()],
        InvocationCommandLimits {
            timeout: Duration::from_secs(30),
            deadline,
            output_limit: 1024,
        },
        &[],
        journal.clone(),
        permit.clone(),
    );
    assert!(matches!(result, Err(ref error) if error.code == "timeout"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read_dir(journal_path).unwrap().count(), 0);
    // The same elapsed absolute cutoff cannot become a fresh relative budget.
    assert_eq!(
        start_journaled_command_until(
            &fixture.isolation,
            &["never-launched".into()],
            InvocationCommandLimits {
                timeout: Duration::from_secs(30),
                deadline,
                output_limit: 1024,
            },
            &[],
            journal,
            permit,
        )
        .err()
        .unwrap()
        .code,
        "timeout"
    );
}
