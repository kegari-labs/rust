//! Shared control identity data, without admission or process ownership.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lease {
    pub runtime_id: Uuid,
    pub owner_generation: u64,
    pub owner_id: Uuid,
}
