use chrono::{Duration, Utc};
use kegari_types::snapshot::*;
use kegari_v0::snapshot::*;
use serde_json::json;
use uuid::Uuid;

#[test]
fn complete_large_nested_projection_and_unicode_text_roundtrip() {
    let projection = json!({"parents":(0..180).map(|i|json!({"id":i,"children":[{"id":i+1000}]})).collect::<Vec<_>>(),"description":"🌱漢字".repeat(40_000)});
    let runtime = Uuid::new_v4();
    let now = Utc::now();
    let mut snapshots = Snapshots::default();
    let first = snapshots
        .capture("os:501", runtime, &json!({}), &projection, now)
        .unwrap();
    let mut pages = vec![first];
    while let Some(cursor) = pages.last().unwrap().next_cursor {
        pages.push(
            snapshots
                .page(
                    pages[0].snapshot_id,
                    cursor,
                    "os:501",
                    runtime,
                    &json!({}),
                    now,
                )
                .unwrap(),
        );
    }
    assert!(pages.len() > 1);
    assert!(pages.iter().all(|p| p.entries.len() <= 100));
    assert_eq!(assemble(&pages).unwrap(), projection);
    assert_eq!(
        assemble(&pages[..pages.len() - 1]).unwrap_err().code,
        "incomplete_snapshot"
    );
}
#[test]
fn cursor_cannot_cross_actor_runtime_filter_or_expiry() {
    let runtime = Uuid::new_v4();
    let now = Utc::now();
    let mut snapshots = Snapshots::default();
    let first = snapshots
        .capture(
            "os:501",
            runtime,
            &json!({"workspace":"a"}),
            &json!((0..200).collect::<Vec<_>>()),
            now,
        )
        .unwrap();
    let cursor = first.next_cursor.unwrap();
    for (actor, target, filter) in [
        ("os:502", runtime, json!({"workspace":"a"})),
        ("os:501", Uuid::new_v4(), json!({"workspace":"a"})),
        ("os:501", runtime, json!({"workspace":"b"})),
    ] {
        assert_eq!(
            snapshots
                .page(first.snapshot_id, cursor, actor, target, &filter, now)
                .unwrap_err()
                .code,
            "access_denied"
        );
    }
    assert_eq!(
        snapshots
            .page(
                first.snapshot_id,
                cursor,
                "os:501",
                runtime,
                &json!({"workspace":"a"}),
                now + Duration::minutes(5)
            )
            .unwrap_err()
            .code,
        "snapshot_expired"
    );
}
#[test]
fn mixed_snapshot_and_tampered_chunk_never_render_complete() {
    let runtime = Uuid::new_v4();
    let now = Utc::now();
    let mut snapshots = Snapshots::default();
    let a = snapshots
        .capture(
            "os:501",
            runtime,
            &json!({}),
            &json!("x".repeat(90_000)),
            now,
        )
        .unwrap();
    let mut changed = a.clone();
    if let Node::Text { text, .. } = &mut changed.entries[0] {
        text.replace_range(..1, "y");
    }
    assert!(assemble(&[changed]).is_err());
    let b = snapshots
        .capture(
            "os:501",
            runtime,
            &json!({}),
            &json!("x".repeat(90_000)),
            now,
        )
        .unwrap();
    assert!(assemble(&[a, b]).is_err());
}
