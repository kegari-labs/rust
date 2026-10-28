//! A directory's creation identity survives later worker operations. Cleanup
//! observes the entire family; a per-operation registration cannot hide a peer.
use super::*;

const MAX_FAMILY_RECORDS: usize = 16_384;
const MAX_FAMILY_MEMBERS: usize = 4096;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryFamily {
    schema_version: u32,
    runtime_id: Uuid,
    directory_id: Uuid,
    original_operation: Uuid,
    marker: PathBuf,
    directory_digest: String,
    execution_root: PathBuf,
    repository: PathBuf,
    root_identity: (u64, u64),
    repository_identity: (u64, u64),
    members: BTreeMap<Uuid, FamilyMember>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FamilyMember {
    registration: String,
    predecessor: Option<Uuid>,
    resume_reference: Option<String>,
}
pub(super) struct FamilySnapshot {
    pub(super) digest: String,
    pub(super) registrations: BTreeMap<Uuid, ExecutionRegistration>,
}

/// Read-only proof of a closed family's exact current files and provenance.
/// Only a separately admitted typed resume can enroll another operation.
#[derive(Clone)]
pub struct VerifiedDirectoryReuse {
    housekeeping: Housekeeping,
    registration: ExecutionRegistration,
    source: crate::git::VerifiedReusableSource,
    predecessor_receipt_digest: String,
    family_digest: String,
    observations_digest: String,
    inventory_digest: String,
    digest: String,
}
impl VerifiedDirectoryReuse {
    pub fn predecessor(&self) -> Uuid {
        self.registration.binding.operation_id
    }
    pub fn binding(&self) -> &ExecutionBinding {
        &self.registration.binding
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    /// Safe within a Control admission predicate: bounded filesystem reads only.
    pub fn revalidate(&self) -> Result<()> {
        self.source.revalidate_filesystem()?;
        let current = self.housekeeping.registration(self.predecessor())?;
        let family = self
            .housekeeping
            .directory_family(&current)?
            .ok_or_else(|| {
                Error::new(
                    "resume_incompatible",
                    "legacy directory ownership cannot admit reuse",
                )
            })?;
        if canonical_digest(1, &current)? != canonical_digest(1, &self.registration)?
            || family.digest != self.family_digest
            || self
                .housekeeping
                .cleanup_observations(&current, &family.registrations, None)?
                != self.observations_digest
            || canonical_digest(1, &self.housekeeping.control.operation(self.predecessor())?)?
                != self.predecessor_receipt_digest
            || canonical_digest(1, &inventory(&current.isolation.execution_root)?)?
                != self.inventory_digest
        {
            return Err(Error::new(
                "stale_result",
                "directory reuse provenance or files changed",
            ));
        }
        for member in family.registrations.values() {
            for process in &member.processes {
                if self.housekeeping.probe.observe(process)? == ProcessObservation::Running {
                    return Err(Error::new(
                        "reconciliation_required",
                        "directory reuse retains a live owned process",
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Created only after the actual resumed permit and both durable family/index
/// records agree. It preserves the original directory marker, not an alias copy.
#[derive(Clone)]
pub struct EnrolledDirectoryReuse {
    housekeeping: Housekeeping,
    registration: ExecutionRegistration,
    source: crate::git::VerifiedReusableSource,
    members: BTreeSet<Uuid>,
    admission_digest: String,
    resume_digest: String,
    digest: String,
}
impl EnrolledDirectoryReuse {
    pub(crate) fn creation_ledger(&self) -> Result<(Uuid, HookLedger)> {
        self.revalidate()?;
        let id = self
            .registration
            .family_id
            .ok_or_else(|| Error::new("record_corrupt", "enrolled creation has no family"))?;
        let family: DirectoryFamily = self
            .housekeeping
            .root
            .child("cleanup")?
            .read(&format!("family-{id}.json"))?;
        let original = self.housekeeping.registration(family.original_operation)?;
        let ledger = self.housekeeping.current_hook_ledger(&original)?;
        if ledger.binding_digest != canonical_digest(1, &original.binding)? {
            return Err(Error::new(
                "execution_identity",
                "original creation ledger differs from its family",
            ));
        }
        Ok((family.original_operation, ledger))
    }
    pub fn operation_id(&self) -> Uuid {
        self.registration.binding.operation_id
    }
    pub fn binding(&self) -> &ExecutionBinding {
        &self.registration.binding
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    /// The successor may attach its own exact hook ledger and process records;
    /// none of those changes may replace its execution or family membership.
    pub fn revalidate(&self) -> Result<()> {
        self.source.revalidate_filesystem()?;
        let current = self.housekeeping.registration(self.operation_id())?;
        let family = self
            .housekeeping
            .directory_family(&current)?
            .ok_or_else(|| Error::new("record_corrupt", "enrolled directory lost its family"))?;
        let receipt = self.housekeeping.control.operation(self.operation_id())?;
        let resume =
            crate::control::RunResumeReference::from_receipt(&receipt)?.ok_or_else(|| {
                Error::new(
                    "record_corrupt",
                    "enrolled operation lost its resume admission",
                )
            })?;
        if current.binding != self.registration.binding
            || current.family_id != self.registration.family_id
            || current.marker != self.registration.marker
            || current.directory_digest != self.registration.directory_digest
            || current.workflow_path != self.registration.workflow_path
            || current.workflow_embedded != self.registration.workflow_embedded
            || current.workflow_digest != self.registration.workflow_digest
            || canonical_digest(1, &current.isolation)?
                != canonical_digest(1, &self.registration.isolation)?
            || family
                .registrations
                .keys()
                .copied()
                .collect::<BTreeSet<_>>()
                != self.members
            || canonical_digest(1, &receipt.expected)? != self.admission_digest
            || canonical_digest(1, &resume)? != self.resume_digest
        {
            return Err(Error::new(
                "stale_result",
                "enrolled directory identity or admission changed",
            ));
        }
        Ok(())
    }
    /// Observe an already completed enrollment after legitimate source edits or
    /// later family enrollment. This grants no source reuse or launch authority;
    /// the ordinary revalidate path above still requires its exact clean snapshot.
    pub(crate) fn verify_retained_admission(
        &self,
        observed: &crate::control::Receipt,
    ) -> Result<()> {
        let current = self.housekeeping.registration(self.operation_id())?;
        let family = self
            .housekeeping
            .directory_family(&current)?
            .ok_or_else(|| Error::new("record_corrupt", "retained enrollment lost its family"))?;
        let actual = self.housekeeping.control.operation(self.operation_id())?;
        let resume =
            crate::control::RunResumeReference::from_receipt(&actual)?.ok_or_else(|| {
                Error::new(
                    "record_corrupt",
                    "retained enrollment lost its resume admission",
                )
            })?;
        let members = family
            .registrations
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        if actual.phase != crate::control::ReceiptPhase::Reconciled
            || canonical_digest(1, &actual)? != canonical_digest(1, observed)?
            || current.binding != self.registration.binding
            || current.family_id != self.registration.family_id
            || current.marker != self.registration.marker
            || current.directory_digest != self.registration.directory_digest
            || current.workflow_path != self.registration.workflow_path
            || current.workflow_embedded != self.registration.workflow_embedded
            || current.workflow_digest != self.registration.workflow_digest
            || canonical_digest(1, &current.isolation)?
                != canonical_digest(1, &self.registration.isolation)?
            || !self.members.is_subset(&members)
            || canonical_digest(1, &actual.expected)? != self.admission_digest
            || canonical_digest(1, &resume)? != self.resume_digest
        {
            return Err(Error::new(
                "stale_result",
                "retained enrollment identity, membership or admission changed",
            ));
        }
        Ok(())
    }
}

impl std::fmt::Debug for VerifiedDirectoryReuse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VerifiedDirectoryReuse")
            .field("predecessor", &self.predecessor())
            .field("digest", &self.digest)
            .finish()
    }
}
impl std::fmt::Debug for EnrolledDirectoryReuse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnrolledDirectoryReuse")
            .field("operation", &self.operation_id())
            .field("digest", &self.digest)
            .finish()
    }
}

fn identity(path: &Path) -> Result<(u64, u64)> {
    let directory = PrivateDir::open(path)?;
    let metadata = std::fs::symlink_metadata(directory.path())
        .map_err(|e| Error::io("inspect directory family identity", e))?;
    Ok((metadata.dev(), metadata.ino()))
}
fn execution_work_key(binding: &ExecutionBinding) -> Result<String> {
    crate::authority::IssueKey::new(
        &binding.work.linear_endpoint,
        binding.work.linear_workspace,
        binding.work.child.unwrap_or(binding.work.parent),
    )?
    .identity()
}
fn immutable_registration(value: &ExecutionRegistration) -> Result<String> {
    let mut identity = json!({"schema":value.schema_version,"binding":value.binding,
        "isolation":value.isolation,"marker":value.marker,"directory":value.directory_digest,
        "workflow_path":value.workflow_path,"workflow_digest":value.workflow_digest,"family":value.family_id,"hook_location":value.hook_location_digest});
    if let Some(embedded) = &value.workflow_embedded {
        identity["workflow_embedded"] = serde_json::to_value(embedded)
            .map_err(|_| Error::invalid("cannot encode installed workflow identity"))?;
    }
    canonical_digest(1, &identity)
}

impl Housekeeping {
    /// Recover a fixed metadata read root from an actual protected execution
    /// registration. The configured source remains the operator checkout; the
    /// owned clone is authorized by its original admission and closed family.
    /// No Git process runs here and no unchanged-source claim is made.
    pub(crate) fn retained_execution_read_root(
        &self,
        configuration: &kegari_config::Config,
        binding: &ExecutionBinding,
        isolation: &crate::isolation::ValidatedIsolation,
        directory: &OwnedDirectory,
        workflow: &Workflow,
    ) -> Result<crate::git::VerifiedHostReadRoot> {
        self.control.serialize_observation(|state| {
            self.registration_updates_allowed(state)?;
            let storage = self.root.child("cleanup")?;
            let _guard = storage.section_lock("families.lock")?;
            binding.validate(isolation)?;
            directory.verify(binding, isolation)?;
            let registration = self.registration(binding.operation_id)?;
            let receipt = self.control.operation(binding.operation_id)?;
            let run = crate::control::RunReference::from_receipt(&receipt)?.ok_or_else(|| {
                Error::new(
                    "resume_incompatible",
                    "retained execution has no actual run admission",
                )
            })?;
            let workspace = configuration
                .workspaces
                .values()
                .find(|w| w.id == run.workspace)
                .ok_or_else(|| {
                    Error::new(
                        "scope_violation",
                        "retained execution workspace is no longer configured",
                    )
                })?;
            let repository = workspace
                .repositories
                .get(&binding.repository.id)
                .ok_or_else(|| {
                    Error::new(
                        "scope_violation",
                        "retained execution repository is no longer configured",
                    )
                })?;
            let operator = Path::new(&repository.source)
                .canonicalize()
                .map_err(|e| Error::io("resolve retained execution configured source", e))?;
            let configured_root = Path::new(&configuration.runtime.execution_root)
                .canonicalize()
                .map_err(|e| Error::io("resolve configured execution root", e))?;
            let configured_workflow = crate::config::load_workflow(workspace)?;
            if configuration.runtime.id != binding.runtime_id
                || registration.binding != *binding
                || registration.directory_digest != directory.digest()
                || registration.marker != directory.marker()
                || registration.workflow_path != workflow.path
                || registration.workflow_embedded != workflow.embedded
                || registration.workflow_digest != workflow.digest
                || canonical_digest(1, &registration.isolation)?
                    != canonical_digest(1, isolation.plan())?
                || !configured_workflow.same_source(workflow)
                || !isolation
                    .plan()
                    .execution_root
                    .starts_with(&configured_root)
                || directory.identity().operator_checkout != operator
                || workspace.linear_workspace_id != binding.work.linear_workspace
                || repository.hostname != binding.repository.hostname
                || receipt.runtime_id != binding.runtime_id
                || receipt.phase != ReceiptPhase::Reconciled
                || run.phase.work != execution_work_key(binding)?
                || receipt.canonical_targets != [run.phase.work.clone()]
                || state
                    .active_operations
                    .iter()
                    .any(|p| p.work_key == run.phase.work)
                || state
                    .unresolved_operations
                    .values()
                    .any(|targets| targets.contains(&run.phase.work))
            {
                return Err(Error::new(
                    "resume_incompatible",
                    "retained metadata root differs from its exact closed configured execution",
                ));
            }
            let family = self.directory_family(&registration)?.ok_or_else(|| {
                Error::new(
                    "resume_incompatible",
                    "legacy directory cannot supply retained session metadata authority",
                )
            })?;
            let observations =
                self.cleanup_observations(&registration, &family.registrations, None)?;
            for (operation, member) in &family.registrations {
                let closed = self.control.operation(*operation)?;
                let result = closed.observed_result.as_ref().ok_or_else(|| {
                    Error::new(
                        "reconciliation_required",
                        "retained directory member has no observed exit",
                    )
                })?;
                if closed.phase != ReceiptPhase::Reconciled
                    || result["local_execution"] != "exited"
                    || result["unresolved_effects"]
                        .as_array()
                        .is_none_or(|effects| !effects.is_empty())
                {
                    return Err(Error::new(
                        "reconciliation_required",
                        "retained directory member is not fully reconciled",
                    ));
                }
                for process in &member.processes {
                    if self.probe.observe(process)? == ProcessObservation::Running {
                        return Err(Error::new(
                            "reconciliation_required",
                            "retained directory member still has a live owned process",
                        ));
                    }
                }
            }
            let lineage = canonical_digest(
                1,
                &json!({
                    "purpose":"retained_execution_read","registration":registration,
                    "family":family.digest,"observations":observations,"receipt":receipt,
                    "configuration":crate::config::config_revision(configuration)?,"workflow":workflow.digest,
                }),
            )?;
            crate::git::retained_execution_host_read_root(
                configuration,
                binding,
                isolation,
                directory,
                lineage,
            )
        })
    }

    pub fn prepare_directory_reuse(
        &self,
        source: &crate::git::VerifiedReusableSource,
        isolation: &crate::isolation::ValidatedIsolation,
        directory: &OwnedDirectory,
        workflow: &Workflow,
    ) -> Result<VerifiedDirectoryReuse> {
        self.control.serialize_observation(|state| {
            self.registration_updates_allowed(state)?;
            let storage = self.root.child("cleanup")?;
            let _guard = storage.section_lock("families.lock")?;
            let binding = source.binding();
            binding.validate(isolation)?;
            directory.verify(binding, isolation)?;
            source.revalidate_filesystem()?;
            let registration = self.registration(binding.operation_id)?;
            let receipt = self.control.operation(binding.operation_id)?;
            let run = crate::control::RunReference::from_receipt(&receipt)?.ok_or_else(|| Error::new("resume_incompatible", "directory predecessor is not an admitted run"))?;
            if registration.binding != *binding
                || registration.directory_digest != directory.digest()
                || registration.marker != directory.marker()
                || registration.workflow_path != workflow.path
                || registration.workflow_embedded != workflow.embedded
                || registration.workflow_digest != workflow.digest
                || canonical_digest(1, &registration.isolation)? != canonical_digest(1, isolation.plan())?
                || receipt.runtime_id != binding.runtime_id
                || receipt.phase != ReceiptPhase::Reconciled
                || run.phase.work != execution_work_key(binding)?
                || state.active_operations.iter().any(|p| p.work_key == run.phase.work)
                || state.unresolved_operations.values().any(|targets| targets.iter().any(|target| receipt.canonical_targets.contains(target)))
            {
                return Err(Error::new("resume_incompatible", "directory reuse differs from its closed exact execution"));
            }
            let result = receipt.observed_result.as_ref().ok_or_else(|| Error::new("reconciliation_required", "directory predecessor has no observed exit"))?;
            if result["local_execution"] != "exited"
                || result["unresolved_effects"].as_array().is_none_or(|effects| !effects.is_empty())
            {
                return Err(Error::new("reconciliation_required", "directory predecessor is not fully reconciled"));
            }
            let family = self.directory_family(&registration)?.ok_or_else(|| Error::new("resume_incompatible", "legacy directories require independent recovery before reuse"))?;
            let observations_digest = self.cleanup_observations(&registration, &family.registrations, None)?;
            let inventory_digest = canonical_digest(1, &inventory(&registration.isolation.execution_root)?)?;
            let predecessor_receipt_digest = canonical_digest(1, &receipt)?;
            let digest = canonical_digest(1, &json!({"registration":registration,"family":family.digest,"observations":observations_digest,"inventory":inventory_digest,"source":source.digest(),"predecessor":predecessor_receipt_digest}))?;
            let proof = VerifiedDirectoryReuse { housekeeping: self.clone(), registration, source: source.clone(), predecessor_receipt_digest, family_digest: family.digest, observations_digest, inventory_digest, digest };
            proof.revalidate()?;
            Ok(proof)
        })
    }

    pub fn register_reused_execution(
        &self,
        permit: &crate::control::Permit,
        proof: &VerifiedDirectoryReuse,
        binding: &ExecutionBinding,
        isolation: &crate::isolation::ValidatedIsolation,
        directory: &OwnedDirectory,
        workflow: &Workflow,
    ) -> Result<EnrolledDirectoryReuse> {
        let mut enrolled = None;
        self.control.validate_permit(permit, || {
            let storage = self.root.child("cleanup")?;
            let _guard = storage.section_lock("families.lock")?;
            binding.validate(isolation)?;
            directory.verify(binding, isolation)?;
            let old = &proof.registration;
            let mut expected_binding = old.binding.clone();
            expected_binding.operation_id = permit.operation_id;
            let receipt = self.control.operation(permit.operation_id)?;
            let run = crate::control::RunReference::from_receipt(&receipt)?.ok_or_else(|| Error::new("resume_incompatible", "directory enrollment requires actual run admission"))?;
            let reference = crate::control::RunResumeReference::from_receipt(&receipt)?.ok_or_else(|| Error::new("resume_incompatible", "ordinary admission cannot reuse another operation's directory"))?;
            if !Arc::ptr_eq(&self.control, &proof.housekeeping.control)
                || binding != &expected_binding
                || binding.operation_id == proof.predecessor()
                || directory.digest() != old.directory_digest
                || directory.marker() != old.marker
                || canonical_digest(1, isolation.plan())? != canonical_digest(1, &old.isolation)?
                || workflow.path != old.workflow_path
                || workflow.embedded != old.workflow_embedded
                || workflow.digest != old.workflow_digest
                || reference.predecessor() != proof.predecessor()
                || reference.directory_reuse_digest() != proof.digest()
                || reference.predecessor_receipt_digest != proof.predecessor_receipt_digest
                || run.phase.work != execution_work_key(binding)?
                || receipt.runtime_id != binding.runtime_id
                || receipt.phase != ReceiptPhase::Prepared
                || permit.work_key != run.phase.work
            {
                return Err(Error::new("resume_incompatible", "directory enrollment differs from exact admitted resume"));
            }
            let registration = ExecutionRegistration { schema_version: 2, family_id: old.family_id, hook_location_digest: None, binding: binding.clone(), isolation: isolation.plan().clone(), marker: directory.marker().into(), directory_digest: directory.digest().into(), workflow_path: workflow.path.clone(), workflow_embedded: workflow.embedded.clone(), workflow_digest: workflow.digest.clone(), processes: vec![], retention: None };
            let id = old.family_id.ok_or_else(|| Error::new("resume_incompatible", "resumed directory has no family"))?;
            let name = format!("family-{id}.json");
            let resume_digest = canonical_digest(1, &reference)?;
            match self.registration(binding.operation_id) {
                Ok(current) => {
                    proof.source.revalidate_filesystem()?;
                    let mut expected = registration.clone();
                    expected.hook_location_digest = current.hook_location_digest.clone();
                    if immutable_registration(&current)? != immutable_registration(&expected)? {
                        return Err(Error::new("ownership_conflict", "resumed directory registration already differs"));
                    }
                    // A partial two-record enrollment stays blocked; no launch
                    // capability escapes until the complete index is readable.
                    self.directory_family(&current)?;
                }
                Err(error) if error.code == "record_missing" => {
                    proof.revalidate()?;
                    let mut family: DirectoryFamily = storage.read(&name)?;
                    if family.members.len() >= MAX_FAMILY_MEMBERS || family.members.contains_key(&binding.operation_id) {
                        return Err(Error::new("record_limit", "directory family cannot admit another member"));
                    }
                    family.members.insert(binding.operation_id, FamilyMember { registration: immutable_registration(&registration)?, predecessor: Some(proof.predecessor()), resume_reference: Some(resume_digest.clone()) });
                    self.save(&registration)?;
                    storage.write(&name, &family)?;
                }
                Err(error) => return Err(error),
            }
            let current = self.registration(binding.operation_id)?;
            let family = self.directory_family(&current)?.ok_or_else(|| Error::new("record_corrupt", "enrollment did not preserve its family"))?;
            let members = family.registrations.keys().copied().collect();
            let admission_digest = canonical_digest(1, &receipt.expected)?;
            let digest = canonical_digest(1, &json!({"reuse":proof.digest(),"binding":binding,"admission":admission_digest,"resume":resume_digest}))?;
            let value = EnrolledDirectoryReuse { housekeeping: self.clone(), registration: current, source: proof.source.clone(), members, admission_digest, resume_digest, digest };
            value.revalidate()?;
            enrolled = Some(value);
            Ok(())
        })?;
        enrolled.ok_or_else(|| {
            Error::new(
                "ownership_conflict",
                "directory enrollment was not observed",
            )
        })
    }
    // Called while Control and families.lock are held. An interrupted pointer,
    // family or registration write leaves a detectable mismatch; only this exact
    // immutable attachment may finish it, never a replacement location.
    pub(super) fn bind_hook_location(
        &self,
        old: &ExecutionRegistration,
        digest: String,
    ) -> Result<()> {
        if old
            .hook_location_digest
            .as_ref()
            .is_some_and(|value| value != &digest)
        {
            return Err(Error::new(
                "ownership_conflict",
                "hook ledger already has another binding",
            ));
        }
        let mut current = old.clone();
        current.hook_location_digest = Some(digest);
        if let Some(id) = old.family_id {
            let storage = self.root.child("cleanup")?;
            let name = format!("family-{id}.json");
            let mut family: DirectoryFamily = storage.read(&name)?;
            if family.directory_id != id || family.runtime_id != old.binding.runtime_id {
                return Err(Error::new(
                    "record_corrupt",
                    "hook ledger family identity changed",
                ));
            }
            let member = family
                .members
                .get_mut(&old.binding.operation_id)
                .ok_or_else(|| {
                    Error::new("record_corrupt", "hook ledger family member is missing")
                })?;
            let prior = immutable_registration(old)?;
            let next = immutable_registration(&current)?;
            if member.registration != prior && member.registration != next {
                return Err(Error::new(
                    "record_corrupt",
                    "hook ledger family registration changed",
                ));
            }
            member.registration = next;
            storage.write(&name, &family)?;
        }
        self.save(&current)
    }
    pub(super) fn reject_registration_alias(
        &self,
        registration: &ExecutionRegistration,
    ) -> Result<()> {
        let storage = self.root.child("cleanup")?;
        for name in storage.records_bounded(MAX_FAMILY_RECORDS)? {
            let Some(id) = name
                .strip_suffix(".json")
                .and_then(|s| Uuid::parse_str(s).ok())
            else {
                continue;
            };
            if id == registration.binding.operation_id {
                continue;
            }
            let old = self.registration(id)?;
            let a = &old.isolation.execution_root;
            let b = &registration.isolation.execution_root;
            if a.starts_with(b)
                || b.starts_with(a)
                || old.marker == registration.marker
                || old.directory_digest == registration.directory_digest
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "another operation already owns this directory; reuse requires exact family admission",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn create_directory_family(
        &self,
        registration: &ExecutionRegistration,
        directory: &OwnedDirectory,
    ) -> Result<()> {
        let id = directory.identity().directory_id;
        if registration.family_id != Some(id) {
            return Err(Error::new(
                "execution_ownership",
                "new registration lost its exact directory family",
            ));
        }
        let family = DirectoryFamily {
            schema_version: 1,
            runtime_id: registration.binding.runtime_id,
            directory_id: id,
            original_operation: registration.binding.operation_id,
            marker: registration.marker.clone(),
            directory_digest: registration.directory_digest.clone(),
            execution_root: registration.isolation.execution_root.clone(),
            repository: registration.isolation.repository.clone(),
            root_identity: identity(&registration.isolation.execution_root)?,
            repository_identity: identity(&registration.isolation.repository)?,
            members: BTreeMap::from([(
                registration.binding.operation_id,
                FamilyMember {
                    registration: immutable_registration(registration)?,
                    predecessor: None,
                    resume_reference: None,
                },
            )]),
        };
        let storage = self.root.child("cleanup")?;
        let name = format!("family-{id}.json");
        match storage.read::<DirectoryFamily>(&name) {
            Ok(old) if canonical_digest(1, &old)? == canonical_digest(1, &family)? => Ok(()),
            Ok(_) => Err(Error::new(
                "ownership_conflict",
                "directory family already has different ownership",
            )),
            Err(error) if error.code == "record_missing" => storage.write(&name, &family),
            Err(error) => Err(error),
        }
    }

    pub(super) fn directory_family(
        &self,
        registration: &ExecutionRegistration,
    ) -> Result<Option<FamilySnapshot>> {
        let Some(id) = registration.family_id else {
            // Preserve legacy single-operation cleanup, but never infer that
            // multiple registrations describe an authorized shared directory.
            self.reject_registration_alias(registration)?;
            let storage = self.root.child("cleanup")?;
            for name in storage.records_bounded(MAX_FAMILY_RECORDS)? {
                if name.starts_with("family-") {
                    let known: DirectoryFamily = storage.read(&name)?;
                    if known.directory_digest == registration.directory_digest
                        || known
                            .members
                            .contains_key(&registration.binding.operation_id)
                    {
                        return Err(Error::new(
                            "record_corrupt",
                            "family member cannot downgrade to legacy ownership",
                        ));
                    }
                }
            }
            return Ok(None);
        };
        let family: DirectoryFamily = self
            .root
            .child("cleanup")?
            .read(&format!("family-{id}.json"))?;
        if family.schema_version != 1
            || family.directory_id != id
            || family.runtime_id != registration.binding.runtime_id
            || family.marker != registration.marker
            || family.directory_digest != registration.directory_digest
            || family.execution_root != registration.isolation.execution_root
            || family.repository != registration.isolation.repository
            || family.root_identity != identity(&family.execution_root)?
            || family.repository_identity != identity(&family.repository)?
            || family.members.is_empty()
            || family.members.len() > MAX_FAMILY_MEMBERS
            || !family
                .members
                .contains_key(&registration.binding.operation_id)
        {
            return Err(Error::new(
                "ownership_conflict",
                "directory family identity, inventory or filesystem changed",
            ));
        }
        let mut members = BTreeMap::new();
        // Interrupted enrollment must not hide an index written before its
        // family entry, and a foreign family must not alias any owned path.
        for name in self
            .root
            .child("cleanup")?
            .records_bounded(MAX_FAMILY_RECORDS)?
        {
            let Some(operation) = name
                .strip_suffix(".json")
                .and_then(|s| Uuid::parse_str(s).ok())
            else {
                continue;
            };
            let current = self.registration(operation)?;
            if current.family_id == Some(id) {
                if !family.members.contains_key(&operation) {
                    return Err(Error::new(
                        "record_corrupt",
                        "unlisted directory family registration",
                    ));
                }
            } else if current
                .isolation
                .execution_root
                .starts_with(&family.execution_root)
                || family
                    .execution_root
                    .starts_with(&current.isolation.execution_root)
                || current.marker == family.marker
                || current.directory_digest == family.directory_digest
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "foreign registration aliases a directory family",
                ));
            }
        }
        for (operation, member) in &family.members {
            let current = self.registration(*operation)?;
            let mut expected_binding = registration.binding.clone();
            expected_binding.operation_id = *operation;
            if current.family_id != Some(id)
                || current.binding != expected_binding
                || current.marker != family.marker
                || current.directory_digest != family.directory_digest
                || canonical_digest(1, &current.isolation)?
                    != canonical_digest(1, &registration.isolation)?
                || current.workflow_path != registration.workflow_path
                || current.workflow_embedded != registration.workflow_embedded
                || current.workflow_digest != registration.workflow_digest
                || immutable_registration(&current)? != member.registration
            {
                return Err(Error::new(
                    "record_corrupt",
                    "family member registration changed or is incomplete",
                ));
            }
            if *operation == family.original_operation {
                if member.predecessor.is_some() || member.resume_reference.is_some() {
                    return Err(Error::new(
                        "record_corrupt",
                        "original directory family member has resume lineage",
                    ));
                }
            } else {
                let predecessor = member.predecessor.ok_or_else(|| {
                    Error::new("record_corrupt", "family member has no predecessor")
                })?;
                if predecessor == *operation || !family.members.contains_key(&predecessor) {
                    return Err(Error::new(
                        "record_corrupt",
                        "family predecessor is missing or self-referential",
                    ));
                }
                let receipt = self.control.operation(*operation)?;
                let reference = crate::control::RunResumeReference::from_receipt(&receipt)?
                    .ok_or_else(|| {
                        Error::new(
                            "record_corrupt",
                            "family member has no actual resumed admission",
                        )
                    })?;
                if reference.predecessor() != predecessor
                    || member.resume_reference.as_ref() != Some(&canonical_digest(1, &reference)?)
                {
                    return Err(Error::new(
                        "record_corrupt",
                        "family member admission lineage changed",
                    ));
                }
            }
            members.insert(*operation, current);
        }
        if !members.contains_key(&family.original_operation) {
            return Err(Error::new(
                "record_corrupt",
                "directory family lost original ownership",
            ));
        }
        for operation in family.members.keys() {
            let mut cursor = *operation;
            let mut seen = BTreeSet::new();
            while cursor != family.original_operation {
                if !seen.insert(cursor) {
                    return Err(Error::new(
                        "record_corrupt",
                        "directory family lineage is cyclic",
                    ));
                }
                cursor = family
                    .members
                    .get(&cursor)
                    .and_then(|m| m.predecessor)
                    .ok_or_else(|| {
                        Error::new("record_corrupt", "directory family lineage is incomplete")
                    })?;
            }
        }
        // Include mutable registrations too: new process and retention facts
        // invalidate a prior cleanup token even without another family member.
        let locations: BTreeMap<_, _> = members
            .iter()
            .map(|(id, member)| Ok((*id, self.hook_location(member)?)))
            .collect::<Result<_>>()?;
        let digest = canonical_digest(
            1,
            &json!({"family":family,"registrations":members,"hook_locations":locations}),
        )?;
        Ok(Some(FamilySnapshot {
            digest,
            registrations: members,
        }))
    }
}
