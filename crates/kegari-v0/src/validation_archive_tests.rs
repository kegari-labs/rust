//! Data-only archive fixtures. These do not create ValidationPlan, worker,
//! execution or native capabilities. Signed data is tested as data only.
use super::*;
use crate::evidence::{Generation, WorkIdentity};
use crate::execution::{
    CheckExecutionRecord, DiscoveredCheckProvenance, ExecutionEvidenceClass, SourceSnapshot,
};
use crate::validation::{
    DiscoveryJournalReference, DiscoveryReadContent, DiscoveryReadReference, ValidationPlanRecord,
    ValidationRequirement,
};
use chrono::{DateTime, Utc};
use kegari_digest::canonical_digest;
use kegari_execution::CommandObservation;
use kegari_execution::CommandOutcome;
use kegari_git::{RepositoryId, Revision};
use kegari_platform::BinaryIdentity;
use std::path::PathBuf;

fn at(seconds: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(seconds, 0).unwrap()
}

fn base_fixture() -> (ValidationEvidence, RevisionBinding) {
    let digest = bytes_digest(b"descriptive fixture");
    let repository = RepositoryId {
        hostname: "github.com".into(),
        id: "42".into(),
    };
    let revision = Revision {
        commit: "a".repeat(40),
        tree: "b".repeat(40),
    };
    let work = WorkIdentity {
        linear_endpoint: "https://linear.invalid/graphql".into(),
        linear_workspace: Uuid::from_u128(1),
        parent: Uuid::from_u128(2),
        child: Some(Uuid::from_u128(3)),
        generation: Generation {
            authorization: digest.clone(),
            definition: digest.clone(),
            correction: digest.clone(),
            scope: digest.clone(),
        },
    };
    let binding = ExecutionBinding {
        runtime_id: Uuid::from_u128(4),
        operation_id: Uuid::from_u128(5),
        work,
        repository: repository.clone(),
        revision: revision.clone(),
        candidate_vector_digest: None,
        isolation_digest: digest.clone(),
    };
    let source = SourceSnapshot {
        revision: revision.clone(),
        files: BTreeMap::new(),
    };
    let plan = ValidationPlanRecord {
        schema_version: 1,
        original_binding: binding.clone(),
        binding: binding.clone(),
        correction: None,
        kind: CheckKind::Local,
        workflow_digest: digest.clone(),
        instruction_digest: digest.clone(),
        tool_contract_digest: digest.clone(),
        context_digest: digest.clone(),
        source_digest: source.digest().unwrap(),
        predecessor: None,
        ordinal: 1,
        reason: "actual task requires the declared check".into(),
        requirements: vec![ValidationRequirement {
            id: "behavior".into(),
            statement: "requested behavior".into(),
            rationale: "repository evidence".into(),
            argv: vec!["unfamiliar-tool".into(), "verify".into()],
            cwd: ".".into(),
            services: vec![],
            artifacts: vec![],
        }],
        supporting_reads: vec![DiscoveryReadReference {
            schema_version: 1,
            operation_id: binding.operation_id,
            call_id: Uuid::from_u128(6),
            turn: "turn".into(),
            instruction_digest: digest.clone(),
            correction_input_digest: digest.clone(),
            result_digest: digest.clone(),
            journal: DiscoveryJournalReference {
                leaf: "event.json".into(),
                digest: digest.clone(),
                bytes: 10,
            },
            content: DiscoveryReadContent::Context {
                key: "task".into(),
                digest: digest.clone(),
                bytes: 10,
            },
        }],
        created_at: at(1),
    };
    let invocation = Uuid::from_u128(7);
    let transcript = PathBuf::from(format!(
        "/owned/scratch/validation-transcript-{invocation}.json"
    ));
    let binary = BinaryIdentity {
        path: "/usr/bin/fixture".into(),
        digest: digest.clone(),
        bytes: 1,
    };
    let observation = CommandObservation {
        schema_version: 1,
        requested_argv: plan.requirements[0].argv.clone(),
        selected_executable: binary.path.clone(),
        canonical_executable: binary.clone(),
        launcher: binary,
        cwd: "/owned/repository".into(),
        environment: BTreeMap::from([("HOME".into(), "/owned/scratch".into())]),
        original_isolation_digest: digest.clone(),
        isolation_digest: digest.clone(),
        launch_digest: digest.clone(),
        backend_policy_digest: format!("sha256:v5:{}", "a".repeat(64)),
    };
    let plan_digest = canonical_digest(1, &plan).unwrap();
    let attempt = CheckExecutionRecord {
        schema_version: 4,
        discovered: Some(DiscoveredCheckProvenance {
            schema_version: 1,
            plan: plan.clone(),
            plan_digest: plan_digest.clone(),
            invocation_id: invocation,
            observation: observation.clone(),
            service_observations: vec![],
            prepared_at: at(2),
            deadline_at: at(12),
            command_timeout_ms: 10_000,
            output_bytes: 1024,
            transcript: transcript.clone(),
            artifacts: vec![transcript.clone()],
        }),
        correction: None,
        bundle: None,
        binding: binding.clone(),
        workflow_digest: digest.clone(),
        check_name: "behavior".into(),
        kind: CheckKind::Local,
        before: source.clone(),
        after: source,
        artifacts: vec![ArtifactProof {
            path: transcript,
            digest: digest.clone(),
            bytes: 10,
            validation_operation: binding.operation_id,
            candidate_vector_digest: None,
        }],
        command: CommandOutcome {
            schema_version: 1,
            argv: observation.requested_argv,
            cwd: observation.cwd.clone(),
            isolation_digest: digest,
            exit_code: Some(0),
            termination_signal: None,
            timed_out: false,
            stdout: "observed output".into(),
            stderr: String::new(),
            output_truncated: false,
            elapsed_ms: 1,
        },
        started_at: at(2),
        completed_at: at(3),
        evidence_class: ExecutionEvidenceClass::ConfinedExecution,
        successful: true,
        blocker: None,
    };
    let revision = RevisionBinding {
        repository,
        base: "c".repeat(40),
        head: revision.commit.clone(),
        tested_commit: revision.commit,
        tested_tree: revision.tree,
    };
    (
        ValidationEvidence {
            schema_version: 1,
            plan_digest,
            original_cwd: "/owned/repository".into(),
            history: vec![plan],
            attempts: vec![attempt],
        },
        revision,
    )
}

/// Shared descriptive data for parent-origin readback tests. It contains one
/// superseded failed attempt and one selected successful attempt, each with
/// different original artifact bytes. It mints no original worker capability.
pub(crate) fn descriptive_fixture() -> (
    ValidationArchiveNamespace,
    ValidationEvidence,
    BTreeMap<String, Vec<u8>>,
) {
    let (mut evidence, _) = base_fixture();
    let first_bytes = b"actual retained failed-attempt transcript".to_vec();
    let second_bytes = b"actual retained successful-attempt transcript".to_vec();
    let first = &mut evidence.attempts[0];
    first.successful = false;
    first.blocker = Some("observed nonzero exit".into());
    first.command.exit_code = Some(1);
    first.artifacts[0].bytes = first_bytes.len() as u64;
    first.artifacts[0].digest = bytes_digest(&first_bytes);
    let mut next = evidence.history[0].clone();
    next.ordinal = 2;
    next.predecessor = Some(evidence.plan_digest.clone());
    next.created_at = at(4);
    next.reason = "retained failure requires another actual attempt".into();
    evidence.plan_digest = canonical_digest(1, &next).unwrap();
    let mut successful = first.clone();
    successful.successful = true;
    successful.blocker = None;
    successful.command.exit_code = Some(0);
    successful.started_at = at(5);
    successful.completed_at = at(6);
    let provenance = successful.discovered.as_mut().unwrap();
    provenance.plan = next.clone();
    provenance.plan_digest = evidence.plan_digest.clone();
    provenance.invocation_id = Uuid::from_u128(8);
    provenance.prepared_at = at(5);
    provenance.deadline_at = at(15);
    provenance.transcript = format!(
        "/owned/scratch/validation-transcript-{}.json",
        provenance.invocation_id
    )
    .into();
    provenance.artifacts = vec![provenance.transcript.clone()];
    successful.artifacts[0].path = provenance.transcript.clone();
    successful.artifacts[0].bytes = second_bytes.len() as u64;
    successful.artifacts[0].digest = bytes_digest(&second_bytes);
    evidence.history.push(next);
    evidence.attempts.push(successful);
    let namespace = ValidationArchiveNamespace {
        workspace_id: Uuid::from_u128(9),
        runtime_id: evidence.history[0].binding.runtime_id,
    };
    let bytes = BTreeMap::from([
        (
            canonical_digest(1, &evidence.attempts[0].artifacts[0]).unwrap(),
            first_bytes,
        ),
        (
            canonical_digest(1, &evidence.attempts[1].artifacts[0]).unwrap(),
            second_bytes,
        ),
    ]);
    (namespace, evidence, bytes)
}

struct Fixture {
    _temp: tempfile::TempDir,
    records: PrivateDir,
    key: AttestationKey,
    namespace: ValidationArchiveNamespace,
    evidence: ValidationEvidence,
    artifacts: BTreeMap<String, Vec<u8>>,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let records = PrivateDir::create(&root.join("records")).unwrap();
        let (namespace, evidence, artifacts) = descriptive_fixture();
        Self {
            _temp: temp,
            records,
            key: AttestationKey::new("archive-fixture".into(), vec![71; 32]).unwrap(),
            namespace,
            evidence,
            artifacts,
        }
    }
    fn retain(&self) -> Result<ResolvedValidationArchive> {
        retain(
            &self.records,
            &self.key,
            self.namespace.clone(),
            &self.evidence,
            |proof| {
                self.artifacts
                    .get(&canonical_digest(1, proof)?)
                    .cloned()
                    .ok_or_else(|| invalid("fixture artifact missing"))
            },
        )
    }
    fn manifest(&self, reference: &ValidationArchiveReference) -> Signed<Manifest> {
        serde_json::from_slice(
            &self
                .records
                .read_bytes(
                    &manifest_leaf(&reference.manifest_digest).unwrap(),
                    MANIFEST_BYTES as u64,
                )
                .unwrap(),
        )
        .unwrap()
    }
    fn store_manifest(&self, manifest: &Manifest) -> ValidationArchiveReference {
        let bytes = encode(&self.key.sign(PURPOSE, manifest).unwrap(), MANIFEST_BYTES).unwrap();
        let reference = ValidationArchiveReference {
            schema_version: 1,
            namespace: manifest.namespace.clone(),
            manifest_digest: bytes_digest(&bytes),
            evidence_digest: manifest.evidence_digest.clone(),
        };
        store_exact(
            &self.records,
            &manifest_leaf(&reference.manifest_digest).unwrap(),
            &bytes,
        )
        .unwrap();
        reference
    }
}

#[test]
fn complete_history_and_failed_artifacts_roundtrip_without_scratch_paths() {
    let f = Fixture::new();
    // These paths were never created: callback supplies original retained bytes.
    assert!(
        f.evidence
            .attempts
            .iter()
            .flat_map(|a| &a.artifacts)
            .all(|p| !p.path.exists())
    );
    let archived = f.retain().unwrap();
    assert_eq!(archived.evidence(), &f.evidence);
    assert!(!archived.evidence().attempts[0].successful);
    assert_eq!(archived.evidence().history.len(), 2);
    let cold = read(&f.records, &f.key, &f.namespace, archived.reference()).unwrap();
    for proof in f.evidence.attempts.iter().flat_map(|a| &a.artifacts) {
        assert_eq!(
            cold.read_artifact(proof).unwrap(),
            f.artifacts[&canonical_digest(1, proof).unwrap()]
        );
    }
    assert_eq!(archived.reference(), f.retain().unwrap().reference());
    assert_eq!(cold.retained_files().len(), 4); // manifest, history, both artifact chunks
    for (leaf, digest) in cold.retained_files() {
        assert_eq!(
            bytes_digest(&f.records.read_bytes(leaf, CHUNK_BYTES as u64).unwrap()),
            *digest
        );
    }
    let before = cold.retained_files().clone();
    let leaf = chunk_leaf(&f.evidence.attempts[0].artifacts[0].digest).unwrap();
    f.records.write_bytes(&leaf, b"later corruption").unwrap();
    assert_eq!(cold.retained_files(), &before); // same successful read, not fresh hashes
    assert!(read(&f.records, &f.key, &f.namespace, archived.reference()).is_err());
}

#[test]
fn namespace_key_reference_and_unknown_fields_cannot_substitute_owner() {
    let f = Fixture::new();
    let archived = f.retain().unwrap();
    let mut other = f.namespace.clone();
    other.workspace_id = Uuid::new_v4();
    assert!(read(&f.records, &f.key, &other, archived.reference()).is_err());
    other = f.namespace.clone();
    other.runtime_id = Uuid::new_v4();
    assert!(read(&f.records, &f.key, &other, archived.reference()).is_err());
    let wrong_key = AttestationKey::new(f.key.id().into(), vec![72; 32]).unwrap();
    assert_eq!(
        read(&f.records, &wrong_key, &f.namespace, archived.reference())
            .unwrap_err()
            .code,
        "invalid_attestation"
    );
    let mut reference = archived.reference().clone();
    reference.evidence_digest = canonical_digest(1, &"different").unwrap();
    assert!(read(&f.records, &f.key, &f.namespace, &reference).is_err());
    let mut value = serde_json::to_value(archived.reference()).unwrap();
    value["qualified"] = true.into();
    assert!(serde_json::from_value::<ValidationArchiveReference>(value).is_err());
    let mut wrong_runtime = f.namespace.clone();
    wrong_runtime.runtime_id = Uuid::new_v4();
    assert!(
        retain(&f.records, &f.key, wrong_runtime, &f.evidence, |_| panic!(
            "refuse before callback"
        ))
        .is_err()
    );
}

#[test]
fn missing_failed_artifact_and_manifest_cannot_be_hidden_by_latest_success() {
    let f = Fixture::new();
    let archived = f.retain().unwrap();
    let failed = &f.evidence.attempts[0].artifacts[0];
    let leaf = chunk_leaf(&failed.digest).unwrap();
    std::fs::remove_file(f.records.path().join(&leaf)).unwrap();
    assert_eq!(
        read(&f.records, &f.key, &f.namespace, archived.reference())
            .unwrap_err()
            .code,
        "record_missing"
    );
    let bytes = &f.artifacts[&canonical_digest(1, failed).unwrap()];
    f.records.create_bytes_exclusive(&leaf, bytes).unwrap();
    let mut manifest = f.manifest(archived.reference()).payload;
    manifest.artifacts.retain(|a| a.proof != *failed);
    manifest.artifact_bytes -= failed.bytes;
    let omitted = f.store_manifest(&manifest); // signed but incomplete descriptive inventory
    assert!(read(&f.records, &f.key, &f.namespace, &omitted).is_err());
    std::fs::remove_file(
        f.records
            .path()
            .join(manifest_leaf(&archived.reference().manifest_digest).unwrap()),
    )
    .unwrap();
    assert!(read(&f.records, &f.key, &f.namespace, archived.reference()).is_err());
}

#[test]
fn chunk_reordering_substitution_and_oversized_signed_claims_refuse() {
    let mut f = Fixture::new();
    let mut large = vec![b'a'; CHUNK_BYTES];
    large.extend_from_slice(b"different-tail");
    let proof = &mut f.evidence.attempts[0].artifacts[0];
    let old = canonical_digest(1, proof).unwrap();
    proof.bytes = large.len() as u64;
    proof.digest = bytes_digest(&large);
    f.artifacts.remove(&old);
    f.artifacts
        .insert(canonical_digest(1, proof).unwrap(), large);
    let archived = f.retain().unwrap();
    let original = f.manifest(archived.reference()).payload;
    let index = original
        .artifacts
        .iter()
        .position(|a| a.blob.chunks.len() == 2)
        .unwrap();
    let large_proof = &original.artifacts[index].proof;
    assert_eq!(
        archived.read_artifact(large_proof).unwrap(),
        f.artifacts[&canonical_digest(1, large_proof).unwrap()]
    );
    let mut changed = original.clone();
    changed.artifacts[index].blob.chunks.swap(0, 1);
    let reference = f.store_manifest(&changed);
    assert!(read(&f.records, &f.key, &f.namespace, &reference).is_err());
    let mut changed = original.clone();
    changed.artifacts[index].blob.chunks[0].digest = bytes_digest(b"foreign");
    let reference = f.store_manifest(&changed);
    assert!(read(&f.records, &f.key, &f.namespace, &reference).is_err());
    let mut changed = original;
    changed.artifact_bytes = ARTIFACT_BYTES + 1;
    let reference = f.store_manifest(&changed);
    assert_eq!(
        read(&f.records, &f.key, &f.namespace, &reference)
            .unwrap_err()
            .code,
        "validation_archive_limit"
    );
}

#[test]
fn complete_history_above_old_inline_record_limit_has_compact_authenticated_reference() {
    let mut f = Fixture::new();
    f.evidence.attempts[0].command.stdout = "observed failed output\n".repeat(15_000);
    f.evidence.attempts[0]
        .discovered
        .as_mut()
        .unwrap()
        .output_bytes = 512 * 1024;
    let original = encode(&f.evidence, MAX_VALIDATION_EVIDENCE_BYTES).unwrap();
    assert!(original.len() > 300 * 1024);
    assert!(original.len() < 1024 * 1024);
    let archived = f.retain().unwrap();
    assert!(encode(archived.reference(), MANIFEST_BYTES).unwrap().len() < 1024);
    let cold = read(&f.records, &f.key, &f.namespace, archived.reference()).unwrap();
    assert_eq!(
        encode(cold.evidence(), MAX_VALIDATION_EVIDENCE_BYTES).unwrap(),
        original
    );
    assert_eq!(cold.evidence().history.len(), 2);
    assert_eq!(cold.evidence().attempts.len(), 2);
    assert!(!cold.evidence().attempts[0].successful);
    for proof in f.evidence.attempts.iter().flat_map(|a| &a.artifacts) {
        assert_eq!(
            cold.read_artifact(proof).unwrap(),
            f.artifacts[&canonical_digest(1, proof).unwrap()]
        );
    }
}

#[test]
fn declared_aggregate_and_evidence_bounds_refuse_before_artifact_reads_or_writes() {
    let mut f = Fixture::new();
    f.evidence.attempts[0].artifacts[0].bytes = ARTIFACT_BYTES;
    let result = retain(&f.records, &f.key, f.namespace.clone(), &f.evidence, |_| {
        panic!("overbound callback")
    });
    assert_eq!(result.unwrap_err().code, "validation_archive_limit");
    assert_eq!(std::fs::read_dir(f.records.path()).unwrap().count(), 0);
    let (namespace, mut evidence, _) = descriptive_fixture();
    evidence.attempts[0].command.stdout = "x".repeat(MAX_VALIDATION_EVIDENCE_BYTES);
    assert_eq!(
        retain(&f.records, &f.key, namespace, &evidence, |_| panic!(
            "overbound callback"
        ))
        .unwrap_err()
        .code,
        "validation_archive_limit"
    );
    assert_eq!(std::fs::read_dir(f.records.path()).unwrap().count(), 0);
}

#[test]
fn original_callback_mismatch_and_existing_partial_leaf_never_get_repaired() {
    let f = Fixture::new();
    let failed = &f.evidence.attempts[0].artifacts[0];
    let leaf = chunk_leaf(&failed.digest).unwrap();
    f.records.create_bytes_exclusive(&leaf, b"partial").unwrap();
    assert!(f.retain().is_err());
    assert_eq!(
        f.records.read_bytes(&leaf, CHUNK_BYTES as u64).unwrap(),
        b"partial"
    );
    assert!(
        !f.records
            .records()
            .unwrap()
            .iter()
            .any(|n| n.starts_with("validation-archive-"))
    );
    std::fs::remove_file(f.records.path().join(leaf)).unwrap();
    assert!(
        retain(
            &f.records,
            &f.key,
            f.namespace.clone(),
            &f.evidence,
            |_| Ok(b"wrong original bytes".to_vec())
        )
        .is_err()
    );
    assert!(
        !f.records
            .records()
            .unwrap()
            .iter()
            .any(|n| n.starts_with("validation-archive-"))
    );
}

#[test]
fn integrated_archive_binds_entire_vector_and_signed_manifest_to_full_history() {
    let mut f = Fixture::new();
    let vector = bytes_digest(b"actual complete candidate vector description");
    let mut previous = None;
    f.evidence.schema_version = 2;
    for (plan, attempt) in f.evidence.history.iter_mut().zip(&mut f.evidence.attempts) {
        plan.kind = CheckKind::Integrated;
        plan.original_binding.candidate_vector_digest = Some(vector.clone());
        plan.binding.candidate_vector_digest = Some(vector.clone());
        plan.predecessor = previous;
        let digest = canonical_digest(1, plan).unwrap();
        previous = Some(digest.clone());
        attempt.kind = CheckKind::Integrated;
        attempt.binding = plan.binding.clone();
        let provenance = attempt.discovered.as_mut().unwrap();
        provenance.plan = plan.clone();
        provenance.plan_digest = digest;
        for proof in &mut attempt.artifacts {
            let old = canonical_digest(1, proof).unwrap();
            let bytes = f.artifacts.remove(&old).unwrap();
            proof.candidate_vector_digest = Some(vector.clone());
            f.artifacts
                .insert(canonical_digest(1, proof).unwrap(), bytes);
        }
    }
    f.evidence.plan_digest = previous.unwrap();
    let archived = f.retain().unwrap();
    assert_eq!(archived.evidence(), &f.evidence);
    assert!(
        archived
            .evidence()
            .attempts
            .iter()
            .all(|a| a.binding.candidate_vector_digest.as_ref() == Some(&vector))
    );
    let mut manifest = f.manifest(archived.reference()).payload;
    manifest.binding.candidate_vector_digest = Some(bytes_digest(b"substituted vector"));
    let substituted = f.store_manifest(&manifest);
    assert!(read(&f.records, &f.key, &f.namespace, &substituted).is_err());
    let mut manifest = f.manifest(archived.reference()).payload;
    manifest.original_binding.work.generation.scope = bytes_digest(b"different original scope");
    let substituted = f.store_manifest(&manifest);
    assert!(read(&f.records, &f.key, &f.namespace, &substituted).is_err());
}

#[cfg(unix)]
#[test]
fn same_byte_hardlink_or_symlink_substitution_is_not_protected_archive_storage() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let archived = f.retain().unwrap();
    let proof = &f.evidence.attempts[0].artifacts[0];
    let path = f.records.path().join(chunk_leaf(&proof.digest).unwrap());
    let sibling = f.records.path().join("same-byte-alias.bin");
    std::fs::hard_link(&path, &sibling).unwrap();
    assert_eq!(
        read(&f.records, &f.key, &f.namespace, archived.reference())
            .unwrap_err()
            .code,
        "unsafe_storage"
    );
    std::fs::remove_file(&path).unwrap();
    symlink(&sibling, &path).unwrap();
    assert!(read(&f.records, &f.key, &f.namespace, archived.reference()).is_err());
}
