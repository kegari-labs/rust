//! Host-only connection construction and verified credential imports. No worker
//! receives this registry, Keychain access, an arbitrary endpoint or a raw token.
use crate::{
    reconcile::WorkspaceClients,
    secrets::{Secret, SecretStore, resolve_reference},
};
use kegari_config::{Config, Connection, ProviderKind};
use kegari_digest::canonical_digest;
use kegari_providers::HttpTransport;
use kegari_providers::{
    AccountIdentity, GithubClient, HttpPolicy, LinearClient, verify_connection_identity,
};
use kegari_types::error::{Error, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

fn policy(connection: &Connection, timeout: Duration) -> Result<HttpPolicy> {
    HttpPolicy::new(
        connection
            .endpoint
            .as_deref()
            .ok_or_else(|| Error::invalid("provider connection requires its canonical endpoint"))?,
        connection.account_id.clone(),
        timeout,
    )
}

/// This capability is produced only by an actual adapter identity observation.
/// It is not deserializable and does not grant any workflow/operator authority.
pub struct VerifiedCredential {
    connection_digest: String,
    identity: AccountIdentity,
    secret: Secret,
}
impl VerifiedCredential {
    pub fn identity(&self) -> &AccountIdentity {
        &self.identity
    }
    /// Use only within the serialized setup/configuration transaction. On any
    /// Keychain failure the caller retains its old configuration unchanged.
    pub fn import(
        self,
        connection: &mut Connection,
        name: &str,
        store: &dyn SecretStore,
    ) -> Result<AccountIdentity> {
        if name.trim().is_empty()
            || name.len() > 512
            || name.chars().any(char::is_control)
            || canonical_digest(1, connection)? != self.connection_digest
        {
            return Err(Error::new(
                "identity_mismatch",
                "credential import does not match its verified connection",
            ));
        }
        store.save(name, &self.secret)?;
        connection.credential_ref = Some(format!("keychain:{name}"));
        Ok(self.identity)
    }
}
pub async fn verify_credential(
    connection: &Connection,
    secret: Secret,
    timeout: Duration,
    transport: Option<Arc<dyn HttpTransport>>,
) -> Result<VerifiedCredential> {
    if connection.kind == ProviderKind::Codex {
        return Err(Error::new(
            "authentication_required",
            "Codex uses its isolated supported ChatGPT login, not a provider API secret import",
        ));
    }
    let identity = verify_connection_identity(
        policy(connection, timeout)?,
        secret.credential()?,
        connection.kind,
        transport,
    )
    .await?;
    Ok(VerifiedCredential {
        connection_digest: canonical_digest(1, connection)?,
        identity,
        secret,
    })
}

pub struct ConnectionRegistry {
    revision: String,
    clients: BTreeMap<Uuid, WorkspaceClients>,
    blocked: BTreeMap<Uuid, Error>,
    secret_environment: BTreeSet<String>,
}
impl ConnectionRegistry {
    /// Building clients performs no HTTP requests or secret writes. Each later
    /// provider read/mutation rechecks the configured canonical account identity.
    /// Missing credentials are contained to the workspaces using that connection.
    pub fn bind(
        config: &Config,
        secrets: &dyn SecretStore,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        Self::bind_with_transports(config, secrets, env, &BTreeMap::new())
    }
    pub fn bind_with_transports(
        config: &Config,
        secrets: &dyn SecretStore,
        env: impl Fn(&str) -> Option<String>,
        transports: &BTreeMap<String, Arc<dyn HttpTransport>>,
    ) -> Result<Self> {
        config.validate()?;
        let mut values = BTreeMap::new();
        let mut failures = BTreeMap::new();
        let mut secret_environment = BTreeSet::new();
        for (name, connection) in &config.connections {
            if connection.kind == ProviderKind::Codex {
                continue;
            }
            let Some(reference) = connection.credential_ref.as_deref() else {
                failures.insert(
                    name.clone(),
                    Error::new("missing_secret", "provider credential reference is missing"),
                );
                continue;
            };
            if let Some(variable) = reference.strip_prefix('$') {
                secret_environment.insert(variable.to_owned());
            }
            match resolve_reference(reference, secrets, &env) {
                Ok(secret) => {
                    values.insert(name.clone(), secret);
                }
                Err(error) => {
                    failures.insert(name.clone(), error);
                }
            }
        }
        let timeout = Duration::from_millis(config.runtime.request_timeout_ms);
        let mut clients = BTreeMap::new();
        let mut blocked = BTreeMap::new();
        for workspace in config.workspaces.values() {
            let build = || -> Result<WorkspaceClients> {
                for name in [&workspace.linear_connection, &workspace.github_connection] {
                    if let Some(error) = failures.get(name) {
                        return Err(error.clone());
                    }
                }
                let linear = &config.connections[&workspace.linear_connection];
                let github = &config.connections[&workspace.github_connection];
                let linear_secret = values
                    .get(&workspace.linear_connection)
                    .ok_or_else(|| Error::new("missing_secret", "Linear credential unavailable"))?;
                let github_secret = values
                    .get(&workspace.github_connection)
                    .ok_or_else(|| Error::new("missing_secret", "GitHub credential unavailable"))?;
                Ok(WorkspaceClients {
                    linear: LinearClient::with_transport(
                        policy(linear, timeout)?,
                        workspace.linear_workspace_id,
                        workspace.linear.clone(),
                        linear_secret.credential()?,
                        transports.get(&workspace.linear_connection).cloned(),
                    )?,
                    github: GithubClient::with_transport(
                        policy(github, timeout)?,
                        workspace.github.clone(),
                        github_secret.credential()?,
                        transports.get(&workspace.github_connection).cloned(),
                    )?,
                })
            };
            match build() {
                Ok(bound) => {
                    clients.insert(workspace.id, bound);
                }
                Err(error) => {
                    blocked.insert(workspace.id, error);
                }
            }
        }
        Ok(Self {
            revision: crate::config::config_revision(config)?,
            clients,
            blocked,
            secret_environment,
        })
    }
    pub fn clients(&self, config: &Config) -> Result<&BTreeMap<Uuid, WorkspaceClients>> {
        if crate::config::config_revision(config)? != self.revision {
            return Err(Error::new(
                "stale_config",
                "provider registry belongs to an earlier configuration revision",
            ));
        }
        Ok(&self.clients)
    }
    pub fn blocked(&self) -> &BTreeMap<Uuid, Error> {
        &self.blocked
    }
    pub fn secret_environment(&self) -> &BTreeSet<String> {
        &self.secret_environment
    }
}
