//! Compatibility evidence comes from the actual complete assembly. Displayed
//! manifests alone never mint permission to resume a provider thread.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResumeSemantics {
    schema_version: u32,
    static_prompt: String,
    boundary: String,
    context: String,
    repository_guidance: String,
    rendered_workflow: String,
}

/// Sealed pair of independently assembled operations. All remaining runtime,
/// source, ownership, account and provider-history gates are still required.
#[derive(Clone, Debug)]
pub struct VerifiedResumeInstructions {
    previous: String,
    current: String,
    digest: String,
}
impl VerifiedResumeInstructions {
    pub fn previous_manifest(&self) -> &str {
        &self.previous
    }
    pub fn current_manifest(&self) -> &str {
        &self.current
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

pub(super) fn components(
    boundary: &Value,
    context: &Value,
    repository: &str,
    rendered: &str,
    static_prompt: String,
) -> Result<Option<ResumeSemantics>> {
    let mut boundary = boundary.clone();
    let mut context = context.clone();
    // Only these documented admission metadata fields may differ. Arbitrary
    // strings, native issue observations and nested operation-like keys remain
    // exact. The actual rendered workflow is never normalized: interpolation
    // of an operation or attempt can therefore make resumption incompatible.
    let attempt = &context["attempt"];
    if !attempt.is_null() && !attempt.as_u64().is_some_and(|n| n > 0) {
        return Ok(None);
    }
    boundary["operation_id"] = Value::Null;
    context["operation"]["id"] = Value::Null;
    context["attempt"] = Value::Null;
    Ok(Some(ResumeSemantics {
        schema_version: 1,
        static_prompt,
        boundary: canonical_digest(1, &boundary)?,
        context: canonical_digest(1, &context)?,
        repository_guidance: bytes_digest(repository.as_bytes()),
        rendered_workflow: bytes_digest(rendered.as_bytes()),
    }))
}

pub(super) fn compatible_manifests(
    previous: &InstructionManifest,
    current: &InstructionManifest,
) -> Result<bool> {
    let (Some(old), Some(new)) = (&previous.resume_semantics, &current.resume_semantics) else {
        return Ok(false);
    };
    if old.schema_version != 1 || old != new {
        return Ok(false);
    }
    let mut normalized = current.clone();
    normalized.operation_id = previous.operation_id;
    normalized.context_digest = previous.context_digest.clone();
    normalized.prompt_digest = previous.prompt_digest.clone();
    Ok(canonical_digest(ASSEMBLY_SCHEMA, previous)?
        == canonical_digest(ASSEMBLY_SCHEMA, &normalized)?)
}

pub(super) fn assembly_seal(
    prompt: &str,
    manifest: &InstructionManifest,
    digest: &str,
) -> Result<String> {
    if bytes_digest(prompt.as_bytes()) != manifest.prompt_digest
        || canonical_digest(ASSEMBLY_SCHEMA, manifest)? != digest
    {
        return Err(Error::new(
            "resume_incompatible",
            "assembly bytes or manifest identity changed",
        ));
    }
    canonical_digest(
        1,
        &serde_json::json!({"manifest":digest,"prompt":manifest.prompt_digest}),
    )
}

impl AssembledInstructions {
    pub fn verify_resume(&self, current: &Self) -> Result<VerifiedResumeInstructions> {
        if self.assembly_seal != assembly_seal(&self.prompt, &self.manifest, &self.manifest_digest)?
            || current.assembly_seal
                != assembly_seal(&current.prompt, &current.manifest, &current.manifest_digest)?
            || !compatible_manifests(&self.manifest, &current.manifest)?
        {
            return Err(Error::new(
                "resume_incompatible",
                "independently assembled effective instructions differ",
            ));
        }
        let previous = self.manifest_digest.clone();
        let current = current.manifest_digest.clone();
        let digest = canonical_digest(
            1,
            &serde_json::json!({"previous":previous,"current":current,"semantics":self.manifest.resume_semantics}),
        )?;
        Ok(VerifiedResumeInstructions {
            previous,
            current,
            digest,
        })
    }
}
