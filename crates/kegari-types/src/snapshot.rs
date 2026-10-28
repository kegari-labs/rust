//! Shared immutable inspection-page data.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Segment {
    Field(String),
    Index(usize),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    Object {
        path: Vec<Segment>,
        keys: Vec<String>,
    },
    Array {
        path: Vec<Segment>,
        length: usize,
    },
    Scalar {
        path: Vec<Segment>,
        value: Value,
    },
    Text {
        path: Vec<Segment>,
        offset: usize,
        total_bytes: usize,
        complete_digest: String,
        text: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub snapshot_id: Uuid,
    pub observed_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub projection_digest: String,
    pub cursor: Option<Uuid>,
    pub next_cursor: Option<Uuid>,
    pub total_entries: usize,
    pub entries: Vec<Node>,
}
