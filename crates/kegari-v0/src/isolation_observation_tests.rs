//! Filesystem/serialization components only: no command, sandbox or provider is
//! started. Executable byte stubs are observations, never native qualification.
use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

struct Fixture {
    _temp: tempfile::TempDir,
    isolation: ValidatedIsolation,
    external: PathBuf,
    resource: PathBuf,
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
                path: resource.clone(),
                directory: true,
            }],
            source_writes: vec![],
            resource_writes: vec![],
            forbidden_roots: vec![],
            managed_execution: None,
        };
        plan.managed_execution = Some(BoundManagedExecution::automatic(&plan).unwrap());
        Self {
            _temp: temp,
            isolation: plan.validate().unwrap(),
            external,
            resource,
        }
    }
    fn program(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.isolation.plan.repository.join(relative);
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
}

#[test]
fn cwd_derivation_retains_one_original_boundary_across_dot_and_chained_changes() {
    let fixture = Fixture::new();
    let original = &fixture.isolation;
    let dot = original.with_cwd(Path::new(".")).unwrap();
    assert_eq!(dot.digest(), original.digest());
    dot.verify_cwd_derivation(original).unwrap();
    let nested = dot.with_cwd(Path::new("nested")).unwrap();
    let deeper = nested.with_cwd(Path::new("deeper")).unwrap();
    deeper.verify_cwd_derivation(original).unwrap();
    deeper.verify_cwd_derivation(&nested).unwrap();
    assert_eq!(
        deeper.plan.cwd,
        original.plan.repository.join("nested/deeper")
    );
    assert_eq!(deeper.original_digest(), original.digest());
    assert!(std::sync::Arc::ptr_eq(
        dot.origin.as_ref().unwrap(),
        deeper.origin.as_ref().unwrap()
    ));
    let mut original_fields = deeper.plan.clone();
    original_fields.cwd = original.plan.cwd.clone();
    assert_eq!(
        canonical_digest(1, &original_fields).unwrap(),
        original.digest()
    );
    assert_eq!(
        serde_json::to_value(&original.plan.managed_execution).unwrap(),
        serde_json::to_value(&deeper.plan.managed_execution).unwrap()
    );
}

#[test]
fn cwd_derivation_canonicalizes_internal_alias_and_refuses_escape_or_invalid_input() {
    let fixture = Fixture::new();
    let original = &fixture.isolation;
    symlink(
        original.plan.repository.join("nested"),
        original.plan.repository.join("inside"),
    )
    .unwrap();
    symlink(&fixture.external, original.plan.repository.join("outside")).unwrap();
    assert_eq!(
        original.with_cwd(Path::new("inside")).unwrap().plan.cwd,
        original.plan.repository.join("nested")
    );
    for path in [
        Path::new("outside"),
        Path::new(".."),
        fixture.external.as_path(),
        original.plan.scratch.as_path(),
    ] {
        assert_eq!(original.with_cwd(path).unwrap_err().code, "scope_violation");
    }
    for path in [
        PathBuf::new(),
        PathBuf::from("\0"),
        PathBuf::from("a".repeat(MAX_COMMAND_CWD_BYTES + 1)),
    ] {
        assert_eq!(original.with_cwd(&path).unwrap_err().code, "invalid_config");
    }
    fs::write(original.plan.repository.join("file"), b"not a directory").unwrap();
    assert_eq!(
        original.with_cwd(Path::new("file")).unwrap_err().code,
        "scope_violation"
    );
}

#[test]
fn forged_cwd_derivation_cannot_change_grants_or_reset_original_identity() {
    let fixture = Fixture::new();
    let original = &fixture.isolation;
    let mut derived = original.with_cwd(Path::new("nested")).unwrap();
    // An otherwise valid, newly hashed source grant still violates the original
    // retained boundary. Public callers cannot mutate these private fields.
    derived.plan.source_writes.push(PathGrant {
        path: derived.plan.cwd.clone(),
        directory: true,
    });
    derived.digest = canonical_digest(1, &derived.plan).unwrap();
    assert_eq!(derived.revalidate().unwrap_err().code, "scope_violation");
    assert_eq!(
        derived.verify_cwd_derivation(original).unwrap_err().code,
        "scope_violation"
    );
    let derived = original.with_cwd(Path::new("nested")).unwrap();
    let reconstructed_without_origin = derived.plan.validate().unwrap();
    assert_eq!(
        reconstructed_without_origin
            .verify_cwd_derivation(original)
            .unwrap_err()
            .code,
        "scope_violation"
    );
    let other = Fixture::new();
    assert_eq!(
        derived
            .verify_cwd_derivation(&other.isolation)
            .unwrap_err()
            .code,
        "scope_violation"
    );
}

#[test]
fn cwd_derivation_does_not_recapture_changed_immutable_resource_contents() {
    let fixture = Fixture::new();
    fs::write(fixture.resource.join("input"), b"substitution").unwrap();
    let derived = fixture.isolation.with_cwd(Path::new("nested")).unwrap();
    derived.verify_cwd_derivation(&fixture.isolation).unwrap();
    assert!(derived.revalidate_resource_contents().is_err());
}

#[test]
fn observation_input_bounds_refuse_before_executable_lookup() {
    let fixture = Fixture::new();
    let invalid = [
        vec![],
        vec![String::new()],
        vec!["nonexistent".into(), "nul\0argument".into()],
        vec!["a".repeat(MAX_COMMAND_ARG_BYTES + 1)],
        vec!["x".into(); MAX_COMMAND_ARGS + 1],
        vec!["a".repeat(MAX_COMMAND_ARG_BYTES); 17],
    ];
    for argv in invalid {
        assert_eq!(
            observe_command(&fixture.isolation, &argv).unwrap_err().code,
            "invalid_config"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn command_observation_retains_requested_alias_exact_environment_and_launch_projection() {
    let fixture = Fixture::new();
    let target = fixture.program("actual", b"#!/bin/sh\nexit 0\n");
    let alias = fixture.isolation.plan.repository.join("alias");
    symlink(&target, &alias).unwrap();
    let isolation = fixture.isolation.with_cwd(Path::new("nested")).unwrap();
    let argv = vec![
        "../alias".into(),
        "literal spaces; $HOME".into(),
        String::new(),
    ];
    let observation = observe_command(&isolation, &argv).unwrap();
    assert_eq!(observation.requested_argv, argv);
    assert_eq!(observation.selected_executable, alias);
    assert_eq!(observation.canonical_executable.path, target);
    assert_eq!(
        observation.canonical_executable.digest,
        bytes_digest(b"#!/bin/sh\nexit 0\n")
    );
    assert_eq!(observation.cwd, isolation.plan.cwd);
    assert_eq!(
        observation.environment,
        BTreeMap::from([
            ("PATH".into(), isolation.command_path().unwrap()),
            (
                "HOME".into(),
                isolation.plan.scratch.to_str().unwrap().into()
            ),
            (
                "TMPDIR".into(),
                isolation.plan.scratch.to_str().unwrap().into()
            ),
            ("LANG".into(), "C.UTF-8".into()),
        ])
    );
    assert_eq!(
        observation.original_isolation_digest,
        fixture.isolation.digest()
    );
    assert_eq!(observation.isolation_digest, isolation.digest());
    assert_eq!(
        observation.launch_identity(),
        command_launch_identity(&isolation, &argv).unwrap()
    );
    let rendered = render_command(&isolation, &argv).unwrap();
    assert_eq!(
        observation,
        observe_rendered_command(&isolation, &rendered).unwrap()
    );
    assert_eq!(
        repository_command_prerequisite(&isolation)
            .unwrap_err()
            .code,
        "managed_profile_unqualified"
    );
    let json = serde_json::to_value(&observation).unwrap();
    assert_eq!(
        serde_json::from_value::<CommandObservation>(json.clone()).unwrap(),
        observation
    );
    let mut unsupported = json.clone();
    unsupported["schema_version"] = serde_json::json!(2);
    assert!(serde_json::from_value::<CommandObservation>(unsupported).is_err());
    let mut extra = json;
    extra["authority"] = serde_json::json!(true);
    assert!(serde_json::from_value::<CommandObservation>(extra).is_err());
}

#[cfg(target_os = "macos")]
#[test]
fn observation_changes_on_actual_target_bytes_arguments_or_cwd_without_claiming_authority() {
    let fixture = Fixture::new();
    let target = fixture.program("tool", b"original executable bytes");
    let argv = vec![target.to_str().unwrap().to_owned()];
    let first = observe_command(&fixture.isolation, &argv).unwrap();
    fs::write(&target, b"changed executable bytes").unwrap();
    let changed = observe_command(&fixture.isolation, &argv).unwrap();
    assert_ne!(first.canonical_executable, changed.canonical_executable);
    assert_eq!(
        first.original_isolation_digest,
        changed.original_isolation_digest
    );
    let arguments =
        observe_command(&fixture.isolation, &[argv[0].clone(), "argument".into()]).unwrap();
    assert_ne!(changed.launch_digest, arguments.launch_digest);
    let nested = fixture.isolation.with_cwd(Path::new("nested")).unwrap();
    let moved = observe_command(&nested, &argv).unwrap();
    assert_ne!(changed.isolation_digest, moved.isolation_digest);
    assert_ne!(changed.cwd, moved.cwd);
    assert_eq!(
        changed.original_isolation_digest,
        moved.original_isolation_digest
    );
}

#[test]
fn expired_original_deadline_refuses_before_probe_journal_or_unqualified_start() {
    use crate::{
        control::{Actor, ControlHost, Lease, Lifecycle, MutationRequest},
        platform::{ProcessIdentity, ProcessObservation, ProcessProbe},
        process_records::ProcessJournal,
        storage::PrivateDir,
    };
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
        journal,
        permit,
    );
    assert!(matches!(result, Err(ref error) if error.code == "timeout"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read_dir(journal_path).unwrap().count(), 0);
    // The same elapsed absolute cutoff cannot become a fresh relative budget.
    assert_eq!(
        remaining_bundle_timeout(Duration::from_secs(30), Instant::now(), Some(deadline))
            .unwrap_err()
            .code,
        "timeout"
    );
}

#[tokio::test]
async fn invocation_streams_share_one_raw_allowance_and_both_reach_eof() {
    let budget = std::sync::Arc::new(CommandOutputBudget::new(8));
    let mut out = b"sixout".as_slice();
    let mut err = b"sixerr".as_slice();
    let (out_result, err_result) = tokio::join!(
        bounded_output(&mut out, 8, Some(budget.clone())),
        bounded_output(&mut err, 8, Some(budget.clone())),
    );
    let (out_bytes, out_truncated) = out_result.unwrap();
    let (err_bytes, err_truncated) = err_result.unwrap();
    assert_eq!(out_bytes.len() + err_bytes.len(), 8);
    assert!(out_truncated || err_truncated);
    assert!(out.is_empty() && err.is_empty());
    assert_eq!(budget.take(1), 0);
    // The old unshared collector contract remains per-stream.
    let (out, err) = tokio::join!(
        bounded_output(b"12345678".as_slice(), 8, None),
        bounded_output(b"12345678".as_slice(), 8, None),
    );
    assert_eq!(out.unwrap().0.len() + err.unwrap().0.len(), 16);
}

#[test]
fn invocation_rendered_output_bounds_utf8_and_redaction_without_exposing_matched_secrets() {
    let (out, err, truncated) = rendered_command_output(&[0xff; 4], b"ok", &[], Some(4)).unwrap();
    assert!(truncated);
    assert_eq!(out, "\u{fffd}");
    assert_eq!(err, "o");
    assert_eq!(out.len() + err.len(), 4);
    let secret = "SECRET".to_owned();
    let (out, err, truncated) =
        rendered_command_output(b"SECRET", b"SECRET", std::slice::from_ref(&secret), Some(8))
            .unwrap();
    assert!(truncated);
    assert!(out.len() + err.len() <= 8);
    assert!(!out.contains(&secret) && !err.contains(&secret));
    assert_eq!(out, "[REDACTE");
    assert!(err.is_empty());
    let (out, err, truncated) =
        rendered_command_output(b"SECRET", b"SECRET", &[secret], None).unwrap();
    assert_eq!(
        (out.as_str(), err.as_str(), truncated),
        ("[REDACTED]", "[REDACTED]", false)
    );
    assert_eq!(
        rendered_command_output(
            b"anything",
            b"sandbox-exec: sandbox_apply failed",
            &[],
            Some(0)
        )
        .unwrap_err()
        .code,
        "isolation_unavailable"
    );
}

#[test]
fn environment_observation_uses_exact_command_environment_without_launch_or_lookup() {
    let fixture = Fixture::new();
    let marker = fixture.isolation.plan.scratch.join("must-not-be-created");
    let program = fixture.program(
        "unfamiliar-environment-fixture",
        format!("#!/bin/sh\nprintf launched > '{}'\n", marker.display()).as_bytes(),
    );
    let original = canonical_digest(1, fixture.isolation.plan()).unwrap();
    let observation = observe_environment(&fixture.isolation).unwrap();
    let rendered = render_command(
        &fixture.isolation,
        &[program.to_str().unwrap().to_owned(), "literal $HOME".into()],
    )
    .unwrap();
    assert_eq!(observation.environment, rendered.environment);
    assert_eq!(observation.environment.len(), 4);
    assert_eq!(
        observation
            .environment
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec!["HOME", "LANG", "PATH", "TMPDIR"]
    );
    assert_eq!(
        observation.environment["HOME"],
        fixture.isolation.plan.scratch.to_str().unwrap()
    );
    assert_eq!(
        observation.environment["TMPDIR"],
        observation.environment["HOME"]
    );
    assert_eq!(observation.environment["LANG"], "C.UTF-8");
    assert_eq!(
        observation.search_path,
        observation.environment["PATH"]
            .split(':')
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    );
    assert_eq!(observation.limits.max_arguments, MAX_COMMAND_ARGS);
    assert_eq!(observation.limits.max_argument_bytes, MAX_COMMAND_ARG_BYTES);
    assert_eq!(observation.limits.max_argv_bytes, MAX_COMMAND_BYTES);
    assert_eq!(observation.limits.max_cwd_bytes, MAX_COMMAND_CWD_BYTES);
    assert_eq!(observation.limits.network, EnvironmentNetwork::Denied);
    assert_eq!(
        observation.limits.detached_descendants,
        EnvironmentDetachedDescendants::OutsideGuarantee
    );
    assert_eq!(
        observation.native_prerequisite.as_ref().unwrap().code,
        "managed_profile_unqualified"
    );
    assert_eq!(
        canonical_digest(1, fixture.isolation.plan()).unwrap(),
        original
    );
    assert!(!marker.exists());
    // The observation does not enumerate a software catalog: a program can be
    // removed without making observation itself require executable resolution.
    fs::remove_file(program).unwrap();
    assert_eq!(
        observe_environment(&fixture.isolation).unwrap(),
        observation
    );
}

#[test]
fn environment_observation_retains_original_cwd_and_domain_across_derivation() {
    let fixture = Fixture::new();
    let original = observe_environment(&fixture.isolation).unwrap();
    let nested = fixture
        .isolation
        .with_cwd(Path::new("nested/deeper"))
        .unwrap();
    let derived = observe_environment(&nested).unwrap();
    assert_eq!(derived.original_cwd, original.cwd);
    assert_eq!(
        derived.cwd,
        fixture.isolation.plan.repository.join("nested/deeper")
    );
    assert_eq!(derived.original_isolation_digest, original.isolation_digest);
    assert_ne!(derived.isolation_digest, original.isolation_digest);
    assert_eq!(derived.original_domain_digest, original.domain_digest);
    assert_eq!(derived.domain_digest, original.domain_digest);
    assert_eq!(
        derived.original_backend_policy_digest,
        original.backend_policy_digest
    );
    assert_eq!(
        derived.backend_policy_digest,
        original.backend_policy_digest
    );
    assert_eq!(derived.search_path, original.search_path);
    assert_eq!(derived.environment, original.environment);
    assert_eq!(derived.role, ExecutionRole::Child);
}

#[test]
fn environment_observation_missing_domain_is_unavailable_and_changed_boundary_is_error() {
    let fixture = Fixture::new();
    for role in [
        ExecutionRole::Coordination,
        ExecutionRole::IndependentReview,
        ExecutionRole::Review,
        ExecutionRole::Child,
    ] {
        let mut plan = fixture.isolation.plan.clone();
        plan.role = role;
        plan.managed_execution = None;
        let isolation = plan.validate().unwrap();
        assert_eq!(
            observe_environment(&isolation).unwrap_err().code,
            "execution_environment_unavailable"
        );
    }
    let mut forged = fixture.isolation.with_cwd(Path::new("nested")).unwrap();
    forged.plan.source_writes.push(PathGrant {
        path: forged.plan.cwd.clone(),
        directory: true,
    });
    forged.digest = canonical_digest(1, &forged.plan).unwrap();
    assert_eq!(
        observe_environment(&forged).unwrap_err().code,
        "scope_violation"
    );
    // Even identical directory contents cannot adopt a replacement root.
    let original = fixture.resource.with_file_name("original-resource");
    fs::rename(&fixture.resource, &original).unwrap();
    fs::create_dir(&fixture.resource).unwrap();
    fs::write(fixture.resource.join("input"), b"original").unwrap();
    assert_eq!(
        observe_environment(&fixture.isolation).unwrap_err().code,
        "identity_mismatch"
    );
}

#[test]
fn environment_observation_does_not_recapture_or_attest_resource_contents() {
    let fixture = Fixture::new();
    let original = observe_environment(&fixture.isolation).unwrap();
    fs::write(
        fixture.resource.join("input"),
        b"changed immutable resource",
    )
    .unwrap();
    // This reports the originally bound domain and effective environment only.
    // It neither updates that digest nor substitutes for execution preflight or
    // the full content checks required before source/read/result acceptance.
    assert_eq!(observe_environment(&fixture.isolation).unwrap(), original);
    assert!(fixture.isolation.revalidate_resource_contents().is_err());
    assert!(
        fixture
            .isolation
            .plan
            .managed_execution
            .as_ref()
            .unwrap()
            .revalidate_resource_versions_until(std::time::Instant::now() + Duration::from_secs(1))
            .is_err()
    );
}

#[test]
fn environment_observation_has_closed_descriptive_schema_and_bounded_public_size() {
    let fixture = Fixture::new();
    let observation = observe_environment(&fixture.isolation).unwrap();
    let value = serde_json::to_value(&observation).unwrap();
    assert_eq!(
        serde_json::from_value::<EnvironmentObservation>(value.clone()).unwrap(),
        observation
    );
    for (field, changed) in [
        ("schema_version", serde_json::json!(2)),
        ("qualified", serde_json::json!(true)),
    ] {
        let mut invalid = value.clone();
        invalid[field] = changed;
        assert!(serde_json::from_value::<EnvironmentObservation>(invalid).is_err());
    }
    let mut invalid = value;
    invalid["limits"]["network"] = serde_json::json!("allowed");
    assert!(serde_json::from_value::<EnvironmentObservation>(invalid).is_err());
    let mut plan = fixture.isolation.plan.clone();
    plan.read_roots.clear();
    for index in 0..150 {
        let path = fixture
            .resource
            .join(format!("root-{index:03}-{}", "x".repeat(180)));
        fs::create_dir(&path).unwrap();
        plan.read_roots.push(PathGrant {
            path,
            directory: true,
        });
    }
    plan.managed_execution = Some(BoundManagedExecution::automatic(&plan).unwrap());
    let large = plan.validate().unwrap();
    assert_eq!(
        observe_environment(&large).unwrap_err().code,
        "execution_environment_limit"
    );
}

#[test]
fn environment_inspection_and_original_file_membership_never_renew_expired_cutoff() {
    let fixture = Fixture::new();
    let deadline = std::time::Instant::now() - Duration::from_secs(1);
    assert_eq!(
        observe_environment_until(&fixture.isolation, deadline)
            .unwrap_err()
            .code,
        "managed_resource_limit"
    );
    assert_eq!(
        fixture
            .isolation
            .revalidate_inspection_until(deadline)
            .unwrap_err()
            .code,
        "managed_resource_limit"
    );
    let domain = fixture.isolation.plan.managed_execution.as_ref().unwrap();
    let path = fixture.resource.join("input");
    let digest = bytes_digest(b"original");
    assert_eq!(
        domain
            .verify_resource_file_until(&fixture.resource, &path, &digest, 8, deadline)
            .unwrap_err()
            .code,
        "managed_resource_limit"
    );
    domain
        .verify_resource_file_until(
            &fixture.resource,
            &path,
            &digest,
            8,
            std::time::Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    assert!(
        domain
            .verify_resource_file_until(
                &fixture.resource,
                &path,
                &bytes_digest(b"changed!"),
                8,
                std::time::Instant::now() + Duration::from_secs(5),
            )
            .is_err()
    );
}

#[test]
fn environment_inspection_does_not_scan_writable_descendants_or_qualify_their_launch() {
    let fixture = Fixture::new();
    let mut plan = fixture.isolation.plan.clone();
    plan.source_writes.push(PathGrant {
        path: plan.repository.clone(),
        directory: true,
    });
    let isolation = plan.validate().unwrap();
    let original = observe_environment(&isolation).unwrap();
    let source = isolation.plan.repository.join("new-source");
    fs::write(&source, b"mutable source").unwrap();
    fs::hard_link(&source, isolation.plan.repository.join("linked-source")).unwrap();
    // Description still refers only to the original boundary; inspecting it
    // must not recursively walk newly created mutable source descendants.
    assert_eq!(
        observe_environment_until(
            &isolation,
            std::time::Instant::now() + Duration::from_secs(5)
        )
        .unwrap(),
        original
    );
    assert_eq!(
        original.native_prerequisite.unwrap().code,
        "managed_profile_unqualified"
    );
    // The actual command path retains the existing full writable-tree guard.
    assert_eq!(isolation.revalidate().unwrap_err().code, "scope_violation");
    assert_eq!(
        render_command(&isolation, &["anything".into()])
            .err()
            .unwrap()
            .code,
        "scope_violation"
    );
}
