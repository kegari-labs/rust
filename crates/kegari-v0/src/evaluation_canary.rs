//! Optional finite host-input diagnostic. These data/owners grant no execution.
use super::*;
use crate::storage::{PrivateDir, RetainedPrivateFile};
use crate::worker::{CaptureBudget, WorkerContract};
use ring::rand::{SecureRandom, SystemRandom};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const POLICY_BYTES: usize = 4096;
const CHUNK_BYTES: usize = 64 * 1024;
const MAX_RECORDS: usize = MAX_FILES * 2;
const FORMS: [&str; 4] = ["literal_utf8", "base64_standard", "hex_lower", "hex_upper"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanaryPolicyReference {
    pub schema_version: u32,
    pub path: String,
    pub digest: String,
}
impl CanaryPolicyReference {
    pub(super) fn validate(&self) -> Result<&str> {
        let name = self
            .path
            .strip_prefix("diagnostics/")
            .and_then(|p| p.strip_suffix(".json"))
            .filter(|name| identifier(name))
            .ok_or_else(|| {
                Error::invalid("canary policy requires diagnostics/<identifier>.json")
            })?;
        if self.schema_version != 1 {
            return Err(Error::invalid("unsupported canary policy reference"));
        }
        crate::evidence::validate_digest(&self.digest)?;
        Ok(name)
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    schema_version: u32,
    purpose: String,
    recipe: String,
    placement: String,
    host_input_set: String,
    forms: Vec<String>,
}
fn validate_policy(reference: &CanaryPolicyReference, bytes: &[u8]) -> Result<()> {
    reference.validate()?;
    if bytes.len() > POLICY_BYTES || bytes_digest(bytes) != reference.digest {
        return Err(Error::new(
            "canary_policy_changed",
            "canary policy bytes differ from declaration",
        ));
    }
    let policy: Policy = serde_json::from_slice(bytes)
        .map_err(|_| Error::invalid("invalid canary diagnostic policy"))?;
    if policy.schema_version != 1
        || policy.purpose != "independent_canary_diagnostic"
        || policy.recipe != "os_random_256_ascii_v1"
        || policy.placement != "private_grader_child_v1"
        || policy.host_input_set != "declared_host_inputs_v1"
        || policy.forms.iter().map(String::as_str).collect::<Vec<_>>() != FORMS
    {
        return Err(Error::invalid("unsupported canary diagnostic policy"));
    }
    Ok(())
}
/// The fixed parent is opened without following links. Only the declared leaf
/// is then opened relative to that held directory, never an arbitrary pathname.
pub(super) fn load_policy(
    suite_root: &Path,
    reference: &CanaryPolicyReference,
    before_read: &mut dyn FnMut(usize) -> Result<()>,
) -> Result<Vec<u8>> {
    reference.validate()?;
    let root = suite_root
        .canonicalize()
        .map_err(|e| Error::io("resolve canary suite", e))?;
    let parent = root.join("diagnostics");
    let metadata = std::fs::symlink_metadata(&parent)
        .map_err(|e| Error::io("inspect canary policy directory", e))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Error::new(
            "scope_violation",
            "canary diagnostics parent is not a real directory",
        ));
    }
    #[cfg(unix)]
    let bytes = {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::fs::OpenOptionsExt;
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&parent)
            .map_err(|e| Error::io("open canary policy directory", e))?;
        let leaf = std::ffi::CString::new(format!("{}.json", reference.validate()?))
            .map_err(|_| Error::invalid("invalid canary policy leaf"))?;
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(Error::io(
                "open declared canary policy",
                std::io::Error::last_os_error(),
            ));
        }
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        fixture_capture::read_open_file(&mut file, POLICY_BYTES, before_read)?.into_bytes()
    };
    #[cfg(not(unix))]
    let bytes =
        fixture_capture::read_file_guarded(&root.join(&reference.path), POLICY_BYTES, before_read)?
            .into_bytes();
    validate_policy(reference, &bytes)?;
    Ok(bytes)
}
/// The genuine native fixture includes the standard empty correction array.
/// Absence is accepted for compatible fresh input; any other present shape is
/// unavailable, never an empty history inferred from malformed content.
fn empty_corrections(input: &Value) -> bool {
    match input.get("corrections") {
        None => true,
        Some(Value::Array(values)) => values.is_empty(),
        Some(_) => false,
    }
}
pub(super) fn validate_variant(variant: &Variant) -> Result<()> {
    if let Some(reference) = &variant.canary_diagnostic {
        reference.validate()?;
        if variant.role != AgentRole::IndependentReview
            || variant.phase != "independent_review"
            || variant.seed != Seed::Fresh
        {
            return Err(Error::invalid(
                "canary diagnostic requires a fresh independent review",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanaryIdentity {
    pub runtime: Uuid,
    pub workspace: Uuid,
    pub command: Uuid,
    pub operation: Uuid,
    pub trial_digest: String,
    pub suite_digest: String,
    pub fixture_digest: String,
    pub scorer_digest: String,
    pub policy_digest: String,
    pub profile_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanaryRecordReference {
    pub leaf: String,
    pub digest: String,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanaryBinding {
    pub operation: Uuid,
    pub workspace: Uuid,
    pub instruction_digest: String,
    pub context_digest: String,
    pub tool_contract_digest: String,
    pub permission_digest: String,
    pub isolation_digest: String,
    pub source_revision: String,
    pub materialization_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanaryHostInputs {
    Unavailable { reason: String },
    AbsentInDeclaredSet { input_count: usize },
    Collision { input_digest: String, form: String },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanaryDiagnostic {
    pub schema_version: u32,
    pub identity: CanaryIdentity,
    pub materialized: bool,
    pub initial: Option<CanaryRecordReference>,
    pub binding: Option<CanaryBinding>,
    /// Original protected record of actual permit/registered-contract enrollment.
    pub enrollment: Option<CanaryRecordReference>,
    pub records: Vec<CanaryRecordReference>,
    pub host_inputs: CanaryHostInputs,
    pub provider_input_inventory: String,
    pub output_channel_coverage: String,
    pub final_readback: bool,
    pub error: Option<Error>,
}
impl CanaryDiagnostic {
    pub fn validate(&self) -> Result<()> {
        let invalid = || {
            Error::new(
                "evaluation_record_conflict",
                "invalid canary diagnostic data",
            )
        };
        if self.schema_version != 1
            || self.identity.operation.is_nil()
            || self.identity.command.is_nil()
            || self.identity.runtime.is_nil()
            || self.identity.workspace.is_nil()
            || self.records.len() > MAX_RECORDS
            || self.output_channel_coverage != "not_qualified"
            || self.provider_input_inventory != "unavailable"
        {
            return Err(invalid());
        }
        for digest in [
            &self.identity.trial_digest,
            &self.identity.suite_digest,
            &self.identity.fixture_digest,
            &self.identity.scorer_digest,
            &self.identity.policy_digest,
            &self.identity.profile_digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        let mut names = BTreeSet::new();
        for record in &self.records {
            if record.leaf.is_empty()
                || record.leaf.len() > 128
                || !record
                    .leaf
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
                || record.leaf.contains("..")
                || !names.insert(&record.leaf)
                || record.bytes > CHUNK_BYTES as u64
            {
                return Err(invalid());
            }
            crate::evidence::validate_digest(&record.digest)?;
        }
        if self
            .initial
            .as_ref()
            .is_some_and(|r| r.leaf != "initial.json" || !self.records.contains(r))
            || self.materialized
                && (self.initial.is_none()
                    || !self.records.iter().any(|r| {
                        r.leaf == "policy.json"
                            && r.digest == self.identity.policy_digest
                            && r.bytes <= POLICY_BYTES as u64
                    })
                    || !self.records.iter().any(|r| {
                        r.leaf == "marker.bin"
                            && r.bytes == (b"kegari-private-canary-v1-".len() + 64) as u64
                    }))
            || self.enrollment.as_ref().is_some_and(|r| {
                r.leaf != "enrollment.json" || !self.records.contains(r) || self.binding.is_none()
            })
            || self.final_readback
                && (!self.materialized
                    || self.binding.is_none()
                    || self.enrollment.is_none()
                    || self.error.is_some()
                    || !matches!(
                        self.host_inputs,
                        CanaryHostInputs::AbsentInDeclaredSet { .. }
                    ))
        {
            return Err(invalid());
        }
        if let Some(binding) = &self.binding {
            let bytes = crate::digest::canonical_bytes(binding)?;
            if binding.operation != self.identity.operation
                || binding.workspace != self.identity.workspace
                || self
                    .initial
                    .as_ref()
                    .is_none_or(|r| r.digest != binding.materialization_digest)
                || !self.records.iter().any(|r| {
                    r.leaf == "binding.json"
                        && r.bytes == bytes.len() as u64
                        && r.digest == bytes_digest(&bytes)
                })
                || !matches!(binding.source_revision.len(), 40 | 64)
                || !binding
                    .source_revision
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
            {
                return Err(invalid());
            }
            for digest in [
                &binding.instruction_digest,
                &binding.context_digest,
                &binding.tool_contract_digest,
                &binding.permission_digest,
                &binding.isolation_digest,
            ] {
                crate::evidence::validate_digest(digest)?;
            }
        }
        match &self.host_inputs {
            CanaryHostInputs::AbsentInDeclaredSet { input_count } => {
                if !self.final_readback
                    || *input_count == 0
                    || *input_count > MAX_RECORDS
                    || *input_count
                        != self
                            .records
                            .iter()
                            .filter(|r| r.leaf.starts_with("input-") && r.leaf.ends_with(".json"))
                            .count()
                {
                    return Err(invalid());
                }
            }
            CanaryHostInputs::Collision { input_digest, form } => {
                crate::evidence::validate_digest(input_digest)?;
                if !FORMS.contains(&form.as_str()) {
                    return Err(invalid());
                }
            }
            CanaryHostInputs::Unavailable { reason } if reason.is_empty() || reason.len() > 128 => {
                return Err(invalid());
            }
            _ => (),
        }
        Ok(())
    }
}

struct State {
    directory: Option<Arc<PrivateDir>>,
    marker: Option<Vec<u8>>,
    original_files: BTreeMap<String, Arc<RetainedPrivateFile>>,
    stage_attempted: bool,
    contract: Option<Arc<WorkerContract>>,
    scanned_inputs: usize,
    input_complete: bool,
    diagnostic: CanaryDiagnostic,
}
/// Non-deserializable ownership of original diagnostic files, never permission.
pub(crate) struct CanaryMaterialization {
    identity: CanaryIdentity,
    reference: CanaryPolicyReference,
    budget: Arc<CaptureBudget>,
    deadline: tokio::time::Instant,
    state: Mutex<State>,
}
impl std::fmt::Debug for CanaryMaterialization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never include private marker bytes or their reversible encodings.
        formatter
            .debug_struct("CanaryMaterialization")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}
pub(crate) struct CanaryStageInput<'a> {
    pub runtime: Uuid,
    pub workspace: Uuid,
    pub command: Uuid,
    pub operation: Uuid,
    pub suite: &'a SuiteManifest,
    pub variant: &'a Variant,
    pub trial: &'a TrialKey,
    pub deadline: tokio::time::Instant,
}
impl CanaryMaterialization {
    /// Allocate/retain this owner before any stage I/O, including partial failure.
    pub(crate) fn pending(input: CanaryStageInput<'_>) -> Result<Arc<Self>> {
        validate_variant(input.variant)?;
        verify_trial_declaration(input.suite, input.variant, input.trial)?;
        let reference = input
            .variant
            .canary_diagnostic
            .clone()
            .ok_or_else(|| Error::invalid("canary policy is absent"))?;
        let profile = input
            .suite
            .profiles
            .iter()
            .find(|p| p.name == input.trial.profile)
            .and_then(|p| p.binding.as_ref())
            .ok_or_else(|| Error::invalid("canary trial profile unresolved"))?;
        let identity = CanaryIdentity {
            runtime: input.runtime,
            workspace: input.workspace,
            command: input.command,
            operation: input.operation,
            trial_digest: canonical_digest(1, input.trial)?,
            suite_digest: input.suite.digest()?,
            fixture_digest: input.variant.fixture_digest.clone(),
            scorer_digest: input.variant.scorer_digest.clone(),
            policy_digest: reference.digest.clone(),
            profile_digest: canonical_digest(1, profile)?,
        };
        let diagnostic = CanaryDiagnostic {
            schema_version: 1,
            identity: identity.clone(),
            materialized: false,
            initial: None,
            binding: None,
            enrollment: None,
            records: vec![],
            host_inputs: CanaryHostInputs::Unavailable {
                reason: "materialization_pending".into(),
            },
            provider_input_inventory: "unavailable".into(),
            output_channel_coverage: "not_qualified".into(),
            final_readback: false,
            error: None,
        };
        diagnostic.validate()?;
        Ok(Arc::new(Self {
            identity,
            reference,
            budget: Arc::new(CaptureBudget::default()),
            deadline: input.deadline,
            state: Mutex::new(State {
                directory: None,
                marker: None,
                original_files: BTreeMap::new(),
                stage_attempted: false,
                contract: None,
                scanned_inputs: 0,
                input_complete: false,
                diagnostic,
            }),
        }))
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>> {
        self.state
            .lock()
            .map_err(|_| Error::new("canary_unavailable", "canary owner lock unavailable"))
    }
    fn current(&self) -> Result<()> {
        if tokio::time::Instant::now() >= self.deadline {
            return Err(Error::new(
                "evaluation_deadline",
                "original canary work deadline expired",
            ));
        }
        Ok(())
    }
    fn reserve(&self, bytes: usize) -> Result<()> {
        self.current()?;
        self.budget.reserve(bytes)
    }
    pub(crate) fn note_failure(&self, error: &Error) {
        if let Ok(mut state) = self.state.lock() {
            state.diagnostic.final_readback = false;
            if !matches!(
                state.diagnostic.host_inputs,
                CanaryHostInputs::Collision { .. }
            ) {
                state.diagnostic.host_inputs = CanaryHostInputs::Unavailable {
                    reason: error.code.clone(),
                };
            }
            if state.diagnostic.error.is_none() {
                state.diagnostic.error = Some(error.clone());
            }
        }
    }
    pub(crate) fn diagnostic(&self) -> Result<CanaryDiagnostic> {
        let diagnostic = self.lock()?.diagnostic.clone();
        diagnostic.validate()?;
        Ok(diagnostic)
    }
    pub(crate) fn stage(&self, suite_root: &Path, grader: &PrivateDir) -> Result<()> {
        let result = self.stage_inner(suite_root, grader);
        if let Err(error) = &result {
            self.note_failure(error);
        }
        result
    }
    fn stage_inner(&self, suite_root: &Path, grader: &PrivateDir) -> Result<()> {
        {
            let mut state = self.lock()?;
            if state.stage_attempted {
                return Err(Error::new(
                    "canary_conflict",
                    "original canary cannot be regenerated",
                ));
            }
            state.stage_attempted = true;
        }
        self.current()?;
        let policy = load_policy(suite_root, &self.reference, &mut |bytes| {
            self.reserve(bytes)
        })?;
        // One generation attempt; error or collision never selects another value.
        self.reserve(32)?;
        let mut random = [0; 32];
        SystemRandom::new()
            .fill(&mut random)
            .map_err(|_| Error::new("canary_unavailable", "OS canary randomness unavailable"))?;
        let marker = format!("kegari-private-canary-v1-{}", hex(&random, false)).into_bytes();
        self.reserve(marker.len())?;
        grader.revalidate()?;
        let path = grader.path().join("canary-diagnostic");
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                return Err(Error::new(
                    "canary_conflict",
                    "original canary directory already exists",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(Error::io("inspect original canary directory", error)),
        }
        // Existing protected storage creates descriptor-relative without
        // following any ancestor. Every data record below is independently
        // exclusive, including if another owner creates this child concurrently.
        let directory = Arc::new(grader.child("canary-diagnostic")?);
        grader.revalidate()?;
        {
            let mut state = self.lock()?;
            state.directory = Some(directory);
            state.marker = Some(marker.clone());
        }
        let initial = self.captured_json(&serde_json::json!({"schema_version":1,"identity":self.identity,
            "policy":self.reference,"marker_digest":bytes_digest(&marker),"marker_bytes":marker.len(),
            "recipe":"os_random_256_ascii_v1","forms":FORMS,"worker_binding":"pending"}))?;
        let reference = self.write_original("initial.json", &initial)?;
        self.lock()?.diagnostic.initial = Some(reference);
        self.write_original("policy.json", &policy)?;
        self.write_original("marker.bin", &marker)?;
        self.verify_originals()?;
        self.lock()?.diagnostic.materialized = true;
        Ok(())
    }
    fn reserve_chunks(&self, bytes: usize) -> Result<()> {
        let mut remaining = bytes;
        while remaining != 0 {
            let next = remaining.min(CHUNK_BYTES);
            self.reserve(next)?;
            remaining -= next;
        }
        Ok(())
    }
    /// Charge each emitted JSON chunk before retaining it; canonical ordering
    /// then operates only on this already bounded finite representation.
    fn captured_json<T: Serialize + ?Sized>(&self, value: &T) -> Result<Vec<u8>> {
        struct ChargedWriter<'a> {
            owner: &'a CanaryMaterialization,
            bytes: Vec<u8>,
            error: Option<Error>,
        }
        impl std::io::Write for ChargedWriter<'_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                for chunk in bytes.chunks(CHUNK_BYTES) {
                    if let Err(error) = self.owner.reserve(chunk.len()) {
                        self.error = Some(error);
                        return Err(std::io::Error::other("canary JSON capture refused"));
                    }
                    self.bytes.extend_from_slice(chunk);
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = ChargedWriter {
            owner: self,
            bytes: Vec::new(),
            error: None,
        };
        let encoded = serde_json::to_writer(&mut writer, value);
        if let Some(error) = writer.error {
            return Err(error);
        }
        encoded.map_err(|_| Error::invalid("canary input cannot be encoded"))?;
        self.reserve_chunks(writer.bytes.len())?;
        // Canonical serialization preserves the existing no-float identity rule.
        // Its work is charged from the same exact serialized input size first.
        crate::digest::canonical_bytes(value)
    }
    fn write_original(&self, leaf: &str, bytes: &[u8]) -> Result<CanaryRecordReference> {
        if bytes.len() > CHUNK_BYTES {
            return Err(Error::new(
                "observation_limit",
                "canary record exceeds chunk bound",
            ));
        }
        self.reserve(bytes.len())?;
        let reference = CanaryRecordReference {
            leaf: leaf.into(),
            digest: bytes_digest(bytes),
            bytes: bytes.len() as u64,
        };
        let directory = {
            let mut state = self.lock()?;
            if state.diagnostic.records.len() >= MAX_RECORDS
                || state.diagnostic.records.iter().any(|r| r.leaf == leaf)
            {
                return Err(Error::new(
                    "canary_conflict",
                    "original canary record identity already used or exhausted",
                ));
            }
            // Retain the expected original before the fallible exclusive write.
            state.diagnostic.records.push(reference.clone());
            state
                .directory
                .clone()
                .ok_or_else(|| Error::new("canary_unavailable", "canary directory absent"))?
        };
        directory.create_bytes_exclusive_retained(leaf, bytes, |anchor| {
            let mut state = self.lock()?;
            if state.original_files.contains_key(leaf) {
                return Err(Error::new(
                    "canary_conflict",
                    "original private identity already retained",
                ));
            }
            state.original_files.insert(leaf.to_owned(), anchor);
            Ok(())
        })?;
        Ok(reference)
    }
    fn verify_originals(&self) -> Result<()> {
        let (directory, references, anchors) = {
            let state = self.lock()?;
            (
                state
                    .directory
                    .clone()
                    .ok_or_else(|| Error::new("canary_unavailable", "canary directory absent"))?,
                state.diagnostic.records.clone(),
                state.original_files.clone(),
            )
        };
        for reference in references {
            self.reserve(reference.bytes as usize + 1)?;
            let anchor = anchors.get(&reference.leaf).ok_or_else(|| {
                Error::new(
                    "canary_unavailable",
                    "original private creation identity unavailable",
                )
            })?;
            let bytes = directory
                .read_retained_bytes(anchor, reference.bytes.saturating_add(1).max(1))
                .map_err(|error| {
                    if error.code == "ownership_conflict" {
                        Error::new(
                            "canary_conflict",
                            "original canary private file identity changed",
                        )
                    } else {
                        error
                    }
                })?;
            if bytes.len() as u64 != reference.bytes || bytes_digest(&bytes) != reference.digest {
                return Err(Error::new(
                    "canary_conflict",
                    "original canary protected bytes changed",
                ));
            }
        }
        Ok(())
    }
}
fn hex(bytes: &[u8], upper: bool) -> String {
    const LOWER: &[u8] = b"0123456789abcdef";
    const UPPER: &[u8] = b"0123456789ABCDEF";
    let alphabet = if upper { UPPER } else { LOWER };
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(alphabet[(byte >> 4) as usize] as char);
        result.push(alphabet[(byte & 15) as usize] as char);
    }
    result
}

impl CanaryMaterialization {
    pub(super) fn bind_actual(
        &self,
        contract: &Arc<WorkerContract>,
        fixture: &PreparedFixture,
    ) -> Result<()> {
        let result = self.bind_inner(contract, fixture);
        if let Err(error) = &result {
            self.note_failure(error);
        }
        result
    }
    fn bind_inner(&self, contract: &Arc<WorkerContract>, fixture: &PreparedFixture) -> Result<()> {
        self.current()?;
        self.verify_originals()?;
        let assembled = contract.instructions();
        let manifest = &assembled.manifest;
        let state = self.lock()?;
        if !state.diagnostic.materialized
            || state.contract.is_some()
            || state.diagnostic.error.is_some()
            || manifest.operation_id != self.identity.operation
            || manifest.workspace_id != self.identity.workspace
            || fixture.fixture_digest != self.identity.fixture_digest
            || fixture.worker_manifest.as_ref() != Some(&assembled.manifest_digest)
            || manifest.role != AgentRole::IndependentReview
            || manifest.phase != "independent_review"
            || !matches!(
                contract.authority(),
                crate::worker::WorkerAuthority::Independent(_)
            )
            || fixture.prior.is_some()
            || !empty_corrections(&fixture.input)
            || !manifest.permissions.source_write_paths.is_empty()
            || !manifest.permissions.resource_write_paths.is_empty()
            || fixture.context_binding.is_none()
        {
            return Err(Error::new(
                "canary_binding",
                "canary requires its original bound fresh independent worker",
            ));
        }
        let directory = state
            .directory
            .clone()
            .ok_or_else(|| Error::new("canary_unavailable", "canary directory unavailable"))?;
        let initial =
            state.diagnostic.initial.clone().ok_or_else(|| {
                Error::new("canary_unavailable", "canary initial record unavailable")
            })?;
        drop(state);
        verify_grader_boundary(contract.isolation(), &[directory.path().to_owned()])?;
        verify_grader_instructions(manifest, &[directory.path().to_owned()])?;
        if !matches!(
            manifest.provider_discovery,
            instructions::ProviderDiscovery::Unavailable { .. }
        ) {
            // v1 names the current finite host set; future discovery needs an
            // explicit projection rather than silently widening this one.
            return Err(Error::new(
                "canary_inputs_unavailable",
                "provider input projection is unsupported by policy v1",
            ));
        }
        let binding = CanaryBinding {
            operation: manifest.operation_id,
            workspace: manifest.workspace_id,
            instruction_digest: assembled.manifest_digest.clone(),
            context_digest: manifest.context_digest.clone(),
            tool_contract_digest: contract.tools().digest().into(),
            permission_digest: canonical_digest(1, &manifest.permissions)?,
            isolation_digest: contract.isolation().digest().into(),
            source_revision: manifest.repository_revision.clone().ok_or_else(|| {
                Error::new("canary_binding", "actual source revision unavailable")
            })?,
            materialization_digest: initial.digest,
        };
        let bytes = self.captured_json(&binding)?;
        self.write_original("binding.json", &bytes)?;
        let mut state = self.lock()?;
        state.contract = Some(contract.clone());
        state.diagnostic.binding = Some(binding);
        Ok(())
    }
    /// Called by the genuine ScopedWorker after its actual registered owner gate.
    pub(crate) fn verify_registered(
        &self,
        permit: &crate::control::Permit,
        contract: &Arc<WorkerContract>,
        identity: &IndependentBoundaryIdentity,
    ) -> Result<Arc<CaptureBudget>> {
        self.current()?;
        let state = self.lock()?;
        if state
            .contract
            .as_ref()
            .is_none_or(|actual| !Arc::ptr_eq(actual, contract))
            || state.diagnostic.error.is_some()
            || !state.input_complete
            || state.diagnostic.enrollment.is_some()
            || permit.operation_id != self.identity.operation
            || identity.owner_generation != permit.lease.owner_generation
            || identity.owner != permit.lease.owner_id
            || identity.permit_digest != canonical_digest(1, permit)?
            || identity.contract_scope_digest != permit.scope_digest
            || identity.operation != self.identity.operation
            || identity.runtime != self.identity.runtime
            || identity.workspace != self.identity.workspace
            || identity.instruction_manifest_digest != contract.instructions().manifest_digest
            || identity.context_digest != contract.instructions().manifest.context_digest
            || identity.tool_contract_digest != contract.tools().digest()
            || identity.isolation_digest != contract.isolation().digest()
        {
            return Err(Error::new(
                "canary_binding",
                "canary enrollment lost actual registered worker identity",
            ));
        }
        drop(state);
        // This is after the real host callback returns, never file I/O or a
        // nested Control call inside that callback. Initial/binding stay immutable.
        let bytes = self.captured_json(&serde_json::json!({"schema_version":1,
            "identity":self.identity,"actual_permit":permit,"registered_boundary":identity}))?;
        let enrollment = self.write_original("enrollment.json", &bytes)?;
        self.lock()?.diagnostic.enrollment = Some(enrollment);
        Ok(self.budget.clone())
    }
    pub(super) fn verify_trial(
        &self,
        suite: &SuiteManifest,
        variant: &Variant,
        trial: &TrialKey,
        profile: &ProfileBinding,
    ) -> Result<()> {
        if self.identity.suite_digest != suite.digest()?
            || self.identity.trial_digest != canonical_digest(1, trial)?
            || self.identity.fixture_digest != variant.fixture_digest
            || self.identity.scorer_digest != variant.scorer_digest
            || variant.canary_diagnostic.as_ref() != Some(&self.reference)
            || self.identity.profile_digest != canonical_digest(1, profile)?
        {
            return Err(Error::new(
                "canary_binding",
                "canary owner differs from actual declared trial",
            ));
        }
        Ok(())
    }
    pub(super) fn preflight(
        &self,
        fixture: &PreparedFixture,
        assembled: &AssembledInstructions,
        context: &Value,
        tools: &BoundToolContracts,
        current: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        let result = self.preflight_inner(fixture, assembled, context, tools, current);
        if let Err(error) = &result {
            self.note_failure(error);
        }
        result
    }
    fn preflight_inner(
        &self,
        fixture: &PreparedFixture,
        assembled: &AssembledInstructions,
        context: &Value,
        tools: &BoundToolContracts,
        current: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        current()?;
        self.current()?;
        self.verify_originals()?;
        {
            let state = self.lock()?;
            if state.input_complete
                || state.scanned_inputs != 0
                || state.diagnostic.error.is_some()
                || state
                    .diagnostic
                    .binding
                    .as_ref()
                    .is_none_or(|b| b.instruction_digest != assembled.manifest_digest)
            {
                return Err(Error::new(
                    "canary_conflict",
                    "canary input projection cannot reset or rebind",
                ));
            }
        }
        // Reserve before additional encoding, hashing or input scanning. These
        // are the exact already-bound values, not a reconstruction from reports.
        self.reserve_chunks(assembled.prompt.len())?;
        let context_bytes = self.captured_json(context)?;
        let tool_bytes = self.captured_json(tools.definitions())?;
        let input_bytes = self.captured_json(&fixture.input)?;
        if assembled.manifest.prompt_digest != bytes_digest(assembled.prompt.as_bytes())
            || assembled.manifest.context_digest != canonical_digest(1, context)?
            || fixture.worker_manifest.as_ref() != Some(&assembled.manifest_digest)
            || fixture.prior.is_some()
            || !empty_corrections(&fixture.input)
        {
            return Err(Error::new(
                "canary_binding",
                "actual canary input identity changed",
            ));
        }
        tools.verify(assembled, tools.definitions())?;
        self.capture_input("assembled_prompt", assembled.prompt.as_bytes(), current)?;
        self.capture_input("bound_context", &context_bytes, current)?;
        self.capture_input("tool_definitions", &tool_bytes, current)?;
        self.capture_input("fixture_input", &input_bytes, current)?;
        let tree =
            fixture_capture::CapturedTree::capture_observed(&fixture.repository, &mut |bytes| {
                current()?;
                self.reserve(bytes)
            })?;
        if tree.snapshot() != fixture.before {
            return Err(Error::new(
                "canary_binding",
                "owned source capture differs from bound fixture",
            ));
        }
        for (path, file) in tree.entries() {
            self.capture_input(&format!("source:{path}"), file.bytes(), current)?;
        }
        for supplied in &assembled.manifest.supplied_files {
            let captured =
                fixture_capture::read_file_guarded(&supplied.path, supplied.bytes, &mut |bytes| {
                    current()?;
                    self.reserve(bytes)
                })?;
            if captured.bytes().len() != supplied.bytes
                || bytes_digest(captured.bytes()) != supplied.digest
            {
                return Err(Error::new(
                    "canary_binding",
                    "original supplied instruction bytes changed",
                ));
            }
            self.capture_input(
                &format!("supplied:{}", supplied.path.display()),
                captured.bytes(),
                current,
            )?;
        }
        fixture.verify_initial_scratch()?;
        current()?;
        self.verify_originals()?;
        self.lock()?.input_complete = true;
        Ok(())
    }
    fn capture_input(
        &self,
        label: &str,
        bytes: &[u8],
        current: &dyn Fn() -> Result<()>,
    ) -> Result<()> {
        current()?;
        // The fixed recipe's private marker copy and encoded forms are charged
        // before allocation, rather than borrowed from a fresh capture budget.
        self.reserve(b"kegari-private-canary-v1-".len() + 64)?;
        let (marker, index) = {
            let state = self.lock()?;
            (
                state.marker.clone().ok_or_else(|| {
                    Error::new("canary_unavailable", "original marker unavailable")
                })?,
                state.scanned_inputs,
            )
        };
        if index >= MAX_FILES || label.len() > 4096 {
            return Err(Error::new(
                "observation_limit",
                "canary input inventory exceeds bound",
            ));
        }
        let form_bytes = marker.len() + marker.len().div_ceil(3) * 4 + marker.len() * 4;
        self.reserve(form_bytes)?;
        let forms = marker_forms(&marker);
        let matched = self.scan_input(bytes, &forms, current)?;
        let digest = self.input_digest(bytes, current)?;
        if let Some(form) = matched {
            // Preserve the observed collision even if its following immutable
            // payload write fails; no retry can replace it with a negative.
            self.lock()?.diagnostic.host_inputs = CanaryHostInputs::Collision {
                input_digest: digest.clone(),
                form: form.into(),
            };
        }
        let mut chunks = Vec::new();
        for (part, chunk) in bytes.chunks(CHUNK_BYTES).enumerate() {
            current()?;
            chunks.push(self.write_original(&format!("input-{index:04}-{part:04}.bin"), chunk)?);
        }
        let record = self.captured_json(
            &serde_json::json!({"schema_version":1,"index":index,"label":label,
            "digest":digest,"bytes":bytes.len(),"chunks":chunks,"forms":FORMS,
            "matched_form":matched}),
        )?;
        self.write_original(&format!("input-{index:04}.json"), &record)?;
        self.lock()?.scanned_inputs += 1;
        if matched.is_some() {
            return Err(Error::new(
                "canary_input_collision",
                "private marker appears in the declared host input set",
            ));
        }
        Ok(())
    }
    fn scan_input(
        &self,
        bytes: &[u8],
        forms: &[(&'static str, Vec<u8>)],
        current: &dyn Fn() -> Result<()>,
    ) -> Result<Option<&'static str>> {
        // Preserve fixed form precedence. Each actual scan is bounded and
        // guarded; overlap retains matches spanning a chunk boundary.
        for (form, needle) in forms {
            if needle.is_empty() || needle.len() > CHUNK_BYTES {
                return Err(Error::new("canary_binding", "unsupported marker form size"));
            }
            for start in (0..bytes.len()).step_by(CHUNK_BYTES) {
                current()?;
                let from = start.saturating_sub(needle.len() - 1);
                let end = start.saturating_add(CHUNK_BYTES).min(bytes.len());
                let window = &bytes[from..end];
                self.reserve(window.len())?;
                if window.windows(needle.len()).any(|part| part == needle) {
                    return Ok(Some(*form));
                }
            }
        }
        current()?;
        self.current()?;
        Ok(None)
    }
    fn input_digest(&self, bytes: &[u8], current: &dyn Fn() -> Result<()>) -> Result<String> {
        use sha2::{Digest, Sha256};
        let mut digest = Sha256::new();
        for chunk in bytes.chunks(CHUNK_BYTES) {
            current()?;
            self.reserve(chunk.len())?;
            digest.update(chunk);
        }
        current()?;
        self.reserve(64)?;
        Ok(format!("sha256:{}", hex(&digest.finalize(), false)))
    }
    pub(crate) fn finalize(&self) -> CanaryDiagnostic {
        let result = self
            .current()
            .and_then(|()| self.verify_originals())
            .and_then(|()| {
                let mut state = self.lock()?;
                if !state.input_complete
                    || state.contract.is_none()
                    || state.diagnostic.enrollment.is_none()
                    || state.diagnostic.error.is_some()
                {
                    return Err(Error::new(
                        "canary_inputs_unavailable",
                        "original canary input projection is incomplete",
                    ));
                }
                state.diagnostic.final_readback = true;
                state.diagnostic.host_inputs = CanaryHostInputs::AbsentInDeclaredSet {
                    input_count: state.scanned_inputs,
                };
                Ok(())
            });
        if let Err(error) = &result {
            self.note_failure(error);
        }
        // Poisoned state never grants a negative; retain the original identity.
        self.diagnostic().unwrap_or_else(|error| CanaryDiagnostic {
            schema_version: 1,
            identity: self.identity.clone(),
            materialized: false,
            initial: None,
            binding: None,
            enrollment: None,
            records: vec![],
            host_inputs: CanaryHostInputs::Unavailable {
                reason: error.code.clone(),
            },
            provider_input_inventory: "unavailable".into(),
            output_channel_coverage: "not_qualified".into(),
            final_readback: false,
            error: Some(error),
        })
    }
}
fn marker_forms(marker: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    use base64::Engine;
    vec![
        (FORMS[0], marker.to_vec()),
        (
            FORMS[1],
            base64::engine::general_purpose::STANDARD
                .encode(marker)
                .into_bytes(),
        ),
        (FORMS[2], hex(marker, false).into_bytes()),
        (FORMS[3], hex(marker, true).into_bytes()),
    ]
}

impl CanaryDiagnostic {
    pub(crate) fn validate_report(
        &self,
        report: &TrialReport,
        variant: &Variant,
        profile: Option<&ProfileBinding>,
    ) -> Result<()> {
        self.validate()?;
        let conflict = || {
            Error::new(
                "evaluation_record_conflict",
                "canary diagnostic contradicts original report identity",
            )
        };
        let policy = variant.canary_diagnostic.as_ref().ok_or_else(conflict)?;
        if self.identity.trial_digest != canonical_digest(1, &report.trial)?
            || self.identity.suite_digest != report.suite_digest
            || self.identity.fixture_digest != report.fixture_digest
            || self.identity.scorer_digest != report.scorer_digest
            || self.identity.policy_digest != policy.digest
            || self.identity.profile_digest != canonical_digest(1, profile.ok_or_else(conflict)?)?
        {
            return Err(conflict());
        }
        if report.profile.as_ref().is_some_and(|actual| {
            canonical_digest(1, actual).is_ok_and(|digest| digest != self.identity.profile_digest)
        }) {
            return Err(conflict());
        }
        if let Some(protocol) = &report.declared_protocol
            && (protocol.operation != self.identity.operation
                || protocol.runtime != self.identity.runtime
                || protocol.workspace != self.identity.workspace
                || protocol.trial_digest.as_deref() != Some(self.identity.trial_digest.as_str())
                || protocol.suite_digest != self.identity.suite_digest
                || protocol.fixture_digest != self.identity.fixture_digest)
        {
            return Err(conflict());
        }
        if let Some(broker) = &report.independent_broker
            && (broker.operation != self.identity.operation
                || broker.runtime != self.identity.runtime
                || broker.workspace != self.identity.workspace
                || broker.trial_digest != self.identity.trial_digest
                || broker.suite_digest != self.identity.suite_digest
                || broker.fixture_digest != self.identity.fixture_digest)
        {
            return Err(conflict());
        }
        if let Some(boundary) = &report.independent_boundary
            && let IndependentBoundaryCheck::MatchedAtCollection { identity } = &boundary.check
            && (identity.operation != self.identity.operation
                || identity.runtime != self.identity.runtime
                || identity.workspace != self.identity.workspace)
        {
            return Err(conflict());
        }
        if let Some(binding) = &self.binding {
            match &report.instruction_manifest {
                Some(value) => {
                    let manifest: instructions::InstructionManifest =
                        serde_json::from_value(value.clone()).map_err(|_| conflict())?;
                    if manifest.role != AgentRole::IndependentReview
                        || manifest.phase != "independent_review"
                        || manifest.operation_id != self.identity.operation
                        || manifest.workspace_id != self.identity.workspace
                        || binding.instruction_digest
                            != canonical_digest(instructions::ASSEMBLY_SCHEMA, &manifest)?
                        || binding.context_digest != manifest.context_digest
                        || binding.permission_digest != canonical_digest(1, &manifest.permissions)?
                        || manifest.repository_revision.as_deref()
                            != Some(binding.source_revision.as_str())
                        || report.tool_contract_digest.as_deref()
                            != Some(binding.tool_contract_digest.as_str())
                        || manifest.permissions.enforcement_profile != binding.isolation_digest
                    {
                        return Err(conflict());
                    }
                }
                None if self.final_readback => return Err(conflict()),
                None => (), // Original pre-model failure can retain binding data, never absence success.
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "evaluation_canary_tests.rs"]
mod tests;
