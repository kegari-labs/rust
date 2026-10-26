//! Versioned host configuration. Parsing validates shape, never contacts a provider
//! or grants authority; connection identities and policies require preflight.
use crate::digest::canonical_digest;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

#[path = "config_evaluation.rs"]
mod evaluation;
pub use evaluation::EvaluationBinding;

fn schema_one() -> u32 {
    1
}
fn poll_ms() -> u64 {
    30_000
}
fn attempts() -> u32 {
    3
}
fn turns() -> u32 {
    20
}
fn retry_ms() -> u64 {
    300_000
}
fn request_ms() -> u64 {
    5_000
}
fn silence_ms() -> u64 {
    3_600_000
}
fn stall_ms() -> u64 {
    300_000
}
fn free_bytes() -> u64 {
    1_073_741_824
}
fn execution_root() -> String {
    "~/.kegari/runs".into()
}
fn codex_argv() -> Vec<String> {
    vec![
        "codex".into(),
        "app-server".into(),
        "--listen".into(),
        "stdio://".into(),
    ]
}
fn main_ref() -> String {
    "main".into()
}
fn staging_ref() -> String {
    "staging".into()
}
fn review_context() -> String {
    "kegari/codex-review".into()
}
fn github_hostname() -> String {
    "github.com".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub runtime: RuntimeConfig,
    pub connections: BTreeMap<String, Connection>,
    pub workspaces: BTreeMap<String, Workspace>,
    pub control: ControlConfig,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    pub id: Uuid,
    pub host_id: String,
    #[serde(default = "execution_root")]
    pub execution_root: String,
    pub max_agents: u32,
    #[serde(default = "poll_ms")]
    pub poll_ms: u64,
    #[serde(default = "attempts")]
    pub max_attempts_per_phase: u32,
    #[serde(default = "attempts")]
    pub max_review_rounds: u32,
    #[serde(default = "turns")]
    pub max_turns: u32,
    #[serde(default = "retry_ms")]
    pub retry_cap_ms: u64,
    #[serde(default = "request_ms")]
    pub request_timeout_ms: u64,
    #[serde(default = "silence_ms")]
    pub stream_silence_ms: u64,
    #[serde(default = "stall_ms")]
    pub stall_ms: u64,
    #[serde(default = "free_bytes")]
    pub min_free_bytes: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Linear,
    Github,
    Codex,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub kind: ProviderKind,
    pub account_id: String,
    pub endpoint: Option<String>,
    pub credential_ref: Option<String>,
    pub auth_home: Option<String>,
    #[serde(default = "codex_argv")]
    pub argv: Vec<String>,
    pub max_agents: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: Uuid,
    pub linear_workspace_id: Uuid,
    pub linear_connection: String,
    pub github_connection: String,
    pub codex_connection: String,
    pub workflow: String,
    pub linear: LinearSelection,
    pub github: GithubSelection,
    #[serde(default)]
    pub independent_review: IndependentReview,
    pub actors: Actors,
    pub profiles: Profiles,
    pub routing_labels: RoutingLabels,
    pub max_agents: u32,
    #[serde(default)]
    pub max_agents_by_state: BTreeMap<String, u32>,
    pub release: ReleaseConfig,
    pub repositories: BTreeMap<String, Repository>,
    #[serde(default)]
    pub test_resources: BTreeMap<String, TestResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<EvaluationBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_execution: Option<ManagedExecution>,
}

pub const MAX_MANAGED_EXECUTABLES: usize = 256;
pub const MAX_MANAGED_READ_ROOTS: usize = 64;
pub const MAX_MANAGED_EXECUTABLE_MAPPINGS: usize = 65_536;
const MAX_MANAGED_PATH_BYTES: usize = 4096;

/// Host-owned declaration only. Actual filesystem binding and native
/// qualification are separate prerequisites; this configuration grants neither.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedExecution {
    pub lifecycle: ManagedLifecycle,
    pub executables: Vec<ManagedExecutable>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_roots: Vec<ManagedReadRoot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_executables: Option<ManagedGeneratedExecutables>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedLifecycle {
    ForegroundV1,
    /// Explicit finite services owned by one admitted validation check. This
    /// does not declare reusable daemons or widen resource/network authority.
    CheckBundleV1,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedExecutable {
    pub path: String,
    pub digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedReadRoot {
    pub path: String,
    pub directory: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<ManagedResourceContent>,
}
/// Expected resource content and selected executable mappings, not a claim of
/// observed content or supported-profile qualification.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedResourceContent {
    pub digest: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub executable_mappings: Vec<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagedGeneratedExecutables {
    ScratchBuildV1,
}
impl ManagedExecution {
    pub fn validate(&self) -> Result<()> {
        if self.executables.is_empty()
            || self.executables.len() > MAX_MANAGED_EXECUTABLES
            || self.read_roots.len() > MAX_MANAGED_READ_ROOTS
        {
            return Err(Error::invalid(
                "managed execution inventory is empty or exceeds its bound",
            ));
        }
        let mut executables = BTreeSet::new();
        for executable in &self.executables {
            managed_path_syntax(&executable.path)?;
            crate::evidence::validate_digest(&executable.digest)?;
            if !executables.insert(&executable.path) {
                return Err(Error::invalid("managed executable paths must be unique"));
            }
        }
        let mut reads = BTreeSet::new();
        for read in &self.read_roots {
            managed_path_syntax(&read.path)?;
            if !reads.insert(&read.path) {
                return Err(Error::invalid("managed read paths must be unique"));
            }
            if let Some(content) = &read.content {
                crate::evidence::validate_digest(&content.digest)?;
                if content.executable_mappings.len() > MAX_MANAGED_EXECUTABLE_MAPPINGS {
                    return Err(Error::invalid(
                        "managed executable mappings exceed their bound",
                    ));
                }
                let mut mappings = BTreeSet::new();
                for mapping in &content.executable_mappings {
                    if mapping.is_empty()
                        || mapping.len() > MAX_MANAGED_PATH_BYTES
                        || mapping.contains('\\')
                        || mapping.chars().any(char::is_control)
                        || (read.directory
                            && mapping
                                .split('/')
                                .any(|part| matches!(part, "" | "." | "..")))
                        || (!read.directory && mapping != ".")
                        || !mappings.insert(mapping)
                    {
                        return Err(Error::invalid(
                            "managed executable mappings must be unique normalized resource-relative file names",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}
fn managed_path_syntax(value: &str) -> Result<()> {
    if value.len() > MAX_MANAGED_PATH_BYTES || value == "/" {
        return Err(Error::invalid(
            "managed paths must be bounded and narrower than filesystem root",
        ));
    }
    path_syntax(value)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestResource {
    pub path: String,
    pub writable: bool,
    pub disposable: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearSelection {
    pub teams: Selector,
    pub projects: Selector,
    pub users: Selector,
    pub authorization_label_id: Uuid,
    pub states: BTreeMap<String, StateMapping>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Selector {
    All(String),
    Ids(Vec<String>),
}
impl Selector {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::All(value) if value == "all" => Ok(()),
            Self::Ids(ids)
                if !ids.is_empty()
                    && ids.iter().all(|s| !s.trim().is_empty())
                    && ids.iter().collect::<BTreeSet<_>>().len() == ids.len() =>
            {
                Ok(())
            }
            _ => Err(Error::invalid(
                "selector must be the literal 'all' or a nonempty unique ID list",
            )),
        }
    }
    pub fn contains(&self, id: &str) -> bool {
        match self {
            Self::All(value) => value == "all",
            Self::Ids(ids) => ids.iter().any(|v| v == id),
        }
    }
}
impl LinearSelection {
    pub fn includes(
        &self,
        team: &str,
        project: Option<&str>,
        creator: &str,
        assignee: Option<&str>,
    ) -> bool {
        self.teams.contains(team)
            && (matches!(&self.projects, Selector::All(v) if v == "all")
                || project.is_some_and(|p| self.projects.contains(p)))
            && (self.users.contains(creator) || assignee.is_some_and(|a| self.users.contains(a)))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateMapping {
    pub prepared: Uuid,
    pub running: Uuid,
    pub blocked: Uuid,
    pub child_done: Uuid,
    pub parent_accepted: Uuid,
    pub cancelled: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubSelection {
    #[serde(default)]
    pub organizations: Vec<String>,
    #[serde(default)]
    pub repositories: Vec<String>,
    #[serde(default)]
    pub pr_authors: Vec<String>,
}
impl GithubSelection {
    pub fn permits_repository(&self, repository_id: &str, organization_id: Option<&str>) -> bool {
        self.repositories.iter().any(|id| id == repository_id)
            || organization_id.is_some_and(|o| self.organizations.iter().any(|id| id == o))
    }
    pub fn permits_pr_author(&self, author_id: &str) -> bool {
        self.pr_authors.iter().any(|id| id == author_id)
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndependentReview {
    #[serde(default)]
    pub enabled: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actors {
    pub authorizers: BTreeSet<Uuid>,
    pub correctors: BTreeSet<Uuid>,
    pub acceptors: BTreeSet<Uuid>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub tool: String,
    pub model: String,
    pub reasoning: String,
    pub speed: Speed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speed {
    Standard,
    Fast,
}
impl Profile {
    pub fn validate(&self) -> Result<()> {
        if self.tool != "codex" {
            return Err(Error::new(
                "unsupported_provider",
                "only the pinned Codex app-server is supported",
            ));
        }
        if self.model.trim().is_empty() || self.model.contains(char::is_whitespace) {
            return Err(Error::invalid(
                "profile model must be a nonempty exact model identifier",
            ));
        }
        if ![
            "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
        ]
        .contains(&self.reasoning.as_str())
        {
            return Err(Error::invalid("profile reasoning value is unsupported"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profiles {
    pub coordination: Profile,
    pub independent_review: Option<Profile>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutingLabels {
    pub tool: BTreeMap<Uuid, String>,
    pub model: BTreeMap<Uuid, String>,
    pub reasoning: BTreeMap<Uuid, String>,
    pub speed: BTreeMap<Uuid, Speed>,
}
impl RoutingLabels {
    pub fn resolve(&self, selected: &BTreeSet<Uuid>) -> Result<Profile> {
        fn one<T: Clone>(
            map: &BTreeMap<Uuid, T>,
            selected: &BTreeSet<Uuid>,
            dimension: &str,
        ) -> Result<T> {
            let found: Vec<_> = map.iter().filter(|(id, _)| selected.contains(id)).collect();
            if found.len() != 1 {
                return Err(Error::invalid(format!(
                    "child needs exactly one {dimension} routing label"
                )));
            }
            Ok(found[0].1.clone())
        }
        let profile = Profile {
            tool: one(&self.tool, selected, "tool")?,
            model: one(&self.model, selected, "model")?,
            reasoning: one(&self.reasoning, selected, "reasoning")?,
            speed: one(&self.speed, selected, "speed")?,
        };
        profile.validate()?;
        Ok(profile)
    }
    fn validate(&self) -> Result<()> {
        if self.tool.is_empty()
            || self.model.is_empty()
            || self.reasoning.is_empty()
            || self.speed.is_empty()
        {
            return Err(Error::invalid(
                "routing label mappings must define all four dimensions",
            ));
        }
        let all: Vec<_> = self
            .tool
            .keys()
            .chain(self.model.keys())
            .chain(self.reasoning.keys())
            .chain(self.speed.keys())
            .collect();
        if all.iter().any(|id| id.is_nil())
            || all.iter().collect::<BTreeSet<_>>().len() != all.len()
        {
            return Err(Error::invalid(
                "routing label IDs must be nonnil and distinct across dimensions",
            ));
        }
        if self.tool.values().any(|s| s != "codex")
            || self.model.values().any(|s| s.trim().is_empty())
        {
            return Err(Error::invalid(
                "routing labels contain unsupported tool or empty model",
            ));
        }
        for reasoning in self.reasoning.values() {
            Profile {
                tool: "codex".into(),
                model: "validated-at-preflight".into(),
                reasoning: reasoning.clone(),
                speed: Speed::Standard,
            }
            .validate()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseConfig {
    pub mode: ReleaseMode,
    pub spacing_seconds: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseMode {
    Automatic,
    Manual,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub source: String,
    #[serde(default = "github_hostname")]
    pub hostname: String,
    #[serde(default = "main_ref")]
    pub main: String,
    #[serde(default = "staging_ref")]
    pub staging: String,
    pub local_checks: Vec<String>,
    pub integrated_checks: Vec<String>,
    pub ci: Vec<CiSuite>,
    pub production_ci: Vec<CiSuite>,
    #[serde(default = "review_context")]
    pub review_context: String,
    pub no_change_validation: ExactRevisionWorkflow,
    pub deployment: Deployment,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CiSuite {
    pub workflow_id: u64,
    pub required_checks: Vec<String>,
    pub proof: HostedProofRoute,
    /// A configured declarative predicate, evaluated by the host, never by a worker.
    pub inapplicable_when: Option<InapplicablePredicate>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum InapplicablePredicate {
    PathsUnchanged(Vec<String>),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExactRevisionWorkflow {
    pub workflow_id: u64,
    pub workflow_revision: String,
    pub source_input: String,
    pub operation_input: String,
    pub observed_source_output: String,
    pub proof: HostedProofRoute,
}

/// Reviewed caller and reusable workflow revisions that may mint hosted proof.
/// Mutable runner labels and unpinned workflow names never establish trust.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedProofRoute {
    pub workflow_ref: String,
    pub workflow_sha: String,
    pub job_workflow_ref: String,
    pub job_workflow_sha: String,
    pub event_name: String,
}
impl HostedProofRoute {
    pub fn validate(&self) -> Result<()> {
        for sha in [&self.workflow_sha, &self.job_workflow_sha] {
            if sha.len() != 40
                || !sha
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(Error::invalid(
                    "hosted proof workflow revisions require exact lowercase 40-character Git SHAs",
                ));
            }
        }
        for reference in [&self.workflow_ref, &self.job_workflow_ref] {
            let Some((path, revision)) = reference.rsplit_once('@') else {
                return Err(Error::invalid(
                    "hosted proof workflow reference needs repository/path@revision",
                ));
            };
            let components: Vec<_> = path.split('/').collect();
            if components.len() != 5
                || components[2] != ".github"
                || components[3] != "workflows"
                || components.iter().any(|s| {
                    s.is_empty()
                        || *s == "."
                        || *s == ".."
                        || s.contains(['\\', '\0', '@', ':'])
                        || s.chars().any(char::is_whitespace)
                })
                || !(components[4].ends_with(".yml") || components[4].ends_with(".yaml"))
                || revision.is_empty()
                || revision.chars().any(char::is_whitespace)
                || revision.contains(['@', '\\', '\0'])
            {
                return Err(Error::invalid(
                    "hosted proof workflow reference must be an exact owner/repository/.github/workflows/file.yml@revision",
                ));
            }
        }
        if !self
            .job_workflow_ref
            .ends_with(&format!("@{}", self.job_workflow_sha))
        {
            return Err(Error::invalid(
                "trusted reusable workflow must be referenced at its exact configured SHA",
            ));
        }
        if ![
            "workflow_dispatch",
            "pull_request",
            "push",
            "workflow_call",
            "merge_group",
        ]
        .contains(&self.event_name.as_str())
        {
            return Err(Error::invalid("hosted proof trigger event is unsupported"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Deployment {
    None(String),
    Steps { steps: Vec<DeploymentStep> },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentStep {
    pub id: String,
    pub target_id: String,
    pub environment_id: String,
    pub workflow: ExactRevisionWorkflow,
    #[serde(default)]
    pub depends_on: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlConfig {
    #[serde(default = "schema_one")]
    pub api_version: u32,
    pub local_users: BTreeMap<String, Operator>,
    #[serde(default)]
    pub tailscale_users: BTreeMap<String, Operator>,
    pub https: Option<HttpsConfig>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operator {
    pub id: String,
    pub linear_user_id: Uuid,
    pub roles: BTreeSet<ControlRole>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlRole {
    Inspect,
    Configure,
    Lifecycle,
    Correct,
    Accept,
    Release,
    Recover,
    Evaluate,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpsConfig {
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub dns_name: String,
    pub certificate: String,
    pub private_key: String,
    pub tailscale_executable: String,
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text).map_err(|e| {
            Error::invalid(format!(
                "invalid typed TOML configuration at byte {}",
                e.span().map_or(0, |span| span.start)
            ))
        })?;
        config.validate()?;
        Ok(config)
    }
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io("read configuration", e))?;
        Self::parse(&text)
    }
    /// Resolve documented path references on the execution host as one atomic
    /// configuration transformation. This reads environment values but never
    /// resolves credential references into this non-secret configuration.
    pub fn resolve_paths(
        &mut self,
        home: &Path,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<()> {
        fn resolve(
            value: &str,
            home: &Path,
            env: &impl Fn(&str) -> Option<String>,
        ) -> Result<String> {
            resolve_host_path(value, home, env)?
                .into_os_string()
                .into_string()
                .map_err(|_| Error::invalid("configured host path is not UTF-8"))
        }
        let mut next = self.clone();
        next.runtime.execution_root = resolve(&next.runtime.execution_root, home, &env)?;
        for connection in next.connections.values_mut() {
            if let Some(auth_home) = &mut connection.auth_home {
                *auth_home = resolve(auth_home, home, &env)?;
            }
        }
        for workspace in next.workspaces.values_mut() {
            workspace.workflow = resolve(&workspace.workflow, home, &env)?;
            if let Some(managed) = &mut workspace.managed_execution {
                for executable in &mut managed.executables {
                    executable.path = resolve(&executable.path, home, &env)?;
                }
                for read in &mut managed.read_roots {
                    read.path = resolve(&read.path, home, &env)?;
                }
            }
            if let Some(evaluation) = &mut workspace.evaluation {
                evaluation.suite_manifest = resolve(&evaluation.suite_manifest, home, &env)?;
                evaluation.report_root = resolve(&evaluation.report_root, home, &env)?;
                evaluation.fixture_root = resolve(&evaluation.fixture_root, home, &env)?;
            }
            for repository in workspace.repositories.values_mut() {
                repository.source = resolve(&repository.source, home, &env)?;
            }
            for resource in workspace.test_resources.values_mut() {
                resource.path = resolve(&resource.path, home, &env)?;
            }
        }
        if let Some(https) = &mut next.control.https {
            https.certificate = resolve(&https.certificate, home, &env)?;
            https.private_key = resolve(&https.private_key, home, &env)?;
            https.tailscale_executable = resolve(&https.tailscale_executable, home, &env)?;
        }
        next.validate()?;
        for workspace in next.workspaces.values() {
            if let Some(evaluation) = &workspace.evaluation {
                evaluation.validate_resolved_paths(&next, workspace)?;
            }
        }
        *self = next;
        Ok(())
    }
    /// Read-only cross-validation; it never runs commands, hooks or evaluations.
    /// Paths must first have been resolved on the execution host.
    pub fn load_workflows(&self) -> Result<BTreeMap<Uuid, crate::workflow::Workflow>> {
        let mut loaded = BTreeMap::new();
        for workspace in self.workspaces.values() {
            let path = Path::new(&workspace.workflow);
            if !path.is_absolute() {
                return Err(Error::invalid(
                    "resolve configured workflow path on the host before preflight",
                ));
            }
            let workflow = crate::workflow::Workflow::load(path)?;
            for repository in workspace.repositories.values() {
                for name in &repository.local_checks {
                    if !workflow.front_matter.checks.local.contains_key(name) {
                        return Err(Error::invalid(format!(
                            "required local check {name} is absent from the configured workflow"
                        )));
                    }
                }
                for name in &repository.integrated_checks {
                    if !workflow.front_matter.checks.integrated.contains_key(name) {
                        return Err(Error::invalid(format!(
                            "required integrated check {name} is absent from the configured workflow"
                        )));
                    }
                }
            }
            loaded.insert(workspace.id, workflow);
        }
        Ok(loaded)
    }
    pub fn revision(&self) -> Result<String> {
        canonical_digest(1, self)
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 || self.control.api_version != 1 {
            return Err(Error::invalid("unsupported configuration or API schema"));
        }
        if self.runtime.id.is_nil() || self.runtime.host_id.trim().is_empty() {
            return Err(Error::invalid(
                "runtime identity and verified host registration are required",
            ));
        }
        for (name, value) in [
            ("max_agents", self.runtime.max_agents as u64),
            ("poll_ms", self.runtime.poll_ms),
            (
                "max_attempts_per_phase",
                self.runtime.max_attempts_per_phase as u64,
            ),
            ("max_review_rounds", self.runtime.max_review_rounds as u64),
            ("max_turns", self.runtime.max_turns as u64),
            ("retry_cap_ms", self.runtime.retry_cap_ms),
            ("request_timeout_ms", self.runtime.request_timeout_ms),
            ("stream_silence_ms", self.runtime.stream_silence_ms),
            ("stall_ms", self.runtime.stall_ms),
        ] {
            if value == 0 {
                return Err(Error::invalid(format!("runtime.{name} must be positive")));
            }
        }
        path_syntax(&self.runtime.execution_root)?;
        let mut auth_homes = BTreeMap::new();
        for (name, connection) in &self.connections {
            nonempty(name, "connection name")?;
            nonempty(&connection.account_id, "connection account ID")?;
            match connection.kind {
                ProviderKind::Codex => {
                    let home = connection.auth_home.as_ref().ok_or_else(|| {
                        Error::invalid("Codex connection requires isolated auth_home")
                    })?;
                    path_syntax(home)?;
                    // Even connections to the same account receive separate auth homes;
                    // quota identity is separately deduplicated by account ID.
                    if auth_homes.insert(home, name).is_some() {
                        return Err(Error::invalid("Codex auth_home paths must be distinct"));
                    }
                    if connection.max_agents.is_none_or(|limit| limit == 0) {
                        return Err(Error::invalid("Codex account max_agents must be positive"));
                    }
                    if connection.argv.is_empty()
                        || connection
                            .argv
                            .iter()
                            .any(|v| v.is_empty() || v.contains('\0'))
                    {
                        return Err(Error::invalid(
                            "Codex argv must be a nonempty argument array",
                        ));
                    }
                    if connection.credential_ref.is_some() {
                        return Err(Error::invalid(
                            "Codex uses its supported isolated auth home, not a Kegari token",
                        ));
                    }
                }
                ProviderKind::Linear | ProviderKind::Github => {
                    canonical_https_endpoint(
                        connection
                            .endpoint
                            .as_deref()
                            .ok_or_else(|| Error::invalid("provider endpoint is required"))?,
                    )?;
                    secret_reference(connection.credential_ref.as_deref().ok_or_else(|| {
                        Error::invalid("provider credential reference is required")
                    })?)?;
                    if connection.auth_home.is_some() || connection.max_agents.is_some() {
                        return Err(Error::invalid(
                            "auth_home and max_agents are Codex connection fields",
                        ));
                    }
                }
            }
        }
        let mut workspace_ids = BTreeSet::new();
        for (name, workspace) in &self.workspaces {
            nonempty(name, "workspace name")?;
            if workspace.id.is_nil()
                || workspace.linear_workspace_id.is_nil()
                || !workspace_ids.insert(workspace.id)
            {
                return Err(Error::invalid("workspace IDs must be nonnil and unique"));
            }
            for (reference, expected) in [
                (&workspace.linear_connection, ProviderKind::Linear),
                (&workspace.github_connection, ProviderKind::Github),
                (&workspace.codex_connection, ProviderKind::Codex),
            ] {
                if self
                    .connections
                    .get(reference)
                    .is_none_or(|c| c.kind != expected)
                {
                    return Err(Error::invalid(format!(
                        "workspace references missing or wrong-kind connection: {reference}"
                    )));
                }
            }
            path_syntax(&workspace.workflow)?;
            if let Some(managed) = &workspace.managed_execution {
                managed.validate()?;
            }
            workspace.linear.teams.validate()?;
            workspace.linear.projects.validate()?;
            workspace.linear.users.validate()?;
            if workspace.linear.authorization_label_id.is_nil()
                || workspace.linear.states.is_empty()
            {
                return Err(Error::invalid(
                    "authorization label and per-team state mappings are required",
                ));
            }
            for (team, states) in &workspace.linear.states {
                nonempty(team, "state mapping team")?;
                if [
                    states.prepared,
                    states.running,
                    states.blocked,
                    states.child_done,
                    states.parent_accepted,
                    states.cancelled,
                ]
                .iter()
                .any(Uuid::is_nil)
                {
                    return Err(Error::invalid("state IDs cannot be nil"));
                }
            }
            unique_nonempty(
                &workspace.github.organizations,
                "GitHub organizations",
                true,
            )?;
            unique_nonempty(&workspace.github.repositories, "GitHub repositories", true)?;
            unique_nonempty(
                &workspace.github.pr_authors,
                "GitHub PR authors",
                !workspace.independent_review.enabled,
            )?;
            if workspace.github.organizations.is_empty() && workspace.github.repositories.is_empty()
            {
                return Err(Error::invalid(
                    "select at least one GitHub organization or repository",
                ));
            }
            if workspace.actors.authorizers.is_empty()
                || workspace.actors.correctors.is_empty()
                || workspace.actors.acceptors.is_empty()
                || workspace
                    .actors
                    .authorizers
                    .iter()
                    .chain(&workspace.actors.correctors)
                    .chain(&workspace.actors.acceptors)
                    .any(Uuid::is_nil)
            {
                return Err(Error::invalid(
                    "all three actor allowlists must be nonempty, verified Linear user IDs",
                ));
            }
            workspace.profiles.coordination.validate()?;
            if workspace.independent_review.enabled
                && workspace.profiles.independent_review.is_none()
            {
                return Err(Error::invalid(
                    "enabled independent review needs an explicit profile",
                ));
            }
            if let Some(profile) = &workspace.profiles.independent_review {
                profile.validate()?;
            }
            workspace.routing_labels.validate()?;
            if workspace.max_agents == 0
                || workspace.max_agents_by_state.iter().any(|(state, limit)| {
                    state.trim().is_empty() || state != &state.trim().to_lowercase() || *limit == 0
                })
            {
                return Err(Error::invalid(
                    "workspace capacity and normalized per-state limits must be positive",
                ));
            }
            if workspace.repositories.is_empty() {
                return Err(Error::invalid("workspace requires repository contracts"));
            }
            for (id, repository) in &workspace.repositories {
                nonempty(id, "canonical repository ID")?;
                repository.validate()?;
            }
            for (id, resource) in &workspace.test_resources {
                nonempty(id, "test resource ID")?;
                path_syntax(&resource.path)?;
                if resource.writable && !resource.disposable {
                    return Err(Error::invalid(
                        "writable test resources must be explicitly disposable",
                    ));
                }
            }
        }
        evaluation::validate_bindings(self)?;
        if self.control.local_users.is_empty() {
            return Err(Error::invalid("at least one local operator is required"));
        }
        for (uid, operator) in &self.control.local_users {
            if uid.parse::<u32>().is_err() {
                return Err(Error::invalid(
                    "local operator keys are numeric OS user IDs",
                ));
            }
            operator.validate()?;
        }
        for (user, operator) in &self.control.tailscale_users {
            if user
                .parse::<u64>()
                .ok()
                .is_none_or(|id| id == 0 || id.to_string() != *user)
            {
                return Err(Error::invalid(
                    "Tailscale operator keys must be canonical positive numeric UserProfile.ID values, not display login names",
                ));
            }
            operator.validate()?;
        }
        if let Some(https) = &self.control.https {
            if https.addresses.is_empty()
                || https
                    .addresses
                    .iter()
                    .any(|address| !is_tailscale_address(*address))
                || https.port == 0
                || self.control.tailscale_users.is_empty()
            {
                return Err(Error::invalid(
                    "HTTPS needs explicit Tailscale addresses, port and permitted users",
                ));
            }
            if https.dns_name.is_empty()
                || https.dns_name.contains(['/', ':', '*'])
                || !https.dns_name.ends_with(".ts.net")
            {
                return Err(Error::invalid("HTTPS requires a full tailnet DNS hostname"));
            }
            path_syntax(&https.certificate)?;
            path_syntax(&https.private_key)?;
            path_syntax(&https.tailscale_executable)?;
        }
        Ok(())
    }
    /// Strictest connection cap per actual provider account; aliases do not multiply capacity.
    pub fn account_limits(&self) -> BTreeMap<String, u32> {
        let mut limits = BTreeMap::<String, u32>::new();
        for connection in self
            .connections
            .values()
            .filter(|c| c.kind == ProviderKind::Codex)
        {
            if let Some(limit) = connection.max_agents {
                limits
                    .entry(connection.account_id.clone())
                    .and_modify(|cap| *cap = (*cap).min(limit))
                    .or_insert(limit);
            }
        }
        limits
    }
}
impl Repository {
    fn validate(&self) -> Result<()> {
        path_syntax(&self.source)?;
        if self.main != "main"
            || self.staging != "staging"
            || self.review_context != "kegari/codex-review"
        {
            return Err(Error::invalid(
                "v0 requires main, staging and kegari/codex-review",
            ));
        }
        nonempty(&self.hostname, "GitHub hostname")?;
        if self.hostname.contains(['/', ':', '@']) {
            return Err(Error::invalid("invalid GitHub hostname"));
        }
        unique_nonempty(&self.local_checks, "local checks", false)?;
        unique_nonempty(&self.integrated_checks, "integrated checks", false)?;
        for suites in [&self.ci, &self.production_ci] {
            if suites.is_empty() {
                return Err(Error::invalid(
                    "complete staging and production hosted CI suites are required",
                ));
            }
            let mut ids = BTreeSet::new();
            for suite in suites {
                if suite.workflow_id == 0 || !ids.insert(suite.workflow_id) {
                    return Err(Error::invalid(
                        "CI workflow IDs must be positive and unique",
                    ));
                }
                unique_nonempty(&suite.required_checks, "required CI checks", false)?;
                suite.proof.validate()?;
                if let Some(InapplicablePredicate::PathsUnchanged(paths)) = &suite.inapplicable_when
                {
                    unique_nonempty(paths, "inapplicable paths", false)?;
                    for path in paths {
                        validate_write_path(path)?;
                    }
                }
            }
        }
        self.no_change_validation.validate()?;
        match &self.deployment {
            Deployment::None(value) if value == "none" => (),
            Deployment::Steps { steps } if !steps.is_empty() => {
                let mut ids = BTreeSet::new();
                for step in steps {
                    nonempty(&step.id, "deployment step ID")?;
                    nonempty(&step.target_id, "deployment target ID")?;
                    if step
                        .environment_id
                        .parse::<u64>()
                        .ok()
                        .is_none_or(|id| id == 0 || id.to_string() != step.environment_id)
                    {
                        return Err(Error::invalid(
                            "deployment environment IDs must be canonical positive numeric GitHub environment IDs",
                        ));
                    }
                    if !ids.insert(step.id.as_str()) {
                        return Err(Error::invalid("duplicate deployment step ID"));
                    }
                    step.workflow.validate()?;
                }
                let mut done = BTreeSet::new();
                while done.len() < steps.len() {
                    let before = done.len();
                    for step in steps {
                        if step.depends_on.iter().all(|id| done.contains(id.as_str())) {
                            done.insert(step.id.as_str());
                        }
                    }
                    if before == done.len() {
                        return Err(Error::invalid(
                            "deployment dependencies are missing or cyclic",
                        ));
                    }
                }
            }
            _ => {
                return Err(Error::invalid(
                    "deployment must be 'none' or a finite configured step DAG",
                ));
            }
        }
        Ok(())
    }
}
impl ExactRevisionWorkflow {
    fn validate(&self) -> Result<()> {
        self.proof.validate()?;
        if self.proof.workflow_sha != self.workflow_revision
            || self.proof.event_name != "workflow_dispatch"
        {
            return Err(Error::invalid(
                "exact-source dispatch proof must pin the configured caller revision and workflow_dispatch event",
            ));
        }
        if self.workflow_id == 0
            || self.workflow_revision.len() != 40
            || !self
                .workflow_revision
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::invalid(
                "workflow requires a positive ID and immutable 40-character Git revision",
            ));
        }
        nonempty(&self.source_input, "exact-source workflow input")?;
        nonempty(
            &self.operation_input,
            "operation correlation workflow input",
        )?;
        nonempty(&self.observed_source_output, "observed exact-source result")?;
        if self.source_input == self.operation_input {
            return Err(Error::invalid("source and operation inputs must differ"));
        }
        Ok(())
    }
}
impl Operator {
    fn validate(&self) -> Result<()> {
        nonempty(&self.id, "operator identity")?;
        if self.linear_user_id.is_nil() || self.roles.is_empty() {
            return Err(Error::invalid(
                "operator needs Linear identity and explicit roles",
            ));
        }
        Ok(())
    }
}
fn nonempty(value: &str, name: &str) -> Result<()> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(Error::invalid(format!(
            "{name} must be nonempty and contain no NUL"
        )));
    }
    Ok(())
}
fn unique_nonempty(values: &[String], name: &str, allow_empty: bool) -> Result<()> {
    if (!allow_empty && values.is_empty())
        || values
            .iter()
            .any(|v| v.trim().is_empty() || v.contains('\0'))
        || values.iter().collect::<BTreeSet<_>>().len() != values.len()
    {
        return Err(Error::invalid(format!(
            "{name} must contain unique nonempty IDs"
        )));
    }
    Ok(())
}
fn canonical_https_endpoint(value: &str) -> Result<()> {
    let url = url::Url::parse(value)
        .map_err(|_| Error::invalid("provider endpoint must be a canonical HTTPS URL"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.as_str() != value
    {
        return Err(Error::invalid(
            "provider endpoint must be canonical HTTPS without userinfo, query or fragment",
        ));
    }
    Ok(())
}
fn secret_reference(value: &str) -> Result<()> {
    if let Some(name) = value.strip_prefix('$')
        && valid_env_name(name)
    {
        return Ok(());
    }
    if let Some(name) = value.strip_prefix("keychain:")
        && !name.trim().is_empty()
        && !name.contains(['\0', '\n', '\r'])
    {
        return Ok(());
    }
    Err(Error::invalid(
        "credential_ref must be keychain:NAME or a whole $NAME reference; plaintext credentials are forbidden",
    ))
}
fn valid_env_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}
fn path_syntax(value: &str) -> Result<()> {
    nonempty(value, "path")?;
    if let Some(name) = value.strip_prefix('$') {
        if valid_env_name(name) {
            return Ok(());
        }
        return Err(Error::invalid(
            "only whole-string $NAME path references are allowed",
        ));
    }
    if value.contains('$') || value.starts_with('~') && value != "~" && !value.starts_with("~/") {
        return Err(Error::invalid(
            "shell interpolation and ~user paths are not supported",
        ));
    }
    if !(value.starts_with('/') || value == "~" || value.starts_with("~/")) {
        return Err(Error::invalid(
            "host paths must be absolute or start with ~/",
        ));
    }
    if Path::new(value)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(Error::invalid("path traversal is forbidden"));
    }
    Ok(())
}
/// Expand documented references on the host. It never invokes a shell. The caller
/// must subsequently enforce canonical containment/ownership for the operation.
pub fn resolve_host_path(
    value: &str,
    home: &Path,
    env: impl Fn(&str) -> Option<String>,
) -> Result<PathBuf> {
    path_syntax(value)?;
    let expanded = if let Some(name) = value.strip_prefix('$') {
        let resolved = env(name).filter(|v| !v.is_empty()).ok_or_else(|| {
            Error::invalid(format!(
                "path environment variable {name} is unset or empty"
            ))
        })?;
        if resolved.contains('$') {
            return Err(Error::invalid(
                "recursive environment expansion is forbidden",
            ));
        }
        resolved
    } else {
        value.to_owned()
    };
    path_syntax(&expanded)?;
    let path = if expanded == "~" {
        home.to_path_buf()
    } else if let Some(rest) = expanded.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(expanded)
    };
    if !path.is_absolute() {
        return Err(Error::invalid("expanded path is not absolute"));
    }
    Ok(path)
}
pub fn is_tailscale_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            octets[0] == 100 && (64..=127).contains(&octets[1])
        }
        IpAddr::V6(v6) => {
            let octets = v6.octets();
            octets[..6] == [0xfd, 0x7a, 0x11, 0x5c, 0xa1, 0xe0]
        }
    }
}
pub fn validate_write_path(value: &str) -> Result<()> {
    if value == "." {
        return Ok(());
    }
    if value.is_empty()
        || value.contains(['\0', '\\', '*', '?', '[', ']'])
        || Path::new(value).is_absolute()
        || Path::new(value).components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        return Err(Error::invalid(
            "write paths must be explicit repository-relative prefixes or files",
        ));
    }
    Ok(())
}

/// A syntactically invalid update cannot preserve a potentially revoked grant.
/// Callers fence existing effects on this result; valid snapshots remain immutable.
#[derive(Clone, Debug)]
pub enum ConfigReload {
    Unchanged,
    Valid {
        next: Box<Config>,
        revision: String,
    },
    Invalid {
        reason: String,
        fence_all_mutations: bool,
    },
}
pub fn inspect_reload(current_revision: &str, proposed: &str) -> ConfigReload {
    match Config::parse(proposed).and_then(|c| {
        let revision = c.revision()?;
        Ok((c, revision))
    }) {
        Ok((_, revision)) if revision == current_revision => ConfigReload::Unchanged,
        Ok((next, revision)) => ConfigReload::Valid {
            next: Box::new(next),
            revision,
        },
        Err(error) => ConfigReload::Invalid {
            reason: error.to_string(),
            fence_all_mutations: true,
        },
    }
}
