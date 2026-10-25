//! Kegari's deterministic control and delivery core.
/// Deterministic identity of compiled runtime sources, assets and dependency lock.
/// The installed executable also has a separate exact-byte digest in owner metadata.
pub const BUILD_ID: &str = env!("KEGARI_BUILD_ID");
pub mod api;
pub mod app;
pub mod authority;
pub mod cli;
pub mod client;
pub mod codex;
pub mod config;
pub mod configuration;
pub mod connections;
pub mod control;
pub mod corrections;
pub mod delivery;
pub mod deployment;
pub mod digest;
pub mod error;
pub mod evaluation;
pub mod evaluation_reports;
pub mod evidence;
pub mod execution;
pub mod git;
pub mod host;
pub mod housekeeping;
pub mod instructions;
pub mod isolation;
pub mod local_cli;
pub mod observations;
pub mod orchestration;
pub mod platform;
pub mod preparation;
pub mod process_group;
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
pub mod runtime_native;
pub mod runtime_schedule;
pub mod runtime_workers;
pub mod runtime_workflow;
pub mod scheduler;
pub mod secrets;
pub mod snapshot;
pub mod storage;
pub mod supervisor;
pub mod transport;
pub mod worker;
pub mod worker_run;
pub mod workflow;

pub mod decisions;

pub mod host_process;
