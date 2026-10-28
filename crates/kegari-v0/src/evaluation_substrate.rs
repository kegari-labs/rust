//! A declared empty fixture substrate, never a runtime admission or source proof.
use super::*;
use kegari_context::workflow::Workflow;

pub(super) const WORKFLOW_LEAF: &str = "coordination-workflow.md";
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
        if declaration.schema_version != 1
            || declaration.kind != "coordination_empty"
            || declaration.workflow != WORKFLOW_LEAF
            || variant.role != AgentRole::Coordination
            || variant.phase != "coordination"
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
            WORKFLOW_LEAF,
            "repository",
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
        if !writes.is_empty() {
            return Err(invalid());
        }
        let workflow_file = captured.file(WORKFLOW_LEAF)?;
        let text = std::str::from_utf8(workflow_file.bytes()).map_err(|_| invalid())?;
        // Fixed workflow cannot be rebound to a future scratch/resource path.
        if text.contains("@KegariScratch@") {
            return Err(invalid());
        }
        let workflow_path = root.join(WORKFLOW_LEAF);
        let workflow = Workflow::parse(text, &workflow_path)?;
        no_execution(&workflow)?;
        Ok(Some(Self {
            workflow_path,
            workflow_digest: workflow.digest,
            workflow_mode: captured.snapshot()[WORKFLOW_LEAF].mode,
        }))
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
        if variant.role != AgentRole::Coordination
            || variant.phase != "coordination"
            || !matches!(variant.seed, Seed::Fresh)
            || has_repository_identity
            || workflow_digest != self.workflow_digest
            || self.workflow_path.starts_with(repository)
        {
            return Err(invalid());
        }
        self.verify()?;
        require_empty(repository)
    }
}
