#![allow(dead_code)]
use super::model::*;
use kegari_digest::canonical_digest;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub fn digest(letter: char) -> String {
    format!("sha256:{}", letter.to_string().repeat(64))
}
pub fn repository() -> RepositoryId {
    RepositoryId {
        hostname: "github.com".into(),
        id: "42".into(),
    }
}
pub fn revision(letter: char) -> Revision {
    Revision {
        commit: letter.to_string().repeat(40),
        tree: letter.to_string().repeat(64),
    }
}
pub fn identity() -> CommitIdentity {
    CommitIdentity {
        name: "Fixture Human".into(),
        email: "fixture@example.test".into(),
    }
}
pub fn metadata() -> SourceCommitMetadata {
    SourceCommitMetadata {
        identity: identity(),
        timestamp: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        message: "feat: preserve source\n\nSigned-off-by: Fixture Human <fixture@example.test>\n"
            .into(),
    }
}
pub fn delta(child: u128) -> DeltaManifest {
    DeltaManifest {
        schema_version: DELTA_SCHEMA,
        git_version: SUPPORTED_GIT_VERSION.into(),
        repository: repository(),
        parent_id: Uuid::from_u128(1),
        child_id: Uuid::from_u128(child),
        work_generation: digest('a'),
        correction_generation: digest('b'),
        scope: WriteScope {
            paths: vec!["src/".into(), "README.md".into()],
        },
        validated_base: revision('1'),
        validated_head: revision('2'),
        staging_parent: revision('3'),
        staging_commit: revision('4'),
        patch_digest: digest('c'),
        paths: vec!["README.md".into(), "src/日本語.rs".into()],
        replacement: None,
    }
}
pub fn replacement() -> DeltaManifest {
    let mut inner = delta(4);
    inner.replacement = Some(ReplacementMapping {
        originals: vec![delta(2), delta(3)],
        reason: "resolve exact overlap".into(),
        staging_result_digest: digest('d'),
    });
    let mut outer = delta(5);
    outer.replacement = Some(ReplacementMapping {
        originals: vec![inner],
        reason: "preserve nested history".into(),
        staging_result_digest: digest('e'),
    });
    outer
}
pub fn review() -> ReviewDelta {
    let original = delta(2);
    ReviewDelta {
        schema_version: 1,
        git_version: original.git_version,
        repository: original.repository,
        parent_id: original.parent_id,
        child_id: original.child_id,
        work_generation: original.work_generation,
        correction_generation: original.correction_generation,
        scope: original.scope,
        base: original.validated_base,
        head: original.validated_head,
        patch_digest: original.patch_digest,
        paths: original.paths,
        replacement_digest: None,
    }
}
pub fn integration() -> ProductionIntegration {
    ProductionIntegration {
        repository: repository(),
        parent_id: Uuid::from_u128(1),
        release_operation: Uuid::from_u128(6),
        pull_request: 17,
        source_candidate_digest: digest('a'),
        accepted_evidence_token: digest('b'),
        acceptance_digest: digest('c'),
        production_parent: revision('1'),
        candidate: revision('2'),
        merged: revision('3'),
        observed_main: revision('4'),
        selected_delta_digests: vec![delta(2).digest().unwrap()],
        selected_patch_digests: vec![digest('c')],
        selected_contributions: BTreeMap::from([(
            delta(2).digest().unwrap(),
            BTreeSet::from([delta(2).digest().unwrap()]),
        )]),
        combined_patch_digest: digest('d'),
    }
}
pub fn candidate() -> CandidateManifest {
    CandidateManifest {
        schema_version: 1,
        operation_id: Uuid::from_u128(7),
        runtime_id: Uuid::from_u128(8),
        repository: repository(),
        parent_id: Uuid::from_u128(1),
        work_generation: digest('a'),
        correction_generation: digest('b'),
        git_version: SUPPORTED_GIT_VERSION.into(),
        production: revision('1'),
        ordered_delta_digests: vec![replacement().digest().unwrap()],
        ordered_patch_digests: vec![digest('c')],
        selected_contributions: BTreeMap::from([(
            replacement().digest().unwrap(),
            replacement().original_contributions().unwrap(),
        )]),
        production_integrations: vec![integration()],
        result: Some(revision('5')),
        combined_patch_digest: Some(digest('f')),
    }
}
fn record<T: Serialize>(map: &mut Map<String, Value>, name: &str, value: &T) {
    map.insert(name.into(), json!({"json": serde_json::to_string(value).unwrap(), "digest": canonical_digest(1, value).unwrap()}));
}
pub fn corpus() -> Value {
    let mut records = Map::new();
    record(&mut records, "repository", &repository());
    record(&mut records, "revision", &revision('1'));
    record(
        &mut records,
        "branch",
        &BranchName {
            title_digest: digest('a'),
            suffix: "Ångström-日本語".into(),
            name: "CHILD-12-Ångström-日本語".into(),
        },
    );
    record(&mut records, "scope", &delta(2).scope);
    record(&mut records, "delta", &delta(2));
    record(&mut records, "replacement_delta", &replacement());
    record(
        &mut records,
        "replacement",
        replacement().replacement.as_ref().unwrap(),
    );
    record(
        &mut records,
        "empty_replacement",
        &ReplacementMapping {
            originals: vec![],
            reason: "empty retained data".into(),
            staging_result_digest: digest('a'),
        },
    );
    record(&mut records, "review", &review());
    let mut replaced_review = review();
    replaced_review.replacement_digest = Some(digest('e'));
    record(&mut records, "replaced_review", &replaced_review);
    record(&mut records, "integration", &integration());
    record(&mut records, "candidate", &candidate());
    let mut pending = candidate();
    pending.result = None;
    pending.combined_patch_digest = None;
    record(&mut records, "pending_candidate", &pending);
    record(&mut records, "commit_identity", &identity());
    record(&mut records, "source_metadata", &metadata());
    record(
        &mut records,
        "source_entry",
        &SourceTreeEntry {
            path: "src/日本語.rs".into(),
            mode: "100755".into(),
            object: Some("a".repeat(40)),
        },
    );
    record(
        &mut records,
        "deleted_entry",
        &SourceTreeEntry {
            path: "removed.txt".into(),
            mode: "100644".into(),
            object: None,
        },
    );
    record(
        &mut records,
        "git_commit_identity",
        &GitCommitIdentity {
            name: "Fixture Human".into(),
            email: "fixture@example.test".into(),
            date: "2023-11-14T23:13:20+01:00".into(),
        },
    );
    json!({"records": records, "methods": {
        "delta": delta(2).digest().unwrap(), "delta_review": delta(2).review_digest().unwrap(),
        "replacement": replacement().digest().unwrap(), "replacement_review": replacement().review_digest().unwrap(),
        "contributions": replacement().original_contributions().unwrap(),
        "review": review().digest().unwrap(), "integration": integration().digest().unwrap(),
    }})
}
