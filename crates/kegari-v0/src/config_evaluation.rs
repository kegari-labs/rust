//! Declared local evaluation inputs. Configuration never grants execution.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationBinding {
    pub schema_version: u32,
    pub suite_manifest: String,
    pub suite_digest: String,
    pub report_root: String,
    pub fixture_root: String,
    pub case: String,
    pub variant: String,
    pub profile: String,
    pub repository_id: u64,
    pub pull_request: u64,
    pub author_id: u64,
    pub base_commit: String,
    pub head_commit: String,
}
impl EvaluationBinding {
    pub(crate) fn connection(workspace: Uuid, provider: &str) -> String {
        format!("evaluation-{workspace}-{provider}")
    }
    pub(crate) fn endpoint(workspace: Uuid, provider: &str) -> String {
        format!(
            "https://{provider}.{workspace}.evaluation.invalid{}",
            if provider == "linear" { "/graphql" } else { "" }
        )
    }
    pub(crate) fn validate(&self, config: &Config, workspace: &Workspace) -> Result<()> {
        if self.schema_version != 1
            || self.repository_id == 0
            || self.pull_request == 0
            || self.author_id == 0
        {
            return Err(Error::invalid("invalid declared evaluation namespace"));
        }
        crate::evidence::validate_digest(&self.suite_digest)?;
        crate::git::validate_oid(&self.base_commit)?;
        crate::git::validate_oid(&self.head_commit)?;
        if self.base_commit == self.head_commit {
            return Err(Error::invalid(
                "evaluation requires an actual distinct PR base/head",
            ));
        }
        for value in [&self.case, &self.variant, &self.profile] {
            if value.is_empty()
                || value.len() > 128
                || !value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
            {
                return Err(Error::invalid(
                    "evaluation names must be bounded identifiers",
                ));
            }
        }
        for path in [&self.suite_manifest, &self.report_root, &self.fixture_root] {
            path_syntax(path)?;
        }
        if !workspace.independent_review.enabled
            || workspace.profiles.independent_review.is_none()
            || workspace.release.mode != ReleaseMode::Manual
            || workspace.repositories.len() != 1
            || !workspace
                .repositories
                .contains_key(&self.repository_id.to_string())
            || !workspace.test_resources.is_empty()
            || !workspace.github.organizations.is_empty()
            || workspace.github.repositories != vec![self.repository_id.to_string()]
            || workspace.github.pr_authors != vec![self.author_id.to_string()]
            || workspace
                .repositories
                .values()
                .any(|r| !matches!(&r.deployment, Deployment::None(v) if v == "none"))
        {
            return Err(Error::invalid(
                "evaluation workspace must be independent-review-only with one undeployed repository and no test resources",
            ));
        }
        if workspace.repositories.values().any(|repository| {
            repository.hostname != format!("github.{}.evaluation.invalid", workspace.id)
        }) {
            return Err(Error::invalid(
                "evaluation repository must use its exact reserved synthetic hostname",
            ));
        }
        for provider in ["github", "linear"] {
            let name = Self::connection(workspace.id, provider);
            let configured = if provider == "github" {
                &workspace.github_connection
            } else {
                &workspace.linear_connection
            };
            let connection = config.connections.get(&name).ok_or_else(|| {
                Error::invalid("evaluation provider requires its reserved connection")
            })?;
            if configured != &name
                || connection.endpoint.as_deref()
                    != Some(Self::endpoint(workspace.id, provider).as_str())
            {
                return Err(Error::invalid(
                    "evaluation provider namespace differs from its reserved connection",
                ));
            }
            if config.workspaces.values().any(|other| {
                other.id != workspace.id
                    && (other.github_connection == name || other.linear_connection == name)
            }) {
                return Err(Error::invalid(
                    "evaluation connections cannot be shared with another workspace",
                ));
            }
        }
        Ok(())
    }
    /// Filesystem/current source checks are repeated by the explicit producer.
    /// This structural check already rejects obvious exposure before preflight.
    pub(crate) fn validate_resolved_paths(
        &self,
        config: &Config,
        _workspace: &Workspace,
    ) -> Result<()> {
        let report = Path::new(&self.report_root);
        let fixture = Path::new(&self.fixture_root);
        let suite = Path::new(&self.suite_manifest);
        let execution = Path::new(&config.runtime.execution_root);
        for path in [report, fixture, suite, execution] {
            if !path.is_absolute() {
                return Err(Error::invalid(
                    "evaluation paths must be resolved on the execution host",
                ));
            }
        }
        let overlaps = |a: &Path, b: &Path| a.starts_with(b) || b.starts_with(a);
        if overlaps(report, fixture)
            || overlaps(report, execution)
            || overlaps(fixture, execution)
            || report == suite
            || suite.starts_with(report)
            || config.workspaces.values().any(|w| {
                w.repositories
                    .values()
                    .any(|r| overlaps(report, Path::new(&r.source)))
            })
            || config.workspaces.values().any(|w| {
                w.repositories
                    .values()
                    .any(|r| overlaps(fixture, Path::new(&r.source)))
            })
        {
            return Err(Error::invalid(
                "evaluation report/grader or disposable fixture roots overlap worker/product sources",
            ));
        }
        Ok(())
    }
}

pub(crate) fn validate_bindings(config: &Config) -> Result<()> {
    for workspace in config.workspaces.values() {
        if let Some(binding) = &workspace.evaluation {
            binding.validate(config, workspace)?;
        } else if workspace.github_connection.starts_with("evaluation-")
            || workspace.linear_connection.starts_with("evaluation-")
        {
            return Err(Error::invalid(
                "reserved evaluation connections require an evaluation-only workspace",
            ));
        }
    }
    Ok(())
}
