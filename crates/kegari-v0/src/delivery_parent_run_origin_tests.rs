use super::*;

fn storage() -> (tempfile::TempDir, PrivateDir) {
    let temp = tempfile::tempdir().unwrap();
    let records = PrivateDir::create(&temp.path().canonicalize().unwrap().join("records")).unwrap();
    (temp, records)
}

#[test]
fn original_history_capture_includes_failed_archive_bytes_and_refuses_later_changes() {
    // Actual protected archive I/O through the production origin collector;
    // the descriptive fixture creates no parent/worker/readiness capability.
    let (_temp, records) = storage();
    let key = AttestationKey::new("archive-origin-fixture".into(), vec![7; 32]).unwrap();
    let (namespace, evidence, artifacts) = crate::validation_archive::tests::descriptive_fixture();
    let archive =
        crate::validation_archive::retain(&records, &key, namespace.clone(), &evidence, |proof| {
            Ok(artifacts[&canonical_digest(1, proof)?].clone())
        })
        .unwrap();
    let failed = &evidence
        .attempts
        .iter()
        .find(|a| !a.successful)
        .unwrap()
        .artifacts[0];
    let failed_bytes = &artifacts[&canonical_digest(1, failed).unwrap()];
    let failed_leaf = archive
        .retained_files()
        .iter()
        .find(|(_, digest)| **digest == bytes_digest(failed_bytes))
        .unwrap()
        .0;
    let mut files = BTreeMap::new();
    retain_validation_archive(
        &records,
        &key,
        namespace.workspace_id,
        namespace.runtime_id,
        archive.reference(),
        &mut files,
    )
    .unwrap();
    assert_eq!(&files, archive.retained_files());
    let before = files.clone();
    records
        .write_bytes(failed_leaf, b"changed failed output")
        .unwrap();
    assert!(
        retain_validation_archive(
            &records,
            &key,
            namespace.workspace_id,
            namespace.runtime_id,
            archive.reference(),
            &mut files,
        )
        .is_err()
    );
    assert_eq!(
        files, before,
        "failed capture must not replace original hashes"
    );

    records.write_bytes(failed_leaf, failed_bytes).unwrap();
    files.insert(
        failed_leaf.clone(),
        bytes_digest(b"different original capture"),
    );
    let conflict = files.clone();
    assert!(
        retain_validation_archive(
            &records,
            &key,
            namespace.workspace_id,
            namespace.runtime_id,
            archive.reference(),
            &mut files,
        )
        .is_err()
    );
    assert_eq!(files, conflict);
}

#[test]
fn original_digest_refuses_chunk_mutation_before_first_history_capture() {
    // Actual protected file I/O and the production byte collector only. No
    // parent origin, joined worker, signed seal or cold capability is created.
    let (_temp, records) = storage();
    let chunks: Vec<String> = vec!["original-0.bin".into(), "original-1.bin".into()];
    records.write_bytes(&chunks[0], b"original ").unwrap();
    records.write_bytes(&chunks[1], b"artifact").unwrap();
    let original = b"original artifact";
    let digest = bytes_digest(original);
    records.write_bytes(&chunks[1], b"altered!").unwrap();
    for expected_bytes in [None, Some(original.len() as u64)] {
        let mut files = BTreeMap::new();
        assert_eq!(
            retain_artifact_chunks(&records, &chunks, &digest, expected_bytes, &mut files)
                .unwrap_err()
                .code,
            "parent_retirement"
        );
        assert!(
            files.is_empty(),
            "a failed first capture must retain no replacement hashes"
        );
    }
    records.write_bytes(&chunks[1], b"artifact").unwrap();
    let mut files = BTreeMap::new();
    retain_artifact_chunks(
        &records,
        &chunks,
        &digest,
        Some(original.len() as u64),
        &mut files,
    )
    .unwrap();
    assert_eq!(
        files,
        BTreeMap::from([
            (chunks[0].clone(), bytes_digest(b"original ")),
            (chunks[1].clone(), bytes_digest(b"artifact"))
        ])
    );
    // Idempotent reuse is valid only for the same exact retained bytes.
    retain_artifact_chunks(&records, &chunks, &digest, None, &mut files).unwrap();
}

#[test]
fn original_artifact_order_length_duplicates_and_missing_chunks_fail_closed() {
    let (_temp, records) = storage();
    let chunks: Vec<String> = vec!["part-0.bin".into(), "part-1.bin".into()];
    records.write_bytes(&chunks[0], b"first").unwrap();
    records.write_bytes(&chunks[1], b"second").unwrap();
    let digest = bytes_digest(b"firstsecond");
    for (selected, expected) in [
        (vec![chunks[1].clone(), chunks[0].clone()], Some(11)),
        (vec![chunks[0].clone(), chunks[0].clone()], None),
        (chunks.clone(), Some(10)),
        (chunks.clone(), Some(12)),
        (chunks.clone(), Some(0)),
        (chunks.clone(), Some(256 * 1024 * 1024 + 1)),
        (vec![chunks[0].clone(); 65], None),
        (vec![], Some(11)),
    ] {
        let mut files = BTreeMap::from([("prior.bin".into(), bytes_digest(b"prior"))]);
        let before = files.clone();
        assert!(
            retain_artifact_chunks(&records, &selected, &digest, expected, &mut files).is_err()
        );
        assert_eq!(files, before);
    }
    std::fs::remove_file(records.path().join(&chunks[1])).unwrap();
    let mut files = BTreeMap::new();
    assert_eq!(
        retain_artifact_chunks(&records, &chunks, &digest, None, &mut files)
            .unwrap_err()
            .code,
        "record_missing"
    );
    assert!(files.is_empty());
    // The existing delta contract permits a truly empty patch with its exact
    // empty digest, unlike nonempty integrated artifacts.
    retain_artifact_chunks(&records, &[], &bytes_digest(b""), None, &mut files).unwrap();
    assert!(files.is_empty());
}

#[test]
fn chunk_byte_bound_and_conflicting_shared_chunk_cannot_replace_prior_capture() {
    let (_temp, records) = storage();
    let chunks: Vec<String> = vec!["shared.bin".into()];
    records.write_bytes(&chunks[0], b"old").unwrap();
    let mut files = BTreeMap::new();
    retain_artifact_chunks(&records, &chunks, &bytes_digest(b"old"), None, &mut files).unwrap();
    let before = files.clone();
    records.write_bytes(&chunks[0], b"new").unwrap();
    assert!(
        retain_artifact_chunks(&records, &chunks, &bytes_digest(b"new"), None, &mut files).is_err()
    );
    assert_eq!(files, before);
    let oversized = vec![b'x'; 4 * 1024 * 1024 + 1];
    records.write_bytes(&chunks[0], &oversized).unwrap();
    assert!(
        retain_artifact_chunks(
            &records,
            &chunks,
            &bytes_digest(&oversized),
            None,
            &mut BTreeMap::new()
        )
        .is_err()
    );
}
