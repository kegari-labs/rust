//! Descriptive artifact data, without protected storage or execution authority.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactProof {
    pub path: PathBuf,
    pub digest: String,
    pub bytes: u64,
    pub validation_operation: Uuid,
    pub candidate_vector_digest: Option<String>,
}
