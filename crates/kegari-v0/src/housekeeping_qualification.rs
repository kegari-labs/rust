//! Explicit local recording of a human-reviewed qualification. This does not
//! execute checks or infer readiness from a successful build.
use super::*;
use kegari_control_api::identity::Principal;
use kegari_secrets::{AttestationKey, Signed};

#[derive(Clone, Debug, Serialize)]
pub struct RecordedQualification {
    pub request_id: Uuid,
    pub runtime_id: Uuid,
    pub record_path: PathBuf,
    pub qualification_digest: String,
    pub reviewed_by: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct HandoverCandidate {
    pub binary: BinaryIdentity,
    pub runtime_build: String,
    pub instruction_digest: String,
    pub metadata: HandoverInfo,
}

/// Inspect the exact candidate identities needed in a human-reviewed record.
/// This runs only the existing bounded metadata entry point, not qualification.
pub async fn inspect_handover_candidate(replacement: BinaryIdentity) -> Result<HandoverCandidate> {
    let info = replacement_info(&replacement).await?;
    Ok(HandoverCandidate {
        binary: replacement,
        runtime_build: info.runtime_build.clone(),
        instruction_digest: canonical_digest(1, &info)?,
        metadata: info,
    })
}

pub fn load_runtime_qualification(
    host: &HostIdentity,
    id: Uuid,
    key: &AttestationKey,
) -> Result<VerifiedRuntimeQualification> {
    if id.is_nil() {
        return Err(Error::invalid(
            "qualification record identity must not be nil",
        ));
    }
    let root = PrivateDir::open(&host.state_root())?;
    let records = PrivateDir::open(&root.path().join("qualifications"))?;
    let signed: Signed<RuntimeQualification> = records.read(&format!("{id}.json"))?;
    let verified = verify_runtime_qualification(&signed, key)?;
    let owner: OwnerRecord = root.read("owner.json")?;
    if owner.uid != host.uid
        || owner.host_id != host.host_id
        || owner.root != host.state_root()
        || owner.runtime_id != verified.record.runtime_id
        || verified.key_id != format!("keychain:{}:attestation-v1", owner.runtime_id)
    {
        return Err(Error::new(
            "identity_mismatch",
            "qualification belongs to another registered runtime",
        ));
    }
    Ok(verified)
}

/// The caller explicitly attests to reviewing all four existing evidence
/// categories. The file digests and exact binary/build/asset identities are
/// checked through the same handover verifier used when applying the result.
#[allow(clippy::too_many_arguments)]
pub async fn record_runtime_qualification(
    host: &HostIdentity,
    replacement: BinaryIdentity,
    record: RuntimeQualification,
    key: &AttestationKey,
    probe: &dyn ProcessProbe,
    actor: Actor,
    request_id: Uuid,
) -> Result<RecordedQualification> {
    actor.require("configure")?;
    if request_id.is_nil()
        || !matches!(actor.principal, Some(Principal::Unix { uid }) if uid == host.uid)
        || record.reviewed_by != actor.identity
    {
        return Err(Error::new(
            "access_denied",
            "qualification requires this host's mapped human reviewer",
        ));
    }
    // Check current operator mapping before invoking a candidate executable or
    // reading qualification evidence. Recheck under the stopped-host locks below.
    let root = PrivateDir::open(&host.state_root())?;
    let config = load_configuration(&root)?;
    crate::control_command::verify_current_actor(&config, &actor, "configure")?;
    let signed = key.sign("runtime_qualification", &record)?;
    let verified = verify_runtime_qualification(&signed, key)?;
    let plan = inspect_handover(host, replacement, &verified, probe).await?;

    let _control = root.lock("control.lock")?;
    let _runtime = root.lock("runtime.lock")?;
    let state = handover_state(host, &root, probe)?;
    crate::control_command::verify_current_actor(&state.config, &actor, "configure")?;
    if plan.owner_digest != canonical_digest(1, &state.owner)?
        || plan.config_digest != crate::config::config_revision(&state.config)?
        || plan.intent_digest != canonical_digest(1, &state.intent)?
        || plan.shutdown_digest != canonical_digest(1, &state.shutdown)?
        || plan.receipts_digest != canonical_digest(1, &state.receipts)?
    {
        return Err(Error::new(
            "stale_handover_plan",
            "runtime changed during qualification recording",
        ));
    }
    plan.replacement.revalidate(true)?;
    // Recheck the evidence immediately before persistence. A file replacement
    // between inspection and acknowledgement cannot inherit the prior digest.
    verify_runtime_qualification(&signed, key)?;
    let directory = root.child("qualifications")?;
    let name = format!("{request_id}.json");
    match directory.read::<Signed<RuntimeQualification>>(&name) {
        Ok(prior) if canonical_digest(1, &prior)? == verified.digest => {}
        Ok(_) => {
            return Err(Error::new(
                "operation_conflict",
                "qualification request identity was reused with different evidence",
            ));
        }
        Err(error) if error.code == "record_missing" => directory.write(&name, &signed)?,
        Err(error) => return Err(error),
    }
    Ok(RecordedQualification {
        request_id,
        runtime_id: record.runtime_id,
        record_path: directory.path().join(name),
        qualification_digest: verified.digest,
        reviewed_by: actor.identity,
    })
}

fn load_configuration(root: &PrivateDir) -> Result<kegari_config::Config> {
    let bytes = root.read_bytes("config.toml", 8 * 1024 * 1024)?;
    kegari_config::Config::parse(
        std::str::from_utf8(&bytes)
            .map_err(|_| Error::invalid("persisted configuration is not UTF-8"))?,
    )
}

/// Apply an inspected token, or read back the exact previously admitted request.
/// Replaying a successful handover must not require its old owner state to exist.
#[allow(clippy::too_many_arguments)]
pub async fn apply_recorded_handover(
    host: &HostIdentity,
    replacement: BinaryIdentity,
    qualification: &VerifiedRuntimeQualification,
    probe: &dyn ProcessProbe,
    actor: Actor,
    request_id: Uuid,
    expected_plan: &str,
) -> Result<Receipt> {
    actor.require("configure")?;
    actor.require("recover")?;
    if request_id.is_nil()
        || expected_plan.is_empty()
        || !matches!(actor.principal, Some(Principal::Unix { uid }) if uid == host.uid)
    {
        return Err(Error::new(
            "access_denied",
            "handover requires a local operator, request identity and inspected plan",
        ));
    }
    {
        let root = PrivateDir::open(&host.state_root())?;
        let _control = root.lock("control.lock")?;
        let config = load_configuration(&root)?;
        crate::control_command::verify_current_actor(&config, &actor, "configure")?;
        crate::control_command::verify_current_actor(&config, &actor, "recover")?;
        let store = ReceiptStore::new(root.child("receipts")?);
        match store.get(request_id) {
            Ok(_) => {
                let mutation = MutationRequest {
                    request_id,
                    runtime_id: qualification.record.runtime_id,
                    actor: actor.clone(),
                    kind: "handover".into(),
                    canonical_targets: vec![qualification.record.runtime_id.to_string()],
                    expected: BTreeMap::from([("handover_plan".into(), expected_plan.to_owned())]),
                    payload: json!({"replacement":replacement,"qualification":qualification.digest}),
                };
                return Ok(store.prepare(&mutation, 0)?.0);
            }
            Err(error) if error.code == "record_missing" => {}
            Err(error) => return Err(error),
        }
    }
    let plan = inspect_handover(host, replacement, qualification, probe).await?;
    if plan.token != expected_plan {
        return Err(Error::new(
            "stale_handover_plan",
            "handover facts changed after inspection; inspect the new plan before applying",
        ));
    }
    apply_handover(host, &plan, qualification, probe, actor, request_id).await
}
