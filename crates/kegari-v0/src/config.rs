//! Backend configuration identity, workflow integration and reload inspection.
use crate::digest::canonical_digest;
use kegari_config::{Config, Workspace};
use kegari_types::error::{Error, Result};
use std::{collections::BTreeMap, path::Path};
use uuid::Uuid;

/// Preserve the canonical identity of the configuration exactly as supplied.
pub fn config_revision(config: &Config) -> Result<String> {
    canonical_digest(1, config)
}

/// Omission selects the installed generic workflow; an explicit selection
/// always requires that exact resolved file and never falls back.
pub fn load_workflow(workspace: &Workspace) -> Result<crate::workflow::Workflow> {
    match workspace.workflow.as_deref() {
        None => crate::workflow::Workflow::installed_default(),
        Some(selected) => {
            let path = Path::new(selected);
            if !path.is_absolute() {
                return Err(Error::invalid(
                    "resolve configured workflow path on the host before preflight",
                ));
            }
            crate::workflow::Workflow::load(path)
        }
    }
}

/// Read-only cross-validation; it never runs commands, hooks or evaluations.
/// Paths must first have been resolved on the execution host.
pub fn load_workflows(config: &Config) -> Result<BTreeMap<Uuid, crate::workflow::Workflow>> {
    let mut loaded = BTreeMap::new();
    for workspace in config.workspaces.values() {
        let workflow = load_workflow(workspace)?;
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
        let revision = config_revision(&c)?;
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
