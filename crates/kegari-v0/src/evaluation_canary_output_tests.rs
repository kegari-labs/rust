//! Data reducer + real protected-file components only. No test constructs a
//! CanaryOutputEnrollment, WorkerContract, permit, registered runtime or native run.
use super::*;
use serde_json::json;

struct Case {
    _temp: tempfile::TempDir,
    data: OutputCapture,
}
impl Case {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let parent = Arc::new(PrivateDir::create(&root.join("original")).unwrap());
        let marker = b"component-only-output-canary-123456789".to_vec();
        let identity = CanaryIdentity {
            runtime: Uuid::from_u128(1),
            workspace: Uuid::from_u128(2),
            command: Uuid::from_u128(3),
            operation: Uuid::from_u128(4),
            trial_digest: bytes_digest(b"trial"),
            suite_digest: bytes_digest(b"suite"),
            fixture_digest: bytes_digest(b"fixture"),
            scorer_digest: bytes_digest(b"scorer"),
            policy_digest: bytes_digest(b"policy"),
            profile_digest: bytes_digest(b"profile"),
        };
        let mut core = vec![];
        for (leaf, bytes) in [
            ("marker.bin", marker.as_slice()),
            ("enrollment.json", b"component-only-enrollment".as_slice()),
        ] {
            let mut anchor = None;
            parent
                .create_bytes_exclusive_retained(leaf, bytes, |value| {
                    anchor = Some(value);
                    Ok(())
                })
                .unwrap();
            core.push(CoreOriginal {
                reference: CanaryRecordReference {
                    leaf: leaf.into(),
                    digest: bytes_digest(bytes),
                    bytes: bytes.len() as u64,
                },
                anchor: anchor.unwrap(),
            });
        }
        let forms = marker_forms(&marker)
            .into_iter()
            .zip([
                OutputForm::LiteralUtf8,
                OutputForm::Base64Standard,
                OutputForm::HexLower,
                OutputForm::HexUpper,
            ])
            .map(|((_, bytes), form)| (form, bytes))
            .collect();
        let mut report = CanaryOutputObservation::pending(identity.clone());
        report.enrollment = Some(digest(b"component-only-enrollment"));
        report.issues = vec![OutputIssue::CoverageNotQualified];
        let data = OutputCapture {
            identity,
            budget: Arc::new(CaptureBudget::default()),
            deadline: tokio::time::Instant::now() + Duration::from_secs(60),
            parent,
            core,
            marker,
            forms,
            capture: Mutex::new(Capture {
                report,
                directory: None,
                originals: vec![],
                streams: BTreeMap::new(),
                event: 0,
                established: None,
                stopped: false,
            }),
        };
        data.initialize();
        Self { _temp: temp, data }
    }
    fn request(&self, args: Value, labels: Option<&str>) {
        let label = labels.unwrap_or("component-id");
        let params =
            json!({"threadId":label,"turnId":label,"callId":label,"tool":label,"arguments":args});
        self.data.observe_protocol(
            &ProtocolObservation::ServerRequest {
                state: None,
                rpc_id: &json!(1),
                sequence: 1,
                method: "item/tool/call",
                params: &params,
            },
            false,
        );
    }
    fn path(&self, reference: &OutputRecord) -> PathBuf {
        self.data
            .parent
            .path()
            .join("outputs")
            .join(format!("record-{:05}.bin", reference.ordinal))
    }
}
#[test]
fn exact_forms_match_across_scan_chunks_and_piece_boundaries() {
    let marker = b"declared-component-marker";
    for (name, needle) in marker_forms(marker) {
        let form = match name {
            "literal_utf8" => OutputForm::LiteralUtf8,
            "base64_standard" => OutputForm::Base64Standard,
            "hex_lower" => OutputForm::HexLower,
            _ => OutputForm::HexUpper,
        };
        let forms = vec![(form, needle.clone())];
        for split in 1..needle.len() {
            let mut bytes = vec![b'.'; CHUNK_BYTES - split];
            bytes.extend_from_slice(&needle);
            bytes.push(b'.');
            let result = scan_matches(&forms, &[], &[], 0, &bytes, &|_| Ok(())).unwrap();
            assert_eq!(result.len(), 1);
            assert_eq!(
                result[0].1,
                vec![OutputSpan {
                    piece: 0,
                    offset: CHUNK_BYTES - split,
                    bytes: needle.len()
                }]
            );
            let result = scan_matches(
                &forms,
                &needle[..split],
                &[OutputSpan {
                    piece: 0,
                    offset: 3,
                    bytes: split,
                }],
                1,
                &needle[split..],
                &|_| Ok(()),
            )
            .unwrap();
            assert_eq!(result.len(), 1);
            assert_eq!(
                result[0].1,
                vec![
                    OutputSpan {
                        piece: 0,
                        offset: 3,
                        bytes: split
                    },
                    OutputSpan {
                        piece: 1,
                        offset: 0,
                        bytes: needle.len() - split
                    }
                ]
            );
        }
    }
}
#[test]
fn root_string_array_and_key_projection_precede_shape_refusal() {
    for shape in 0..3 {
        let case = Case::new();
        let marker = String::from_utf8(case.data.marker.clone()).unwrap();
        let value = match shape {
            0 => json!(marker),
            1 => json!([false, null, 17, marker]),
            _ => json!({marker.clone():"benign"}),
        };
        case.request(value, None);
        let report = case.data.finalize();
        assert!(
            report
                .matches
                .iter()
                .any(|m| m.form == OutputForm::LiteralUtf8
                    && m.authentication == MatchAuthentication::Authenticated)
        );
        assert_eq!(report.coverage, OutputCoverage::Partial);
        assert!(report.issues.contains(&OutputIssue::CoverageNotQualified));
        assert!(
            report
                .pieces
                .iter()
                .all(|p| p.attribution == OutputAttribution::OriginalReaderUnbound)
        );
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(!encoded.contains(&marker));
        report.validate().unwrap();
    }
}
#[test]
fn label_only_marker_is_protected_metadata_not_matched_content() {
    let case = Case::new();
    let marker = String::from_utf8(case.data.marker.clone()).unwrap();
    case.request(json!({"body":"benign body"}), Some(&marker));
    let report = case.data.finalize();
    assert!(report.matches.is_empty());
    assert!(!serde_json::to_string(&report).unwrap().contains(&marker));
    let metadata = report.pieces[0].metadata.as_ref().unwrap();
    let bytes = std::fs::read(case.path(metadata)).unwrap();
    assert!(String::from_utf8(bytes).unwrap().contains(&marker));
    // Safe new DTO only: unchanged legacy protocol/broker labels are outside
    // this test's secrecy claim; empty matches are still partial, never pass.
    assert_eq!(report.coverage, OutputCoverage::Partial);
}
#[test]
fn positive_original_readback_does_not_require_unrelated_record_success() {
    let case = Case::new();
    let marker = String::from_utf8(case.data.marker.clone()).unwrap();
    case.request(json!(marker), None);
    case.request(json!("unrelated later text"), None);
    let snapshot = case.data.snapshot();
    let unrelated = snapshot.pieces.last().unwrap().records[0].clone();
    std::fs::remove_file(case.path(&unrelated)).unwrap();
    let report = case.data.finalize();
    assert!(!report.final_readback);
    assert!(
        report
            .matches
            .iter()
            .any(|m| m.authentication == MatchAuthentication::Authenticated)
    );
    assert!(report.issues.contains(&OutputIssue::Storage));
}
#[test]
fn identical_byte_substitution_of_matched_record_refuses_first_authentication() {
    let case = Case::new();
    case.request(
        json!(String::from_utf8(case.data.marker.clone()).unwrap()),
        None,
    );
    let original = case.data.snapshot().pieces[0].records[0].clone();
    let path = case.path(&original);
    let bytes = std::fs::read(&path).unwrap();
    std::fs::rename(&path, path.with_extension("retained")).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    // Same bytes and path cannot replace the descriptor-creation identity.
    let report = case.data.finalize();
    assert!(
        report
            .matches
            .iter()
            .all(|m| m.authentication != MatchAuthentication::Authenticated)
    );
    assert!(!report.final_readback);
    assert!(
        report.issues.contains(&OutputIssue::Binding)
            || report.issues.contains(&OutputIssue::Storage)
    );
}
#[test]
fn substituted_marker_refuses_positive_but_preserves_captured_spans() {
    let case = Case::new();
    case.request(
        json!(String::from_utf8(case.data.marker.clone()).unwrap()),
        None,
    );
    let before = case.data.snapshot();
    let path = case.data.parent.path().join("marker.bin");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::rename(&path, path.with_extension("original")).unwrap();
    std::fs::write(&path, bytes).unwrap();
    let after = case.data.finalize();
    assert_eq!(after.matches.len(), before.matches.len());
    assert_eq!(after.matches[0].spans, before.matches[0].spans);
    assert!(
        after
            .matches
            .iter()
            .all(|m| m.authentication != MatchAuthentication::Authenticated)
    );
}
#[test]
fn exclusive_write_refusal_keeps_known_membership_without_restart() {
    let case = Case::new();
    let directory = case.data.lock().unwrap().directory.clone().unwrap();
    directory
        .create_bytes_exclusive("record-00000.bin", b"occupied")
        .unwrap();
    case.request(json!("body"), None);
    let snapshot = case.data.snapshot();
    assert_eq!(snapshot.pieces.len(), 1);
    assert!(!snapshot.pieces[0].stored);
    assert_eq!(snapshot.error_records.len(), 1);
    {
        let state = case.data.lock().unwrap();
        // Failed piece 0 is retained alongside the successfully protected
        // error record 1; observing the error must not erase either member.
        assert_eq!(state.originals.len(), 2);
        assert!(state.originals[0].anchor.is_none());
        assert_eq!(state.originals[1].reference, snapshot.error_records[0]);
        assert!(state.originals[1].anchor.is_some());
        let bytes = case.data.read(&state, &snapshot.error_records[0]).unwrap();
        let error: Error = serde_json::from_slice(&bytes).unwrap();
        assert!(!error.code.is_empty());
    }
    case.request(json!("must not restart"), None);
    assert_eq!(case.data.snapshot().pieces.len(), 1);
    assert_eq!(case.data.lock().unwrap().originals.len(), 2);
    assert!(!case.data.finalize().final_readback);
}
#[test]
fn original_deadline_and_shared_budget_refuse_without_output_reset() {
    let mut case = Case::new();
    case.data.deadline = tokio::time::Instant::now();
    case.request(json!("text"), None);
    assert!(case.data.snapshot().issues.contains(&OutputIssue::Deadline));
    assert!(case.data.snapshot().pieces.is_empty());
    let case = Case::new();
    while case.data.budget.reserve(CHUNK_BYTES).is_ok() {}
    // Initialization already charged the shared budget, so whole chunks leave
    // a remainder. Exhaust that capacity before observing the next event.
    while case.data.budget.reserve(1).is_ok() {}
    assert_eq!(
        case.data.budget.reserve(1).unwrap_err().code,
        "observation_limit"
    );
    case.request(json!("text"), None);
    assert!(case.data.snapshot().issues.contains(&OutputIssue::Budget));
    assert!(case.data.snapshot().pieces.is_empty());
    assert!(!case.data.finalize().final_readback);
    assert_eq!(
        case.data.budget.reserve(1).unwrap_err().code,
        "observation_limit"
    );
}
#[test]
fn output_digest_and_public_schema_reject_arbitrary_metadata_labels() {
    assert!(serde_json::from_value::<OutputDigest>(json!("marker-in-label")).is_err());
    let case = Case::new();
    let mut value = serde_json::to_value(case.data.snapshot()).unwrap();
    value["pointer"] = json!("secret-key");
    assert!(serde_json::from_value::<CanaryOutputObservation>(value).is_err());
}
#[test]
fn pure_chunk_scan_charges_before_work_and_never_loses_split_match() {
    use std::cell::Cell;
    let charged = Cell::new(0);
    let result = scan_matches(
        &[(OutputForm::LiteralUtf8, b"abc".to_vec())],
        b"ab",
        &[OutputSpan {
            piece: 0,
            offset: 0,
            bytes: 2,
        }],
        1,
        b"c",
        &|n| {
            charged.set(charged.get() + n);
            Err(Error::new("evaluation_deadline", "component cutoff"))
        },
    );
    assert_eq!(result.unwrap_err().code, "evaluation_deadline");
    assert!(charged.get() > 0);
    let result = scan_matches(
        &[(OutputForm::LiteralUtf8, b"abc".to_vec())],
        b"ab",
        &[OutputSpan {
            piece: 0,
            offset: 0,
            bytes: 2,
        }],
        1,
        b"c",
        &|_| Ok(()),
    )
    .unwrap();
    assert_eq!(result.len(), 1);
}
fn session_data() -> SessionState {
    let mut state = SessionState::new(
        crate::codex::SessionBinding {
            schema_version: 1,
            account_id: "component".into(),
            work_identity: "component".into(),
            operation_id: Uuid::from_u128(4).to_string(),
            generation: "component".into(),
            instruction_digest: bytes_digest(b"instructions"),
            isolation_digest: bytes_digest(b"isolation"),
            profile: crate::codex::CodexProfile {
                model: "component".into(),
                reasoning: "high".into(),
                speed: "standard".into(),
            },
            cwd: PathBuf::from("/private/tmp/explicit-component"),
        },
        2,
    )
    .unwrap();
    state.thread_id = Some("thread".into());
    state.turn_id = Some("turn".into());
    state.turns_started = 1;
    state
}
#[test]
fn delta_positive_survives_different_complete_text_without_negative_inference() {
    let case = Case::new();
    let state = session_data();
    let marker = String::from_utf8(case.data.marker.clone()).unwrap();
    let split = marker.len() / 2;
    for part in [&marker[..split], &marker[split..]] {
        let params = json!({"threadId":"thread","turnId":"turn","itemId":"message","delta":part});
        case.data.observe_protocol(
            &ProtocolObservation::AgentMessageDelta {
                state: &state,
                params: &params,
                accepted: true,
            },
            false,
        );
    }
    let params = json!({"threadId":"thread","turnId":"turn","item":{"id":"message","type":"agentMessage","text":"different final text"}});
    case.data.observe_protocol(
        &ProtocolObservation::CompletedItem {
            state: &state,
            params: &params,
            accepted: true,
        },
        false,
    );
    let report = case.data.finalize();
    assert!(
        report
            .matches
            .iter()
            .any(|m| m.channel == OutputChannel::AgentDelta
                && m.conservative_concatenation
                && m.authentication == MatchAuthentication::Authenticated)
    );
    assert!(
        report
            .pieces
            .iter()
            .all(|p| p.attribution == OutputAttribution::RejectedOrForeign)
    );
    assert_eq!(report.coverage, OutputCoverage::Partial);
}
#[test]
fn partial_output_data_survives_archive_failure_record_and_old_absence_bytes() {
    let mut suite: SuiteManifest = serde_json::from_slice(
        &std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations/manifest.json"))
            .unwrap(),
    )
    .unwrap();
    let index = suite
        .variants
        .iter()
        .position(|v| v.id == "independent_malicious")
        .unwrap();
    suite.profiles[0].binding = Some(ProfileBinding {
        profile: kegari_config::Profile {
            tool: "codex".into(),
            model: "component-only".into(),
            reasoning: "high".into(),
            speed: kegari_config::Speed::Standard,
        },
        account_id: "component-only".into(),
        operator_reference: "no-native-capability".into(),
    });
    suite.profiles[0].unresolved = None;
    let policy = CanaryPolicyReference {
        schema_version: 2,
        path: "diagnostics/output.json".into(),
        digest: bytes_digest(b"component policy metadata"),
    };
    suite.variants[index].canary_diagnostic = Some(policy.clone());
    let variant = suite.variants[index].clone();
    let trial = suite
        .plan()
        .unwrap()
        .into_iter()
        .find(|t| t.variant == variant.id)
        .unwrap();
    let owner = CanaryMaterialization::pending(CanaryStageInput {
        runtime: Uuid::new_v4(),
        workspace: Uuid::new_v4(),
        command: Uuid::new_v4(),
        operation: Uuid::new_v4(),
        suite: &suite,
        variant: &variant,
        trial: &trial,
        deadline: tokio::time::Instant::now() + Duration::from_secs(30),
    })
    .unwrap();
    // Pending owner is genuine diagnostic allocation only; no stage, worker,
    // enrollment, account authority or execution capability is manufactured.
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let path = root.join("reports");
    let store = ReportStore::create(&path, &suite).unwrap();
    store.started(&trial).unwrap();
    let error = Error::new("component_failure", "explicit pre-model failure");
    ReportStore::finish_owned_failure_with_canary(
        &path,
        &suite,
        &variant,
        &trial,
        &error,
        false,
        CanaryFailureObservations {
            inputs: Some(owner.diagnostic().unwrap()),
            outputs: owner.output_snapshot(),
        },
    )
    .unwrap();
    let archive = crate::evaluation_reports::ReportArchive::open(&path).unwrap();
    let report = archive.reports().iter().find(|r| r.trial == trial).unwrap();
    assert_eq!(report.outcome, TrialOutcome::Blocked);
    assert!(report.grade.is_none());
    assert!(report.canary_output.as_ref().unwrap().matches.is_empty());
    assert!(!report.canary_output.as_ref().unwrap().final_readback);
    let mut legacy = serde_json::to_value(report).unwrap();
    legacy.as_object_mut().unwrap().remove("canary_output");
    let decoded: TrialReport = serde_json::from_value(legacy.clone()).unwrap();
    assert!(decoded.canary_output.is_none());
    assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
}
#[test]
fn policy_two_requires_exact_projection_and_policy_one_does_not_enable_it() {
    let common = json!({"schema_version":1,"purpose":"independent_canary_diagnostic","recipe":"os_random_256_ascii_v1","placement":"private_grader_child_v1","host_input_set":"declared_host_inputs_v1","forms":FORMS});
    let mut reference = CanaryPolicyReference {
        schema_version: 1,
        path: "diagnostics/output.json".into(),
        digest: bytes_digest(&serde_json::to_vec(&common).unwrap()),
    };
    validate_policy(&reference, &serde_json::to_vec(&common).unwrap()).unwrap();
    let mut policy = common.clone();
    policy["schema_version"] = json!(2);
    policy["output_projection"] = json!(PROJECTION);
    policy["matching"] = json!(MATCHING);
    let bytes = serde_json::to_vec(&policy).unwrap();
    reference.schema_version = 2;
    reference.digest = bytes_digest(&bytes);
    validate_policy(&reference, &bytes).unwrap();
    for key in ["output_projection", "matching"] {
        let mut missing = policy.clone();
        missing.as_object_mut().unwrap().remove(key);
        let bytes = serde_json::to_vec(&missing).unwrap();
        reference.digest = bytes_digest(&bytes);
        assert!(validate_policy(&reference, &bytes).is_err());
    }
    let mut null_extension = common.clone();
    null_extension["output_projection"] = Value::Null;
    let null_bytes = serde_json::to_vec(&null_extension).unwrap();
    reference.schema_version = 1;
    reference.digest = bytes_digest(&null_bytes);
    assert!(validate_policy(&reference, &null_bytes).is_err());
    let mut wrong = policy;
    wrong["schema_version"] = json!(1);
    let bytes = serde_json::to_vec(&wrong).unwrap();
    reference.schema_version = 1;
    reference.digest = bytes_digest(&bytes);
    assert!(validate_policy(&reference, &bytes).is_err());
}

#[test]
fn delta_stream_crosses_callback_establishment_boundary_without_losing_attribution() {
    let case = Case::new();
    let state = session_data();
    let marker = String::from_utf8(case.data.marker.clone()).unwrap();
    let split = marker.len() / 2;
    let prefix =
        json!({"threadId":"thread","turnId":"turn","itemId":"message","delta":&marker[..split]});
    case.data.observe_protocol(
        &ProtocolObservation::UnboundNotification {
            method: "item/agentMessage/delta",
            params: &prefix,
        },
        false,
    );
    // Plain reducer input only. This is not a registered ScopedWorker callback
    // or an enrollment constructor, and establishes no native/accepted-turn fact.
    case.data.observe_protocol(
        &ProtocolObservation::ThreadEstablished { state: &state },
        true,
    );
    let suffix =
        json!({"threadId":"thread","turnId":"turn","itemId":"message","delta":&marker[split..]});
    case.data.observe_protocol(
        &ProtocolObservation::AgentMessageDelta {
            state: &state,
            params: &suffix,
            accepted: true,
        },
        true,
    );
    let report = case.data.finalize();
    assert_eq!(report.pieces.len(), 2);
    assert_eq!(report.pieces[0].stream, report.pieces[1].stream);
    assert_eq!(
        report.pieces[0].attribution,
        OutputAttribution::OriginalReaderUnbound
    );
    assert_eq!(
        report.pieces[1].attribution,
        OutputAttribution::BoundSession
    );
    let found = report
        .matches
        .iter()
        .find(|m| m.form == OutputForm::LiteralUtf8)
        .unwrap();
    assert!(found.conservative_concatenation);
    assert_eq!(found.authentication, MatchAuthentication::Authenticated);
    assert_eq!(
        found.spans,
        vec![
            OutputSpan {
                piece: 0,
                offset: 0,
                bytes: split
            },
            OutputSpan {
                piece: 1,
                offset: 0,
                bytes: marker.len() - split
            }
        ]
    );
    // Keep callback-kind provenance separate from the shared tuple identity.
    let first = std::fs::read(case.path(report.pieces[0].metadata.as_ref().unwrap())).unwrap();
    let second = std::fs::read(case.path(report.pieces[1].metadata.as_ref().unwrap())).unwrap();
    assert!(String::from_utf8(first).unwrap().contains("unbound"));
    assert!(String::from_utf8(second).unwrap().contains("agent_delta"));
    assert_eq!(report.coverage, OutputCoverage::Partial);
    assert!(report.issues.contains(&OutputIssue::CoverageNotQualified));
}
#[test]
fn distinct_or_unidentified_delta_items_never_share_concatenation_streams() {
    for key in ["threadId", "turnId", "itemId"] {
        for replacement in [
            Some(json!("different")),
            None,
            Some(Value::Null),
            Some(json!(7)),
            Some(json!("")),
        ] {
            let case = Case::new();
            let state = session_data();
            let marker = String::from_utf8(case.data.marker.clone()).unwrap();
            let split = marker.len() / 2;
            let mut prefix = json!({"threadId":"thread","turnId":"turn","itemId":"message","delta":&marker[..split]});
            let mut suffix = json!({"threadId":"thread","turnId":"turn","itemId":"message","delta":&marker[split..]});
            match &replacement {
                Some(Value::String(text)) if text == "different" => {
                    suffix[key] = json!(text);
                }
                Some(value) => {
                    prefix[key] = value.clone();
                    suffix[key] = value.clone();
                }
                None => {
                    prefix.as_object_mut().unwrap().remove(key);
                    suffix.as_object_mut().unwrap().remove(key);
                }
            }
            case.data.observe_protocol(
                &ProtocolObservation::UnboundNotification {
                    method: "item/agentMessage/delta",
                    params: &prefix,
                },
                false,
            );
            case.data.observe_protocol(
                &ProtocolObservation::AgentMessageDelta {
                    state: &state,
                    params: &suffix,
                    accepted: false,
                },
                false,
            );
            let report = case.data.finalize();
            assert_eq!(report.pieces.len(), 2);
            assert_ne!(
                report.pieces[0].stream, report.pieces[1].stream,
                "identity key {key}: {replacement:?}"
            );
            assert!(report.matches.is_empty());
            if !matches!(replacement,Some(Value::String(ref text)) if text=="different") {
                assert!(report.issues.contains(&OutputIssue::Schema));
            }
            assert_eq!(report.coverage, OutputCoverage::Partial);
        }
    }
}
