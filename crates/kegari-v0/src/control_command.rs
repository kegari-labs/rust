//! Shared durable command identity and current-operator policy.
use crate::control::MutationRequest;
use kegari_config::{Config, ControlRole, Operator};
use kegari_control_api::identity::{Actor, Principal};
use kegari_types::api::CommandRequest;
use kegari_types::error::{Error, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

pub fn command_effect_id(command: Uuid) -> Uuid {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"kegari-v0/control-command-effect/v1\0");
    hash.update(command.as_bytes());
    let hash = hash.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}
pub fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| Error::new("invalid_input", format!("command requires {key}")))
}
pub fn parse_id(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value).map_err(|_| Error::new("invalid_input", "expected canonical UUID"))
}
pub fn resolve_workspace(config: &Config, reference: Option<&str>) -> Result<Option<Uuid>> {
    let Some(reference) = reference else {
        return Ok(None);
    };
    let found: Vec<_> = config
        .workspaces
        .iter()
        .filter(|(name, w)| reference == name.as_str() || reference == w.id.to_string())
        .map(|(_, w)| w.id)
        .collect();
    match found.as_slice() {
        [id] => Ok(Some(*id)),
        [] => Err(Error::new(
            "unknown_resource",
            "workspace is not configured in this runtime",
        )),
        _ => Err(Error::new(
            "ambiguous_reference",
            "workspace reference is ambiguous",
        )),
    }
}
pub fn verified_operator<'a>(config: &'a Config, actor: &Actor) -> Result<&'a Operator> {
    let operator = match &actor.principal {
        Some(Principal::Unix { uid }) => config.control.local_users.get(&uid.to_string()),
        Some(Principal::Tailscale { user_id, .. }) => {
            if config.control.https.is_none() {
                return Err(Error::new(
                    "access_denied",
                    "remote control was disabled after authentication",
                ));
            }
            config.control.tailscale_users.get(&user_id.to_string())
        }
        None => None,
    }
    .ok_or_else(|| {
        Error::new(
            "access_denied",
            "authenticated principal no longer has an operator mapping",
        )
    })?;
    if operator.id != actor.identity {
        return Err(Error::new(
            "access_denied",
            "authenticated principal mapping changed",
        ));
    }
    Ok(operator)
}
pub fn verify_current_actor(config: &Config, actor: &Actor, role: &str) -> Result<()> {
    actor.require(role)?;
    let expected = match role {
        "inspect" => ControlRole::Inspect,
        "configure" => ControlRole::Configure,
        "lifecycle" => ControlRole::Lifecycle,
        "correct" => ControlRole::Correct,
        "accept" => ControlRole::Accept,
        "release" => ControlRole::Release,
        "recover" => ControlRole::Recover,
        "evaluate" => ControlRole::Evaluate,
        _ => return Err(Error::new("access_denied", "unknown operator role")),
    };
    if !verified_operator(config, actor)?.roles.contains(&expected) {
        return Err(Error::new("access_denied", "operator role was revoked"));
    }
    Ok(())
}
pub fn mutation_request(
    actor: &Actor,
    request: &CommandRequest,
    targets: Vec<String>,
) -> MutationRequest {
    let mut expected = BTreeMap::new();
    if let Some(revision) = &request.expected_config_revision {
        expected.insert("config_revision".into(), revision.clone());
    }
    if let Some(generation) = request.expected_control_generation {
        expected.insert("control_generation".into(), generation.to_string());
    }
    MutationRequest {
        request_id: request.request_id,
        runtime_id: request.runtime_id,
        actor: actor.clone(),
        kind: format!("{:?}", request.command),
        canonical_targets: targets,
        expected,
        payload: request.args.clone(),
    }
}
