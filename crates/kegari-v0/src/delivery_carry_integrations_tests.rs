//! Data-only complete-inventory tests. These records are deliberately synthetic;
//! no test here constructs a VerifiedProductionIntegration or a carry capability.
use super::*;
use kegari_git::{CandidateManifest, ProductionIntegration, Revision};

fn revision(value: char) -> Revision {
    Revision {
        commit: value.to_string().repeat(40),
        tree: value.to_string().repeat(40),
    }
}
fn fixture() -> (CandidateManifest, ProductionIntegration) {
    let repository = RepositoryId {
        hostname: "github.com".into(),
        id: "42".into(),
    };
    let record = ProductionIntegration {
        repository: repository.clone(),
        parent_id: Uuid::from_u128(1),
        release_operation: Uuid::from_u128(2),
        pull_request: 7,
        source_candidate_digest: canonical_digest(1, &"source candidate").unwrap(),
        accepted_evidence_token: canonical_digest(1, &"original evidence").unwrap(),
        acceptance_digest: canonical_digest(1, &"original acceptance").unwrap(),
        production_parent: revision('a'),
        candidate: revision('b'),
        merged: revision('c'),
        observed_main: revision('d'),
        selected_delta_digests: vec![canonical_digest(1, &"delta").unwrap()],
        selected_patch_digests: vec![canonical_digest(1, &"patch").unwrap()],
        selected_contributions: BTreeMap::from([(
            "child".into(),
            BTreeSet::from(["original contribution".into()]),
        )]),
        combined_patch_digest: canonical_digest(1, &"combined patch").unwrap(),
    };
    let candidate = CandidateManifest {
        schema_version: 1,
        operation_id: Uuid::from_u128(3),
        runtime_id: Uuid::from_u128(4),
        repository,
        parent_id: record.parent_id,
        work_generation: canonical_digest(1, &"work").unwrap(),
        correction_generation: canonical_digest(1, &"correction").unwrap(),
        git_version: kegari_git::SUPPORTED_GIT_VERSION.into(),
        production: record.observed_main.clone(),
        ordered_delta_digests: vec![],
        ordered_patch_digests: vec![],
        selected_contributions: BTreeMap::new(),
        production_integrations: vec![record.clone()],
        result: Some(revision('e')),
        combined_patch_digest: Some(canonical_digest(1, &"new patch").unwrap()),
    };
    (candidate, record)
}
fn rejected(candidates: &[CandidateManifest], records: &[ProductionIntegration], message: &str) {
    let error = require_complete_records(candidates, records.iter()).unwrap_err();
    assert_eq!(error.code, "production_provenance");
    assert_eq!(error.message, message);
}

#[test]
fn complete_record_bijection_rejects_missing_extra_duplicate_and_substitution() {
    let (candidate, record) = fixture();
    let candidates = [candidate.clone()];
    require_complete_records(&candidates, [&record]).unwrap();
    rejected(
        &candidates,
        &[],
        "current carry lacks its complete declared production integrations",
    );
    let message =
        "observed carry integration is extra, duplicate or differs from its signed full record";
    rejected(&candidates, &[record.clone(), record.clone()], message);
    let mut extra = record.clone();
    extra.release_operation = Uuid::from_u128(999);
    rejected(&candidates, &[record.clone(), extra], message);
    let mut changed = record.clone();
    changed.acceptance_digest = canonical_digest(1, &"substituted acceptance").unwrap();
    rejected(&candidates, &[changed], message);
    let mut changed = record.clone();
    changed.selected_contributions.clear();
    rejected(&candidates, &[changed], message);
    let mut changed = record.clone();
    changed.observed_main = revision('f');
    rejected(&candidates, &[changed], message);

    let message =
        "signed carry integration inventory is duplicate or changes its exact candidate source";
    let mut duplicate = candidate.clone();
    duplicate.production_integrations.push(record.clone());
    rejected(&[duplicate], std::slice::from_ref(&record), message);
    for field in 0..3 {
        let mut changed = candidate.clone();
        match field {
            0 => changed.repository.id = "84".into(),
            1 => changed.parent_id = Uuid::from_u128(999),
            _ => changed.production = revision('f'),
        }
        rejected(&[changed], std::slice::from_ref(&record), message);
    }
}

#[test]
fn multiple_repositories_require_each_exact_declared_record() {
    let (first, first_record) = fixture();
    let mut second = first.clone();
    second.repository.id = "84".into();
    second.production_integrations[0].repository = second.repository.clone();
    let second_record = second.production_integrations[0].clone();
    let candidates = [first, second];
    require_complete_records(&candidates, [&second_record, &first_record]).unwrap();
    rejected(
        &candidates,
        &[first_record],
        "current carry lacks its complete declared production integrations",
    );
    require_complete_records(&[], std::iter::empty()).unwrap();
}
