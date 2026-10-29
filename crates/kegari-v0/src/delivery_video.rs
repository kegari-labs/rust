//! Candidate recordings originate in actual reviewed Codex validation outputs.
//! Provider upload/readback stays brokered; this module launches no media tools.
use super::*;
use crate::execution::ExecutionEvidenceClass;
use crate::providers::{LinearAssetReference, download_uploaded_asset};
use crate::validation_archive::ResolvedValidationArchive;
use kegari_providers::{MAX_VIDEO_BYTES, linear_video_markdown};

const PURPOSE: &str = "kegari/candidate-video/v1";
const INSPECTION: &str = "kegari-video-inspection.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inspection {
    schema_version: u32,
    recording_digest: String,
    recording_bytes: u64,
    duration_seconds: f64,
    width: u64,
    height: u64,
    video_codec: String,
    audio_codec: Option<String>,
    decoded_frames: u64,
    decoded_duration_seconds: f64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoBinding {
    runtime: Uuid,
    work: WorkIdentity,
    contract: String,
    vector: String,
    integrated: String,
    check: String,
    recording: ArtifactProof,
    inspection: ArtifactProof,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoIntent {
    schema_version: u32,
    admission: Uuid,
    scope: String,
    binding: VideoBinding,
    reference: LinearAssetReference,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoRecord {
    schema_version: u32,
    intent: VideoIntent,
    evidence: evidence::VideoEvidence,
}
struct SelectedVideo {
    check: String,
    recording: ArtifactProof,
    inspection: ArtifactProof,
    facts: Inspection,
    bytes: Vec<u8>,
}
fn blocked(message: &str) -> Error {
    Error::new("video_gate", message)
}

/// A bounded container check supplements actual inspected/decoded execution
/// evidence. Container structure alone never establishes successful playback.
fn mp4_container(bytes: &[u8]) -> Result<()> {
    let mut offset = 0usize;
    let mut count = 0;
    let (mut ftyp, mut moov, mut mdat) = (false, false, false);
    while offset < bytes.len() {
        count += 1;
        if count > 4096 || bytes.len() - offset < 8 {
            return Err(blocked("recording has malformed or excessive MP4 boxes"));
        }
        let prefix = &bytes[offset..];
        let small = u32::from_be_bytes(prefix[..4].try_into().unwrap()) as u64;
        let (size, header) = if small == 1 {
            if prefix.len() < 16 {
                return Err(blocked("recording has a truncated MP4 box"));
            }
            (u64::from_be_bytes(prefix[8..16].try_into().unwrap()), 16)
        } else if small == 0 {
            (prefix.len() as u64, 8)
        } else {
            (small, 8)
        };
        if size < header || size > prefix.len() as u64 {
            return Err(blocked("recording MP4 box exceeds the actual artifact"));
        }
        let body = &prefix[header as usize..size as usize];
        match &prefix[4..8] {
            b"ftyp" => {
                if ftyp || body.len() < 8 || (body.len() - 8) % 4 != 0 {
                    return Err(blocked("recording has invalid MP4 format metadata"));
                }
                ftyp = body[..4] == *b"isom"
                    || body[..4] == *b"iso2"
                    || body[..4] == *b"mp41"
                    || body[..4] == *b"mp42"
                    || body[..4] == *b"avc1"
                    || body[8..]
                        .chunks_exact(4)
                        .any(|b| matches!(b, b"isom" | b"iso2" | b"mp41" | b"mp42" | b"avc1"));
                if !ftyp {
                    return Err(blocked("recording is not a supported MP4 container"));
                }
            }
            b"moov" => moov |= !body.is_empty(),
            b"mdat" => mdat |= !body.is_empty(),
            _ => {}
        }
        offset += size as usize;
    }
    if !ftyp || !moov || !mdat {
        return Err(blocked(
            "recording lacks actual MP4 metadata or media bytes",
        ));
    }
    Ok(())
}
fn select_video(archive: &ResolvedValidationArchive, vector: &str) -> Result<SelectedVideo> {
    let mut selected = None;
    for check in archive.evidence().selected()? {
        for inspection in check
            .artifacts
            .iter()
            .filter(|a| a.path.file_name().and_then(|p| p.to_str()) == Some(INSPECTION))
        {
            if selected.is_some() {
                return Err(blocked(
                    "visual result has multiple selected recording inspections",
                ));
            }
            if check.evidence_class != ExecutionEvidenceClass::CodexExecution
                || check.codex.as_ref().is_none_or(|p| p.schema_version != 2)
                || check.kind != CheckKind::Integrated
                || check.binding.candidate_vector_digest.as_deref() != Some(vector)
                || !check.successful
                || check.command.exit_code != Some(0)
                || check.command.timed_out
                || check.command.output_truncated
                || inspection.bytes == 0
                || inspection.bytes > 8192
            {
                return Err(blocked(
                    "recording inspection needs a successful actual candidate-bound native check",
                ));
            }
            check.validate_shape()?;
            let facts: Inspection = serde_json::from_slice(archive.artifact_bytes(inspection)?)
                .map_err(|_| {
                    blocked("recording inspection output has no supported bounded schema")
                })?;
            if facts.schema_version != 1
                || facts.recording_bytes == 0
                || facts.recording_bytes > MAX_VIDEO_BYTES as u64
                || !facts.duration_seconds.is_finite()
                || facts.duration_seconds <= 0.0
                || facts.duration_seconds > 24.0 * 3600.0
                || facts.width == 0
                || facts.width > 16384
                || facts.height == 0
                || facts.height > 16384
                || facts.video_codec != "h264"
                || facts.audio_codec.as_deref().is_some_and(|c| c != "aac")
                || facts.decoded_frames == 0
                || facts.decoded_frames > 100_000_000
                || !facts.decoded_duration_seconds.is_finite()
                || facts.decoded_duration_seconds < facts.duration_seconds * 0.99
                || facts.decoded_duration_seconds > facts.duration_seconds + 1.0
            {
                return Err(blocked(
                    "recording lacks complete actual H.264/AAC inspection and decode results",
                ));
            }
            let recordings: Vec<_> = check
                .artifacts
                .iter()
                .filter(|a| {
                    a.digest == facts.recording_digest
                        && a.bytes == facts.recording_bytes
                        && a.path != inspection.path
                        && a.path.extension().and_then(|e| e.to_str()) == Some("mp4")
                        && a.candidate_vector_digest.as_deref() == Some(vector)
                        && a.validation_operation == check.binding.operation_id
                })
                .collect();
            let [recording] = recordings.as_slice() else {
                return Err(blocked(
                    "inspection does not identify exactly one freshly recorded candidate MP4",
                ));
            };
            let bytes = archive.read_artifact(recording)?;
            mp4_container(&bytes)?;
            selected = Some(SelectedVideo {
                check: canonical_digest(1, check)?,
                recording: (*recording).clone(),
                inspection: inspection.clone(),
                facts,
                bytes,
            });
        }
    }
    selected.ok_or_else(|| blocked("visual result needs a declared fresh MP4 and kegari-video-inspection.json from actual integrated Codex recording/inspection/decode execution"))
}
fn acknowledged(receipt: &Receipt) -> Result<&Value> {
    use crate::control::ReceiptPhase;
    if !matches!(
        receipt.phase,
        ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
    ) {
        return Err(Error::new(
            "operation_uncertain",
            "recording publication awaits reconciliation of its original provider operation",
        ));
    }
    let result = receipt
        .observed_result
        .as_ref()
        .ok_or_else(|| blocked("recording operation has no actual acknowledgement"))?;
    if result["effect"] == "not_sent" || result.get("error").is_some() {
        return Err(blocked(
            "recording provider operation did not complete successfully",
        ));
    }
    Ok(result)
}
fn video_leaf(evidence: &evidence::VideoEvidence) -> Result<String> {
    Ok(format!(
        "candidate-video-{}.json",
        canonical_digest(1, evidence)?.rsplit(':').next().unwrap()
    ))
}
impl ParentContract {
    pub(crate) fn visual(&self) -> bool {
        self.visual
    }
}
impl DeliveryCoordinator {
    pub(crate) fn visual_validation_context(visual: bool) -> Value {
        if !visual {
            return Value::Null;
        }
        json!({"required":true,"recording":"Produce a real standard MP4 of the changed end-to-end flow against this exact candidate, using Codex native tools and safe data. Discover suitable commands from the repository and available environment; Kegari supplies no media runner or toolset recipe.",
            "inspection":"In the same enrolled successful integrated validation command, inspect the recording and actually decode its full video stream. Declare both the fresh .mp4 output and kegari-video-inspection.json before execution. The JSON must record measured output of that inspection/decode, never a hand-authored substitute, model assertion or success boolean. Preserve actual native command transcript and diagnostics; failed, partial or truncated decoding is not sufficient.",
            "inspection_output":{"schema_version":1,"recording_digest":"sha256:<actual recording bytes>","recording_bytes":"actual size","duration_seconds":"measured duration","width":"actual width","height":"actual height","video_codec":"h264","audio_codec":"aac or null","decoded_frames":"actual fully decoded frame count","decoded_duration_seconds":"actual decoded duration"},
            "review":"Review the actual command, transcript, recording and measured inspection output in the complete original validation archive. Check that the commands really record/inspect/decode the changed flow; fabricated JSON or an unrelated demonstration is insufficient. Do not report visual readiness unless this evidence is adequate. Kegari performs brokered private Linear upload and exact authenticated readback after review."})
    }
    fn video_binding(
        &self,
        contract: &ParentContract,
        integrated: &IntegratedEvidence,
        vector: &str,
        selected: &SelectedVideo,
    ) -> Result<VideoBinding> {
        if integrated.signed.payload.vector_digest != vector
            || integrated.signed.payload.contract_digest != contract.digest()?
        {
            return Err(blocked(
                "recording integrated evidence differs from current parent candidate",
            ));
        }
        Ok(VideoBinding {
            runtime: self.host.snapshot()?.intent.runtime_id,
            work: contract.work().clone(),
            contract: contract.digest()?,
            vector: vector.into(),
            integrated: integrated.digest()?,
            check: selected.check.clone(),
            recording: selected.recording.clone(),
            inspection: selected.inspection.clone(),
        })
    }
    async fn video_effect(
        &self,
        admission: &DeliveryAdmission,
        request_id: Uuid,
        binding: &VideoBinding,
        command: ProviderMutation,
    ) -> Result<Receipt> {
        self.current(admission, &[DeliveryPhase::ParentEvidence])?;
        require_effect_policy(self.contract(admission)?, admission.scope.phase)?;
        let current = self.current_gate.clone();
        let gate = admission.gate.clone();
        self.broker
            .mutate(
                self.host.clone(),
                MutationRequest {
                    request_id,
                    runtime_id: admission.scope.runtime_id,
                    actor: Actor {
                        identity: format!("owner:{}", admission.permit.lease.owner_id),
                        roles: BTreeSet::new(),
                        principal: None,
                    },
                    kind: String::new(),
                    canonical_targets: vec![],
                    expected: BTreeMap::from([
                        ("delivery_scope".into(), admission.scope.digest()?),
                        ("work".into(), admission.scope.work.digest()?),
                        (
                            "control_generation".into(),
                            admission.permit.control_generation.to_string(),
                        ),
                        ("candidate_video".into(), canonical_digest(1, binding)?),
                    ]),
                    payload: Value::Null,
                },
                Some(admission.permit.clone()),
                None,
                command,
                move || current(&gate),
            )
            .await
    }
    pub(crate) async fn publish_candidate_video(
        &self,
        admission: &DeliveryAdmission,
        contract: &ParentContract,
        assignment: &ParentReviewAssignment,
    ) -> Result<Option<VerifiedVideoPublication>> {
        if !contract.visual {
            return Ok(None);
        }
        assignment.matches_worker(contract)?;
        self.current(admission, &[DeliveryPhase::ParentEvidence])?;
        if admission.scope.work != *contract.work()
            || admission.scope.candidate_vector_digest.as_deref()
                != Some(assignment.vector.digest())
        {
            return Err(blocked(
                "recording publication differs from admitted parent candidate",
            ));
        }
        let archive = assignment.integrated.archive.as_ref().ok_or_else(|| {
            blocked(
                "visual publication requires the authenticated complete native validation archive",
            )
        })?;
        let selected = select_video(archive, assignment.vector.digest())?;
        let binding = self.video_binding(
            contract,
            &assignment.integrated,
            assignment.vector.digest(),
            &selected,
        )?;
        let leaf = format!("candidate-video-intent-{}.json", admission.operation_id());
        let intent = {
            let _lock = self.records.section_lock("candidate-video.lock")?;
            match self.records.read::<Signed<VideoIntent>>(&leaf) {
                Ok(signed) => {
                    self.key.verify(PURPOSE, &signed)?;
                    let intent = signed.payload;
                    if intent.schema_version != 1
                        || intent.admission != admission.operation_id()
                        || intent.scope != admission.scope.digest()?
                        || intent.binding != binding
                    {
                        return Err(blocked(
                            "retained visual publication belongs to another exact candidate/admission",
                        ));
                    }
                    intent
                }
                Err(error) if error.code == "record_missing" => {
                    let intent = VideoIntent {
                        schema_version: 1,
                        admission: admission.operation_id(),
                        scope: admission.scope.digest()?,
                        binding,
                        reference: LinearAssetReference {
                            issue: contract.work().parent,
                            grant_operation: Uuid::new_v4(),
                            upload_operation: Uuid::new_v4(),
                            association_operation: Uuid::new_v4(),
                        },
                    };
                    self.records
                        .write(&leaf, &self.key.sign(PURPOSE, &intent)?)?;
                    intent
                }
                Err(error) => return Err(error),
            }
        };
        let grant = self
            .video_effect(
                admission,
                intent.reference.grant_operation,
                &intent.binding,
                ProviderMutation::LinearUploadRequest {
                    issue: intent.reference.issue,
                    filename: format!("kegari-candidate-{}.mp4", &selected.recording.digest[7..23]),
                    content_type: "video/mp4".into(),
                    size: selected.bytes.len() as u32,
                },
            )
            .await?;
        let asset_url = acknowledged(&grant)?["asset_url"]
            .as_str()
            .ok_or_else(|| blocked("recording upload grant has no actual asset URL"))?
            .to_owned();
        let uploaded = self
            .video_effect(
                admission,
                intent.reference.upload_operation,
                &intent.binding,
                ProviderMutation::LinearUploadBytes {
                    issue: intent.reference.issue,
                    grant_operation: intent.reference.grant_operation,
                    asset_url: asset_url.clone(),
                    content_digest: selected.recording.digest.clone(),
                    bytes: selected.bytes,
                },
            )
            .await?;
        let result = acknowledged(&uploaded)?;
        if result["uploaded"] != true
            || result["asset_url"].as_str() != Some(&asset_url)
            || result["content_digest"].as_str() != Some(&selected.recording.digest)
            || result["content_type"] != "video/mp4"
        {
            return Err(blocked(
                "recording bytes have no exact successful upload acknowledgement",
            ));
        }
        let body = format!(
            "{}\n\nCandidate recording for {}. Integrated check {}.\n<!-- kegari:candidate-video:{} -->",
            linear_video_markdown(&asset_url)?,
            assignment.vector.digest(),
            selected.check,
            canonical_digest(1, &intent.binding)?
        );
        let associated = self
            .video_effect(
                admission,
                intent.reference.association_operation,
                &intent.binding,
                ProviderMutation::LinearComment {
                    issue: intent.reference.issue,
                    body,
                },
            )
            .await?;
        acknowledged(&associated)?;
        let linear =
            self.broker.linear.as_ref().ok_or_else(|| {
                blocked("Linear is unavailable for authenticated recording readback")
            })?;
        let observed = download_uploaded_asset(linear, &self.host, &intent.reference).await?;
        if observed.digest() != selected.recording.digest
            || observed.bytes().len() as u64 != selected.recording.bytes
        {
            return Err(blocked(
                "authenticated playback bytes differ from the actual inspected recording",
            ));
        }
        self.current(admission, &[DeliveryPhase::ParentEvidence])?;
        let evidence = evidence::VideoEvidence {
            asset_digest: selected.recording.digest,
            media_type: "video/mp4".into(),
            video_codec: selected.facts.video_codec,
            audio_codec: selected.facts.audio_codec,
            candidate_vector_digest: assignment.vector.digest().into(),
            validation_operation: selected.recording.validation_operation,
            linear_asset_id: observed.asset_url().into(),
            parent_association_id: observed.comment().id.to_string(),
            associated_parent: contract.work().parent,
            playback_verified: true,
        };
        let record = VideoRecord {
            schema_version: 1,
            intent,
            evidence: evidence.clone(),
        };
        self.records
            .write(&video_leaf(&evidence)?, &self.key.sign(PURPOSE, &record)?)?;
        Ok(Some(VerifiedVideoPublication { evidence }))
    }
    pub(crate) async fn reobserve_candidate_video(
        &self,
        contract: &ParentContract,
        integrated: &IntegratedEvidence,
        vector: &str,
        expected: &evidence::VideoEvidence,
    ) -> Result<()> {
        (self.current_gate)(&contract.gate)?;
        let archive = integrated.archive.as_ref().ok_or_else(|| blocked("historical visual result has no native validation archive; original bytes are preserved"))?;
        let selected = select_video(archive, vector)?;
        let binding = self.video_binding(contract, integrated, vector, &selected)?;
        let signed: Signed<VideoRecord> = self.records.read(&video_leaf(expected)?).map_err(|error| if error.code == "record_missing" { blocked("historical visual result lacks its original authenticated publication record") } else { error })?;
        self.key.verify(PURPOSE, &signed)?;
        let record = signed.payload;
        if record.schema_version != 1
            || record.intent.schema_version != 1
            || record.intent.binding != binding
            || record.evidence != *expected
            || record.intent.reference.issue != contract.work().parent
            || expected.associated_parent != contract.work().parent
            || expected.candidate_vector_digest != vector
            || expected.asset_digest != selected.recording.digest
            || expected.validation_operation != selected.recording.validation_operation
            || expected.video_codec != selected.facts.video_codec
            || expected.audio_codec != selected.facts.audio_codec
            || expected.media_type != "video/mp4"
            || !expected.playback_verified
        {
            return Err(blocked(
                "retained video differs from the actual reviewed candidate/check/recording",
            ));
        }
        for id in [
            record.intent.reference.grant_operation,
            record.intent.reference.upload_operation,
            record.intent.reference.association_operation,
        ] {
            let receipt = self.host.operation(id)?;
            if receipt.expected.get("candidate_video") != Some(&canonical_digest(1, &binding)?)
                || receipt.expected.get("delivery_scope") != Some(&record.intent.scope)
            {
                return Err(blocked(
                    "recording receipt belongs to another candidate publication",
                ));
            }
        }
        let linear = self
            .broker
            .linear
            .as_ref()
            .ok_or_else(|| blocked("Linear is unavailable for recording readback"))?;
        let observed =
            download_uploaded_asset(linear, &self.host, &record.intent.reference).await?;
        if observed.digest() != expected.asset_digest
            || observed.asset_url() != expected.linear_asset_id
            || observed.comment().id.to_string() != expected.parent_association_id
        {
            return Err(blocked(
                "current authenticated recording association changed",
            ));
        }
        (self.current_gate)(&contract.gate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation_archive::{self, ValidationArchiveNamespace};

    fn box_bytes(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut bytes = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(payload);
        bytes
    }
    /// Descriptive archive fixture only; synthetic bytes do not claim real
    /// media decoding or live provider qualification.
    fn archive(
        decoded_frames: u64,
    ) -> (
        tempfile::TempDir,
        ResolvedValidationArchive,
        String,
        Vec<u8>,
    ) {
        let vector = bytes_digest(b"actual fixture candidate vector");
        let (_, old, old_bytes) = validation_archive::tests::descriptive_fixture();
        let mut check = old.attempts[1].clone();
        let transcript = old_bytes[&canonical_digest(1, &check.artifacts[0]).unwrap()].clone();
        let mut plan = old.history[0].clone();
        plan.kind = CheckKind::Integrated;
        plan.binding.candidate_vector_digest = Some(vector.clone());
        plan.original_binding = plan.binding.clone();
        plan.requirements[0].artifacts = vec![
            "/owned/repository/recording.mp4".into(),
            format!("/owned/repository/{INSPECTION}").into(),
        ];
        check.binding = plan.binding.clone();
        check.kind = CheckKind::Integrated;
        check.codex.as_mut().unwrap().plan = Some(plan.clone());
        check.artifacts[0].candidate_vector_digest = Some(vector.clone());
        let mut bytes = box_bytes(b"ftyp", b"isom\0\0\0\0isommp42");
        bytes.extend(box_bytes(b"moov", b"synthetic fixture metadata"));
        bytes.extend(box_bytes(b"mdat", b"synthetic fixture media"));
        let facts = Inspection {
            schema_version: 1,
            recording_digest: bytes_digest(&bytes),
            recording_bytes: bytes.len() as u64,
            duration_seconds: 2.0,
            width: 1280,
            height: 720,
            video_codec: "h264".into(),
            audio_codec: None,
            decoded_frames,
            decoded_duration_seconds: 2.0,
        };
        let inspection_bytes = serde_json::to_vec(&facts).unwrap();
        let recording = ArtifactProof {
            path: "/owned/repository/recording.mp4".into(),
            digest: bytes_digest(&bytes),
            bytes: bytes.len() as u64,
            validation_operation: check.binding.operation_id,
            candidate_vector_digest: Some(vector.clone()),
        };
        let inspection = ArtifactProof {
            path: format!("/owned/repository/{INSPECTION}").into(),
            digest: bytes_digest(&inspection_bytes),
            bytes: inspection_bytes.len() as u64,
            validation_operation: check.binding.operation_id,
            candidate_vector_digest: Some(vector.clone()),
        };
        for proof in [&recording, &inspection] {
            check.codex.as_mut().unwrap().artifact_freshness.push(serde_json::from_value(json!({"path":proof.path,"before":null,
                "after":{"bytes":proof.bytes,"digest":proof.digest,"modified":[2,0],"identity":[1,2],"changed":[2,0]}})).unwrap());
        }
        check
            .artifacts
            .extend([recording.clone(), inspection.clone()]);
        let original = BTreeMap::from([
            (
                canonical_digest(1, &check.artifacts[0]).unwrap(),
                transcript,
            ),
            (canonical_digest(1, &recording).unwrap(), bytes.clone()),
            (canonical_digest(1, &inspection).unwrap(), inspection_bytes),
        ]);
        let evidence = evidence::ValidationEvidence {
            schema_version: 4,
            plan_digest: canonical_digest(1, &plan).unwrap(),
            original_cwd: old.original_cwd,
            history: vec![plan],
            attempts: vec![check],
            prior_history: vec![],
        };
        let temp = tempfile::tempdir().unwrap();
        let records =
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("records")).unwrap();
        let key = AttestationKey::new("video-fixture".into(), vec![52; 32]).unwrap();
        let archive = validation_archive::retain(
            &records,
            &key,
            ValidationArchiveNamespace {
                workspace_id: Uuid::from_u128(9),
                runtime_id: evidence.history[0].binding.runtime_id,
            },
            &evidence,
            |proof| Ok(original[&canonical_digest(1, proof)?].clone()),
        )
        .unwrap();
        (temp, archive, vector, bytes)
    }
    #[test]
    fn candidate_video_requires_selected_native_inspection_and_exact_fresh_bytes() {
        let (_temp, archive, vector, bytes) = archive(48);
        let selected = select_video(&archive, &vector).unwrap();
        assert_eq!(selected.bytes, bytes);
        assert_eq!(selected.recording.digest, bytes_digest(&bytes));
        assert_eq!(selected.facts.decoded_frames, 48);
        assert_eq!(
            select_video(&archive, &bytes_digest(b"another candidate"))
                .err()
                .unwrap()
                .code,
            "video_gate"
        );
        let (_failed_temp, incomplete, vector, _) = self::archive(0);
        assert_eq!(
            select_video(&incomplete, &vector).err().unwrap().code,
            "video_gate"
        );
        let mut truncated = bytes;
        truncated.pop();
        assert_eq!(mp4_container(&truncated).unwrap_err().code, "video_gate");
    }
}
