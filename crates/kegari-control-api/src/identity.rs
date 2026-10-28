//! Operator identity values; current principal and role authorization remain backend policy.
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub identity: String,
    pub roles: BTreeSet<String>,
    pub principal: Option<Principal>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
pub enum Principal {
    Unix { uid: u32 },
    Tailscale { user_id: u64, node_id: String },
}
impl Actor {
    pub fn require(&self, role: &str) -> Result<()> {
        if self.identity.trim().is_empty() || !self.roles.contains(role) {
            Err(Error::new(
                "access_denied",
                format!("authenticated actor lacks {role} authority"),
            ))
        } else {
            Ok(())
        }
    }
}
