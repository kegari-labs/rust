use super::*;
use kegari_storage::PrivateDir;
use uuid::Uuid;

fn variant() -> Variant {
    SuiteManifest::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations/manifest.json"))
        .unwrap()
        .variants
        .remove(0)
}
fn scorer_bytes() -> Vec<u8> {
    serde_json::to_vec(&Scorer {
        version: GRADER_VERSION.into(),
        assertions: vec![Assertion::ChangedPaths {
            id: "source".into(),
            paths: BTreeSet::new(),
        }],
    })
    .unwrap()
}
#[test]
fn actual_scorer_is_staged_privately_before_fixture_and_never_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = PrivateDir::create(&root.join("suite")).unwrap();
    let report = PrivateDir::create(&root.join("report")).unwrap();
    let bytes = scorer_bytes();
    source
        .create_bytes_exclusive("scorer.json", &bytes)
        .unwrap();
    let mut variant = variant();
    variant.scorer = "scorer.json".into();
    variant.scorer_digest = bytes_digest(&bytes);
    let operation = Uuid::new_v4();
    let staged = prepare_private_scorer(source.path(), &variant, &report, operation).unwrap();
    let leaf = format!("{}.json", variant.id);
    assert_eq!(
        staged.read_bytes(&leaf, MAX_EVIDENCE_BYTES as u64).unwrap(),
        bytes
    );
    Scorer::load(&staged.path().join(&leaf), &variant.scorer_digest).unwrap();
    assert!(prepare_private_scorer(source.path(), &variant, &report, operation).is_err());
    assert_eq!(
        staged.read_bytes(&leaf, MAX_EVIDENCE_BYTES as u64).unwrap(),
        bytes
    );
    source.write_bytes("scorer.json", b"changed").unwrap();
    assert_eq!(
        prepare_private_scorer(source.path(), &variant, &report, Uuid::new_v4())
            .unwrap_err()
            .code,
        "grader_changed"
    );
    assert_eq!(
        staged.read_bytes(&leaf, MAX_EVIDENCE_BYTES as u64).unwrap(),
        bytes
    );
}
#[test]
#[cfg(unix)]
fn linked_or_substituted_scorer_cannot_enter_private_staging() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = PrivateDir::create(&root.join("suite")).unwrap();
    let report = PrivateDir::create(&root.join("report")).unwrap();
    let bytes = scorer_bytes();
    source
        .create_bytes_exclusive("actual.json", &bytes)
        .unwrap();
    std::os::unix::fs::symlink(
        source.path().join("actual.json"),
        source.path().join("scorer.json"),
    )
    .unwrap();
    let mut variant = variant();
    variant.scorer = "scorer.json".into();
    variant.scorer_digest = bytes_digest(&bytes);
    assert!(prepare_private_scorer(source.path(), &variant, &report, Uuid::new_v4()).is_err());
    std::fs::remove_file(source.path().join("scorer.json")).unwrap();
    std::fs::hard_link(
        source.path().join("actual.json"),
        source.path().join("scorer.json"),
    )
    .unwrap();
    assert_eq!(
        prepare_private_scorer(source.path(), &variant, &report, Uuid::new_v4())
            .unwrap_err()
            .code,
        "scope_violation"
    );
    assert!(!report.path().join("grader").exists());
}
