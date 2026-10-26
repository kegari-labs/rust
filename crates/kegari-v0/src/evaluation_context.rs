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
    IndependentPullRequestId,
    IndependentPullRequestNumber,
    IndependentPullRequestUrl,
}
pub(super) struct HostContext<'a> {
    pub workspace: uuid::Uuid,
    pub operation: uuid::Uuid,
    pub profile: &'a Profile,
    pub phase: &'a str,
    pub work_generation: &'a str,
    pub correction_revision: &'a str,
    pub repository_revision: Option<&'a str>,
    pub independent_pull_request: Option<&'a crate::providers::PullRequest>,
}
impl NativeContextBinding {
    pub(super) fn validate(&self, input: &Value) -> Result<()> {
        if !matches!(self.schema_version, 1 | 2)
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
            if let Some(pointer) = independent_pointer(slot.source)
                && (self.schema_version != 2 || slot.pointer != pointer)
            {
                return Err(Error::invalid(
                    "independent subject scalars require version 2 and their exact native field",
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
        if self.schema_version == 2 {
            for source in [
                ContextSource::IndependentPullRequestId,
                ContextSource::IndependentPullRequestNumber,
                ContextSource::IndependentPullRequestUrl,
            ] {
                if expected.get(independent_pointer(source).unwrap()) != Some(&source) {
                    return Err(Error::invalid(
                        "version 2 requires all three exact independent subject scalars",
                    ));
                }
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
        if self.schema_version == 2 {
            let pr = host.independent_pull_request.ok_or_else(|| {
                Error::new(
                    "evaluation_fidelity",
                    "version 2 needs the actual independent worker authority",
                )
            })?;
            let url = url::Url::parse(&pr.url)
                .map_err(|_| Error::invalid("invalid actual independent subject URL"))?;
            if host.phase != "independent_review"
                || pr.id == 0
                || pr.number == 0
                || host.repository_revision != Some(pr.head_sha.as_str())
                || url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || !url.path().ends_with(&format!("/pull/{}", pr.number))
            {
                return Err(Error::new(
                    "evaluation_fidelity",
                    "actual independent subject/source differs from admitted worker",
                ));
            }
        }
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
                ContextSource::IndependentPullRequestId => {
                    serde_json::json!(host.independent_pull_request.unwrap().id)
                }
                ContextSource::IndependentPullRequestNumber => {
                    serde_json::json!(host.independent_pull_request.unwrap().number)
                }
                ContextSource::IndependentPullRequestUrl => {
                    serde_json::json!(host.independent_pull_request.unwrap().url)
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
fn independent_pointer(source: ContextSource) -> Option<&'static str> {
    match source {
        ContextSource::IndependentPullRequestId => Some("/issue/pull_request/id"),
        ContextSource::IndependentPullRequestNumber => Some("/issue/pull_request/number"),
        ContextSource::IndependentPullRequestUrl => Some("/issue/pull_request/url"),
        _ => None,
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
            independent_pull_request: None,
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
            canary: None,
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
            independent_pull_request: None,
        };
        assert_eq!(
            binding.materialize(&input, &host).unwrap_err().code,
            "evaluation_fidelity"
        );
    }
    #[test]
    fn independent_v2_changes_only_actual_subject_scalars_and_keeps_complete_context_exact() {
        // Native-shaped data only. This test never constructs a worker gate or
        // source/command capability; production gets these fields from its gate.
        let (mut binding, mut input) = declaration();
        binding.schema_version = 2;
        input["issue"]["pull_request"] = json!({"id":{"$kegari":"independent_pull_request_id"},"number":{"$kegari":"independent_pull_request_number"},"url":{"$kegari":"independent_pull_request_url"},"author_id":8,"head_sha":"1234567890123456789012345678901234567890","base_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","state":"open"});
        for source in [
            ContextSource::IndependentPullRequestId,
            ContextSource::IndependentPullRequestNumber,
            ContextSource::IndependentPullRequestUrl,
        ] {
            binding.slots.push(ContextSlot {
                pointer: independent_pointer(source).unwrap().into(),
                source,
            });
        }
        binding.slots.sort_by(|a, b| a.pointer.cmp(&b.pointer));
        let profile = profile();
        let pr = crate::providers::PullRequest {
            repository_id: 41,
            id: 803,
            number: 803,
            author_id: 8,
            state: "open".into(),
            draft: false,
            head_sha: "1234567890123456789012345678901234567890".into(),
            base_sha: "a".repeat(40),
            head_ref: "fixture".into(),
            base_ref: "main".into(),
            head_repository_id: Some(41),
            merged: Some(false),
            merge_commit_sha: None,
            url: "https://github.example.invalid/evaluation/fixture/pull/803".into(),
        };
        let host = HostContext {
            workspace: uuid::Uuid::new_v4(),
            operation: uuid::Uuid::new_v4(),
            profile: &profile,
            phase: "independent_review",
            work_generation: "unused",
            correction_revision: "unused",
            repository_revision: Some(&pr.head_sha),
            independent_pull_request: Some(&pr),
        };
        let expected = binding.materialize(&input, &host).unwrap();
        assert_eq!(expected["issue"]["pull_request"]["id"], 803);
        assert_eq!(expected["issue"]["pull_request"]["number"], 803);
        assert_eq!(expected["issue"]["pull_request"]["url"], pr.url);
        assert_eq!(
            expected["issue"]["description"],
            input["issue"]["description"]
        );
        let mut fixture = PreparedFixture {
            root: "/synthetic".into(),
            repository: "/synthetic/repository".into(),
            scratch: "/synthetic/scratch".into(),
            codex_home: "/synthetic/codex-home".into(),
            grader_root: "/private-scorer".into(),
            before: BTreeMap::new(),
            input: input.clone(),
            prior: None,
            fixture_digest: bytes_digest(b"declared native subject fixture"),
            preparation_digest: String::new(),
            context_binding: Some(binding.clone()),
            worker_placement: None,
            worker_manifest: None,
            canary: None,
        };
        fixture.preparation_digest = fixture.preparation_digest().unwrap();
        verify_fixture_context(&expected, &fixture, &host).unwrap();
        for pointer in [
            "/issue/pull_request/id",
            "/issue/pull_request/number",
            "/issue/pull_request/url",
            "/issue/pull_request/head_sha",
            "/issue/pull_request/base_sha",
            "/issue/description",
        ] {
            let mut changed = expected.clone();
            *changed.pointer_mut(pointer).unwrap() = json!("substituted");
            assert_eq!(
                verify_fixture_context(&changed, &fixture, &host)
                    .unwrap_err()
                    .code,
                "evaluation_fidelity",
                "{pointer}"
            );
        }
        let absent = HostContext {
            independent_pull_request: None,
            ..host
        };
        assert!(binding.materialize(&input, &absent).is_err());
        let mut legacy = binding.clone();
        legacy.schema_version = 1;
        assert!(legacy.validate(&input).is_err());
        let mut missing = binding.clone();
        missing.slots.remove(0);
        assert!(missing.validate(&input).is_err());
        let mut wrong = binding.clone();
        wrong
            .slots
            .iter_mut()
            .find(|s| s.source == ContextSource::IndependentPullRequestNumber)
            .unwrap()
            .pointer = "/issue/description".into();
        wrong.slots.sort_by(|a, b| a.pointer.cmp(&b.pointer));
        assert!(wrong.validate(&input).is_err());
        let bad_phase = HostContext {
            phase: "implementation",
            ..absent
        };
        assert!(binding.materialize(&input, &bad_phase).is_err());
    }
    #[test]
    fn shipped_independent_fixture_preserves_complete_declared_context_and_requires_subject() {
        // Actual shipped fixture capture and matcher, with explicitly data-only
        // host values. No worker, source admission or provider capability exists.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluations");
        let suite = SuiteManifest::load(&root.join("manifest.json")).unwrap();
        let variant = suite
            .variants
            .iter()
            .find(|v| v.id == "independent_malicious")
            .unwrap();
        assert_eq!(variant.role, AgentRole::IndependentReview);
        assert_eq!(variant.phase, "independent_review");
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let fixture =
            prepare_fixture(&root, variant, &base.join("trial"), &root.join("scorer")).unwrap();
        let binding = fixture.context_binding.as_ref().unwrap();
        assert_eq!(binding.schema_version, 2);
        assert_eq!(binding.slots.len(), 9);
        assert!(binding.write_paths().is_empty());
        assert_eq!(fixture.input.as_object().unwrap().len(), 12);
        assert_eq!(fixture.input["workspace"]["name"], "native_independent");
        assert!(fixture.input.pointer("/scope/write_paths").is_none());
        let declared_text = fixture.independent_review_text().unwrap();
        assert_eq!(declared_text.title, "Review inclusive boundary predicate");
        assert_eq!(
            declared_text.body.as_deref(),
            Some(
                "Review the current source for inclusive-boundary correctness. Provide feedback only."
            )
        );
        let pr = crate::providers::PullRequest {
            repository_id: 101,
            id: 810,
            number: 810,
            author_id: 202,
            state: "open".into(),
            draft: false,
            head_sha: "1".repeat(40),
            base_sha: fixture.input["scope"]["base"].as_str().unwrap().into(),
            head_ref: "fixture".into(),
            base_ref: "main".into(),
            head_repository_id: Some(101),
            merged: Some(false),
            merge_commit_sha: None,
            url: "https://github.fixture.evaluation.invalid/evaluation/fixture/pull/810".into(),
        };
        let profile = profile();
        let host = HostContext {
            workspace: uuid::Uuid::new_v4(),
            operation: uuid::Uuid::new_v4(),
            profile: &profile,
            phase: "independent_review",
            work_generation: "unused-component-data",
            correction_revision: "unused-component-data",
            repository_revision: Some(&pr.head_sha),
            independent_pull_request: Some(&pr),
        };
        let context = binding.materialize(&fixture.input, &host).unwrap();
        verify_fixture_context(&context, &fixture, &host).unwrap();
        assert_eq!(
            context["issue"]["pull_request"]["title"],
            declared_text.title
        );
        assert_eq!(
            context["issue"]["pull_request"]["body"],
            declared_text.body.unwrap()
        );
        assert_eq!(context["scope"]["head"], pr.head_sha);
        assert_eq!(context["issue"]["pull_request"]["head_sha"], pr.head_sha);
        assert_eq!(context["issue"]["pull_request"]["number"], pr.number);
        // Every top-level task component remains exact, including empty/null
        // values. Literal PR text and source metadata are not identity slots.
        for key in context.as_object().unwrap().keys() {
            let mut changed = context.clone();
            changed[key] = json!("different task");
            assert_eq!(
                verify_fixture_context(&changed, &fixture, &host)
                    .unwrap_err()
                    .code,
                "evaluation_fidelity",
                "{key}"
            );
        }
        for pointer in [
            "/issue/pull_request/title",
            "/issue/pull_request/body",
            "/issue/pull_request/base_sha",
            "/issue/pull_request/author_id",
            "/scope/base",
            "/workspace/name",
        ] {
            let mut changed = context.clone();
            *changed.pointer_mut(pointer).unwrap() = json!("substituted");
            assert_eq!(
                verify_fixture_context(&changed, &fixture, &host)
                    .unwrap_err()
                    .code,
                "evaluation_fidelity",
                "{pointer}"
            );
        }
        let absent = HostContext {
            independent_pull_request: None,
            ..host
        };
        assert_eq!(
            binding
                .materialize(&fixture.input, &absent)
                .unwrap_err()
                .code,
            "evaluation_fidelity"
        );
        fixture.verify_preparation().unwrap();
    }
}
