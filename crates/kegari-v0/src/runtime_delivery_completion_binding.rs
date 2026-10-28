//! Exact configured identity retained by immutable historical completion seals.
use crate::{evidence::WorkIdentity, git::RepositoryId};
use kegari_config::Config;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;
fn invalid(message: &str) -> Error {
    Error::new("release_completion", message)
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProviderBinding {
    pub(super) account: String,
    pub(super) endpoint: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DependencyBinding {
    pub(super) linear: ProviderBinding,
    pub(super) github: ProviderBinding,
    pub(super) linear_workspace: Uuid,
    pub(super) repositories: BTreeSet<RepositoryId>,
}
impl DependencyBinding {
    pub(super) fn configured(
        config: &Config,
        workspace_id: Uuid,
        repositories: BTreeSet<RepositoryId>,
    ) -> Result<Self> {
        let workspace = config
            .workspaces
            .values()
            .find(|w| w.id == workspace_id)
            .ok_or_else(|| invalid("completed release workspace is no longer configured"))?;
        let provider = |name: &str, kind: kegari_config::ProviderKind| -> Result<ProviderBinding> {
            let connection = config
                .connections
                .get(name)
                .filter(|connection| connection.kind == kind)
                .ok_or_else(|| invalid("completed release provider connection is unavailable"))?;
            let policy = kegari_providers::HttpPolicy::new(
                connection
                    .endpoint
                    .as_deref()
                    .ok_or_else(|| invalid("provider endpoint is absent"))?,
                connection.account_id.clone(),
                std::time::Duration::from_secs(1),
            )?;
            Ok(ProviderBinding {
                account: policy.account_id,
                endpoint: policy.endpoint.to_string(),
            })
        };
        if repositories.is_empty()
            || repositories.iter().any(|repository| {
                workspace
                    .repositories
                    .get(&repository.id.to_string())
                    .is_none_or(|configured| configured.hostname != repository.hostname)
            })
        {
            return Err(invalid(
                "completed release repository inventory is no longer configured",
            ));
        }
        Ok(Self {
            linear: provider(
                &workspace.linear_connection,
                kegari_config::ProviderKind::Linear,
            )?,
            github: provider(
                &workspace.github_connection,
                kegari_config::ProviderKind::Github,
            )?,
            linear_workspace: workspace.linear_workspace_id,
            repositories,
        })
    }
}

impl DependencyBinding {
    pub(super) fn validate_work(
        &self,
        work: &WorkIdentity,
        repositories: &BTreeSet<RepositoryId>,
    ) -> Result<()> {
        work.validate()?;
        if self.linear.endpoint != work.linear_endpoint
            || self.linear_workspace != work.linear_workspace
            || &self.repositories != repositories
            || repositories.is_empty()
        {
            return Err(invalid(
                "historical binding differs from exact native work/repositories",
            ));
        }
        for provider in [&self.linear, &self.github] {
            let observed = kegari_providers::HttpPolicy::new(
                &provider.endpoint,
                provider.account.clone(),
                std::time::Duration::from_secs(1),
            )?;
            if observed.endpoint.as_str() != provider.endpoint
                || observed.account_id != provider.account
            {
                return Err(invalid("historical provider binding is not canonical"));
            }
        }
        for repository in repositories {
            repository.validate()?;
        }
        Ok(())
    }
}
