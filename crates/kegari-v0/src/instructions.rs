//! Immutable runtime instructions, explicit repository context and provenance.
//! Assembly does not attest provider discovery or grant sandbox permissions.
use crate::config::{Profile, validate_write_path};
use crate::digest::{bytes_digest, canonical_digest};
use crate::error::{Error, Result};
use crate::workflow::{Workflow, canonical_roots, contained_path};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const ASSEMBLY_SCHEMA: u32 = 1;
pub const ASSET_VERSION: &str = "1";
pub const MAX_CONTEXT_BYTES: usize = 1_048_576;
pub const SUPPORTED_CODEX_VERSION: &str = "0.164.0-alpha.11.1";
pub const STARTER: &str = include_str!("../examples/WORKFLOW.md");
const ASSET_MANIFEST: &str = include_str!("../instructions/manifest.json");
const ASSETS: &[(&str, &str)] = &[
    ("authority", include_str!("../instructions/authority.md")),
    ("child", include_str!("../instructions/child.md")),
    (
        "coordination",
        include_str!("../instructions/coordination.md"),
    ),
    ("review", include_str!("../instructions/review.md")),
    ("validation", include_str!("../instructions/validation.md")),
    (
        "continuation",
        include_str!("../instructions/continuation.md"),
    ),
];
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Implementation,
    Testing,
    Documentation,
    Coordination,
    Review,
    IndependentReview,
    Validation,
}
impl AgentRole {
    fn asset(self) -> &'static str {
        match self {
            Self::Implementation | Self::Testing | Self::Documentation => "child",
            Self::Coordination => "coordination",
            Self::Review | Self::IndependentReview => "review",
            Self::Validation => "validation",
        }
    }
    pub fn permits(self, capability: ToolCapability) -> bool {
        match capability {
            ToolCapability::ContextRead | ToolCapability::OutcomeSubmit => true,
            ToolCapability::SourceWrite => self.permits_source_writes(),
            ToolCapability::CorrectionProposal => self == Self::Coordination,
            ToolCapability::ReviewFeedback => {
                matches!(self, Self::Review | Self::IndependentReview)
            }
            ToolCapability::ValidationResult => matches!(
                self,
                Self::Implementation | Self::Testing | Self::Documentation | Self::Validation
            ),
        }
    }
    pub fn permits_source_writes(self) -> bool {
        matches!(
            self,
            Self::Implementation | Self::Testing | Self::Documentation
        )
    }
    pub fn permits_repository_execution(self) -> bool {
        matches!(
            self,
            Self::Implementation | Self::Testing | Self::Documentation | Self::Validation
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCapability {
    ContextRead,
    SourceWrite,
    OutcomeSubmit,
    CorrectionProposal,
    ReviewFeedback,
    ValidationResult,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExposedTool {
    pub name: String,
    pub capability: ToolCapability,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionManifest {
    pub read_roots: Vec<PathBuf>,
    pub source_write_paths: Vec<PathBuf>,
    #[serde(default)]
    pub resource_write_paths: Vec<PathBuf>,
    pub scratch_roots: Vec<PathBuf>,
    pub repository_code_execution: bool,
    /// Version/digest of the concrete enforcement policy, not an assertion that
    /// this declaration was tested. The adapter must verify enforcement.
    pub enforcement_profile: String,
    pub network: WorkerNetwork,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerNetwork {
    Denied,
    BrokerOnly,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProviderDiscovery {
    Unavailable {
        reason: String,
    },
    Confirmed {
        protocol_source: String,
        files: Vec<ContextFile>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextFile {
    pub path: PathBuf,
    pub digest: String,
    pub bytes: usize,
    pub kind: ContextKind,
    pub applies_under: Option<PathBuf>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    GlobalInstruction,
    RepositoryInstruction,
    RequiredContext,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionManifest {
    pub schema_version: u32,
    pub runtime_version: String,
    pub runtime_build: String,
    pub asset_version: String,
    pub assets: BTreeMap<String, String>,
    pub workflow_path: PathBuf,
    pub workflow_digest: String,
    pub role: AgentRole,
    pub phase: String,
    pub workspace_id: Uuid,
    pub operation_id: Uuid,
    pub work_generation: String,
    pub correction_revision: String,
    pub context_digest: String,
    pub prompt_digest: String,
    pub repository_revision: Option<String>,
    pub repository_root: Option<PathBuf>,
    pub cwd: PathBuf,
    pub codex_home: PathBuf,
    pub codex_protocol: String,
    pub codex_version: String,
    pub profile: Profile,
    pub supplied_files: Vec<ContextFile>,
    pub provider_discovery: ProviderDiscovery,
    pub tools: Vec<ExposedTool>,
    pub permissions: PermissionManifest,
}
#[derive(Clone, Debug)]
pub struct AssembledInstructions {
    pub prompt: String,
    pub manifest: InstructionManifest,
    pub manifest_digest: String,
}
#[derive(Clone, Debug)]
pub struct RepositoryContext {
    pub root: PathBuf,
    pub revision: String,
    /// Explicit write or review paths identify additional scoped guidance. They
    /// do not grant reads/writes and are never inferred from unrelated files.
    pub scope_paths: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct AssemblyInput<'a> {
    pub workflow: &'a Workflow,
    pub role: AgentRole,
    pub phase: &'a str,
    pub context: &'a Value,
    pub workspace_id: Uuid,
    pub operation_id: Uuid,
    pub work_generation: &'a str,
    pub correction_revision: &'a str,
    pub profile: &'a Profile,
    pub cwd: &'a Path,
    pub codex_home: &'a Path,
    pub codex_version: &'a str,
    pub repository: Option<RepositoryContext>,
    pub required_context: &'a [PathBuf],
    pub project_doc_max_bytes: usize,
    pub permissions: PermissionManifest,
    pub tools: Vec<ExposedTool>,
    pub provider_discovery: ProviderDiscovery,
    /// Secret values known to the host; reject rather than redact an operation's
    /// task silently. Values are never recorded in the manifest or error.
    pub known_secrets: &'a [String],
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetCatalog {
    schema_version: u32,
    asset_version: String,
    runtime_version: String,
    assets: BTreeMap<String, String>,
}

pub fn verify_assets() -> Result<BTreeMap<String, String>> {
    let catalog: AssetCatalog = serde_json::from_str(ASSET_MANIFEST)
        .map_err(|_| Error::invalid("embedded instruction catalog is invalid"))?;
    if catalog.schema_version != 1
        || catalog.asset_version != ASSET_VERSION
        || catalog.runtime_version != env!("CARGO_PKG_VERSION")
        || catalog.assets.len() != ASSETS.len()
    {
        return Err(Error::invalid(
            "runtime instruction asset versions do not match installed binary",
        ));
    }
    for (name, content) in ASSETS {
        if content.trim().is_empty()
            || catalog.assets.get(*name) != Some(&bytes_digest(content.as_bytes()))
        {
            return Err(Error::invalid(
                "required runtime instruction asset is missing or mismatched",
            ));
        }
    }
    Ok(catalog.assets)
}
fn asset(name: &str) -> Result<&'static str> {
    ASSETS
        .iter()
        .find_map(|(candidate, content)| (*candidate == name).then_some(*content))
        .ok_or_else(|| Error::invalid("required role instruction asset is absent"))
}
pub fn assemble(input: AssemblyInput<'_>) -> Result<AssembledInstructions> {
    let catalog = verify_assets()?;
    input.profile.validate()?;
    if input.codex_version != SUPPORTED_CODEX_VERSION {
        return Err(Error::new(
            "unsupported_codex_version",
            "instruction assembly requires the supported Codex protocol profile",
        ));
    }
    if input.workspace_id.is_nil()
        || input.operation_id.is_nil()
        || input.work_generation.is_empty()
        || input.correction_revision.is_empty()
        || input.phase.is_empty()
    {
        return Err(Error::invalid(
            "instruction assembly needs canonical operation and generation identity",
        ));
    }
    let context = input
        .context
        .as_object()
        .ok_or_else(|| Error::invalid("instruction context must be an object"))?;
    const KEYS: &[&str] = &[
        "issue",
        "parent",
        "children",
        "workspace",
        "scope",
        "dependencies",
        "corrections",
        "evidence",
        "attempt",
        "phase",
        "profile",
        "operation",
    ];
    if context.len() != KEYS.len() || KEYS.iter().any(|key| !context.contains_key(*key)) {
        return Err(Error::invalid(
            "instruction context requires exactly the documented context objects; use null/empty optional values",
        ));
    }
    if context["workspace"].get("id").and_then(Value::as_str)
        != Some(input.workspace_id.to_string().as_str())
        || context["operation"].get("id").and_then(Value::as_str)
            != Some(input.operation_id.to_string().as_str())
        || context["phase"].as_str() != Some(input.phase)
        || context["profile"]
            != serde_json::to_value(input.profile)
                .map_err(|_| Error::invalid("cannot encode profile"))?
    {
        return Err(Error::invalid(
            "context/profile/workspace/operation does not match admitted snapshot",
        ));
    }
    reject_secret_fields(input.context)?;
    check_context_strings(input.context, input.known_secrets)?;
    let serialized_context = serde_json::to_string(input.context)
        .map_err(|_| Error::invalid("cannot encode instruction context"))?;
    check_text(&serialized_context, input.known_secrets)?;
    let cwd =
        std::fs::canonicalize(input.cwd).map_err(|e| Error::io("resolve instruction cwd", e))?;
    let codex_home = std::fs::canonicalize(input.codex_home)
        .map_err(|e| Error::io("resolve isolated Codex home", e))?;
    let mut permissions = input.permissions;
    permissions.read_roots = canonical_roots(&permissions.read_roots)?;
    contained_path(&cwd, &permissions.read_roots, true)?;
    for path in &mut permissions.source_write_paths {
        *path = contained_path(path, &permissions.read_roots, false)?;
    }
    for path in &mut permissions.resource_write_paths {
        *path = contained_path(path, &permissions.read_roots, true)?;
    }
    for root in &mut permissions.scratch_roots {
        *root = std::fs::canonicalize(&*root)
            .map_err(|e| Error::io("resolve private scratch root", e))?;
    }
    if (!input.role.permits_source_writes() && !permissions.source_write_paths.is_empty())
        || (!input.role.permits_repository_execution()
            && !permissions.resource_write_paths.is_empty())
        || (!input.role.permits_repository_execution() && permissions.repository_code_execution)
        || permissions.enforcement_profile.trim().is_empty()
    {
        return Err(Error::new(
            "access_denied",
            "role does not permit supplied write/execution boundary",
        ));
    }
    if permissions
        .read_roots
        .iter()
        .any(|root| codex_home.starts_with(root) || root.starts_with(&codex_home))
        || permissions
            .scratch_roots
            .iter()
            .any(|root| codex_home.starts_with(root) || root.starts_with(&codex_home))
    {
        return Err(Error::new(
            "access_denied",
            "worker content roots cannot expose its Codex authentication home",
        ));
    }
    let mut tool_names = BTreeSet::new();
    for tool in &input.tools {
        if tool.name.is_empty()
            || !tool_names.insert(&tool.name)
            || !input.role.permits(tool.capability)
        {
            return Err(Error::new(
                "access_denied",
                "exposed tool does not belong to the admitted role",
            ));
        }
    }
    let mut supplied_files = Vec::new();
    let mut repository_text = String::new();
    let mut read_bytes = 0;
    // Reading these exact guidance names is host context assembly, not granting
    // worker access to the rest of its credential home.
    if let Some(path) = select_instruction(&codex_home)? {
        if std::fs::symlink_metadata(&path)
            .map_err(|e| Error::io("inspect global instruction identity", e))?
            .file_type()
            .is_symlink()
        {
            return Err(Error::new(
                "access_denied",
                "global instruction links cannot expose credential-home files",
            ));
        }
        let safe = contained_path(&path, std::slice::from_ref(&codex_home), true)?;
        read_context(
            &safe,
            ContextKind::GlobalInstruction,
            None,
            input.known_secrets,
            &mut supplied_files,
            &mut repository_text,
            &mut read_bytes,
        )?;
    }
    let mut repository_root = None;
    let mut repository_revision = None;
    if let Some(repository) = &input.repository {
        if repository.revision.len() != 40
            || !repository.revision.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::invalid(
                "repository instruction context requires an exact Git revision",
            ));
        }
        let root = contained_path(&repository.root, &permissions.read_roots, true)?;
        if !cwd.starts_with(&root) {
            return Err(Error::invalid(
                "instruction cwd is outside the isolated repository",
            ));
        }
        if permissions
            .scratch_roots
            .iter()
            .any(|scratch| scratch.starts_with(&root) || root.starts_with(scratch))
        {
            return Err(Error::new(
                "access_denied",
                "scratch permission cannot make the repository writable",
            ));
        }
        if permissions
            .source_write_paths
            .iter()
            .any(|path| !path.starts_with(&root))
        {
            return Err(Error::new(
                "access_denied",
                "source writes exceed the single assigned repository",
            ));
        }
        if permissions
            .resource_write_paths
            .iter()
            .any(|path| path.starts_with(&root) || root.starts_with(path))
        {
            return Err(Error::new(
                "access_denied",
                "test-resource writes cannot reopen repository source",
            ));
        }
        let mut directories = BTreeSet::new();
        add_ancestors(&root, &cwd, &mut directories)?;
        for scope in &repository.scope_paths {
            validate_write_path(scope)?;
            let requested = if scope == "." {
                root.clone()
            } else {
                root.join(scope)
            };
            let resolved = contained_path(&requested, std::slice::from_ref(&root), false)?;
            let directory = if resolved.is_dir() || scope.ends_with('/') {
                resolved
            } else {
                resolved
                    .parent()
                    .ok_or_else(|| Error::invalid("scope lacks a parent"))?
                    .to_path_buf()
            };
            add_ancestors(&root, &directory, &mut directories)?;
        }
        // Path sorting is stable and parents precede their descendants. Scoped
        // instruction blocks retain applies_under rather than changing authority.
        for directory in directories {
            if let Some(path) = select_instruction(&directory)? {
                let safe = contained_path(&path, std::slice::from_ref(&root), true)?;
                read_context(
                    &safe,
                    ContextKind::RepositoryInstruction,
                    Some(directory),
                    input.known_secrets,
                    &mut supplied_files,
                    &mut repository_text,
                    &mut read_bytes,
                )?;
            }
        }
        repository_root = Some(root);
        repository_revision = Some(repository.revision.clone());
    } else if !permissions.source_write_paths.is_empty() {
        return Err(Error::new(
            "access_denied",
            "source writes require an isolated repository identity",
        ));
    }
    if input.project_doc_max_bytes == 0 || read_bytes > input.project_doc_max_bytes {
        return Err(Error::new(
            "context_too_large",
            "required instruction chain exceeds the configured provider context limit",
        ));
    }
    for path in input.required_context {
        let safe = contained_path(path, &permissions.read_roots, true)?;
        if supplied_files.iter().any(|file| file.path == safe) {
            continue;
        }
        read_context(
            &safe,
            ContextKind::RequiredContext,
            None,
            input.known_secrets,
            &mut supplied_files,
            &mut repository_text,
            &mut read_bytes,
        )?;
    }
    let rendered = input.workflow.render(input.context)?;
    check_text(&rendered, input.known_secrets)?;
    let role_asset = input.role.asset();
    let selected_assets = ["authority", role_asset, "continuation"];
    let mut prompt = String::new();
    for name in selected_assets {
        prompt.push_str(asset(name)?);
        prompt.push_str("\n\n");
    }
    prompt.push_str("# Admitted operation boundary\n\n");
    prompt.push_str(&serde_json::to_string(&serde_json::json!({"role":input.role,"phase":input.phase,"workspace_id":input.workspace_id,"operation_id":input.operation_id,"work_generation":input.work_generation,"correction_revision":input.correction_revision,"profile":input.profile,"cwd":cwd,"tools":input.tools,"permissions":permissions})).map_err(|_| Error::invalid("cannot encode operation boundary"))?);
    // Runtime context is mandatory even when the repository chooses a literal
    // workflow without interpolation. JSON keeps issue/comment contents in a
    // data layer; those contents cannot replace the admitted authority boundary.
    prompt.push_str("\n\n# Admitted work context (structured task data)\n\n");
    prompt.push_str(&serialized_context);
    prompt.push_str("\n\n# Applicable repository-owned guidance and required context\n\n");
    prompt.push_str(&repository_text);
    prompt.push_str("\n# Configured repository workflow\n\n");
    prompt.push_str(&rendered);
    if prompt.len() > MAX_CONTEXT_BYTES {
        return Err(Error::new(
            "context_too_large",
            "complete required input exceeds the supported assembly bound",
        ));
    }
    check_text(&prompt, input.known_secrets)?;
    if let ProviderDiscovery::Confirmed {
        protocol_source,
        files,
    } = &input.provider_discovery
        && (protocol_source.trim().is_empty()
            || files
                .iter()
                .any(|file| !supplied_files.iter().any(|supplied| supplied == file)))
    {
        return Err(Error::invalid(
            "provider-confirmed instruction discovery does not match supplied accessible files",
        ));
    }
    let manifest = InstructionManifest {
        schema_version: ASSEMBLY_SCHEMA,
        runtime_version: env!("CARGO_PKG_VERSION").into(),
        runtime_build: crate::BUILD_ID.into(),
        asset_version: ASSET_VERSION.into(),
        assets: selected_assets
            .into_iter()
            .map(|name| (name.to_owned(), catalog[name].clone()))
            .collect(),
        workflow_path: input.workflow.path.clone(),
        workflow_digest: input.workflow.digest.clone(),
        role: input.role,
        phase: input.phase.into(),
        workspace_id: input.workspace_id,
        operation_id: input.operation_id,
        work_generation: input.work_generation.into(),
        correction_revision: input.correction_revision.into(),
        context_digest: canonical_digest(1, input.context)?,
        prompt_digest: bytes_digest(prompt.as_bytes()),
        repository_revision,
        repository_root,
        cwd,
        codex_home,
        codex_protocol: "app-server-v2-stdio".into(),
        codex_version: input.codex_version.into(),
        profile: input.profile.clone(),
        supplied_files,
        provider_discovery: input.provider_discovery,
        tools: input.tools,
        permissions,
    };
    let manifest_digest = canonical_digest(ASSEMBLY_SCHEMA, &manifest)?;
    Ok(AssembledInstructions {
        prompt,
        manifest,
        manifest_digest,
    })
}
fn add_ancestors(root: &Path, directory: &Path, output: &mut BTreeSet<PathBuf>) -> Result<()> {
    if !directory.starts_with(root) {
        return Err(Error::new(
            "access_denied",
            "instruction directory escapes repository",
        ));
    }
    let mut current = directory.to_path_buf();
    loop {
        output.insert(current.clone());
        if current == root {
            break;
        }
        if !current.pop() {
            return Err(Error::invalid("instruction ancestry is invalid"));
        }
    }
    Ok(())
}
fn select_instruction(directory: &Path) -> Result<Option<PathBuf>> {
    for name in ["AGENTS.override.md", "AGENTS.md"] {
        let path = directory.join(name);
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.is_file() && metadata.len() > 0 => return Ok(Some(path)),
            Ok(metadata) if metadata.is_file() => (),
            Ok(_) => {
                return Err(Error::invalid(
                    "repository instruction is not a regular file",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(Error::io("inspect repository instructions", error)),
        }
    }
    Ok(None)
}
fn read_context(
    path: &Path,
    kind: ContextKind,
    applies_under: Option<PathBuf>,
    secrets: &[String],
    files: &mut Vec<ContextFile>,
    text: &mut String,
    total: &mut usize,
) -> Result<()> {
    let metadata = std::fs::metadata(path).map_err(|e| Error::io("inspect required context", e))?;
    if !metadata.is_file() || metadata.len() > MAX_CONTEXT_BYTES as u64 {
        return Err(Error::new(
            "context_too_large",
            "required context is not a bounded regular file",
        ));
    }
    let content =
        std::fs::read_to_string(path).map_err(|e| Error::io("read required context", e))?;
    *total = total.saturating_add(content.len());
    if *total > MAX_CONTEXT_BYTES {
        return Err(Error::new(
            "context_too_large",
            "required context exceeds assembly limit",
        ));
    }
    check_text(&content, secrets)?;
    // JSON encodes delimiters and paths so untrusted file text cannot forge the
    // layer's metadata. Its substantive instructions still retain project scope.
    text.push_str(&serde_json::to_string(&serde_json::json!({"path":path,"kind":kind,"applies_under":applies_under,"content":content})).map_err(|_| Error::invalid("cannot encode repository context"))?);
    text.push('\n');
    files.push(ContextFile {
        path: path.to_path_buf(),
        digest: bytes_digest(content.as_bytes()),
        bytes: content.len(),
        kind,
        applies_under,
    });
    Ok(())
}
fn check_text(text: &str, secrets: &[String]) -> Result<()> {
    if text.contains('\0')
        || secrets
            .iter()
            .filter(|value| !value.is_empty())
            .any(|secret| text.contains(secret))
    {
        return Err(Error::new(
            "access_denied",
            "required input contains a protected value or invalid NUL; no prompt was assembled",
        ));
    }
    Ok(())
}
fn reject_secret_fields(value: &Value) -> Result<()> {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if [
                    "credential",
                    "credentials",
                    "credential_ref",
                    "secret",
                    "secrets",
                    "access_token",
                    "refresh_token",
                    "api_key",
                    "authorization",
                    "private_key",
                    "attestation_key",
                    "auth_home",
                ]
                .contains(&key.to_lowercase().as_str())
                {
                    return Err(Error::new(
                        "access_denied",
                        "secret-bearing fields cannot enter model context",
                    ));
                }
                reject_secret_fields(value)?;
            }
        }
        Value::Array(array) => {
            for value in array {
                reject_secret_fields(value)?;
            }
        }
        _ => (),
    }
    Ok(())
}
impl InstructionManifest {
    /// Operation IDs and dynamic input digests can differ on continuation, but
    /// resumed sessions must have exactly compatible effective instructions.
    pub fn resume_compatible(&self, next: &Self) -> bool {
        self.runtime_version == next.runtime_version
            && self.runtime_build == next.runtime_build
            && self.asset_version == next.asset_version
            && self.assets == next.assets
            && self.workflow_digest == next.workflow_digest
            && self.role == next.role
            && self.workspace_id == next.workspace_id
            && self.work_generation == next.work_generation
            && self.profile == next.profile
            && self.cwd == next.cwd
            && self.repository_root == next.repository_root
            && self.repository_revision == next.repository_revision
            && self.codex_home == next.codex_home
            && self.codex_version == next.codex_version
            && self.supplied_files == next.supplied_files
            && self.permissions == next.permissions
            && canonical_digest(1, &self.tools).ok() == canonical_digest(1, &next.tools).ok()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IncrementalInput {
    pub schema_version: u32,
    pub operation_id: Uuid,
    pub thread_id: String,
    pub turn_id: String,
    pub kind: IncrementalKind,
    pub revision: String,
    pub content_digest: String,
    pub context_reads: Vec<ContextFile>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncrementalKind {
    Correction,
    Continuation,
    Recovery,
    ContextRead,
}
impl IncrementalInput {
    pub fn new(
        operation_id: Uuid,
        thread_id: String,
        turn_id: String,
        kind: IncrementalKind,
        revision: String,
        content: &str,
        known_secrets: &[String],
    ) -> Result<Self> {
        if operation_id.is_nil()
            || thread_id.is_empty()
            || turn_id.is_empty()
            || revision.is_empty()
        {
            return Err(Error::invalid(
                "incremental context must identify its real operation, thread, turn and revision",
            ));
        }
        check_text(content, known_secrets)?;
        if content.len() > MAX_CONTEXT_BYTES {
            return Err(Error::new(
                "context_too_large",
                "incremental context is oversized",
            ));
        }
        Ok(Self {
            schema_version: 1,
            operation_id,
            thread_id,
            turn_id,
            kind,
            revision,
            content_digest: bytes_digest(content.as_bytes()),
            context_reads: Vec::new(),
        })
    }
}
/// Explicit export never overwrites existing guidance and never changes config.
pub fn export_starter(destination: &Path) -> Result<()> {
    verify_assets()?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(destination)
        .map_err(|e| Error::io("create new starter workflow", e))?;
    file.write_all(STARTER.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|e| Error::io("persist starter workflow", e))?;
    if let Some(parent) = destination.parent() {
        std::fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| Error::io("persist starter workflow directory", e))?;
    }
    Ok(())
}

fn check_context_strings(value: &Value, secrets: &[String]) -> Result<()> {
    match value {
        Value::String(text) => check_text(text, secrets)?,
        Value::Object(map) => {
            for (key, value) in map {
                check_text(key, secrets)?;
                check_context_strings(value, secrets)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                check_context_strings(value, secrets)?;
            }
        }
        _ => (),
    }
    Ok(())
}
