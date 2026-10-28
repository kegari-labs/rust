//! Output and startup-boundary tests; no native sandbox is invoked.
use super::*;

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

fn unused_factory(_: &CommandLaunchIdentity) -> Result<SpawnGuard> {
    Err(Error::new("fixture_factory", "fixture journal factory"))
}

#[test]
fn launch_options_preserve_foreground_invocation_and_bundle_output_and_service_modes() {
    use std::sync::Arc;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let foreground = CommandJournal::Factory {
        begin: unused_factory,
        options: CommandLaunchOptions::Foreground {
            timeout: Duration::from_secs(1),
            output_limit: 64,
        },
    };
    assert_eq!(foreground.deadline(), None);
    assert!(!foreground.service());
    assert!(foreground.output_budget(64).is_none());
    assert_eq!(foreground.rendered_output_limit(64), None);

    let invocation = CommandJournal::Factory {
        begin: unused_factory,
        options: CommandLaunchOptions::Invocation(InvocationCommandLimits {
            timeout: Duration::from_secs(1),
            deadline,
            output_limit: 64,
        }),
    };
    assert_eq!(invocation.deadline(), Some(deadline));
    assert!(!invocation.service());
    assert_eq!(invocation.output_budget(64).unwrap().take(65), 64);
    assert_eq!(invocation.rendered_output_limit(64), Some(64));

    let shared = Arc::new(CommandOutputBudget::new(128));
    for (member_kind, service) in [
        (CheckMemberKind::Service, true),
        (CheckMemberKind::Readiness, false),
        (CheckMemberKind::Check, false),
    ] {
        let bundle = CommandJournal::Factory {
            begin: unused_factory,
            options: CommandLaunchOptions::Bundle {
                limits: BundleCommandLimits::new(
                    Duration::from_secs(1),
                    deadline,
                    64,
                    shared.clone(),
                ),
                member_kind,
            },
        };
        assert_eq!(bundle.deadline(), Some(deadline));
        assert_eq!(bundle.service(), service);
        assert!(Arc::ptr_eq(&bundle.output_budget(64).unwrap(), &shared));
        assert_eq!(bundle.rendered_output_limit(64), None);
    }
}

#[test]
fn journal_factory_receives_exact_launch_identity_once_and_preserves_its_failure() {
    use kegari_platform::BinaryIdentity;
    use std::cell::Cell;
    let binary = BinaryIdentity {
        path: "/fixture/executable".into(),
        digest: bytes_digest(b"fixture executable"),
        bytes: 18,
    };
    let identity = CommandLaunchIdentity {
        cwd: "/fixture/cwd".into(),
        launcher: binary.clone(),
        target: binary,
        argv_digest: bytes_digest(b"fixture argv"),
    };
    let called = Cell::new(0);
    let owned_value = String::from("one-shot factory failure");
    let journal = CommandJournal::Factory {
        begin: |observed: &CommandLaunchIdentity| {
            called.set(called.get() + 1);
            assert_eq!(observed, &identity);
            Err(Error::new("fixture_factory", owned_value))
        },
        options: CommandLaunchOptions::Foreground {
            timeout: Duration::from_secs(1),
            output_limit: 64,
        },
    };
    let error = journal.begin(&identity).err().unwrap();
    assert_eq!(called.get(), 1);
    assert_eq!(error.code, "fixture_factory");
    assert_eq!(error.message, "one-shot factory failure");
}

#[test]
fn startup_deadlines_and_qualification_refuse_before_journal_factory() {
    use std::{cell::Cell, fs};
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let execution_root = root.join("owned");
    let repository = execution_root.join("repository");
    let scratch = execution_root.join("scratch");
    for directory in [&repository, &scratch] {
        fs::create_dir_all(directory).unwrap();
    }
    let mut plan = IsolationPlan {
        schema_version: 1,
        role: ExecutionRole::Child,
        execution_root,
        cwd: repository.clone(),
        repository,
        scratch,
        read_roots: vec![],
        source_writes: vec![],
        resource_writes: vec![],
        forbidden_roots: vec![],
        managed_execution: None,
    };
    let called = Cell::new(0);
    for managed in [false, true] {
        if managed {
            plan.managed_execution =
                Some(BoundManagedExecution::automatic(&plan, crate::TEST_BUILD).unwrap());
        }
        let isolation = plan.validate(crate::TEST_BUILD).unwrap();
        for expired in [true, false] {
            let options = if expired {
                CommandLaunchOptions::Invocation(InvocationCommandLimits {
                    timeout: Duration::from_secs(30),
                    deadline: std::time::Instant::now(),
                    output_limit: 64,
                })
            } else {
                CommandLaunchOptions::Foreground {
                    timeout: Duration::from_secs(1),
                    output_limit: 64,
                }
            };
            let error = start_journaled_command(
                &isolation,
                &["never-launched".into()],
                options,
                &[],
                |_| {
                    called.set(called.get() + 1);
                    Err(Error::new(
                        "fixture_factory",
                        "factory must remain uncalled",
                    ))
                },
            )
            .err()
            .unwrap();
            let expected = if expired {
                "timeout"
            } else if !cfg!(target_os = "macos") {
                "isolation_unavailable"
            } else if managed {
                "managed_profile_unqualified"
            } else {
                "process_tree_unverified"
            };
            assert_eq!(error.code, expected);
            assert_eq!(called.get(), 0);
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn rendering_precedes_factory_and_factory_failure_prevents_native_spawn() {
    use std::{cell::Cell, fs, os::unix::fs::PermissionsExt};
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let execution_root = root.join("owned");
    let repository = execution_root.join("repository");
    let scratch = execution_root.join("scratch");
    for directory in [&repository, &scratch] {
        fs::create_dir_all(directory).unwrap();
    }
    let program = repository.join("fixture-command");
    let marker = scratch.join("must-not-run");
    let program_bytes = b"#!/bin/sh\nprintf unexpected > \"$1\"\n";
    fs::write(&program, program_bytes).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    let mut plan = IsolationPlan {
        schema_version: 1,
        role: ExecutionRole::Child,
        execution_root,
        cwd: repository.clone(),
        repository,
        scratch,
        read_roots: vec![],
        source_writes: vec![],
        resource_writes: vec![],
        forbidden_roots: vec![],
        managed_execution: None,
    };
    plan.managed_execution =
        Some(BoundManagedExecution::automatic(&plan, crate::TEST_BUILD).unwrap());
    let isolation = plan.validate(crate::TEST_BUILD).unwrap();
    let argv = vec![
        program.to_string_lossy().into_owned(),
        marker.to_string_lossy().into_owned(),
    ];
    // This inspection reads executable bytes only. The private native path below
    // must stop at the factory's error before guard configuration or OS spawn.
    let expected = crate::command_launch_identity(&isolation, &argv).unwrap();
    let called = Cell::new(0);
    for valid_rendering in [false, true] {
        let selected = if valid_rendering {
            argv.as_slice()
        } else {
            &[]
        };
        let owned_message = String::from("stop after exact rendering, before native spawn");
        let source = CommandJournal::Factory {
            begin: |identity: &CommandLaunchIdentity| {
                called.set(called.get() + 1);
                assert_eq!(identity, &expected);
                Err(Error::new("fixture_stop_before_spawn", owned_message))
            },
            options: CommandLaunchOptions::Foreground {
                timeout: Duration::from_secs(30),
                output_limit: 64,
            },
        };
        let error = spawn_command_inner(
            &isolation,
            selected,
            Duration::from_secs(30),
            64,
            &[],
            Some(source),
        )
        .err()
        .unwrap();
        if valid_rendering {
            assert_eq!(called.get(), 1);
            assert_eq!(error.code, "fixture_stop_before_spawn");
            assert_eq!(
                error.message,
                "stop after exact rendering, before native spawn"
            );
        } else {
            assert_eq!(called.get(), 0);
            assert_eq!(error.code, "invalid_config");
            assert_eq!(
                error.message,
                "command argv or cwd exceeds its finite input bounds"
            );
        }
        assert!(!marker.exists());
        assert!(!root.join("processes").exists());
        assert_eq!(fs::read(&program).unwrap(), program_bytes);
    }
}
