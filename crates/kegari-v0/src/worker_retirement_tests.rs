use super::*;

fn journal() -> (tempfile::TempDir, PrivateDir) {
    let temp = tempfile::tempdir().unwrap();
    let records = PrivateDir::create(&temp.path().canonicalize().unwrap().join("journal")).unwrap();
    (temp, records)
}

#[test]
fn captured_journal_event_and_hash_use_same_bytes_after_file_replacement() {
    // Actual protected file I/O and the production decoder only. These check
    // and hook payloads are data; no worker, exit, check or seal is constructed.
    let (_temp, journal) = journal();
    for (leaf, original, replacement) in [
        (
            "000001.json",
            json!({"kind":"check","record":{"state":"started","check_name":"original","binding":{"operation_id":"component-data"}}}),
            json!({"kind":"after_hook","record":{"error":null}}),
        ),
        (
            "000002.json",
            json!({"kind":"after_hook","record":{"error":{"code":"original_hook_failure"}}}),
            json!({"kind":"after_hook","record":{"error":null}}),
        ),
    ] {
        journal.write(leaf, &original).unwrap();
        let captured = journal.read_bytes(leaf, 4 * 1024 * 1024).unwrap();
        journal.write(leaf, &replacement).unwrap();
        let current = journal.read_bytes(leaf, 4 * 1024 * 1024).unwrap();
        assert_ne!(captured, current);
        let (retained_hash, event) = captured_journal_record(leaf, &captured).unwrap();
        assert_eq!(event, Some(original));
        assert_eq!(retained_hash, bytes_digest(&captured));
        assert_ne!(retained_hash, bytes_digest(&current));
        assert_eq!(
            captured_journal_record(leaf, &current).unwrap(),
            (bytes_digest(&current), Some(replacement))
        );
    }
}

#[test]
fn captured_journal_decoder_preserves_envelope_and_event_size_refusals() {
    let (_temp, journal) = journal();
    let leaf = "000001.json";
    let payload = json!({"kind":"after_hook","record":{"error":null}});
    journal.write(leaf, &payload).unwrap();
    let bytes = journal.read_bytes(leaf, 4 * 1024 * 1024).unwrap();
    let envelope: Value = serde_json::from_slice(&bytes).unwrap();
    for (mutation, code) in [
        ("payload", "record_corrupt"),
        ("digest", "record_corrupt"),
        ("unknown", "record_corrupt"),
        ("version", "incompatible_version"),
    ] {
        let mut changed = envelope.clone();
        match mutation {
            "payload" => changed["payload"]["record"]["error"] = json!("changed"),
            "digest" => changed["digest"] = json!(bytes_digest(b"substituted")),
            "unknown" => changed["unknown"] = json!(true),
            "version" => changed["schema_version"] = json!(2),
            _ => unreachable!(),
        }
        assert_eq!(
            captured_journal_record(leaf, &serde_json::to_vec(&changed).unwrap())
                .unwrap_err()
                .code,
            code,
            "{mutation}"
        );
    }
    // A raw event without the existing protected storage envelope is not valid.
    assert_eq!(
        captured_journal_record(leaf, &serde_json::to_vec(&payload).unwrap())
            .unwrap_err()
            .code,
        "record_corrupt"
    );
    let oversized = vec![b' '; 256 * 1024 + 1];
    let error = captured_journal_record(leaf, &oversized).unwrap_err();
    assert_eq!(error.code, "record_corrupt");
    assert_eq!(error.message, "private file exceeds its size bound");
    // Scope/binding is checked separately before capture and remains byte-bound.
    assert_eq!(
        captured_journal_record("binding.json", &bytes).unwrap(),
        (bytes_digest(&bytes), None)
    );
}
