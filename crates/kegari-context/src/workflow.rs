//! Strict repository workflow parsing and deterministic bounded rendering.
use kegari_digest::bytes_digest;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

pub const DEFAULT_WORKFLOW_VERSION: u32 = 2;
pub const DEFAULT_WORKFLOW: &str = include_str!("../instructions/default-workflow.md");

pub const MAX_WORKFLOW_BYTES: usize = 1_048_576;
pub const MAX_RENDER_BYTES: usize = 8_388_608;
pub const MAX_CHECK_SERVICES: usize = 8;
pub const MAX_SERVICE_NAME_BYTES: usize = 64;
pub const MAX_SERVICE_ARGV: usize = 128;
pub const MAX_SERVICE_ARGUMENT_BYTES: usize = 4096;
pub const MAX_SERVICE_COMMAND_BYTES: usize = 65_536;
pub const MAX_SERVICE_LIFETIME_MS: u64 = 86_400_000;
const MAX_LOOP_ITERATIONS: usize = 10_000;
fn schema_one() -> u32 {
    1
}
fn hook_timeout() -> u64 {
    60_000
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checks {
    #[serde(default)]
    pub local: BTreeMap<String, CheckCommand>,
    #[serde(default)]
    pub integrated: BTreeMap<String, CheckCommand>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckCommand {
    pub argv: Vec<String>,
    pub result_artifact: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<CheckService>,
}
/// A declaration within one check, not permission to start a background task.
/// Services share the check's actual cwd, grants and aggregate execution budget.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CheckService {
    pub name: String,
    pub argv: Vec<String>,
    pub readiness: ServiceReadiness,
    pub max_lifetime_ms: u64,
}
/// Exactly one finite command; any polling must remain within this invocation.
/// Readiness adds no socket, network, filesystem or executable grants.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceReadiness {
    pub argv: Vec<String>,
    pub timeout_ms: u64,
}
impl CheckCommand {
    pub fn validate_services(&self) -> Result<()> {
        if self.services.is_empty() {
            return Ok(());
        }
        if self.services.len() > MAX_CHECK_SERVICES {
            return Err(Error::invalid(
                "check service inventory exceeds its finite bound",
            ));
        }
        bounded_service_argv(&self.argv)?;
        let mut names = BTreeSet::new();
        for service in &self.services {
            let name = service.name.as_bytes();
            if name.is_empty()
                || name.len() > MAX_SERVICE_NAME_BYTES
                || !name[0].is_ascii_alphanumeric()
                || !name
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-' | b'.'))
                || !names.insert(&service.name)
            {
                return Err(Error::invalid(
                    "check services require unique bounded stable names",
                ));
            }
            bounded_service_argv(&service.argv)?;
            bounded_service_argv(&service.readiness.argv)?;
            if service.max_lifetime_ms == 0
                || service.max_lifetime_ms > MAX_SERVICE_LIFETIME_MS
                || service.readiness.timeout_ms == 0
                || service.readiness.timeout_ms > service.max_lifetime_ms
            {
                return Err(Error::invalid(
                    "service readiness and lifetime require ordered finite positive bounds",
                ));
            }
        }
        Ok(())
    }
}
fn bounded_service_argv(argv: &[String]) -> Result<()> {
    if argv.is_empty()
        || argv.len() > MAX_SERVICE_ARGV
        || argv[0].trim().is_empty()
        || argv
            .iter()
            .any(|argument| argument.len() > MAX_SERVICE_ARGUMENT_BYTES || argument.contains('\0'))
        || argv.iter().map(String::len).sum::<usize>() > MAX_SERVICE_COMMAND_BYTES
    {
        return Err(Error::invalid(
            "check bundle commands need bounded argv without NUL",
        ));
    }
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    pub after_create: Option<String>,
    pub before_run: Option<String>,
    pub after_run: Option<String>,
    pub before_remove: Option<String>,
    #[serde(default = "hook_timeout")]
    pub timeout_ms: u64,
}
impl Default for Hooks {
    fn default() -> Self {
        Self {
            after_create: None,
            before_run: None,
            after_run: None,
            before_remove: None,
            timeout_ms: hook_timeout(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontMatter {
    #[serde(default = "schema_one")]
    pub schema_version: u32,
    #[serde(default)]
    pub hooks: Hooks,
    #[serde(default)]
    pub checks: Checks,
}
impl Default for FrontMatter {
    fn default() -> Self {
        Self {
            schema_version: 1,
            hooks: Hooks::default(),
            checks: Checks::default(),
        }
    }
}
/// An installed instruction asset identity, never a filesystem locator or authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddedWorkflow {
    pub version: u32,
    pub digest: String,
}
#[derive(Clone, Debug)]
pub struct Workflow {
    pub path: Option<PathBuf>,
    pub embedded: Option<EmbeddedWorkflow>,
    pub digest: String,
    pub front_matter: FrontMatter,
    pub body: String,
    original_embedded: Option<EmbeddedWorkflow>,
    nodes: Vec<Node>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ResolvedCheck {
    pub cwd: PathBuf,
    pub argv: Vec<String>,
    pub artifact_roots: Vec<PathBuf>,
    pub result_artifact: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<CheckService>,
}
impl Workflow {
    /// The installed generic workflow does not read or infer a repository file.
    pub fn installed_default() -> Result<Self> {
        crate::instructions::verify_assets()?;
        let mut workflow = Self::parse(DEFAULT_WORKFLOW, PathBuf::new())?;
        workflow.path = None;
        workflow.embedded = Some(EmbeddedWorkflow {
            version: DEFAULT_WORKFLOW_VERSION,
            digest: workflow.digest.clone(),
        });
        workflow.original_embedded = workflow.embedded.clone();
        Ok(workflow)
    }
    /// Reload a recorded origin. Absence, conflicting origins and obsolete
    /// embedded assets refuse; an explicit file failure never selects a default.
    pub fn load_recorded(path: Option<&Path>, embedded: Option<&EmbeddedWorkflow>) -> Result<Self> {
        match (path, embedded) {
            (Some(path), None) => Self::load(path),
            (None, Some(expected)) => {
                let workflow = Self::installed_default()?;
                if workflow.embedded.as_ref() != Some(expected) {
                    return Err(Error::new(
                        "stale_config",
                        "recorded installed workflow differs from this runtime",
                    ));
                }
                Ok(workflow)
            }
            _ => Err(Error::invalid(
                "workflow requires exactly one recorded source",
            )),
        }
    }
    /// Validate descriptive origin fields without reading a file or granting work.
    pub fn validate_source(&self) -> Result<()> {
        match (&self.path, &self.embedded) {
            (Some(path), None) if path.is_absolute() => Ok(()),
            (None, Some(identity))
                if identity.version == DEFAULT_WORKFLOW_VERSION
                    && identity.digest == bytes_digest(DEFAULT_WORKFLOW.as_bytes())
                    && identity.digest == self.digest
                    && self.original_embedded.as_ref() == Some(identity)
                    && self.body == DEFAULT_WORKFLOW
                    && serde_json::to_value(&self.front_matter)
                        .map_err(|_| Error::invalid("invalid default workflow metadata"))?
                        == serde_json::to_value(FrontMatter::default())
                            .map_err(|_| Error::invalid("invalid default workflow metadata"))? =>
            {
                Ok(())
            }
            _ => Err(Error::invalid(
                "workflow source is missing, conflicting or obsolete",
            )),
        }
    }
    pub fn same_source(&self, other: &Self) -> bool {
        self.path == other.path && self.embedded == other.embedded && self.digest == other.digest
    }
    pub fn load(path: &Path) -> Result<Self> {
        let metadata = std::fs::metadata(path)
            .map_err(|e| Error::io("read configured workflow metadata", e))?;
        if metadata.len() > MAX_WORKFLOW_BYTES as u64 || !metadata.is_file() {
            return Err(Error::invalid(
                "configured workflow is not a bounded regular file",
            ));
        }
        let bytes = std::fs::read(path).map_err(|e| Error::io("read configured workflow", e))?;
        let text =
            std::str::from_utf8(&bytes).map_err(|_| Error::invalid("workflow must be UTF-8"))?;
        let canonical =
            std::fs::canonicalize(path).map_err(|e| Error::io("resolve configured workflow", e))?;
        Self::parse(text, canonical)
    }
    pub fn parse(text: &str, path: impl Into<PathBuf>) -> Result<Self> {
        if text.len() > MAX_WORKFLOW_BYTES || text.contains('\0') {
            return Err(Error::invalid("workflow is oversized or contains NUL"));
        }
        let (front_matter, body) = if text.lines().next() == Some("---") {
            let mut lines = text.split_inclusive('\n');
            lines.next();
            let mut yaml = String::new();
            let mut offset = text.find('\n').map_or(text.len(), |i| i + 1);
            let mut closed = false;
            for line in lines {
                offset += line.len();
                if line.trim_end_matches(['\r', '\n']) == "---" {
                    closed = true;
                    break;
                }
                yaml.push_str(line);
            }
            if !closed {
                return Err(Error::invalid("workflow front matter lacks closing ---"));
            }
            // Deserializing directly into closed structs rejects duplicate known
            // keys; the pre-pass also rejects duplicates in dynamic check maps.
            let value: serde_yaml::Value = serde_yaml::from_str(&yaml).map_err(|e| {
                Error::invalid(format!(
                    "invalid workflow YAML at line {}",
                    e.location().map_or(0, |position| position.line())
                ))
            })?;
            if !value.is_mapping() {
                return Err(Error::invalid("workflow front matter must be a mapping"));
            }
            reject_yaml_extensions(&value)?;
            let json = serde_json::to_value(value).map_err(|_| {
                Error::invalid("workflow YAML must use finite JSON-compatible scalar types")
            })?;
            let metadata: FrontMatter = serde_json::from_value(json).map_err(|_| {
                Error::invalid(
                    "workflow front matter has unknown keys, missing fields or wrong types",
                )
            })?;
            (metadata, text[offset..].to_owned())
        } else {
            (FrontMatter::default(), text.to_owned())
        };
        if front_matter.schema_version != 1
            || front_matter.hooks.timeout_ms == 0
            || body.trim().is_empty()
        {
            return Err(Error::invalid(
                "workflow needs schema 1, positive hook timeout and nonempty prompt body",
            ));
        }
        for hook in [
            &front_matter.hooks.after_create,
            &front_matter.hooks.before_run,
            &front_matter.hooks.after_run,
            &front_matter.hooks.before_remove,
        ]
        .into_iter()
        .flatten()
        {
            if hook.trim().is_empty() || hook.contains('\0') {
                return Err(Error::invalid(
                    "hook scripts must be nonempty and contain no NUL",
                ));
            }
        }
        for (name, command) in front_matter
            .checks
            .local
            .iter()
            .chain(front_matter.checks.integrated.iter())
        {
            if name.trim().is_empty()
                || command.argv.is_empty()
                || command.argv[0].trim().is_empty()
                || command.argv.iter().any(|v| v.contains('\0'))
                || command.result_artifact.is_empty()
                || command.result_artifact.contains('\0')
            {
                return Err(Error::invalid(
                    "checks need a name, argv command and result_artifact",
                ));
            }
            command.validate_services()?;
        }
        let tokens = tokenize(&body)?;
        let mut cursor = 0;
        let nodes = parse_nodes(&tokens, &mut cursor, &[], 0)?;
        if cursor != tokens.len() {
            return Err(Error::invalid("unexpected closing template tag"));
        }
        Ok(Self {
            path: Some(path.into()),
            embedded: None,
            digest: bytes_digest(text.as_bytes()),
            front_matter,
            body,
            original_embedded: None,
            nodes,
        })
    }
    pub fn render(&self, context: &Value) -> Result<String> {
        let mut context = context
            .as_object()
            .cloned()
            .ok_or_else(|| Error::invalid("template context must be an object"))?;
        let mut output = String::new();
        let mut iterations = 0;
        render_nodes(&self.nodes, &mut context, &mut output, &mut iterations)?;
        Ok(output)
    }
    /// Commands retain exactly their declared argv. Artifact roots are checked
    /// against the execution cwd, never the workflow's containing directory.
    pub fn resolve_check(
        &self,
        command: &CheckCommand,
        cwd: &Path,
        artifact_roots: &[PathBuf],
    ) -> Result<ResolvedCheck> {
        // Recheck declarations supplied through the public typed API as well as
        // parsed workflows; resolution does not grant service execution.
        command.validate_services()?;
        let cwd = std::fs::canonicalize(cwd).map_err(|e| Error::io("resolve check cwd", e))?;
        if !cwd.is_dir() {
            return Err(Error::invalid("check cwd is not an execution directory"));
        }
        let roots = canonical_roots(artifact_roots)?;
        if !roots.iter().any(|root| cwd.starts_with(root)) {
            return Err(Error::invalid(
                "check cwd is outside operation artifact roots",
            ));
        }
        let artifact = Path::new(&command.result_artifact);
        let path = if artifact.is_absolute() {
            artifact.to_path_buf()
        } else {
            cwd.join(artifact)
        };
        let resolved = contained_path(&path, &roots, false)?;
        Ok(ResolvedCheck {
            cwd,
            argv: command.argv.clone(),
            artifact_roots: roots,
            result_artifact: resolved,
            services: command.services.clone(),
        })
    }
    /// Use only for an explicitly identified workflow resource link. This does
    /// not discover links, grant access, or relocate command arguments.
    pub fn resolve_resource(&self, reference: &str, read_roots: &[PathBuf]) -> Result<PathBuf> {
        let resource = Path::new(reference);
        let path = if resource.is_absolute() {
            resource.to_path_buf()
        } else {
            self.path
                .as_ref()
                .and_then(|path| path.parent())
                .ok_or_else(|| Error::invalid("workflow has no resource anchor"))?
                .join(resource)
        };
        contained_path(&path, &canonical_roots(read_roots)?, true)
    }
}
pub fn canonical_roots(roots: &[PathBuf]) -> Result<Vec<PathBuf>> {
    if roots.is_empty() {
        return Err(Error::invalid("operation requires explicit allowed roots"));
    }
    roots
        .iter()
        .map(|root| std::fs::canonicalize(root).map_err(|e| Error::io("resolve allowed root", e)))
        .collect()
}
/// Checks symlink-resolved ancestry, including the nearest existing ancestor of
/// an output that has not been created. Recheck immediately before use.
pub fn contained_path(path: &Path, roots: &[PathBuf], must_exist: bool) -> Result<PathBuf> {
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().contains(&0)
        || path.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err(Error::invalid("path must be absolute and traversal-free"));
    }
    let mut ancestor = path.to_path_buf();
    let mut tail = Vec::new();
    loop {
        match std::fs::symlink_metadata(&ancestor) {
            Ok(_) => break,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !must_exist => {
                tail.push(
                    ancestor
                        .file_name()
                        .ok_or_else(|| Error::invalid("path has no existing ancestor"))?
                        .to_owned(),
                );
                if !ancestor.pop() {
                    return Err(Error::invalid("path has no existing ancestor"));
                }
            }
            Err(e) => return Err(Error::io("resolve operation path", e)),
        }
    }
    let mut resolved = std::fs::canonicalize(ancestor)
        .map_err(|e| Error::io("resolve operation path symlinks", e))?;
    for component in tail.into_iter().rev() {
        resolved.push(component);
    }
    if !roots.iter().any(|root| resolved.starts_with(root)) {
        return Err(Error::new(
            "access_denied",
            "path escapes the operation's allowed roots",
        ));
    }
    Ok(resolved)
}

#[derive(Clone, Debug)]
enum Token {
    Text(String),
    Output(String),
    Tag(String),
}
#[derive(Clone, Debug)]
enum Node {
    Text(String),
    Output(Expression),
    Assign(String, Expression),
    If(Vec<(String, Vec<Node>)>, Vec<Node>),
    For(String, Expression, Vec<Node>, Vec<Node>),
}
#[derive(Clone, Debug)]
struct Expression {
    base: String,
    filters: Vec<(String, Vec<String>)>,
}
fn tokenize(source: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut remaining = source;
    let mut trim_next = false;
    while !remaining.is_empty() {
        let start = [remaining.find("{{"), remaining.find("{%")]
            .into_iter()
            .flatten()
            .min();
        let Some(index) = start else {
            let text = if trim_next {
                remaining.trim_start()
            } else {
                remaining
            };
            tokens.push(Token::Text(text.into()));
            break;
        };
        let text = &remaining[..index];
        let text = if trim_next { text.trim_start() } else { text };
        let left_trim = remaining[index + 2..].starts_with('-');
        let text = if left_trim { text.trim_end() } else { text };
        if !text.is_empty() {
            tokens.push(Token::Text(text.into()));
        }
        let output = remaining[index..].starts_with("{{");
        let closing = if output { "}}" } else { "%}" };
        let begin = index + if left_trim { 3 } else { 2 };
        let end = remaining[begin..]
            .find(closing)
            .ok_or_else(|| Error::invalid("unterminated template expression"))?
            + begin;
        let raw = &remaining[begin..end];
        trim_next = raw.ends_with('-');
        let raw = if trim_next {
            &raw[..raw.len() - 1]
        } else {
            raw
        };
        if raw.trim().is_empty() {
            return Err(Error::invalid("empty template expression"));
        }
        tokens.push(if output {
            Token::Output(raw.trim().into())
        } else {
            Token::Tag(raw.trim().into())
        });
        remaining = &remaining[end + 2..];
    }
    Ok(tokens)
}
fn parse_nodes(
    tokens: &[Token],
    cursor: &mut usize,
    stops: &[&str],
    depth: usize,
) -> Result<Vec<Node>> {
    if depth > 64 {
        return Err(Error::invalid("template nesting exceeds 64 levels"));
    }
    let mut nodes = Vec::new();
    while let Some(token) = tokens.get(*cursor) {
        match token {
            Token::Text(text) => nodes.push(Node::Text(text.clone())),
            Token::Output(expression) => nodes.push(Node::Output(Expression::parse(expression)?)),
            Token::Tag(tag) => {
                let (name, rest) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
                if stops.contains(&name) {
                    return Ok(nodes);
                }
                match name {
                    "assign" => {
                        let (name, expression) = rest
                            .split_once('=')
                            .ok_or_else(|| Error::invalid("assign requires name = expression"))?;
                        let name = name.trim();
                        validate_identifier(name)?;
                        nodes.push(Node::Assign(
                            name.into(),
                            Expression::parse(expression.trim())?,
                        ));
                    }
                    "if" => {
                        validate_condition(rest)?;
                        let mut arms = Vec::new();
                        let mut condition = rest.to_owned();
                        let mut otherwise = Vec::new();
                        loop {
                            *cursor += 1;
                            let body = parse_nodes(
                                tokens,
                                cursor,
                                &["elsif", "else", "endif"],
                                depth + 1,
                            )?;
                            arms.push((condition, body));
                            let Some(Token::Tag(stop)) = tokens.get(*cursor) else {
                                return Err(Error::invalid("if lacks endif"));
                            };
                            if stop == "endif" {
                                break;
                            }
                            if stop == "else" {
                                *cursor += 1;
                                otherwise = parse_nodes(tokens, cursor, &["endif"], depth + 1)?;
                                if !matches!(tokens.get(*cursor), Some(Token::Tag(end)) if end == "endif")
                                {
                                    return Err(Error::invalid("if lacks endif"));
                                }
                                break;
                            }
                            if let Some(next) = stop.strip_prefix("elsif ") {
                                validate_condition(next)?;
                                condition = next.into();
                            } else {
                                return Err(Error::invalid("invalid if control tag"));
                            }
                        }
                        nodes.push(Node::If(arms, otherwise));
                    }
                    "for" => {
                        let (name, expression) = rest
                            .split_once(" in ")
                            .ok_or_else(|| Error::invalid("for requires name in expression"))?;
                        validate_identifier(name.trim())?;
                        let expression = Expression::parse(expression.trim())?;
                        *cursor += 1;
                        let body = parse_nodes(tokens, cursor, &["else", "endfor"], depth + 1)?;
                        let mut otherwise = Vec::new();
                        if matches!(tokens.get(*cursor), Some(Token::Tag(tag)) if tag == "else") {
                            *cursor += 1;
                            otherwise = parse_nodes(tokens, cursor, &["endfor"], depth + 1)?;
                        }
                        if !matches!(tokens.get(*cursor), Some(Token::Tag(tag)) if tag == "endfor")
                        {
                            return Err(Error::invalid("for lacks endfor"));
                        }
                        nodes.push(Node::For(name.trim().into(), expression, body, otherwise));
                    }
                    _ => return Err(Error::invalid(format!("unsupported template tag: {name}"))),
                }
            }
        }
        *cursor += 1;
    }
    Ok(nodes)
}
fn validate_identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .enumerate()
            .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
    {
        return Err(Error::invalid("invalid template identifier"));
    }
    Ok(())
}
/// Split only outside quoted string literals and bracket path selectors.
fn split_outside<'a>(value: &'a str, delimiter: &str) -> Result<Vec<&'a str>> {
    let mut parts = Vec::new();
    let mut quote = None;
    let mut bracket = 0usize;
    let mut last = 0;
    let mut escaped = false;
    for (offset, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote.is_some() && ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        if ch == '[' {
            bracket += 1;
        }
        if ch == ']' {
            bracket = bracket
                .checked_sub(1)
                .ok_or_else(|| Error::invalid("unbalanced template path"))?;
        }
        if bracket == 0 && value[offset..].starts_with(delimiter) && offset >= last {
            parts.push(value[last..offset].trim());
            last = offset + delimiter.len();
        }
    }
    if quote.is_some() || bracket != 0 {
        return Err(Error::invalid("unterminated template string or path"));
    }
    parts.push(value[last..].trim());
    Ok(parts)
}
impl Expression {
    fn parse(source: &str) -> Result<Self> {
        let parts = split_outside(source, "|")?;
        if parts[0].is_empty() {
            return Err(Error::invalid("empty expression"));
        }
        let mut filters = Vec::new();
        for part in &parts[1..] {
            let fields = split_outside(part, ":")?;
            let name = fields[0];
            let needs_argument = matches!(name, "default" | "join");
            if ![
                "json", "escape", "default", "join", "size", "upcase", "downcase", "strip",
            ]
            .contains(&name)
            {
                return Err(Error::invalid(format!("unknown template filter: {name}")));
            }
            if fields.len() > 2
                || (needs_argument && fields.len() != 2)
                || (!needs_argument && fields.len() != 1)
            {
                return Err(Error::invalid(format!(
                    "invalid arguments for filter {name}"
                )));
            }
            let args = if fields.len() == 2 {
                split_outside(fields[1], ",")?
                    .iter()
                    .map(|s| s.to_string())
                    .collect()
            } else {
                Vec::new()
            };
            if needs_argument && (args.len() != 1 || args[0].is_empty()) {
                return Err(Error::invalid(format!(
                    "filter {name} requires one argument"
                )));
            }
            filters.push((name.into(), args));
        }
        Ok(Self {
            base: parts[0].into(),
            filters,
        })
    }
    fn eval(&self, context: &Map<String, Value>) -> Result<Value> {
        let mut value = atom(&self.base, context)?;
        for (filter, arguments) in &self.filters {
            let argument = || atom(&arguments[0], context);
            value = match filter.as_str() {
                "json" => Value::String(
                    serde_json::to_string(&value)
                        .map_err(|e| Error::invalid(format!("cannot encode template JSON: {e}")))?,
                ),
                "default" => {
                    let fallback = argument()?;
                    if value.is_null()
                        || value == Value::Bool(false)
                        || value.as_str().is_some_and(str::is_empty)
                        || value.as_array().is_some_and(Vec::is_empty)
                        || value.as_object().is_some_and(Map::is_empty)
                    {
                        fallback
                    } else {
                        value
                    }
                }
                "size" => Value::from(match &value {
                    Value::String(s) => s.chars().count(),
                    Value::Array(a) => a.len(),
                    Value::Object(o) => o.len(),
                    _ => return Err(Error::invalid("size expects string, array or map")),
                }),
                "join" => {
                    let separator = argument()?;
                    let separator = separator
                        .as_str()
                        .ok_or_else(|| Error::invalid("join separator must be a string"))?;
                    let items = value
                        .as_array()
                        .ok_or_else(|| Error::invalid("join expects an array"))?;
                    Value::String(
                        items
                            .iter()
                            .map(scalar)
                            .collect::<Result<Vec<_>>>()?
                            .join(separator),
                    )
                }
                "escape" => Value::String(
                    value
                        .as_str()
                        .ok_or_else(|| Error::invalid("escape expects a string"))?
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;")
                        .replace('"', "&quot;")
                        .replace('\'', "&#39;"),
                ),
                "upcase" => Value::String(
                    value
                        .as_str()
                        .ok_or_else(|| Error::invalid("upcase expects a string"))?
                        .to_uppercase(),
                ),
                "downcase" => Value::String(
                    value
                        .as_str()
                        .ok_or_else(|| Error::invalid("downcase expects a string"))?
                        .to_lowercase(),
                ),
                "strip" => Value::String(
                    value
                        .as_str()
                        .ok_or_else(|| Error::invalid("strip expects a string"))?
                        .trim()
                        .into(),
                ),
                _ => return Err(Error::invalid("unknown compiled filter")),
            };
        }
        Ok(value)
    }
}
fn atom(source: &str, context: &Map<String, Value>) -> Result<Value> {
    let source = source.trim();
    if source.starts_with('"') {
        return serde_json::from_str::<String>(source)
            .map(Value::String)
            .map_err(|_| Error::invalid("invalid double-quoted template literal"));
    }
    if source.starts_with('\'') {
        if source.len() >= 2 && source.ends_with('\'') {
            return Ok(Value::String(
                source[1..source.len() - 1]
                    .replace("\\'", "'")
                    .replace("\\\\", "\\"),
            ));
        }
        return Err(Error::invalid("invalid string literal"));
    }
    match source {
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        "null" | "nil" => return Ok(Value::Null),
        _ => (),
    }
    if let Ok(number) = source.parse::<serde_json::Number>() {
        return Ok(Value::Number(number));
    }
    let path = variable_path(source)?;
    let first = path
        .first()
        .ok_or_else(|| Error::invalid("empty variable path"))?;
    let mut value = context
        .get(first)
        .ok_or_else(|| Error::invalid(format!("unknown template variable: {first}")))?;
    for key in &path[1..] {
        value = match value {
            Value::Object(object) => object.get(key),
            Value::Array(array) => key.parse::<usize>().ok().and_then(|index| array.get(index)),
            _ => None,
        }
        .ok_or_else(|| {
            Error::invalid(format!("unknown or incompatible template path: {source}"))
        })?;
    }
    Ok(value.clone())
}
fn variable_path(source: &str) -> Result<Vec<String>> {
    fn identifier(rest: &str) -> Result<(&str, &str)> {
        let end = rest.find(['.', '[']).unwrap_or(rest.len());
        let part = &rest[..end];
        if part.is_empty()
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(Error::invalid("invalid variable path"));
        }
        Ok((part, &rest[end..]))
    }
    let (first, mut rest) = identifier(source)?;
    let mut path = vec![first.to_owned()];
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix('.') {
            let (part, tail) = identifier(tail)?;
            path.push(part.to_owned());
            rest = tail;
        } else if let Some(tail) = rest.strip_prefix('[') {
            let mut quote = None;
            let mut escaped = false;
            let mut end = None;
            for (offset, character) in tail.char_indices() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if quote.is_some() && character == '\\' {
                    escaped = true;
                    continue;
                }
                if let Some(q) = quote {
                    if character == q {
                        quote = None;
                    }
                    continue;
                }
                if character == '\'' || character == '"' {
                    quote = Some(character);
                    continue;
                }
                if character == ']' {
                    end = Some(offset);
                    break;
                }
            }
            let end = end.ok_or_else(|| Error::invalid("unclosed path selector"))?;
            let selector = tail[..end].trim();
            if selector.starts_with('"') {
                path.push(
                    serde_json::from_str::<String>(selector)
                        .map_err(|_| Error::invalid("invalid quoted path selector"))?,
                );
            } else if selector.starts_with('\'') && selector.ends_with('\'') && selector.len() >= 2
            {
                let key = &selector[1..selector.len() - 1];
                if key.contains('\'') {
                    return Err(Error::invalid("invalid single-quoted path selector"));
                }
                path.push(key.to_owned());
            } else if selector.parse::<usize>().is_ok() {
                path.push(selector.to_owned());
            } else {
                return Err(Error::invalid(
                    "path selectors must be literal keys or nonnegative indexes",
                ));
            }
            rest = &tail[end + 1..];
        } else {
            return Err(Error::invalid(
                "variable selectors require a dot or bracket separator",
            ));
        }
    }
    Ok(path)
}
fn scalar(value: &Value) -> Result<String> {
    match value {
        Value::Null => Ok(String::new()),
        Value::String(s) => Ok(s.clone()),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(Error::invalid(
            "structured template values require the json or join filter",
        )),
    }
}
fn truthy(value: &Value) -> bool {
    !value.is_null() && value != &Value::Bool(false)
}
fn validate_condition(source: &str) -> Result<()> {
    if source.trim().is_empty() {
        return Err(Error::invalid("if/elsif needs a condition"));
    }
    // Parse filters now, even inside untaken branches. Conditions themselves are
    // evaluated strictly against the actual runtime context.
    for logical in split_outside(source, " or ")? {
        for term in split_outside(logical, " and ")? {
            let mut comparison = false;
            for operator in [" == ", " != ", " >= ", " <= ", " > ", " < ", " contains "] {
                let parts = split_outside(term, operator)?;
                if parts.len() > 1 {
                    if parts.len() != 2 {
                        return Err(Error::invalid(
                            "chained template comparisons are unsupported",
                        ));
                    }
                    for part in parts {
                        Expression::parse(part)?;
                    }
                    comparison = true;
                    break;
                }
            }
            if !comparison {
                Expression::parse(term)?;
            }
        }
    }
    Ok(())
}
fn condition(source: &str, context: &Map<String, Value>) -> Result<bool> {
    let disjunction = split_outside(source, " or ")?;
    if disjunction.len() > 1 {
        let mut result = false;
        for part in disjunction {
            result |= condition(part, context)?;
        }
        return Ok(result);
    }
    let conjunction = split_outside(source, " and ")?;
    if conjunction.len() > 1 {
        let mut result = true;
        for part in conjunction {
            result &= condition(part, context)?;
        }
        return Ok(result);
    }
    for operator in [" == ", " != ", " >= ", " <= ", " > ", " < ", " contains "] {
        let parts = split_outside(source, operator)?;
        if parts.len() == 2 {
            let left = Expression::parse(parts[0])?.eval(context)?;
            let right = Expression::parse(parts[1])?.eval(context)?;
            return match operator {
                " == " => Ok(left == right),
                " != " => Ok(left != right),
                " contains " => match &left {
                    Value::Array(items) => Ok(items.contains(&right)),
                    Value::String(text) => Ok(text.contains(
                        right
                            .as_str()
                            .ok_or_else(|| Error::invalid("string contains requires a string"))?,
                    )),
                    _ => Err(Error::invalid("contains requires an array or string")),
                },
                _ => {
                    let order = match (&left, &right) {
                        (Value::Number(a), Value::Number(b)) => a
                            .as_f64()
                            .and_then(|a| b.as_f64().and_then(|b| a.partial_cmp(&b))),
                        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
                        _ => None,
                    }
                    .ok_or_else(|| Error::invalid("comparison operands have incompatible types"))?;
                    Ok(match operator {
                        " >= " => !order.is_lt(),
                        " <= " => !order.is_gt(),
                        " > " => order.is_gt(),
                        " < " => order.is_lt(),
                        _ => false,
                    })
                }
            };
        }
    }
    Ok(truthy(&Expression::parse(source)?.eval(context)?))
}
fn render_nodes(
    nodes: &[Node],
    context: &mut Map<String, Value>,
    output: &mut String,
    iterations: &mut usize,
) -> Result<()> {
    for node in nodes {
        match node {
            Node::Text(text) => append(output, text)?,
            Node::Output(expression) => append(output, &scalar(&expression.eval(context)?)?)?,
            Node::Assign(name, expression) => {
                let value = expression.eval(context)?;
                context.insert(name.clone(), value);
            }
            Node::If(arms, otherwise) => {
                let mut selected = otherwise;
                for (test, body) in arms {
                    if condition(test, context)? {
                        selected = body;
                        break;
                    }
                }
                render_nodes(selected, context, output, iterations)?;
            }
            Node::For(name, expression, body, otherwise) => {
                let value = expression.eval(context)?;
                let items = match value {
                    Value::Array(items) => items,
                    Value::Object(map) => map
                        .into_iter()
                        .map(|(key, value)| Value::Array(vec![Value::String(key), value]))
                        .collect(),
                    _ => return Err(Error::invalid("for requires an array or map")),
                };
                if items.is_empty() {
                    render_nodes(otherwise, context, output, iterations)?;
                    continue;
                }
                let original = context.get(name).cloned();
                let outer_loop = context.get("forloop").cloned();
                let length = items.len();
                for (index, item) in items.into_iter().enumerate() {
                    *iterations += 1;
                    if *iterations > MAX_LOOP_ITERATIONS {
                        return Err(Error::invalid("template exceeds finite loop bound"));
                    }
                    context.insert(name.clone(), item);
                    context.insert("forloop".into(), serde_json::json!({"index": index + 1, "index0": index, "rindex": length - index, "rindex0": length - index - 1, "length": length, "first": index == 0, "last": index + 1 == length, "parentloop": outer_loop}));
                    render_nodes(body, context, output, iterations)?;
                }
                if let Some(original) = original {
                    context.insert(name.clone(), original);
                } else {
                    context.remove(name);
                }
                if let Some(outer_loop) = outer_loop {
                    context.insert("forloop".into(), outer_loop);
                } else {
                    context.remove("forloop");
                }
            }
        }
    }
    Ok(())
}
fn append(output: &mut String, text: &str) -> Result<()> {
    if output.len().saturating_add(text.len()) > MAX_RENDER_BYTES {
        return Err(Error::invalid("rendered workflow exceeds 8 MiB"));
    }
    output.push_str(text);
    Ok(())
}

fn reject_yaml_extensions(value: &serde_yaml::Value) -> Result<()> {
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (key, value) in map {
                if !matches!(key, serde_yaml::Value::String(_)) {
                    return Err(Error::invalid("workflow YAML keys must be strings"));
                }
                reject_yaml_extensions(value)?;
            }
        }
        serde_yaml::Value::Sequence(items) => {
            for item in items {
                reject_yaml_extensions(item)?;
            }
        }
        serde_yaml::Value::Tagged(_) => {
            return Err(Error::invalid(
                "YAML tagged values cannot alter workflow types",
            ));
        }
        _ => (),
    }
    Ok(())
}
