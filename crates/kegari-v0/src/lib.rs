//! Kegari's deterministic control and delivery core.
// Shared integration fixture helpers name this crate from nested test modules.
#[cfg(test)]
extern crate self as kegari_v0;
#[cfg(test)]
#[path = "../tests/support/pinned_git.rs"]
pub(crate) mod test_pinned_git;
/// Deterministic identity of compiled runtime sources, assets and dependency lock.
/// The installed executable also has a separate exact-byte digest in owner metadata.
pub const BUILD_ID: &str = env!("KEGARI_BUILD_ID");
/// Public CLI and internal metadata identify the same assembled application.
pub const VERSION_WITH_BUILD: &str =
    concat!(env!("CARGO_PKG_VERSION"), " ", env!("KEGARI_BUILD_ID"));
pub mod authority;
pub mod codex_host;
pub mod config;
pub mod configuration;
pub mod connections;
pub mod control;
pub mod control_command;
pub mod corrections;
pub mod delivery;
pub mod deployment;
pub mod error;
pub mod evaluation;
pub mod evaluation_reports;
pub mod evidence;
pub mod execution;
pub mod git;
pub mod housekeeping;
pub mod inspection;
pub mod observations;
pub mod platform;
pub mod preparation;
pub mod process_records;
pub mod providers;
pub mod reconcile;
pub mod release;
pub mod remote_objects;
pub mod runtime;
mod runtime_actions;
mod runtime_claims;
pub mod runtime_delivery;
pub mod runtime_driver;
#[cfg(test)]
mod runtime_fixture;
pub mod runtime_host;
pub mod runtime_native;
pub mod runtime_schedule;
pub mod runtime_workers;
pub mod runtime_workflow;
pub mod validation;
pub mod worker;
pub mod worker_run;

pub mod decisions;

pub mod host_process;
#[cfg(test)]
#[path = "remote_objects_test_probe.rs"]
pub(crate) mod test_process_probe;

pub mod evaluation_provider;

pub(crate) mod validation_archive;

#[cfg(test)]
mod context_integration_tests;
