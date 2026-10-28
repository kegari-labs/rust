//! Declared egress subset only. No negative coverage, grading or execution capability.
use super::*;
use crate::control::Permit;
use kegari_codex::ProtocolObservation;
use kegari_codex::SessionState;

const LIMIT: usize = 4096;
const PROJECTION: &str = "independent_declared_egress_v1";
const MATCHING: &str = "decoded_utf8_atoms_and_channel_concat_v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "String", into = "String")]
pub struct OutputDigest(String);
impl TryFrom<String> for OutputDigest {
    type Error = Error;
    fn try_from(value: String) -> Result<Self> {
        crate::evidence::validate_digest(&value)?;
        Ok(Self(value))
    }
}
impl From<OutputDigest> for String {
    fn from(value: OutputDigest) -> Self {
        value.0
    }
}
fn digest(bytes: &[u8]) -> OutputDigest {
    OutputDigest(bytes_digest(bytes))
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum OutputChannel {
    AgentMessage,
    AgentDelta,
    ToolArguments,
    FeedbackIntent,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputForm {
    LiteralUtf8,
    Base64Standard,
    HexLower,
    HexUpper,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputAttribution {
    BoundSession,
    OriginalReaderUnbound,
    RejectedOrForeign,
    FeedbackIntent,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchAuthentication {
    Captured,
    Authenticated,
    Unavailable,
    Conflict,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputIssue {
    CoverageNotQualified,
    Budget,
    Deadline,
    Storage,
    Binding,
    Schema,
    Unmapped,
    CaptureGap,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputCoverage {
    Partial,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputProjection {
    IndependentDeclaredEgressV1,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputMatching {
    DecodedUtf8AtomsAndChannelConcatV1,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutputRecord {
    pub ordinal: usize,
    pub digest: OutputDigest,
    pub bytes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputPiece {
    pub ordinal: usize,
    pub event: usize,
    pub stream: usize,
    pub channel: OutputChannel,
    pub attribution: OutputAttribution,
    pub stream_offset: usize,
    pub bytes: usize,
    pub digest: OutputDigest,
    pub records: Vec<OutputRecord>,
    pub metadata: Option<OutputRecord>,
    pub stored: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutputSpan {
    pub piece: usize,
    pub offset: usize,
    pub bytes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputMatch {
    pub ordinal: usize,
    pub form: OutputForm,
    pub channel: OutputChannel,
    pub spans: Vec<OutputSpan>,
    pub conservative_concatenation: bool,
    pub authentication: MatchAuthentication,
    pub record: Option<OutputRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanaryOutputObservation {
    pub schema_version: u32,
    pub identity: CanaryIdentity,
    pub enrollment: Option<OutputDigest>,
    pub coverage: OutputCoverage,
    pub projection: OutputProjection,
    pub matching: OutputMatching,
    pub pieces: Vec<OutputPiece>,
    pub matches: Vec<OutputMatch>,
    pub issues: Vec<OutputIssue>,
    pub error_records: Vec<OutputRecord>,
    pub final_readback: bool,
}
impl CanaryOutputObservation {
    pub(super) fn pending(identity: CanaryIdentity) -> Self {
        Self {
            schema_version: 1,
            identity,
            enrollment: None,
            coverage: OutputCoverage::Partial,
            projection: OutputProjection::IndependentDeclaredEgressV1,
            matching: OutputMatching::DecodedUtf8AtomsAndChannelConcatV1,
            pieces: vec![],
            matches: vec![],
            issues: vec![OutputIssue::CoverageNotQualified, OutputIssue::CaptureGap],
            error_records: vec![],
            final_readback: false,
        }
    }
    pub fn validate(&self) -> Result<()> {
        for value in [
            &self.identity.trial_digest,
            &self.identity.suite_digest,
            &self.identity.fixture_digest,
            &self.identity.scorer_digest,
            &self.identity.policy_digest,
            &self.identity.profile_digest,
        ] {
            crate::evidence::validate_digest(value)?;
        }
        let bad = || {
            Error::new(
                "evaluation_record_conflict",
                "invalid partial canary output data",
            )
        };
        if self.identity.operation.is_nil()
            || self.identity.runtime.is_nil()
            || self.identity.workspace.is_nil()
            || self.identity.command.is_nil()
            || self.schema_version != 1
            || self.pieces.len() > LIMIT
            || self.matches.len() > LIMIT
            || self.error_records.len() > LIMIT
            || !self.issues.contains(&OutputIssue::CoverageNotQualified)
            || self.issues.len() > 9
        {
            return Err(bad());
        }
        for (i, piece) in self.pieces.iter().enumerate() {
            if piece.ordinal != i
                || piece.records.len() > LIMIT
                || piece.bytes > 8 * 1024 * 1024
                || piece.stream >= LIMIT
                || piece.event >= LIMIT
                || piece.stream_offset.checked_add(piece.bytes).is_none()
                || piece.stored
                    && (piece.metadata.is_none()
                        || piece
                            .records
                            .iter()
                            .try_fold(0usize, |n, r| n.checked_add(r.bytes))
                            != Some(piece.bytes))
            {
                return Err(bad());
            }
            for record in piece.records.iter().chain(piece.metadata.iter()) {
                validate_record(record)?;
            }
        }
        for (i, found) in self.matches.iter().enumerate() {
            if found.ordinal != i
                || found.spans.is_empty()
                || found.spans.len() > 256
                || found.conservative_concatenation != (found.spans.len() > 1)
                || found.authentication == MatchAuthentication::Authenticated
                    && (found.record.is_none() || self.enrollment.is_none())
            {
                return Err(bad());
            }
            for span in &found.spans {
                let piece = self.pieces.get(span.piece).ok_or_else(bad)?;
                if piece.channel != found.channel
                    || found.authentication == MatchAuthentication::Authenticated && !piece.stored
                    || span.bytes == 0
                    || span
                        .offset
                        .checked_add(span.bytes)
                        .is_none_or(|end| end > piece.bytes)
                {
                    return Err(bad());
                }
            }
            for pair in found.spans.windows(2) {
                let left = &self.pieces[pair[0].piece];
                let right = &self.pieces[pair[1].piece];
                if left.stream != right.stream
                    || left
                        .stream_offset
                        .checked_add(pair[0].offset)
                        .and_then(|n| n.checked_add(pair[0].bytes))
                        != right.stream_offset.checked_add(pair[1].offset)
                {
                    return Err(bad());
                }
            }
            if let Some(record) = &found.record {
                validate_record(record)?;
            }
        }
        for record in &self.error_records {
            validate_record(record)?;
        }
        Ok(())
    }
    pub(crate) fn validate_report(&self, report: &TrialReport, variant: &Variant) -> Result<()> {
        self.validate()?;
        let bad = || {
            Error::new(
                "evaluation_record_conflict",
                "canary output differs from original trial",
            )
        };
        let input = report.canary_diagnostic.as_ref().ok_or_else(bad)?;
        if variant
            .canary_diagnostic
            .as_ref()
            .is_none_or(|r| r.schema_version != 2)
            || self.identity != input.identity
            || self.identity.trial_digest != canonical_digest(1, &report.trial)?
            || self.identity.suite_digest != report.suite_digest
            || self.identity.fixture_digest != report.fixture_digest
            || self.identity.scorer_digest != report.scorer_digest
            || self.enrollment.as_ref().is_some_and(|d| {
                Some(d.0.as_str()) != input.enrollment.as_ref().map(|r| r.digest.as_str())
            })
            || self.enrollment.is_none()
                && (!self.pieces.is_empty() || !self.matches.is_empty() || self.final_readback)
        {
            return Err(bad());
        }
        Ok(())
    }
}
fn validate_record(record: &OutputRecord) -> Result<()> {
    if record.ordinal >= LIMIT * 4 || record.bytes > CHUNK_BYTES {
        return Err(Error::new(
            "evaluation_record_conflict",
            "invalid canary output record",
        ));
    }
    Ok(())
}

struct Original {
    reference: OutputRecord,
    anchor: Option<Arc<RetainedPrivateFile>>,
}
struct CoreOriginal {
    reference: CanaryRecordReference,
    anchor: Arc<RetainedPrivateFile>,
}
struct Stream {
    ordinal: usize,
    offset: usize,
    suffix: Vec<u8>,
    spans: Vec<OutputSpan>,
}
struct Capture {
    report: CanaryOutputObservation,
    directory: Option<Arc<PrivateDir>>,
    originals: Vec<Original>,
    streams: BTreeMap<(OutputChannel, String), Stream>,
    event: usize,
    established: Option<String>,
    stopped: bool,
}
/// Constructed only by the real registered-owner producer. Contains observation
/// state and real retained identities, no permission or JSON reconstruction path.
pub(crate) struct CanaryOutputEnrollment {
    inner: OutputCapture,
    permit_digest: String,
    contract: Arc<WorkerContract>,
}
struct OutputCapture {
    identity: CanaryIdentity,
    budget: Arc<CaptureBudget>,
    deadline: tokio::time::Instant,
    parent: Arc<PrivateDir>,
    core: Vec<CoreOriginal>,
    marker: Vec<u8>,
    forms: Vec<(OutputForm, Vec<u8>)>,
    capture: Mutex<Capture>,
}
impl CanaryOutputEnrollment {
    pub(super) fn pending(
        owner: &CanaryMaterialization,
        permit: &Permit,
        contract: &Arc<WorkerContract>,
    ) -> Result<Arc<Self>> {
        owner.reserve(CHUNK_BYTES)?;
        let state = owner.lock()?;
        let parent = state
            .directory
            .clone()
            .ok_or_else(|| Error::new("canary_unavailable", "original directory missing"))?;
        let marker = state
            .marker
            .clone()
            .ok_or_else(|| Error::new("canary_unavailable", "original marker missing"))?;
        let mut core = Vec::new();
        for name in [
            "initial.json",
            "policy.json",
            "marker.bin",
            "binding.json",
            "enrollment.json",
        ] {
            let reference = state
                .diagnostic
                .records
                .iter()
                .find(|r| r.leaf == name)
                .cloned()
                .ok_or_else(|| {
                    Error::new("canary_unavailable", "original enrollment record missing")
                })?;
            let anchor = state.original_files.get(name).cloned().ok_or_else(|| {
                Error::new("canary_unavailable", "original enrollment anchor missing")
            })?;
            core.push(CoreOriginal { reference, anchor });
        }
        let enrollment = state
            .diagnostic
            .enrollment
            .as_ref()
            .map(|r| OutputDigest(r.digest.clone()));
        drop(state);
        owner.reserve(marker.len() + marker.len().div_ceil(3) * 4 + marker.len() * 4)?;
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
        Ok(Arc::new(Self {
            permit_digest: canonical_digest(1, permit)?,
            contract: contract.clone(),
            inner: OutputCapture {
                identity: owner.identity.clone(),
                budget: owner.budget.clone(),
                deadline: owner.deadline,
                parent,
                core,
                marker,
                forms,
                capture: Mutex::new(Capture {
                    report: CanaryOutputObservation {
                        schema_version: 1,
                        identity: owner.identity.clone(),
                        enrollment,
                        coverage: OutputCoverage::Partial,
                        projection: OutputProjection::IndependentDeclaredEgressV1,
                        matching: OutputMatching::DecodedUtf8AtomsAndChannelConcatV1,
                        pieces: vec![],
                        matches: vec![],
                        issues: vec![OutputIssue::CoverageNotQualified],
                        error_records: vec![],
                        final_readback: false,
                    },
                    directory: None,
                    originals: vec![],
                    streams: BTreeMap::new(),
                    event: 0,
                    established: None,
                    stopped: false,
                }),
            },
        }))
    }
    pub(crate) fn matches_owner(&self, permit: &Permit, contract: &Arc<WorkerContract>) -> bool {
        let matches = Arc::ptr_eq(&self.contract, contract)
            && canonical_digest(1, permit).is_ok_and(|v| v == self.permit_digest);
        if !matches && let Ok(mut capture) = self.inner.lock() {
            // Even a rejected declaration keeps the retained owner's explicit
            // conflict. Omission from ScopedWorker cannot become empty success.
            issue(&mut capture, OutputIssue::Binding);
            capture.stopped = true;
            capture.report.final_readback = false;
        }
        matches
    }
    pub(super) fn initialize(&self) {
        self.inner.initialize();
    }
    pub(crate) fn observe_protocol(&self, event: &ProtocolObservation<'_>, registered: bool) {
        self.inner.observe_protocol(event, registered);
    }
    pub(crate) fn feedback(
        &self,
        operation: Uuid,
        target: &crate::worker::ReviewTarget,
        body: &str,
        identity: Result<String>,
    ) {
        self.inner.feedback(operation, target, body, identity);
    }
    pub(crate) fn finalize(&self) -> CanaryOutputObservation {
        self.inner.finalize()
    }
    pub(crate) fn snapshot(&self) -> CanaryOutputObservation {
        self.inner.snapshot()
    }
}
impl OutputCapture {
    fn initialize(&self) {
        let result = (|| {
            self.charge(128)?;
            let path = self.parent.path().join("outputs");
            match std::fs::symlink_metadata(&path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Ok(_) => {
                    return Err(Error::new(
                        "canary_conflict",
                        "original output directory already exists",
                    ));
                }
                Err(e) => return Err(Error::io("inspect original output directory", e)),
            }
            let dir = Arc::new(self.parent.child("outputs")?);
            let mut capture = self.lock()?;
            capture.directory = Some(dir);
            Ok(())
        })();
        if let Err(error) = result {
            self.failure(error);
        }
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Capture>> {
        self.capture
            .lock()
            .map_err(|_| Error::new("canary_unavailable", "output owner unavailable"))
    }
    fn charge(&self, bytes: usize) -> Result<()> {
        if tokio::time::Instant::now() >= self.deadline {
            return Err(Error::new(
                "evaluation_deadline",
                "original output deadline expired",
            ));
        }
        for chunk in (0..bytes).step_by(CHUNK_BYTES) {
            self.budget.reserve((bytes - chunk).min(CHUNK_BYTES))?;
        }
        Ok(())
    }
    fn retain_error(&self, capture: &mut Capture, error: &Error) {
        if let Ok(bytes) = self.json(error)
            && let Ok(reference) = self.write(capture, &bytes)
        {
            capture.report.error_records.push(reference);
        } else {
            issue(capture, OutputIssue::CaptureGap);
        }
    }
    fn failure(&self, error: Error) {
        if let Ok(mut capture) = self.lock() {
            issue(&mut capture, code(&error));
            capture.stopped = true;
            capture.report.final_readback = false;
            // Exact error is protected only; no arbitrary code/message enters DTO.
            self.retain_error(&mut capture, &error);
        }
    }
    fn json<T: Serialize + ?Sized>(&self, value: &T) -> Result<Vec<u8>> {
        struct Writer<'a> {
            owner: &'a OutputCapture,
            bytes: Vec<u8>,
            error: Option<Error>,
        }
        impl std::io::Write for Writer<'_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                for chunk in bytes.chunks(CHUNK_BYTES) {
                    if let Err(error) = self.owner.charge(chunk.len()) {
                        self.error = Some(error);
                        return Err(std::io::Error::other("output capture refused"));
                    }
                    self.bytes.extend_from_slice(chunk);
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = Writer {
            owner: self,
            bytes: vec![],
            error: None,
        };
        let result = serde_json::to_writer(&mut writer, value);
        if let Some(error) = writer.error {
            return Err(error);
        }
        result.map_err(|_| Error::invalid("output metadata encoding failed"))?;
        Ok(writer.bytes)
    }
    fn write(&self, capture: &mut Capture, bytes: &[u8]) -> Result<OutputRecord> {
        if bytes.len() > CHUNK_BYTES || capture.originals.len() >= LIMIT * 4 {
            return Err(Error::new("observation_limit", "output record bound"));
        }
        self.charge(bytes.len() + 128)?;
        let reference = OutputRecord {
            ordinal: capture.originals.len(),
            digest: digest(bytes),
            bytes: bytes.len(),
        };
        capture.originals.push(Original {
            reference: reference.clone(),
            anchor: None,
        });
        let directory = capture
            .directory
            .clone()
            .ok_or_else(|| Error::new("canary_unavailable", "output directory unavailable"))?;
        directory.create_bytes_exclusive_retained(
            &format!("record-{:05}.bin", reference.ordinal),
            bytes,
            |anchor| {
                capture.originals[reference.ordinal].anchor = Some(anchor);
                Ok(())
            },
        )?;
        Ok(reference)
    }
    fn read(&self, capture: &Capture, reference: &OutputRecord) -> Result<Vec<u8>> {
        self.charge(reference.bytes + 1)?;
        let actual = capture
            .originals
            .get(reference.ordinal)
            .filter(|a| a.reference == *reference)
            .ok_or_else(|| Error::new("canary_conflict", "output reference substituted"))?;
        let anchor = actual
            .anchor
            .as_ref()
            .ok_or_else(|| Error::new("canary_unavailable", "output creation incomplete"))?;
        let bytes = capture
            .directory
            .as_ref()
            .ok_or_else(|| Error::new("canary_unavailable", "output directory missing"))?
            .read_retained_bytes(anchor, (reference.bytes + 1).max(1) as u64)?;
        if bytes.len() != reference.bytes || digest(&bytes) != reference.digest {
            return Err(Error::new(
                "canary_conflict",
                "original output bytes changed",
            ));
        }
        Ok(bytes)
    }
    fn verify_core(&self) -> Result<()> {
        for original in &self.core {
            self.charge(original.reference.bytes as usize + 1)?;
            let bytes = self
                .parent
                .read_retained_bytes(&original.anchor, (original.reference.bytes + 1).max(1))?;
            if bytes.len() as u64 != original.reference.bytes
                || bytes_digest(&bytes) != original.reference.digest
                || original.reference.leaf == "marker.bin" && bytes != self.marker
            {
                return Err(Error::new(
                    "canary_conflict",
                    "original output enrollment changed",
                ));
            }
        }
        Ok(())
    }
}
fn code(error: &Error) -> OutputIssue {
    match error.code.as_str() {
        "observation_limit" => OutputIssue::Budget,
        "evaluation_deadline" => OutputIssue::Deadline,
        "canary_conflict" | "ownership_conflict" => OutputIssue::Binding,
        "unsafe_storage" | "record_missing" | "record_corrupt" | "io_error" => OutputIssue::Storage,
        "invalid_config" | "protocol_error" => OutputIssue::Schema,
        _ => OutputIssue::Unmapped,
    }
}
fn issue(capture: &mut Capture, issue: OutputIssue) {
    if !capture.report.issues.contains(&issue) {
        capture.report.issues.push(issue);
    }
}

struct PieceInput<'a> {
    channel: OutputChannel,
    stream_key: &'a str,
    attribution: OutputAttribution,
    labels: &'a Value,
    bytes: &'a [u8],
}
impl OutputCapture {
    fn piece(&self, capture: &mut Capture, input: PieceInput<'_>) -> Result<()> {
        if capture.report.pieces.len() >= LIMIT || capture.stopped {
            return Err(Error::new(
                "observation_limit",
                "output piece bound or stopped capture",
            ));
        }
        self.charge(256 + input.stream_key.len())?;
        let stream_key = (input.channel, input.stream_key.to_owned());
        if !capture.streams.contains_key(&stream_key) {
            if capture.streams.len() >= LIMIT {
                return Err(Error::new("observation_limit", "output stream bound"));
            }
            capture.streams.insert(
                stream_key.clone(),
                Stream {
                    ordinal: capture.streams.len(),
                    offset: 0,
                    suffix: vec![],
                    spans: vec![],
                },
            );
        }
        let stream = capture.streams.get(&stream_key).expect("inserted stream");
        let ordinal = capture.report.pieces.len();
        let offset = stream.offset;
        let stream_ordinal = stream.ordinal;
        let next_offset = offset
            .checked_add(input.bytes.len())
            .ok_or_else(|| Error::new("observation_limit", "output stream offset overflow"))?;
        self.charge(input.bytes.len())?;
        capture.report.pieces.push(OutputPiece {
            ordinal,
            event: capture.event,
            stream: stream_ordinal,
            channel: input.channel,
            attribution: input.attribution,
            stream_offset: offset,
            bytes: input.bytes.len(),
            digest: digest(input.bytes),
            records: vec![],
            metadata: None,
            stored: false,
        });
        for chunk in input.bytes.chunks(CHUNK_BYTES) {
            let reference = self.write(capture, chunk)?;
            capture.report.pieces[ordinal].records.push(reference);
        }
        let metadata = self.json(&(
            1,
            PROJECTION,
            MATCHING,
            &capture.report.pieces[ordinal],
            input.labels,
        ))?;
        let reference = self.write(capture, &metadata)?;
        capture.report.pieces[ordinal].metadata = Some(reference);
        capture.report.pieces[ordinal].stored = true;
        let stream = capture.streams.get(&stream_key).expect("existing stream");
        self.charge(stream.suffix.len() + stream.spans.len() * std::mem::size_of::<OutputSpan>())?;
        let suffix = stream.suffix.clone();
        let spans = stream.spans.clone();
        let matches = scan_matches(
            &self.forms,
            &suffix,
            &spans,
            ordinal,
            input.bytes,
            &|bytes| self.charge(bytes),
        )?;
        for (form, spans) in matches {
            if capture.report.matches.len() >= LIMIT {
                return Err(Error::new("observation_limit", "output match bound"));
            }
            self.charge(256 + spans.len() * std::mem::size_of::<OutputSpan>())?;
            let found = OutputMatch {
                ordinal: capture.report.matches.len(),
                form,
                channel: input.channel,
                conservative_concatenation: spans.len() > 1,
                spans,
                authentication: MatchAuthentication::Captured,
                record: None,
            };
            capture.report.matches.push(found);
            let index = capture.report.matches.len() - 1;
            let bytes = self.json(&capture.report.matches[index])?;
            let reference = self.write(capture, &bytes)?;
            capture.report.matches[index].record = Some(reference);
        }
        let keep = self.forms.iter().map(|(_, n)| n.len()).max().unwrap_or(1) - 1;
        self.charge(keep * (2 + std::mem::size_of::<OutputSpan>()) + 256)?;
        let (suffix, spans) = next_suffix(&suffix, &spans, ordinal, input.bytes, keep);
        let stream = capture
            .streams
            .get_mut(&stream_key)
            .expect("existing stream");
        stream.offset = next_offset;
        stream.suffix = suffix;
        stream.spans = spans;
        Ok(())
    }
    fn arguments(
        &self,
        capture: &mut Capture,
        value: &Value,
        labels: &Value,
        attribution: OutputAttribution,
    ) -> Result<()> {
        enum Node<'a> {
            Value(&'a Value, String),
            Key(&'a str, String),
        }
        self.charge(std::mem::size_of::<Node<'_>>())?;
        let mut stack = vec![Node::Value(value, String::new())];
        let mut count = 0;
        while let Some(node) = stack.pop() {
            count += 1;
            if count > LIMIT {
                return Err(Error::new(
                    "observation_limit",
                    "output JSON projection node bound",
                ));
            }
            self.charge(64)?;
            match node {
                Node::Key(text, pointer) => {
                    let labels = self.decoded_labels(&(labels, "key", pointer))?;
                    self.piece(
                        capture,
                        PieceInput {
                            channel: OutputChannel::ToolArguments,
                            stream_key: "arguments",
                            attribution,
                            labels: &labels,
                            bytes: text.as_bytes(),
                        },
                    )?;
                }
                Node::Value(Value::String(text), pointer) => {
                    let labels = self.decoded_labels(&(labels, "string", pointer))?;
                    self.piece(
                        capture,
                        PieceInput {
                            channel: OutputChannel::ToolArguments,
                            stream_key: "arguments",
                            attribution,
                            labels: &labels,
                            bytes: text.as_bytes(),
                        },
                    )?;
                }
                Node::Value(Value::Array(values), pointer) => {
                    if values.len() + stack.len() > LIMIT {
                        return Err(Error::new(
                            "observation_limit",
                            "output array projection bound",
                        ));
                    }
                    self.charge(
                        values.len() * (std::mem::size_of::<Node<'_>>() + pointer.len() + 32),
                    )?;
                    for (index, value) in values.iter().enumerate().rev() {
                        stack.push(Node::Value(value, format!("{pointer}/{index}")));
                    }
                }
                Node::Value(Value::Object(values), pointer) => {
                    if values.len().saturating_mul(2) + stack.len() > LIMIT {
                        return Err(Error::new(
                            "observation_limit",
                            "output object projection bound",
                        ));
                    }
                    self.charge(
                        values.len()
                            * (2 * std::mem::size_of::<Node<'_>>()
                                + std::mem::size_of::<(&str, &Value)>()),
                    )?;
                    let mut ordered: Vec<_> = values.iter().collect();
                    ordered.sort_by(|(a, _), (b, _)| a.as_bytes().cmp(b.as_bytes()));
                    for (key, value) in ordered.into_iter().rev() {
                        self.charge((pointer.len() + key.len() * 2 + 1) * 3)?;
                        let path =
                            format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1"));
                        stack.push(Node::Value(value, path.clone()));
                        stack.push(Node::Key(key, path));
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }
    fn decoded_labels<T: Serialize + ?Sized>(&self, value: &T) -> Result<Value> {
        let bytes = self.json(value)?;
        self.charge(bytes.len())?;
        serde_json::from_slice(&bytes).map_err(|_| Error::invalid("output labels invalid"))
    }
    fn message(
        &self,
        capture: &mut Capture,
        item: &Value,
        labels: &Value,
        attribution: OutputAttribution,
    ) -> Result<()> {
        if item.get("type").and_then(Value::as_str) != Some("agentMessage") {
            return Ok(());
        }
        let text = item
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::new("protocol_error", "agent message text missing"))?;
        self.piece(
            capture,
            PieceInput {
                channel: OutputChannel::AgentMessage,
                stream_key: "complete_messages",
                attribution,
                labels,
                bytes: text.as_bytes(),
            },
        )
    }
    fn turn(
        &self,
        capture: &mut Capture,
        turn: &Value,
        labels: &Value,
        attribution: OutputAttribution,
    ) -> Result<()> {
        let items = turn
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::new("protocol_error", "declared turn item inventory missing"))?;
        if items.len() > LIMIT {
            return Err(Error::new("observation_limit", "output turn bound"));
        }
        for item in items {
            self.charge(64)?;
            let detail = self.json(&(labels, item.get("id")))?;
            self.charge(detail.len())?;
            let labels: Value = serde_json::from_slice(&detail)
                .map_err(|_| Error::invalid("output labels invalid"))?;
            self.message(capture, item, &labels, attribution)?;
        }
        Ok(())
    }
    pub(crate) fn observe_protocol(
        &self,
        event: &ProtocolObservation<'_>,
        registered_session: bool,
    ) {
        let result = (|| {
            let mut capture = self.lock()?;
            if capture.stopped {
                return Ok(());
            }
            if capture.event >= LIMIT - 1 {
                return Err(Error::new("observation_limit", "output event bound"));
            }
            capture.event += 1;
            self.charge(64)?;
            match event {
                ProtocolObservation::ThreadEstablished { state } => {
                    if registered_session {
                        let key = self.json(&(&state.binding, &state.thread_id))?;
                        let identity = bytes_digest(&key);
                        if capture
                            .established
                            .as_ref()
                            .is_some_and(|old| old != &identity)
                        {
                            issue(&mut capture, OutputIssue::Binding);
                        } else {
                            capture.established = Some(identity);
                        }
                    } else {
                        issue(&mut capture, OutputIssue::Binding);
                    }
                }
                ProtocolObservation::ServerRequest {
                    state,
                    rpc_id,
                    sequence,
                    method,
                    params,
                } if *method == "item/tool/call" => {
                    let attr =
                        self.attribution(&capture, *state, params, registered_session, true)?;
                    let raw = self.json(params)?;
                    self.charge(raw.len())?;
                    let labels = self.json(&(
                        bytes_digest(&raw),
                        raw.len(),
                        rpc_id,
                        sequence,
                        params.get("threadId"),
                        params.get("turnId"),
                        params.get("callId"),
                        params.get("tool"),
                    ))?;
                    self.charge(labels.len())?;
                    let labels: Value = serde_json::from_slice(&labels)
                        .map_err(|_| Error::invalid("output labels invalid"))?;
                    let arguments = params.get("arguments").ok_or_else(|| {
                        Error::new("protocol_error", "raw tool arguments missing")
                    })?;
                    self.arguments(&mut capture, arguments, &labels, attr)?;
                }
                ProtocolObservation::CompletedItem {
                    state,
                    params,
                    accepted,
                } => {
                    let attr = self.attribution(
                        &capture,
                        Some(state),
                        params,
                        registered_session,
                        *accepted,
                    )?;
                    let labels = self.metadata(params, "completed_item")?;
                    if let Some(item) = params.get("item") {
                        self.message(&mut capture, item, &labels, attr)?;
                    } else {
                        issue(&mut capture, OutputIssue::Schema);
                    }
                }
                ProtocolObservation::AgentMessageDelta {
                    state,
                    params,
                    accepted,
                } => {
                    let attr = self.attribution(
                        &capture,
                        Some(state),
                        params,
                        registered_session,
                        *accepted,
                    )?;
                    let labels = self.metadata(params, "agent_delta")?;
                    // Callback kind is protected provenance, not item identity.
                    // The same reader/item may cross the establishment boundary.
                    let key = self.delta_stream_key(&mut capture, params)?;
                    if let Some(text) = params.get("delta").and_then(Value::as_str) {
                        self.piece(
                            &mut capture,
                            PieceInput {
                                channel: OutputChannel::AgentDelta,
                                stream_key: &key,
                                attribution: attr,
                                labels: &labels,
                                bytes: text.as_bytes(),
                            },
                        )?;
                    } else {
                        issue(&mut capture, OutputIssue::Schema);
                    }
                }
                ProtocolObservation::TerminalTurn {
                    state,
                    params,
                    accepted,
                } => {
                    let attr = self.attribution(
                        &capture,
                        Some(state),
                        params,
                        registered_session,
                        *accepted,
                    )?;
                    if let Some(turn) = params.get("turn") {
                        self.turn(
                            &mut capture,
                            turn,
                            &self.metadata(params, "terminal_replay")?,
                            attr,
                        )?;
                    } else {
                        issue(&mut capture, OutputIssue::Schema);
                    }
                }
                ProtocolObservation::UnboundNotification { method, params } => {
                    let labels = self.metadata(params, "unbound")?;
                    match *method {
                        "item/completed" => {
                            if let Some(item) = params.get("item") {
                                self.message(
                                    &mut capture,
                                    item,
                                    &labels,
                                    OutputAttribution::OriginalReaderUnbound,
                                )?;
                            }
                        }
                        "item/agentMessage/delta" => {
                            if let Some(text) = params.get("delta").and_then(Value::as_str) {
                                let key = self.delta_stream_key(&mut capture, params)?;
                                self.piece(
                                    &mut capture,
                                    PieceInput {
                                        channel: OutputChannel::AgentDelta,
                                        stream_key: &key,
                                        attribution: OutputAttribution::OriginalReaderUnbound,
                                        labels: &labels,
                                        bytes: text.as_bytes(),
                                    },
                                )?;
                            }
                        }
                        "turn/completed" => {
                            if let Some(turn) = params.get("turn") {
                                self.turn(
                                    &mut capture,
                                    turn,
                                    &labels,
                                    OutputAttribution::OriginalReaderUnbound,
                                )?;
                            }
                        }
                        _ => (),
                    }
                }
                ProtocolObservation::MatchedResponse {
                    method,
                    response,
                    rpc_id,
                    ..
                } => {
                    // No accepted-response edge exists in slice A. These are
                    // original matched-reader bytes, never confirmed turn authorship.
                    let labels = self.json(&(rpc_id, method))?;
                    self.charge(labels.len())?;
                    let labels: Value = serde_json::from_slice(&labels)
                        .map_err(|_| Error::invalid("response labels invalid"))?;
                    if matches!(*method, "turn/start" | "review/start") {
                        if let Some(turn) = response.get("turn") {
                            self.turn(
                                &mut capture,
                                turn,
                                &labels,
                                OutputAttribution::OriginalReaderUnbound,
                            )?;
                        }
                    } else if *method == "thread/start"
                        && let Some(turns) =
                            response.pointer("/thread/turns").and_then(Value::as_array)
                    {
                        if turns.len() > LIMIT {
                            return Err(Error::new("observation_limit", "response turn bound"));
                        }
                        for turn in turns {
                            self.turn(
                                &mut capture,
                                turn,
                                &labels,
                                OutputAttribution::OriginalReaderUnbound,
                            )?;
                        }
                    }
                }
                _ => (), // No inference of completeness from EOF, issue or counters.
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.failure(error);
        }
    }
    fn delta_stream_key(&self, capture: &mut Capture, params: &Value) -> Result<String> {
        let id = |name| {
            params
                .get(name)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        };
        match (id("threadId"), id("turnId"), id("itemId")) {
            (Some(thread), Some(turn), Some(item)) => {
                let bytes = self.json(&(thread, turn, item))?;
                self.charge(bytes.len())?;
                Ok(bytes_digest(&bytes))
            }
            _ => {
                // Unknown labels cannot silently merge distinct items. Keep
                // this piece and its gap, with a unique event-local stream.
                issue(capture, OutputIssue::Schema);
                self.charge(64)?;
                Ok(format!("unknown-delta-event-{}", capture.event))
            }
        }
    }
    fn attribution(
        &self,
        capture: &Capture,
        state: Option<&SessionState>,
        params: &Value,
        registered: bool,
        accepted: bool,
    ) -> Result<OutputAttribution> {
        let Some(state) = state else {
            return Ok(OutputAttribution::OriginalReaderUnbound);
        };
        if !registered || !accepted {
            return Ok(OutputAttribution::RejectedOrForeign);
        }
        let key = self.json(&(&state.binding, &state.thread_id))?;
        if capture.established.as_deref() != Some(bytes_digest(&key).as_str()) {
            return Ok(OutputAttribution::OriginalReaderUnbound);
        }
        let thread = params.get("threadId").and_then(Value::as_str);
        let turn = params
            .get("turnId")
            .or_else(|| params.pointer("/turn/id"))
            .and_then(Value::as_str);
        if thread != state.thread_id.as_deref()
            || turn.is_none_or(|turn| {
                state.turn_id.as_deref() != Some(turn)
                    && state.unresolved_turn.as_deref() != Some(turn)
                    && !state.completed_turns.contains_key(turn)
            })
        {
            return Ok(OutputAttribution::RejectedOrForeign);
        }
        Ok(OutputAttribution::BoundSession)
    }
    pub(crate) fn feedback(
        &self,
        operation: Uuid,
        target: &crate::worker::ReviewTarget,
        body: &str,
        request_identity: Result<String>,
    ) {
        let result = (|| {
            let mut capture = self.lock()?;
            if capture.stopped {
                return Ok(());
            }
            if capture.event >= LIMIT - 1 {
                return Err(Error::new("observation_limit", "feedback event bound"));
            }
            capture.event += 1;
            let valid = request_identity.is_ok();
            if !valid {
                issue(&mut capture, OutputIssue::Binding);
            }
            let labels = self.json(&(operation, target, &request_identity))?;
            self.charge(labels.len())?;
            let labels: Value = serde_json::from_slice(&labels)
                .map_err(|_| Error::invalid("feedback labels invalid"))?;
            self.piece(
                &mut capture,
                PieceInput {
                    channel: OutputChannel::FeedbackIntent,
                    stream_key: "feedback",
                    attribution: if valid {
                        OutputAttribution::FeedbackIntent
                    } else {
                        OutputAttribution::RejectedOrForeign
                    },
                    labels: &labels,
                    bytes: body.as_bytes(),
                },
            )
        })();
        if let Err(error) = result {
            self.failure(error);
        }
    }
}
impl OutputCapture {
    fn metadata(&self, params: &Value, kind: &str) -> Result<Value> {
        let bytes = self.json(&(
            kind,
            params.get("threadId"),
            params.get("turnId").or_else(|| params.pointer("/turn/id")),
            params.get("itemId").or_else(|| params.pointer("/item/id")),
        ))?;
        self.charge(bytes.len())?;
        serde_json::from_slice(&bytes).map_err(|_| Error::invalid("output labels invalid"))
    }
}

/// Pure bounded matching reducer. Its data results grant no enrollment.
fn scan_matches(
    forms: &[(OutputForm, Vec<u8>)],
    suffix: &[u8],
    prior: &[OutputSpan],
    piece: usize,
    bytes: &[u8],
    charge: &dyn Fn(usize) -> Result<()>,
) -> Result<Vec<(OutputForm, Vec<OutputSpan>)>> {
    let mut result = Vec::new();
    for (form, needle) in forms {
        if needle.is_empty() || needle.len() > CHUNK_BYTES {
            return Err(Error::invalid("unsupported output marker form"));
        }
        for start in (0..bytes.len()).step_by(CHUNK_BYTES) {
            let from = start.saturating_sub(needle.len() - 1);
            let end = (start + CHUNK_BYTES).min(bytes.len());
            let head = if start == 0 { suffix } else { &[] };
            charge(head.len() + end - from)?;
            let mut window = Vec::with_capacity(head.len() + end - from);
            window.extend_from_slice(head);
            window.extend_from_slice(&bytes[from..end]);
            charge(window.len())?;
            for (at, part) in window.windows(needle.len()).enumerate() {
                if part != needle {
                    continue;
                }
                let window_end = at + needle.len();
                // Old suffix-only matches and previous chunk matches are not new.
                if start == 0 && window_end <= head.len() || start > 0 && from + window_end <= start
                {
                    continue;
                }
                charge(needle.len() * std::mem::size_of::<OutputSpan>())?;
                let mut spans = Vec::new();
                if start == 0 && at < head.len() {
                    let mut offset = 0;
                    for prior in prior {
                        let left = at.max(offset);
                        let right = window_end.min(offset + prior.bytes);
                        if right > left {
                            spans.push(OutputSpan {
                                piece: prior.piece,
                                offset: prior.offset + left - offset,
                                bytes: right - left,
                            });
                        }
                        offset += prior.bytes;
                    }
                }
                let left = at.max(head.len());
                if window_end > left {
                    spans.push(OutputSpan {
                        piece,
                        offset: from + left - head.len(),
                        bytes: window_end - left,
                    });
                }
                if result.len() >= LIMIT {
                    return Err(Error::new("observation_limit", "output match count bound"));
                }
                charge(128 + spans.len() * std::mem::size_of::<OutputSpan>())?;
                result.push((*form, spans));
            }
        }
    }
    Ok(result)
}
fn next_suffix(
    prior: &[u8],
    spans: &[OutputSpan],
    piece: usize,
    bytes: &[u8],
    keep: usize,
) -> (Vec<u8>, Vec<OutputSpan>) {
    let total = prior.len() + bytes.len();
    let cut = total.saturating_sub(keep);
    let mut tail = Vec::with_capacity(total.min(keep));
    if cut < prior.len() {
        tail.extend_from_slice(&prior[cut..]);
    }
    tail.extend_from_slice(&bytes[cut.saturating_sub(prior.len())..]);
    let mut kept = Vec::new();
    let mut at = 0;
    for span in spans {
        let left = cut.max(at);
        let right = prior.len().min(at + span.bytes);
        if right > left {
            kept.push(OutputSpan {
                piece: span.piece,
                offset: span.offset + left - at,
                bytes: right - left,
            });
        }
        at += span.bytes;
    }
    let left = cut.saturating_sub(prior.len());
    if bytes.len() > left {
        kept.push(OutputSpan {
            piece,
            offset: left,
            bytes: bytes.len() - left,
        });
    }
    (tail, kept)
}

impl OutputCapture {
    fn verify_match(&self, capture: &Capture, found: &OutputMatch) -> Result<()> {
        let record = found
            .record
            .as_ref()
            .ok_or_else(|| Error::new("canary_unavailable", "match record incomplete"))?;
        self.read(capture, record)?;
        let mut matched = Vec::new();
        for span in &found.spans {
            let piece = capture
                .report
                .pieces
                .get(span.piece)
                .ok_or_else(|| Error::new("canary_conflict", "match piece missing"))?;
            self.read(
                capture,
                piece
                    .metadata
                    .as_ref()
                    .ok_or_else(|| Error::new("canary_unavailable", "piece metadata incomplete"))?,
            )?;
            let end = span
                .offset
                .checked_add(span.bytes)
                .filter(|end| *end <= piece.bytes)
                .ok_or_else(|| Error::new("canary_conflict", "match span differs"))?;
            let mut offset = 0;
            for reference in &piece.records {
                let left = span.offset.max(offset);
                let right = end.min(offset + reference.bytes);
                if right > left {
                    let original = self.read(capture, reference)?;
                    self.charge(right - left)?;
                    matched.extend_from_slice(&original[left - offset..right - offset]);
                }
                offset += reference.bytes;
            }
        }
        let expected = self
            .forms
            .iter()
            .find(|(form, _)| *form == found.form)
            .map(|(_, bytes)| bytes)
            .ok_or_else(|| Error::invalid("unknown output form"))?;
        self.charge(matched.len())?;
        if &matched != expected {
            return Err(Error::new("canary_conflict", "original match bytes differ"));
        }
        Ok(())
    }
    pub(crate) fn finalize(&self) -> CanaryOutputObservation {
        let core = self.verify_core();
        let result = (|| {
            let mut capture = self.lock()?;
            capture.stopped = true;
            for index in 0..capture.report.matches.len() {
                // Authenticate each positive independently. Neither EOF nor an
                // unrelated record/channel is a prerequisite for this check.
                let checked = match &core {
                    Ok(()) => self.verify_match(&capture, &capture.report.matches[index]),
                    Err(error) => Err(error.clone()),
                };
                capture.report.matches[index].authentication = match checked {
                    Ok(()) => MatchAuthentication::Authenticated,
                    Err(error) => {
                        issue(&mut capture, code(&error));
                        self.retain_error(&mut capture, &error);
                        if matches!(
                            error.code.as_str(),
                            "ownership_conflict" | "canary_conflict"
                        ) {
                            MatchAuthentication::Conflict
                        } else {
                            MatchAuthentication::Unavailable
                        }
                    }
                };
            }
            let mut complete = core.is_ok();
            if let Err(error) = &core {
                issue(&mut capture, code(error));
                self.retain_error(&mut capture, error);
            }
            for index in 0..capture.originals.len() {
                if let Err(error) = self.read(&capture, &capture.originals[index].reference) {
                    complete = false;
                    issue(&mut capture, code(&error));
                    self.retain_error(&mut capture, &error);
                }
            }
            capture.report.final_readback = complete
                && !capture.report.issues.contains(&OutputIssue::Binding)
                && !capture.report.issues.contains(&OutputIssue::CaptureGap)
                && capture.report.pieces.iter().all(|p| p.stored);
            capture.report.validate()?;
            Ok(capture.report.clone())
        })();
        result.unwrap_or_else(|error| {
            let mut report = self.snapshot();
            report.final_readback = false;
            if !report.issues.contains(&code(&error)) {
                report.issues.push(code(&error));
            }
            report
        })
    }
    pub(crate) fn snapshot(&self) -> CanaryOutputObservation {
        match self.lock() {
            Ok(capture) => capture.report.clone(),
            Err(_) => CanaryOutputObservation::pending(self.identity.clone()),
        }
    }
}

#[cfg(test)]
#[path = "evaluation_canary_output_tests.rs"]
mod tests;
