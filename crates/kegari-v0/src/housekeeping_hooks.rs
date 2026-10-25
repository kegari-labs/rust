//! Cleanup follows the actual protected worker hook ledger, including later
//! session transitions, instead of treating an initial copy as current state.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HookLedgerLocation {
    schema_version: u32,
    runtime: Uuid,
    operation: Uuid,
    binding: String,
    directory: PathBuf,
    identity: (u64, u64),
}

impl Housekeeping {
    /// Registers one immutable location after the host creates the real ledger.
    /// The location is outside every worker-writable root and cannot be replaced.
    pub fn register_hook_ledger_location(
        &self,
        operation: Uuid,
        records: &PrivateDir,
    ) -> Result<()> {
        self.control.serialize_observation(|state| {
            self.registration_updates_allowed(state)?;
            let storage = self.root.child("cleanup")?;
            let _guard = storage.lock("families.lock")?;
            let registration = self.registration(operation)?;
            if !records.path().starts_with(self.root.path())
                || records
                    .path()
                    .starts_with(&registration.isolation.execution_root)
            {
                return Err(Error::new(
                    "unsafe_storage",
                    "hook ledger must remain in protected host state",
                ));
            }
            let directory = PrivateDir::open(records.path())?;
            let metadata = std::fs::symlink_metadata(directory.path())
                .map_err(|e| Error::io("inspect hook ledger location", e))?;
            let ledger: HookLedger = directory.read("hooks.json")?;
            let binding = canonical_digest(1, &registration.binding)?;
            if ledger.binding_digest != binding {
                return Err(Error::new(
                    "execution_identity",
                    "actual hook ledger belongs to another execution",
                ));
            }
            let value = HookLedgerLocation {
                schema_version: 1,
                runtime: registration.binding.runtime_id,
                operation,
                binding,
                directory: directory.path().into(),
                identity: (metadata.dev(), metadata.ino()),
            };
            let digest = canonical_digest(1, &value)?;
            if registration
                .hook_location_digest
                .as_ref()
                .is_some_and(|known| known != &digest)
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "actual hook ledger binding cannot be replaced",
                ));
            }
            let name = format!("hook-location-{operation}.json");
            match storage.read::<HookLedgerLocation>(&name) {
                Ok(old) if canonical_digest(1, &old)? == canonical_digest(1, &value)? => Ok(()),
                Ok(_) => Err(Error::new(
                    "ownership_conflict",
                    "actual hook ledger location is already bound",
                )),
                Err(error) if error.code == "record_missing" => storage.write(&name, &value),
                Err(error) => Err(error),
            }?;
            self.bind_hook_location(&registration, digest)
        })
    }

    pub(super) fn hook_location(
        &self,
        registration: &ExecutionRegistration,
    ) -> Result<Option<HookLedgerLocation>> {
        let location = self
            .root
            .child("cleanup")?
            .read::<HookLedgerLocation>(&format!(
                "hook-location-{}.json",
                registration.binding.operation_id
            ));
        match location {
            Ok(location) => {
                if location.schema_version != 1
                    || registration.hook_location_digest.as_ref()
                        != Some(&canonical_digest(1, &location)?)
                    || location.operation != registration.binding.operation_id
                    || location.runtime != registration.binding.runtime_id
                    || location.binding != canonical_digest(1, &registration.binding)?
                    || !location.directory.starts_with(self.root.path())
                    || location
                        .directory
                        .starts_with(&registration.isolation.execution_root)
                {
                    return Err(Error::new(
                        "record_corrupt",
                        "actual hook ledger reference changed identity",
                    ));
                }
                let directory = PrivateDir::open(&location.directory)?;
                let metadata = std::fs::symlink_metadata(directory.path())
                    .map_err(|e| Error::io("inspect hook ledger location", e))?;
                if (metadata.dev(), metadata.ino()) != location.identity {
                    return Err(Error::new(
                        "ownership_conflict",
                        "actual hook ledger directory was replaced",
                    ));
                }
                Ok(Some(location))
            }
            Err(error)
                if error.code == "record_missing"
                    && registration.hook_location_digest.is_none() =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn hook_storage(
        &self,
        registration: &ExecutionRegistration,
    ) -> Result<(PrivateDir, String)> {
        match self.hook_location(registration)? {
            Some(location) => Ok((PrivateDir::open(&location.directory)?, "hooks.json".into())),
            None => Ok((
                self.root.child("cleanup")?,
                format!("hooks-{}.json", registration.binding.operation_id),
            )),
        }
    }
    pub(super) fn current_hook_ledger(
        &self,
        registration: &ExecutionRegistration,
    ) -> Result<HookLedger> {
        let (directory, name) = self.hook_storage(registration)?;
        directory.read(&name)
    }

    pub(super) fn cleanup_observations(
        &self,
        registration: &ExecutionRegistration,
        members: &BTreeMap<Uuid, ExecutionRegistration>,
        active: Option<&VerifiedCleanupExecution>,
    ) -> Result<String> {
        let mut observations = BTreeMap::new();
        for (operation, member) in members {
            let receipt = self.control.operation(*operation)?;
            if receipt.runtime_id != registration.binding.runtime_id
                || matches!(receipt.phase, ReceiptPhase::Prepared | ReceiptPhase::Sent)
            {
                return Err(Error::new(
                    "cleanup_blocked",
                    "directory member has active or foreign ownership",
                ));
            }
            let workflow = Workflow::load(&member.workflow_path)?;
            if workflow.digest != member.workflow_digest {
                return Err(Error::new(
                    "stale_config",
                    "directory member workflow changed",
                ));
            }
            let mut digest = canonical_digest(1, &receipt)?;
            match self.current_hook_ledger(member) {
                Ok(mut ledger) => {
                    // The one current before_remove record belongs to this
                    // cleanup itself, so it cannot invalidate its own token.
                    if *operation == registration.binding.operation_id
                        && let Some(active) = active
                    {
                        ledger.records.retain(|_, record| {
                            record.phase != HookPhase::BeforeRemove
                                || record.cleanup_plan.as_deref() != Some(active.plan.digest())
                        });
                    }

                    if ledger.binding_digest != canonical_digest(1, &member.binding)?
                        || ledger
                            .sessions
                            .values()
                            .any(|s| *s == crate::execution::SessionLifecycle::Launched)
                        || ledger
                            .records
                            .values()
                            .any(|r| r.state == crate::execution::HookState::StartedUnknown)
                    {
                        return Err(Error::new(
                            "cleanup_blocked",
                            "directory member retains active or unknown hooks",
                        ));
                    }
                    digest = canonical_digest(1, &json!({"receipt":receipt,"hooks":ledger}))?;
                }
                Err(error)
                    if error.code == "record_missing"
                        && workflow.front_matter.hooks.before_remove.is_none()
                        && self.hook_location(member)?.is_none() => {}
                Err(error) => return Err(error),
            }
            observations.insert(*operation, digest);
        }
        if registration.family_id.is_some() {
            canonical_digest(1, &observations)
        } else {
            // Keep the legacy token shape; the checks above still reject active
            // or uncertain hooks, and the registration is checked separately.
            canonical_digest(
                1,
                &self.control.operation(registration.binding.operation_id)?,
            )
        }
    }
}
