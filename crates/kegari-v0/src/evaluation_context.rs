//! Versioned fixture declarations for complete native context matching. The
//! fixture declares every substitution before admission; observed task content
//! never selects fields to omit or normalize.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeContextBinding {
    schema_version: u32,
    slots: Vec<ContextSlot>,
    source_write_paths: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextSlot {
    pointer: String,
    source: ContextSource,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ContextSource {
    WorkspaceId,
    OperationId,
    Profile,
    Phase,
    WorkGeneration,
    CorrectionRevision,
    RepositoryRevision,
}
pub(super) struct HostContext<'a> {
    pub workspace: uuid::Uuid,
    pub operation: uuid::Uuid,
    pub profile: &'a Profile,
    pub phase: &'a str,
    pub work_generation: &'a str,
    pub correction_revision: &'a str,
    pub repository_revision: Option<&'a str>,
}
impl NativeContextBinding {
    pub(super) fn validate(&self, input: &Value) -> Result<()> {
        if self.schema_version != 1
            || self.slots.len() < 4
            || self.slots.len() > 64
            || self.source_write_paths.len() > MAX_FILES
            || self
                .slots
                .windows(2)
                .any(|pair| pair[0].pointer >= pair[1].pointer)
            || self
                .source_write_paths
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(Error::invalid(
                "native fixture binding must have bounded, sorted unique slots and paths",
            ));
        }
        for path in &self.source_write_paths {
            relative(path)?;
        }
        let mut expected = BTreeMap::new();
        for slot in &self.slots {
            if !slot.pointer.starts_with('/') || slot.pointer.starts_with("/timeline") {
                return Err(Error::invalid(
                    "native context substitution needs a task JSON pointer",
                ));
            }
            expected.insert(slot.pointer.clone(), slot.source);
        }
        for (pointer, source) in [
            ("/workspace/id", ContextSource::WorkspaceId),
            ("/operation/id", ContextSource::OperationId),
            ("/profile", ContextSource::Profile),
            ("/phase", ContextSource::Phase),
        ] {
            if expected.get(pointer) != Some(&source) {
                return Err(Error::invalid(
                    "native fixture must declare its exact host identity slots",
                ));
            }
        }
        let mut actual = BTreeMap::new();
        let mut context = input.clone();
        context
            .as_object_mut()
            .ok_or_else(|| Error::invalid("native fixture context must be an object"))?
            .remove("timeline");
        collect_slots(&context, "", 0, &mut actual)?;
        if expected != actual {
            return Err(Error::new(
                "evaluation_fidelity",
                "native context placeholders differ from the declared slot inventory",
            ));
        }
        Ok(())
    }
    pub(super) fn write_paths(&self) -> &[String] {
        &self.source_write_paths
    }
    pub(super) fn materialize(&self, input: &Value, host: &HostContext<'_>) -> Result<Value> {
        self.validate(input)?;
        if host.workspace.is_nil() || host.operation.is_nil() || host.phase.is_empty() {
            return Err(Error::invalid(
                "native fixture needs actual nonempty host identities",
            ));
        }
        host.profile.validate()?;
        let mut expected = input.clone();
        expected.as_object_mut().unwrap().remove("timeline");
        for slot in &self.slots {
            let value = match slot.source {
                ContextSource::WorkspaceId => serde_json::json!(host.workspace),
                ContextSource::OperationId => serde_json::json!(host.operation),
                ContextSource::Profile => serde_json::to_value(host.profile)
                    .map_err(|_| Error::invalid("profile encoding"))?,
                ContextSource::Phase => Value::String(host.phase.into()),
                ContextSource::WorkGeneration => {
                    crate::evidence::validate_digest(host.work_generation)?;
                    Value::String(host.work_generation.into())
                }
                ContextSource::CorrectionRevision => {
                    crate::evidence::validate_digest(host.correction_revision)?;
                    Value::String(host.correction_revision.into())
                }
                ContextSource::RepositoryRevision => {
                    let revision = host.repository_revision.ok_or_else(|| {
                        Error::new(
                            "evaluation_fidelity",
                            "native fixture requires an actual repository revision",
                        )
                    })?;
                    crate::git::validate_oid(revision)?;
                    Value::String(revision.into())
                }
            };
            *expected.pointer_mut(&slot.pointer).ok_or_else(|| {
                Error::new(
                    "evaluation_fidelity",
                    "declared native context slot is missing",
                )
            })? = value;
        }
        Ok(expected)
    }
}
fn collect_slots(
    value: &Value,
    pointer: &str,
    depth: usize,
    slots: &mut BTreeMap<String, ContextSource>,
) -> Result<()> {
    if depth > 64 {
        return Err(Error::new(
            "bounded_input",
            "native fixture context is too deeply nested",
        ));
    }
    match value {
        Value::Object(object) => {
            if let Some(source) = object.get("$kegari") {
                if object.len() != 1 {
                    return Err(Error::invalid(
                        "native context placeholder has extra fields",
                    ));
                }
                let source = serde_json::from_value(source.clone())
                    .map_err(|_| Error::invalid("unknown native context placeholder source"))?;
                slots.insert(pointer.into(), source);
            } else {
                for (key, value) in object {
                    let escaped = key.replace('~', "~0").replace('/', "~1");
                    collect_slots(value, &format!("{pointer}/{escaped}"), depth + 1, slots)?;
                }
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_slots(value, &format!("{pointer}/{index}"), depth + 1, slots)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn declaration() -> (NativeContextBinding, Value) {
        let binding = serde_json::from_value(
            json!({"schema_version":1,"source_write_paths":["src/limit.py"],"slots":[
            {"pointer":"/operation/id","source":"operation_id"},
            {"pointer":"/phase","source":"phase"},
            {"pointer":"/profile","source":"profile"},
            {"pointer":"/scope/revision","source":"repository_revision"},
            {"pointer":"/workspace/id","source":"workspace_id"}]}),
        )
        .unwrap();
        let input = json!({"issue":{"description":"Declared real task"},"parent":null,"children":{},
            "workspace":{"id":{"$kegari":"workspace_id"},"name":"Declared synthetic workspace","repositories":["101"]},
            "operation":{"id":{"$kegari":"operation_id"}},"profile":{"$kegari":"profile"},"phase":{"$kegari":"phase"},
            "scope":{"revision":{"$kegari":"repository_revision"}},"dependencies":[],"corrections":[],
            "evidence":{"synthetic_providers":["linear","github"],"native_context":{"original_task":"fixed fixture authority input"}},"attempt":1,
            "timeline":[{"future":"not initial prompt input"}]});
        (binding, input)
    }
    fn profile() -> Profile {
        Profile {
            tool: "codex".into(),
            model: "deterministic-fixture-model".into(),
            reasoning: "high".into(),
            speed: crate::config::Speed::Standard,
        }
    }
    #[test]
    fn native_template_preserves_all_task_fields_and_only_declared_typed_slots() {
        let (binding, input) = declaration();
        let profile = profile();
        let host = HostContext {
            workspace: uuid::Uuid::new_v4(),
            operation: uuid::Uuid::new_v4(),
            profile: &profile,
            phase: "implementation",
            work_generation: "unused",
            correction_revision: "unused",
            repository_revision: Some("1234567890123456789012345678901234567890"),
        };
        let expected = binding.materialize(&input, &host).unwrap();
        assert!(expected.get("timeline").is_none());
        assert_eq!(expected["workspace"]["id"], json!(host.workspace));
        assert_eq!(expected["operation"]["id"], json!(host.operation));
        assert_eq!(expected["issue"], input["issue"]);
        assert_eq!(
            expected["evidence"]["native_context"],
            input["evidence"]["native_context"]
        );
        assert_eq!(expected["workspace"]["name"], input["workspace"]["name"]);
        assert_eq!(
            expected["scope"]["revision"],
            json!(host.repository_revision)
        );
        let mut fixture = PreparedFixture {
            root: "/synthetic".into(),
            repository: "/synthetic/repository".into(),
            scratch: "/synthetic/scratch".into(),
            codex_home: "/synthetic/codex-home".into(),
            grader_root: "/private-scorer".into(),
            before: BTreeMap::new(),
            input,
            prior: None,
            fixture_digest: bytes_digest(b"declared fixture"),
            preparation_digest: String::new(),
            context_binding: Some(binding),
            worker_placement: None,
            worker_manifest: None,
        };
        fixture.preparation_digest = fixture.preparation_digest().unwrap();
        verify_fixture_context(&expected, &fixture, &host).unwrap();
        let suite = SuiteManifest::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations/manifest.json"),
        )
        .unwrap();
        let variant = suite
            .variants
            .iter()
            .find(|v| v.id == "implementation")
            .unwrap();
        verify_declared_phase(variant, variant.role, host.phase).unwrap();
        let different_phase = HostContext {
            phase: "review_fix",
            ..host
        };
        let substituted = fixture
            .context_binding
            .as_ref()
            .unwrap()
            .materialize(&fixture.input, &different_phase)
            .unwrap();
        // A typed slot faithfully materializes the actual worker phase. That is
        // insufficient to prove it is the phase declared for this trial.
        verify_fixture_context(&substituted, &fixture, &different_phase).unwrap();
        assert_eq!(
            verify_declared_phase(variant, variant.role, different_phase.phase)
                .unwrap_err()
                .code,
            "evaluation_fidelity"
        );
        let host = HostContext {
            phase: "implementation",
            ..different_phase
        };
        for pointer in [
            "/issue/description",
            "/evidence/native_context/original_task",
            "/workspace/name",
            "/operation/id",
        ] {
            let mut changed = expected.clone();
            *changed.pointer_mut(pointer).unwrap() = json!("undeclared easier guidance");
            assert_eq!(
                verify_fixture_context(&changed, &fixture, &host)
                    .unwrap_err()
                    .code,
                "evaluation_fidelity"
            );
        }
        let mut extra = expected.clone();
        extra["workspace"]["answer_hint"] = json!("nested hint");
        assert_eq!(
            verify_fixture_context(&extra, &fixture, &host)
                .unwrap_err()
                .code,
            "evaluation_fidelity"
        );
        fixture
            .context_binding
            .as_mut()
            .unwrap()
            .source_write_paths
            .push("other.py".into());
        assert_eq!(
            verify_fixture_context(&expected, &fixture, &host)
                .unwrap_err()
                .code,
            "evaluation_fidelity"
        );
    }
    #[test]
    fn native_slots_cannot_be_added_inferred_duplicated_or_substituted() {
        let (binding, input) = declaration();
        for change in [
            "missing",
            "extra",
            "duplicate",
            "source",
            "decorated",
            "timeline",
            "path",
            "schema",
        ] {
            let mut binding = binding.clone();
            let mut input = input.clone();
            match change {
                "missing" => {
                    input["scope"]["revision"] = Value::Null;
                }
                "extra" => {
                    input["issue"]["undeclared"] = json!({"$kegari":"workspace_id"});
                }
                "duplicate" => {
                    binding.slots.push(binding.slots[0].clone());
                }
                "source" => {
                    input["scope"]["revision"] = json!({"$kegari":"operation_id"});
                }
                "decorated" => {
                    input["operation"]["id"]["hint"] = json!("shortcut");
                }
                "timeline" => {
                    binding.slots[0].pointer = "/timeline/0/future".into();
                }
                "path" => {
                    binding.source_write_paths = vec!["../outside".into()];
                }
                "schema" => {
                    binding.schema_version = 2;
                }
                _ => unreachable!(),
            }
            assert!(binding.validate(&input).is_err(), "{change}");
        }
        let profile = profile();
        let host = HostContext {
            workspace: uuid::Uuid::new_v4(),
            operation: uuid::Uuid::new_v4(),
            profile: &profile,
            phase: "implementation",
            work_generation: "unused",
            correction_revision: "unused",
            repository_revision: None,
        };
        assert_eq!(
            binding.materialize(&input, &host).unwrap_err().code,
            "evaluation_fidelity"
        );
    }
}
