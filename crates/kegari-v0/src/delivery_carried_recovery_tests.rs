//! Storage/selection component tests only. Synthetic signed record data below is
//! intentionally missing actual upstream receipts; it never becomes current
//! PublishedCarry, acceptance readiness, or a native joined worker capability.
use super::*;

fn digest(value: &str) -> String {
    canonical_digest(1, &value).unwrap()
}
fn reference() -> ReviewReceiptReference {
    ReviewReceiptReference {
        operation: Uuid::new_v4(),
        receipt_digest: digest("whole fixture receipt"),
    }
}
fn candidate(id: &str, existing: bool) -> kegari_git::CandidateManifest {
    kegari_git::CandidateManifest {
        schema_version: 1,
        operation_id: Uuid::new_v4(),
        runtime_id: Uuid::new_v4(),
        repository: kegari_git::RepositoryId {
            hostname: "github.invalid".into(),
            id: id.into(),
        },
        parent_id: Uuid::new_v4(),
        work_generation: digest("work"),
        correction_generation: digest("correction"),
        git_version: "fixture data only".into(),
        production: kegari_git::Revision {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        },
        ordered_delta_digests: if existing {
            vec![]
        } else {
            vec![digest("delta")]
        },
        ordered_patch_digests: if existing {
            vec![]
        } else {
            vec![digest("patch")]
        },
        selected_contributions: BTreeMap::new(),
        production_integrations: vec![],
        result: Some(kegari_git::Revision {
            commit: "c".repeat(40),
            tree: "d".repeat(40),
        }),
        combined_patch_digest: Some(digest("combined")),
    }
}
fn binding(candidates: &[kegari_git::CandidateManifest]) -> CarryRecoveryBinding {
    CarryRecoveryBinding {
        schema_version: 1,
        configuration_revision: digest("config"),
        vector_digest: digest("vector"),
        integrated: reference(),
        adequacy: None,
        review: reference(),
        production: candidates
            .iter()
            .map(|c| ProductionSelection {
                repository: c.repository.clone(),
                owner: reference(),
                existing: c.ordered_delta_digests.is_empty(),
                proof_digest: digest(&c.repository.id),
            })
            .collect(),
    }
}
#[test]
fn selected_full_production_vector_rejects_omissions_duplicates_aliases_and_wrong_kind() {
    let candidates = vec![candidate("81", false), candidate("84", true)];
    let expected = binding(&candidates);
    expected.validate_shape(&candidates).unwrap(); // Data contract only.
    let legacy = serde_json::to_value(&expected).unwrap();
    assert!(legacy.get("adequacy").is_none());
    let mut discovered = expected.clone();
    discovered.schema_version = 2;
    discovered.adequacy = Some(reference());
    discovered.validate_shape(&candidates).unwrap();
    assert_eq!(
        discovered
            .phases()
            .map(|(_, phase)| phase)
            .collect::<Vec<_>>(),
        vec![
            "parent_integrated_validation",
            "parent_integrated_review",
            "parent_evidence_review"
        ]
    );
    let mut missing_adequacy = discovered.clone();
    missing_adequacy.adequacy = None;
    assert!(missing_adequacy.validate_shape(&candidates).is_err());
    let mut aliased_adequacy = discovered.clone();
    aliased_adequacy.adequacy = Some(aliased_adequacy.integrated.clone());
    assert!(aliased_adequacy.validate_shape(&candidates).is_err());
    let mut relabelled = discovered;
    relabelled.schema_version = 1;
    assert!(relabelled.validate_shape(&candidates).is_err());

    let mut missing = expected.clone();
    missing.production.pop();
    assert_eq!(
        missing.validate_shape(&candidates).unwrap_err().message,
        "carry recovery omits the complete production vector"
    );
    let mut duplicate = expected.clone();
    duplicate.production[1] = duplicate.production[0].clone();
    assert_eq!(
        duplicate.validate_shape(&candidates).unwrap_err().message,
        "carry recovery production selections are duplicate or differ from the vector"
    );
    let mut owner_alias = expected.clone();
    owner_alias.production[0].owner = owner_alias.integrated.clone();
    assert_eq!(
        owner_alias.validate_shape(&candidates).unwrap_err().code,
        "acceptance_carry"
    );
    let mut wrong_kind = expected.clone();
    wrong_kind.production[1].existing = false;
    assert_eq!(
        wrong_kind.validate_shape(&candidates).unwrap_err().code,
        "acceptance_carry"
    );
    let mut changed_owner = expected.clone();
    changed_owner.production[0].owner.receipt_digest = "unbound".into();
    assert!(changed_owner.validate_shape(&candidates).is_err());
    let mut extra = expected.clone();
    extra.production.push(ProductionSelection {
        repository: candidate("99", true).repository,
        owner: reference(),
        existing: true,
        proof_digest: digest("other"),
    });
    assert_eq!(
        extra.validate_shape(&candidates).unwrap_err().code,
        "acceptance_carry"
    );
}

fn synthetic_record(key: &AttestationKey) -> Signed<CarryPublicationRecord> {
    let runtime = Uuid::new_v4();
    let workspace = Uuid::new_v4();
    let work = WorkIdentity {
        linear_endpoint: "https://linear.invalid/graphql".into(),
        linear_workspace: Uuid::new_v4(),
        parent: Uuid::new_v4(),
        child: None,
        generation: evidence::Generation {
            authorization: digest("auth"),
            definition: digest("definition"),
            correction: digest("correction"),
            scope: digest("scope"),
        },
    };
    let now = Utc::now();
    let evidence = evidence::ParentEvidence {
        archived_integrated_validation: None,
        integrated_validation: None,
        schema_version: 1,
        work: work.clone(),
        required_children: BTreeSet::new(),
        e2e_child: Uuid::new_v4(),
        child_results: BTreeMap::new(),
        required_contributions: BTreeMap::new(),
        executive_summary: "original data".into(),
        touched_components: vec![],
        fulfilled_criteria: BTreeSet::new(),
        caveats: vec![],
        go: true,
        baselines: vec![],
        candidates: vec![candidate("81", false)],
        dependency_snapshot_digest: digest("deps"),
        production_targets: BTreeSet::new(),
        integrated_results: vec![],
        visual: false,
        video: None,
        publication_operation: Uuid::new_v4(),
        created_at: now,
        supersedes: None,
    };
    let parent = ParentResultRecord {
        schema_version: 1,
        runtime_id: runtime,
        contract_digest: digest("contract"),
        vector_digest: digest("vector"),
        integrated_digest: digest("integrated"),
        child_records: vec![],
        coordinator_claim_digest: digest("claim"),
        evidence,
    };
    let original_parent = key.sign("parent_result", &parent).unwrap();
    let mut current = parent;
    current.evidence.executive_summary = "current data".into();
    current.evidence.publication_operation = Uuid::new_v4();
    let current_parent = key.sign("parent_result", &current).unwrap();
    let original_token = original_parent.payload.evidence.token().unwrap();
    let current_token = current_parent.payload.evidence.token().unwrap();
    let writer = Uuid::new_v4();
    let acceptance = evidence::Acceptance {
        schema_version: 1,
        runtime_id: runtime,
        operation_id: Uuid::new_v4(),
        work: work.clone(),
        reviewed_token: original_token.clone(),
        control_actor: "operator".into(),
        linear_actor: Uuid::new_v4().to_string(),
        provider_writer: writer.to_string(),
        accepted_at: now,
    };
    let original_acceptance = key.sign("human_acceptance", &acceptance).unwrap();
    // This is serializable reference data only, with deliberately nonexistent
    // receipts. The actual retained-origin reader must reject it.
    let publication_reference = || {
        json!({"comment":Uuid::new_v4(),"writer":writer,
        "body_digest":digest("body"),"receipt":Uuid::new_v4(),"receipt_digest":digest("receipt")})
    };
    let original: AcceptedOriginRef = serde_json::from_value(json!({
        "runtime":runtime,"workspace":workspace,"work":work,"token":original_token,
        "acceptance_digest":original_acceptance.digest().unwrap(),"parent":publication_reference(),
        "acceptance":publication_reference(),"vector_leaf":format!("parent-vector-{}.json",&digest("vector")[7..]),
        "vector_signed_digest":digest("signed vector"),"vector_digest":digest("vector"),
        "contract_digest":digest("contract"),"baselines_digest":digest("baselines"),
        "candidates_digest":digest("candidates"),"slices_digest":digest("slices")
    })).unwrap();
    let publication = Publication {
        provider: "linear".into(),
        record_id: Uuid::new_v4().to_string(),
        writer_account_id: writer.to_string(),
        payload_digest: digest("publication"),
        observed_at: now,
        url: "https://linear.invalid/comment".into(),
    };
    let repo = current_parent.payload.evidence.candidates[0]
        .repository
        .clone();
    let record = CarryRecord {
        schema_version: 1,
        runtime,
        workspace,
        publication_operation: Uuid::new_v4(),
        publication_scope: DeliveryScope {
            schema_version: 1,
            runtime_id: runtime,
            phase: DeliveryPhase::ParentEvidence,
            work,
            repository: repo.clone(),
            revision: RevisionBinding {
                repository: repo,
                head: "c".repeat(40),
                base: "a".repeat(40),
                tested_commit: "c".repeat(40),
                tested_tree: "d".repeat(40),
            },
            fresh_gate_digest: digest("gate"),
            contract_digest: digest("contract"),
            candidate_vector_digest: Some(digest("vector")),
        },
        original_parent,
        original_acceptance: original_acceptance.clone(),
        original,
        current_parent,
        current_publication: publication.clone(),
        source: reference(),
        baseline_digest: digest("baseline"),
        carry: CarriedAcceptance {
            schema_version: 1,
            original_reviewed_token: original_token,
            original_acceptance_digest: original_acceptance.digest().unwrap(),
            current_evidence_token: current_token,
            unchanged_patch_proof: digest("same patch"),
            production_validation_digest: digest("production"),
            created_at: now,
        },
        references: CarryReferences::default(),
        recovery: None,
    };
    key.sign(
        SAVED_PURPOSE,
        &CarryPublicationRecord {
            signed: key.sign(PURPOSE, &record).unwrap(),
            publication,
            receipt: reference(),
        },
    )
    .unwrap()
}

#[test]
fn legacy_omission_preserves_signed_bytes_and_cannot_recover_current_authority() {
    let temporary = tempfile::tempdir().unwrap();
    let root = Arc::new(
        PrivateDir::create(&temporary.path().canonicalize().unwrap().join("records")).unwrap(),
    );
    let key = Arc::new(AttestationKey::new("reader fixture".into(), vec![41; 32]).unwrap());
    let saved = synthetic_record(&key);
    let workspace = saved.payload.signed.payload.workspace;
    let records = Arc::new(PrivateDir::create(&root.path().join(workspace.to_string())).unwrap());
    let value = serde_json::to_value(&saved).unwrap();
    assert!(
        value["payload"]["signed"]["payload"]
            .get("recovery")
            .is_none()
    );
    let roundtrip: Signed<CarryPublicationRecord> = serde_json::from_value(value).unwrap();
    assert_eq!(roundtrip.digest().unwrap(), saved.digest().unwrap());
    key.verify(SAVED_PURPOSE, &roundtrip).unwrap();
    key.verify(PURPOSE, &roundtrip.payload.signed).unwrap();
    let host = Arc::new(
        ControlHost::setup(
            PrivateDir::create(&temporary.path().canonicalize().unwrap().join("control")).unwrap(),
            saved.payload.signed.payload.runtime,
            Uuid::new_v4(),
        )
        .unwrap(),
    );
    records
        .write(
            &leaf(&saved.payload.signed.payload.carry.current_evidence_token).unwrap(),
            &saved,
        )
        .unwrap();
    let subject = RetainedCarrySubject {
        saved,
        records,
        root,
        host,
        key,
    };
    let error = subject.binding().unwrap_err();
    assert_eq!(error.code, "carry_recovery_unavailable");
    assert_eq!(
        error.message,
        "legacy carry has no producer-written exact recovery selection"
    );
    assert_eq!(subject.validate().unwrap_err().code, "record_missing");
}

#[test]
fn exact_protected_reader_rejects_shadow_malformed_schema_and_bad_hmac() {
    let temporary = tempfile::tempdir().unwrap();
    let root =
        PrivateDir::create(&temporary.path().canonicalize().unwrap().join("records")).unwrap();
    let key = AttestationKey::new("reader fixture".into(), vec![41; 32]).unwrap();
    let signed = synthetic_record(&key);
    let records = PrivateDir::create(
        &root
            .path()
            .join(signed.payload.signed.payload.workspace.to_string()),
    )
    .unwrap();
    let token = &signed.payload.signed.payload.carry.current_evidence_token;
    let leaf = leaf(token).unwrap();
    root.write(&leaf, &signed).unwrap();
    assert!(
        read_retained_carry_record(&records, &key, token)
            .unwrap()
            .is_none()
    );
    records.write(&leaf, &signed).unwrap();
    // Authenticated typed data is still not a recovered-current capability.
    assert_eq!(
        read_retained_carry_record(&records, &key, token)
            .unwrap()
            .unwrap()
            .digest()
            .unwrap(),
        signed.digest().unwrap()
    );
    let mut changed = signed.clone();
    changed.signature = "00".repeat(32);
    records.write(&leaf, &changed).unwrap();
    assert_eq!(
        read_retained_carry_record(&records, &key, token)
            .unwrap_err()
            .code,
        "invalid_attestation"
    );
    records
        .write(
            &leaf,
            &key.sign(SAVED_PURPOSE, &json!({"not_the_producer_schema":true}))
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        read_retained_carry_record(&records, &key, token)
            .unwrap_err()
            .code,
        "record_corrupt"
    );
    std::fs::remove_file(records.path().join(&leaf)).unwrap();
    assert!(
        read_retained_carry_record(&records, &key, token)
            .unwrap()
            .is_none()
    );
}
