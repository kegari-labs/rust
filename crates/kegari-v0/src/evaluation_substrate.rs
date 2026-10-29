//! Declared empty fixture substrates, never runtime admission or source proof.
use super::*;
use kegari_context::workflow::Workflow;

pub(super) const WORKFLOW_LEAF: &str = "coordination-workflow.md";
pub(super) const UNBORN_WORKFLOW_LEAF: &str = "bootstrap-workflow.md";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Declaration {
    schema_version: u32,
    kind: String,
    workflow: String,
}
#[derive(Debug, Serialize)]
pub(super) struct CoordinationSubstrate {
    workflow_path: PathBuf,
    workflow_digest: String,
    workflow_mode: u32,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    unborn: bool,
}
fn invalid() -> Error {
    Error::new(
        "evaluation_fidelity",
        "coordination substrate must be declared empty, fresh and non-executing",
    )
}
pub(super) fn require_empty(path: &Path) -> Result<()> {
    // A file snapshot deliberately excludes .git and empty directories. Neither
    // may be ignored for this distinct, explicitly empty substrate contract.
    let metadata =
        std::fs::symlink_metadata(path).map_err(|e| Error::io("inspect empty substrate", e))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid());
    }
    if std::fs::read_dir(path)
        .map_err(|e| Error::io("read empty substrate", e))?
        .next()
        .transpose()
        .map_err(|e| Error::io("read substrate entry", e))?
        .is_some()
    {
        return Err(invalid());
    }
    Ok(())
}
fn no_execution(workflow: &Workflow) -> Result<()> {
    let f = &workflow.front_matter;
    if !f.checks.local.is_empty()
        || !f.checks.integrated.is_empty()
        || f.hooks.after_create.is_some()
        || f.hooks.before_run.is_some()
        || f.hooks.after_run.is_some()
        || f.hooks.before_remove.is_some()
    {
        return Err(invalid());
    }
    Ok(())
}
impl CoordinationSubstrate {
    pub(super) fn declared(
        captured: &fixture_capture::CapturedTree,
        variant: &Variant,
        root: &Path,
    ) -> Result<Option<Self>> {
        let Some(file) = captured.optional_file("substrate.json") else {
            return Ok(None);
        };
        let declaration: Declaration =
            serde_json::from_slice(file.bytes()).map_err(|_| invalid())?;
        let unborn = declaration.kind == "child_unborn";
        let leaf = if unborn {
            UNBORN_WORKFLOW_LEAF
        } else {
            WORKFLOW_LEAF
        };
        let role_matches = if unborn {
            variant.role == AgentRole::Implementation && variant.phase == "implementation"
        } else {
            variant.role == AgentRole::Coordination && variant.phase == "coordination"
        };
        if declaration.schema_version != 1
            || (!unborn && declaration.kind != "coordination_empty")
            || declaration.workflow != leaf
            || !role_matches
            || !matches!(variant.seed, Seed::Fresh)
            || variant.canary_diagnostic.is_some()
        {
            return Err(invalid());
        }
        // Same immutable capture as fixture_digest; no second source-tree read.
        let allowed = [
            "context.json",
            "context-binding.json",
            "substrate.json",
            leaf,
            "repository",
            if unborn {
                "native-issues.json"
            } else {
                "substrate.json"
            },
        ];
        if captured
            .topology()
            .iter()
            .any(|path| path.to_str().is_none_or(|path| !allowed.contains(&path)))
            || captured.optional_file("repository").is_some()
            || !captured.repository_files().is_empty()
        {
            return Err(invalid());
        }
        let input: Value = serde_json::from_slice(captured.file("context.json")?.bytes())
            .map_err(|_| invalid())?;
        let writes: Vec<String> = if let Some(file) = captured.optional_file("context-binding.json")
        {
            let binding: NativeContextBinding =
                serde_json::from_slice(file.bytes()).map_err(|_| invalid())?;
            binding.validate(&input)?;
            binding.write_paths().to_vec()
        } else {
            serde_json::from_value(
                input
                    .pointer("/scope/write_paths")
                    .cloned()
                    .ok_or_else(invalid)?,
            )
            .map_err(|_| invalid())?
        };
        if (!unborn && !writes.is_empty()) || (unborn && writes.is_empty()) {
            return Err(invalid());
        }
        let workflow_file = captured.file(leaf)?;
        let text = std::str::from_utf8(workflow_file.bytes()).map_err(|_| invalid())?;
        // Fixed workflow cannot be rebound to a future scratch/resource path.
        if text.contains("@KegariScratch@") {
            return Err(invalid());
        }
        let workflow_path = root.join(leaf);
        let workflow = Workflow::parse(text, &workflow_path)?;
        no_execution(&workflow)?;
        if unborn && workflow.digest != Workflow::installed_default()?.digest {
            return Err(invalid());
        }
        Ok(Some(Self {
            workflow_path,
            workflow_digest: workflow.digest,
            workflow_mode: captured.snapshot()[leaf].mode,
            unborn,
        }))
    }
    pub(super) fn is_unborn(&self) -> bool {
        self.unborn
    }
    pub(super) fn workflow_leaf(&self) -> &str {
        if self.unborn {
            UNBORN_WORKFLOW_LEAF
        } else {
            WORKFLOW_LEAF
        }
    }
    pub(super) fn workflow_path(&self) -> &Path {
        &self.workflow_path
    }
    pub(super) fn verify(&self) -> Result<()> {
        let file = fixture_capture::read_file(
            &self.workflow_path,
            kegari_context::workflow::MAX_WORKFLOW_BYTES,
        )?;
        if bytes_digest(file.bytes()) != self.workflow_digest || file.mode() != self.workflow_mode {
            return Err(Error::new(
                "fixture_changed",
                "retained coordination workflow changed",
            ));
        }
        let workflow = Workflow::parse(
            std::str::from_utf8(file.bytes()).map_err(|_| invalid())?,
            &self.workflow_path,
        )?;
        no_execution(&workflow)
    }
    pub(super) fn verify_contract(
        &self,
        variant: &Variant,
        repository: &Path,
        has_repository_identity: bool,
        workflow_digest: &str,
    ) -> Result<()> {
        let role_matches = if self.unborn {
            variant.role == AgentRole::Implementation && variant.phase == "implementation"
        } else {
            variant.role == AgentRole::Coordination && variant.phase == "coordination"
        };
        if !role_matches
            || !matches!(variant.seed, Seed::Fresh)
            || (!self.unborn && has_repository_identity)
            || workflow_digest != self.workflow_digest
            || self.workflow_path.starts_with(repository)
        {
            return Err(invalid());
        }
        self.verify()?;
        if self.unborn {
            if !snapshot(repository)?.is_empty() {
                return Err(invalid());
            }
            Ok(())
        } else {
            require_empty(repository)
        }
    }
}
