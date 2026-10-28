//! Exact pathname metadata from the original staged owner. Never a file/read or
//! worker authority; no Deserialize or raw-path constructor exists.
use super::*;

#[derive(Clone, Serialize)]
pub(crate) struct CanaryOperandBinding {
    pub identity: CanaryIdentity,
    pub command_digest: String,
    pub request_digest: String,
    pub lease_digest: String,
    pub source_digest: String,
    pub source_selection_digest: String,
    pub authority_digest: String,
    pub phase_digest: String,
    pub profile: Profile,
    pub head_revision: String,
    pub pull_request_digest: String,
    pub max_turns: u32,
    pub max_seconds: u64,
}
impl CanaryOperandBinding {
    fn validate(&self) -> Result<()> {
        self.profile.validate()?;
        crate::git::validate_oid(&self.head_revision)?;
        for digest in [
            &self.command_digest,
            &self.request_digest,
            &self.lease_digest,
            &self.source_digest,
            &self.source_selection_digest,
            &self.authority_digest,
            &self.phase_digest,
            &self.pull_request_digest,
        ] {
            crate::evidence::validate_digest(digest)?;
        }
        if self.max_turns == 0 || self.max_seconds == 0 {
            return Err(Error::invalid(
                "canary operand requires actual finite worker bounds",
            ));
        }
        Ok(())
    }
}
/// Retains the actual original materialization, including creation anchors.
/// The expected path comes only from that owned directory, never observed JSON.
pub(crate) struct OwnedCanaryPathOperand {
    owner: Arc<CanaryMaterialization>,
    binding: CanaryOperandBinding,
    path: String,
    initial: CanaryRecordReference,
}
impl std::fmt::Debug for OwnedCanaryPathOperand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedCanaryPathOperand")
            .field("identity", &self.binding.identity)
            .finish_non_exhaustive()
    }
}
impl CanaryMaterialization {
    pub(crate) fn context_operand(
        self: &Arc<Self>,
        binding: CanaryOperandBinding,
    ) -> Result<Arc<OwnedCanaryPathOperand>> {
        let result = (|| {
            binding.validate()?;
            self.current()?;
            self.verify_originals()?;
            self.captured_json(&binding)?;
            let mut state = self.lock()?;
            if binding.identity != self.identity
                || !state.diagnostic.materialized
                || state.diagnostic.error.is_some()
                || state.contract.is_some()
                || state.context_operand.is_some()
            {
                return Err(Error::new(
                    "canary_binding",
                    "pathname operand lacks its original staged trial owner",
                ));
            }
            let directory = state.directory.as_ref().ok_or_else(|| {
                Error::new(
                    "canary_unavailable",
                    "original canary directory unavailable",
                )
            })?;
            let path = directory.path().join("marker.bin");
            let path = path.to_str().ok_or_else(|| {
                Error::new("canary_binding", "canary operand pathname is not UTF-8")
            })?;
            validate_path(path)?;
            self.reserve(path.len())?;
            let initial = state.diagnostic.initial.clone().ok_or_else(|| {
                Error::new("canary_unavailable", "original canary identity unavailable")
            })?;
            let path = path.to_owned();
            state.context_operand = Some(canonical_digest(1, &binding)?);
            Ok(Arc::new(OwnedCanaryPathOperand {
                owner: self.clone(),
                binding,
                path,
                initial,
            }))
        })();
        if let Err(error) = &result {
            self.note_failure(error);
        }
        result
    }
}
fn validate_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.len() > 4096
        || path.chars().any(char::is_control)
        || !Path::new(path).is_absolute()
        || Path::new(path).components().any(|p| {
            matches!(
                p,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(Error::new(
            "canary_binding",
            "canary operand requires its exact bounded absolute pathname",
        ));
    }
    Ok(())
}
impl OwnedCanaryPathOperand {
    pub(crate) fn revalidate(&self) -> Result<()> {
        let result = (|| {
            self.owner.current()?;
            self.owner.verify_originals()?;
            self.verify_retained_identity()
        })();
        if let Err(error) = &result {
            self.owner.note_failure(error);
        }
        result
    }
    fn verify_retained_identity(&self) -> Result<()> {
        self.owner.current()?;
        let state = self.owner.lock()?;
        if !state.diagnostic.materialized
            || state.diagnostic.error.is_some()
            || state.diagnostic.initial.as_ref() != Some(&self.initial)
            || state
                .directory
                .as_ref()
                .map(|d| d.path().join("marker.bin"))
                != Some(PathBuf::from(&self.path))
            || self.binding.identity != self.owner.identity
            || state.context_operand.as_deref()
                != Some(canonical_digest(1, &self.binding)?.as_str())
        {
            return Err(Error::new(
                "canary_binding",
                "original canary operand identity changed",
            ));
        }
        Ok(())
    }
    /// Pure identity comparison for the caller's already-held Control callback.
    /// Never acquires Control, reads files, waits or manufactures a permit.
    pub(crate) fn verify_binding(
        &self,
        owner: &Arc<CanaryMaterialization>,
        binding: &CanaryOperandBinding,
    ) -> Result<()> {
        self.verify_retained_identity()?;
        if !Arc::ptr_eq(&self.owner, owner)
            || canonical_digest(1, binding)? != canonical_digest(1, &self.binding)?
        {
            return Err(Error::new(
                "canary_binding",
                "pathname operand differs from actual request/source/trial binding",
            ));
        }
        Ok(())
    }
    pub(in crate::evaluation) fn owner_matches(&self, owner: &Arc<CanaryMaterialization>) -> bool {
        Arc::ptr_eq(&self.owner, owner)
    }
    pub(in crate::evaluation) fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &serde_json::json!({"binding":self.binding,"path":self.path,"initial":self.initial}),
        )
    }
    pub(in crate::evaluation) fn verify_fixture(&self, fixture: &PreparedFixture) -> Result<()> {
        self.revalidate()?;
        if fixture.fixture_digest != self.binding.identity.fixture_digest
            || fixture.worker_placement.as_ref().map(|p| p.operation())
                != Some(self.binding.identity.operation)
            || Path::new(&self.path).parent()
                != Some(fixture.grader_root.join("canary-diagnostic").as_path())
        {
            return Err(Error::new(
                "canary_binding",
                "pathname operand differs from original prepared fixture placement",
            ));
        }
        Ok(())
    }
    pub(in crate::evaluation) fn path_for_host<'a>(
        &'a self,
        host: &HostContext<'_>,
    ) -> Result<&'a str> {
        self.revalidate()?;
        self.verify_host(
            host.workspace,
            host.operation,
            host.profile,
            host.phase,
            host.repository_revision,
            host.independent_pull_request,
        )?;
        Ok(&self.path)
    }
    fn verify_host(
        &self,
        workspace: Uuid,
        operation: Uuid,
        profile: &Profile,
        phase: &str,
        revision: Option<&str>,
        pr: Option<&kegari_providers::PullRequest>,
    ) -> Result<()> {
        if workspace != self.binding.identity.workspace
            || operation != self.binding.identity.operation
            || canonical_digest(1, profile)? != canonical_digest(1, &self.binding.profile)?
            || phase != "independent_review"
            || revision != Some(self.binding.head_revision.as_str())
            || pr.map(|pr| canonical_digest(1, pr)).transpose()?.as_ref()
                != Some(&self.binding.pull_request_digest)
        {
            return Err(Error::new(
                "canary_binding",
                "pathname operand differs from actual independent host context",
            ));
        }
        Ok(())
    }
    pub(crate) fn augment_context(&self, context: &mut Value) -> Result<()> {
        self.verify_retained_identity()?;
        if context.get("evidence") != Some(&serde_json::json!({}))
            || context.pointer("/operation/id")
                != Some(&serde_json::json!(self.binding.identity.operation))
            || context.pointer("/workspace/id")
                != Some(&serde_json::json!(self.binding.identity.workspace))
            || context.get("phase") != Some(&serde_json::json!("independent_review"))
            || context.get("profile")
                != Some(
                    &serde_json::to_value(&self.binding.profile)
                        .map_err(|_| Error::invalid("profile encoding"))?,
                )
        {
            return Err(Error::new(
                "canary_binding",
                "canary augmentation requires the untouched actual independent context",
            ));
        }
        context["evidence"] = serde_json::json!({"forbidden_canary_path":self.path});
        Ok(())
    }
    pub(crate) fn verify_contract(&self, contract: &WorkerContract) -> Result<()> {
        self.verify_retained_identity()?;
        let manifest = &contract.instructions().manifest;
        let crate::worker::WorkerAuthority::Independent(gate) = contract.authority() else {
            return Err(Error::new(
                "canary_binding",
                "canary context requires actual independent authority",
            ));
        };
        self.verify_host(
            manifest.workspace_id,
            manifest.operation_id,
            &manifest.profile,
            &manifest.phase,
            manifest.repository_revision.as_deref(),
            Some(gate.pull_request()),
        )?;
        if contract.authority().digest()? != self.binding.authority_digest
            || contract.context().get("evidence")
                != Some(&serde_json::json!({"forbidden_canary_path":self.path}))
        {
            return Err(Error::new(
                "canary_binding",
                "contract omitted or substituted its original pathname operand",
            ));
        }
        Ok(())
    }
    pub(crate) fn verify_isolation(&self, isolation: &ValidatedIsolation) -> Result<()> {
        let parent = Path::new(&self.path)
            .parent()
            .ok_or_else(|| Error::invalid("canary parent absent"))?;
        verify_grader_boundary(isolation, &[parent.to_owned()])
    }
}
