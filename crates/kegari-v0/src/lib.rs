//! Kegari's deterministic control and delivery core.
/// Deterministic identity of compiled runtime sources, assets and dependency lock.
/// The installed executable also has a separate exact-byte digest in owner metadata.
pub const BUILD_ID: &str = env!("KEGARI_BUILD_ID");
pub mod authority;
pub mod codex;
pub mod config;
pub mod control;
pub mod digest;
pub mod error;
pub mod evaluation;
pub mod evidence;
pub mod git;
pub mod instructions;
pub mod isolation;
pub mod providers;
pub mod release;
pub mod scheduler;
pub mod snapshot;
pub mod storage;
pub mod workflow;
