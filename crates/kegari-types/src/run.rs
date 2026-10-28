//! Shared run identity and phase data, without admission or recovery authority.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identity-only run-budget record embedded in the atomic admission receipt.
/// It contains no issue text, prompts, credentials or replicated provider state.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunReference {
    pub workspace: Uuid,
    pub account: String,
    #[serde(default)]
    pub normalized_state: Option<String>,
    pub phase: PhaseKey,
    pub profile_digest: String,
    pub instruction_digest: String,
    pub max_attempts: u32,
    pub max_turns: u32,
    pub max_review_rounds: u32,
    pub attempt: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PhaseKey {
    pub work: String,
    pub generation: String,
    pub phase: String,
    pub retry_grant: Option<Uuid>,
}
