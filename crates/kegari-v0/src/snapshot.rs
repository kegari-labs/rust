//! Immutable, actor-bound, in-memory control snapshots. This is pagination state,
//! not authoritative work persistence. Clients verify completeness before output.
use crate::{
    digest::{bytes_digest, canonical_digest},
    error::{Error, Result},
};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

const CHUNK_BYTES: usize = 64 * 1024;
const PAGE_BYTES: usize = 7 * 1024 * 1024;
const PAGE_ENTRIES: usize = 100;
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
impl Node {
    fn path(&self) -> &[Segment] {
        match self {
            Self::Object { path, .. }
            | Self::Array { path, .. }
            | Self::Scalar { path, .. }
            | Self::Text { path, .. } => path,
        }
    }
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
#[derive(Debug, Clone)]
struct Snapshot {
    binding: String,
    pages: Vec<Page>,
}
#[derive(Default)]
pub struct Snapshots {
    snapshots: BTreeMap<Uuid, Snapshot>,
}

fn flatten(
    value: &Value,
    path: &mut Vec<Segment>,
    out: &mut Vec<Node>,
    depth: usize,
) -> Result<()> {
    if depth > 64 {
        return Err(Error::new(
            "snapshot_too_large",
            "snapshot nesting exceeds 64 levels",
        ));
    }
    match value {
        Value::Object(values) => {
            let sorted: BTreeMap<_, _> = values.iter().collect();
            let keys = sorted.keys().map(|s| (*s).clone()).collect();
            out.push(Node::Object {
                path: path.clone(),
                keys,
            });
            for (key, value) in sorted {
                path.push(Segment::Field(key.clone()));
                flatten(value, path, out, depth + 1)?;
                path.pop();
            }
        }
        Value::Array(values) => {
            out.push(Node::Array {
                path: path.clone(),
                length: values.len(),
            });
            for (index, value) in values.iter().enumerate() {
                path.push(Segment::Index(index));
                flatten(value, path, out, depth + 1)?;
                path.pop();
            }
        }
        Value::String(text) if text.len() > CHUNK_BYTES => {
            let digest = bytes_digest(text.as_bytes());
            let mut offset = 0;
            while offset < text.len() {
                let mut end = (offset + CHUNK_BYTES).min(text.len());
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                out.push(Node::Text {
                    path: path.clone(),
                    offset,
                    total_bytes: text.len(),
                    complete_digest: digest.clone(),
                    text: text[offset..end].into(),
                });
                offset = end;
            }
        }
        _ => out.push(Node::Scalar {
            path: path.clone(),
            value: value.clone(),
        }),
    }
    Ok(())
}
impl Snapshots {
    pub fn capture(
        &mut self,
        actor: &str,
        runtime: Uuid,
        filters: &Value,
        projection: &Value,
        now: DateTime<Utc>,
    ) -> Result<Page> {
        if actor.is_empty() {
            return Err(Error::new(
                "access_denied",
                "snapshot requires authenticated actor",
            ));
        }
        self.snapshots
            .retain(|_, snapshot| snapshot.pages[0].expires_at > now);
        if self.snapshots.len() >= 128 {
            return Err(Error::new(
                "rate_limited",
                "too many live inspection snapshots; retry after expiry",
            ));
        }
        let binding = canonical_digest(
            1,
            &serde_json::json!({"actor":actor,"runtime":runtime,"filters":filters}),
        )?;
        let mut entries = Vec::new();
        flatten(projection, &mut vec![], &mut entries, 0)?;
        let total_entries = entries.len();
        let digest = canonical_digest(1, projection)?;
        let id = Uuid::new_v4();
        let expires_at = now + Duration::minutes(5);
        let mut pages = Vec::new();
        let mut current = Vec::new();
        let mut size = 0;
        for entry in entries {
            let encoded = serde_json::to_vec(&entry)
                .map_err(|_| Error::new("snapshot_invalid", "cannot encode snapshot entry"))?
                .len();
            if encoded > PAGE_BYTES {
                return Err(Error::new(
                    "snapshot_too_large",
                    "snapshot entry exceeds response limit",
                ));
            }
            if !current.is_empty() && (current.len() == PAGE_ENTRIES || size + encoded > PAGE_BYTES)
            {
                pages.push(std::mem::take(&mut current));
                size = 0;
            }
            current.push(entry);
            size += encoded;
        }
        if !current.is_empty() {
            pages.push(current);
        }
        let cursors: Vec<Option<Uuid>> = (0..pages.len())
            .map(|i| if i == 0 { None } else { Some(Uuid::new_v4()) })
            .collect();
        let pages: Vec<_> = pages
            .into_iter()
            .enumerate()
            .map(|(i, entries)| Page {
                snapshot_id: id,
                observed_at: now,
                expires_at,
                projection_digest: digest.clone(),
                cursor: cursors[i],
                next_cursor: cursors.get(i + 1).copied().flatten(),
                total_entries,
                entries,
            })
            .collect();
        let first = pages[0].clone();
        self.snapshots.insert(id, Snapshot { binding, pages });
        Ok(first)
    }
    pub fn page(
        &self,
        id: Uuid,
        cursor: Uuid,
        actor: &str,
        runtime: Uuid,
        filters: &Value,
        now: DateTime<Utc>,
    ) -> Result<Page> {
        let snapshot = self.snapshots.get(&id).ok_or_else(|| {
            Error::new(
                "snapshot_expired",
                "inspection snapshot is unavailable; restart the complete read",
            )
        })?;
        if now >= snapshot.pages[0].expires_at {
            return Err(Error::new(
                "snapshot_expired",
                "inspection snapshot expired; do not splice revisions",
            ));
        }
        let binding = canonical_digest(
            1,
            &serde_json::json!({"actor":actor,"runtime":runtime,"filters":filters}),
        )?;
        if binding != snapshot.binding {
            return Err(Error::new(
                "access_denied",
                "snapshot cursor does not belong to this actor, runtime and filters",
            ));
        }
        snapshot
            .pages
            .iter()
            .find(|page| page.cursor == Some(cursor))
            .cloned()
            .ok_or_else(|| Error::new("invalid_input", "unknown snapshot cursor"))
    }
}

/// Return a value only after all pages, UTF-8 chunks and the full projection digest
/// match. No truncated or cross-revision result can be emitted as a complete view.
pub fn assemble(pages: &[Page]) -> Result<Value> {
    let error = || {
        Error::new(
            "incomplete_snapshot",
            "snapshot pages, chunks or complete-content digest do not match",
        )
    };
    let first = pages.first().ok_or_else(error)?;
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        if page.snapshot_id != first.snapshot_id
            || page.projection_digest != first.projection_digest
            || page.observed_at != first.observed_at
            || page.expires_at != first.expires_at
            || page.total_entries != first.total_entries
            || page.cursor != cursor
            || page.entries.len() > PAGE_ENTRIES
            || !seen.insert(page.cursor)
            || (index + 1 < pages.len() && page.next_cursor.is_none())
        {
            return Err(error());
        }
        cursor = page.next_cursor;
        entries.extend(page.entries.clone());
    }
    if cursor.is_some() || entries.len() != first.total_entries {
        return Err(error());
    }
    fn rebuild(
        entries: &[Node],
        index: &mut usize,
        path: &mut Vec<Segment>,
        depth: usize,
    ) -> Result<Value> {
        let error = || {
            Error::new(
                "incomplete_snapshot",
                "snapshot node ordering or text chunks are invalid",
            )
        };
        if depth > 64 {
            return Err(error());
        }
        let node = entries.get(*index).ok_or_else(error)?;
        if node.path() != path.as_slice() {
            return Err(error());
        }
        *index += 1;
        match node {
            Node::Object { keys, .. } => {
                if keys.iter().collect::<BTreeSet<_>>().len() != keys.len() {
                    return Err(error());
                }
                let mut result = serde_json::Map::new();
                for key in keys {
                    path.push(Segment::Field(key.clone()));
                    let value = rebuild(entries, index, path, depth + 1)?;
                    path.pop();
                    result.insert(key.clone(), value);
                }
                Ok(Value::Object(result))
            }
            Node::Array { length, .. } => {
                if *length > entries.len() {
                    return Err(error());
                }
                let mut result = Vec::new();
                for i in 0..*length {
                    path.push(Segment::Index(i));
                    result.push(rebuild(entries, index, path, depth + 1)?);
                    path.pop();
                }
                Ok(Value::Array(result))
            }
            Node::Scalar { value, .. } => {
                if value.is_object() || value.is_array() {
                    return Err(error());
                }
                Ok(value.clone())
            }
            Node::Text {
                offset,
                total_bytes,
                complete_digest,
                text,
                ..
            } => {
                if *offset != 0 || text.is_empty() || text.len() > *total_bytes {
                    return Err(error());
                }
                let mut result = text.clone();
                while result.len() < *total_bytes {
                    match entries.get(*index) {
                        Some(Node::Text {
                            path: next_path,
                            offset: next_offset,
                            total_bytes: next_total,
                            complete_digest: next_digest,
                            text,
                        }) if next_path == path
                            && *next_offset == result.len()
                            && next_total == total_bytes
                            && next_digest == complete_digest
                            && !text.is_empty() =>
                        {
                            result.push_str(text)
                        }
                        _ => return Err(error()),
                    }
                    *index += 1;
                }
                if result.len() != *total_bytes
                    || bytes_digest(result.as_bytes()) != *complete_digest
                {
                    return Err(error());
                }
                Ok(Value::String(result))
            }
        }
    }
    let mut index = 0;
    let value = rebuild(&entries, &mut index, &mut vec![], 0)?;
    if index != entries.len() || canonical_digest(1, &value)? != first.projection_digest {
        return Err(error());
    }
    Ok(value)
}
