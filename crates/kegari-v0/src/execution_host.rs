//! Application admission adapters for the managed execution mechanism.
use crate::control::{CheckBundleLaunch, Permit};
use crate::process_records::ProcessJournal;
use kegari_execution::{
    BundleCommandLimits, CommandLaunchOptions, InvocationCommandLimits, OwnedCommand,
    ValidatedIsolation, journal::SpawnBinding,
};
use kegari_types::error::Result;
use std::{sync::Arc, time::Duration};

pub fn start_journaled_command(
    isolation: &ValidatedIsolation,
    argv: &[String],
    timeout: Duration,
    output_limit: usize,
    secret_values: &[String],
    journal: Arc<ProcessJournal>,
    permit: Permit,
) -> Result<OwnedCommand> {
    start_worker_command(
        isolation,
        argv,
        CommandLaunchOptions::Foreground {
            timeout,
            output_limit,
        },
        secret_values,
        journal,
        permit,
    )
}

pub fn start_journaled_command_until(
    isolation: &ValidatedIsolation,
    argv: &[String],
    limits: InvocationCommandLimits,
    secret_values: &[String],
    journal: Arc<ProcessJournal>,
    permit: Permit,
) -> Result<OwnedCommand> {
    start_worker_command(
        isolation,
        argv,
        CommandLaunchOptions::Invocation(limits),
        secret_values,
        journal,
        permit,
    )
}

fn start_worker_command(
    isolation: &ValidatedIsolation,
    argv: &[String],
    options: CommandLaunchOptions,
    secret_values: &[String],
    journal: Arc<ProcessJournal>,
    permit: Permit,
) -> Result<OwnedCommand> {
    kegari_execution::start_journaled_command(
        isolation,
        argv,
        options,
        secret_values,
        move |identity| {
            journal.begin(SpawnBinding {
                operation_id: permit.operation_id,
                effect_id: uuid::Uuid::new_v4(),
                lease: Some(permit.lease),
                control_owner: None,
                work_key: permit.work_key,
                scope_digest: permit.scope_digest,
                cwd: identity.cwd.clone(),
                executable: identity.launcher.clone(),
                observed_executables: vec![identity.target.clone()],
                argv_digest: identity.argv_digest.clone(),
                kind: "repository_command".into(),
                check_bundle: None,
            })
        },
    )
}

pub(crate) fn start_bundle_command(
    isolation: &ValidatedIsolation,
    argv: &[String],
    limits: BundleCommandLimits,
    journal: Arc<ProcessJournal>,
    launch: CheckBundleLaunch,
) -> Result<OwnedCommand> {
    let member_kind = launch.member().kind.clone();
    kegari_execution::start_journaled_command(
        isolation,
        argv,
        CommandLaunchOptions::Bundle {
            limits,
            member_kind,
        },
        &[],
        move |identity| {
            let permit = launch.permit();
            journal.begin(SpawnBinding {
                operation_id: permit.operation_id,
                effect_id: launch.process_id(),
                lease: Some(permit.lease.clone()),
                control_owner: None,
                work_key: permit.work_key.clone(),
                scope_digest: permit.scope_digest.clone(),
                cwd: identity.cwd.clone(),
                executable: identity.launcher.clone(),
                observed_executables: vec![identity.target.clone()],
                argv_digest: identity.argv_digest.clone(),
                kind: "repository_command".into(),
                check_bundle: Some(launch.member().clone()),
            })
        },
    )
}

#[cfg(test)]
#[path = "execution_host_observation_tests.rs"]
mod observation_tests;
