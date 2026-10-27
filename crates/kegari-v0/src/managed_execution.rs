//! Host-owned managed foreground declarations bound to observed tool identities.
//! A declaration describes the intended boundary; it is not native qualification.
use super::managed_resources::{BoundManagedResource, ResourceBudget};
use super::{ExecutionRole, IsolationPlan, PathGrant, canonical_target, overlap, safe_absolute};
use crate::config::{ManagedExecution, ManagedGeneratedExecutables, ManagedLifecycle};
use crate::digest::canonical_digest;
use crate::error::{Error, Result};
use crate::platform::BinaryIdentity;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundManagedExecution {
    schema_version: u32,
    policy: ManagedExecution,
    policy_digest: String,
    tools: Vec<BinaryIdentity>,
    read_roots: Vec<PathGrant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    resources: Vec<BoundManagedResource>,
}

impl BoundManagedExecution {
    pub fn resolve(policy: &ManagedExecution) -> Result<Self> {
        Self::resolve_with_resources(policy, None)
    }
    fn resolve_with_resources(
        policy: &ManagedExecution,
        retained: Option<&[BoundManagedResource]>,
    ) -> Result<Self> {
        policy.validate()?;
        let mut budget = ResourceBudget::new();
        let mut resources = Vec::new();
        let mut tools = Vec::new();
        let mut paths = BTreeSet::new();
        for tool in &policy.executables {
            let path = PathBuf::from(&tool.path);
            safe_absolute(&path)?;
            let path = path
                .canonicalize()
                .map_err(|error| Error::io("resolve managed executable", error))?;
            if !paths.insert(path.clone()) {
                return Err(Error::invalid("managed executable aliases another entry"));
            }
            let identity = BinaryIdentity::inspect(&path, false)?;
            if identity.digest != tool.digest {
                return Err(Error::new(
                    "identity_mismatch",
                    "managed executable differs from its configured digest",
                ));
            }
            tools.push(identity);
        }
        let mut read_roots = Vec::new();
        let mut paths = BTreeSet::new();
        for input in &policy.read_roots {
            let configured = PathBuf::from(&input.path);
            safe_absolute(&configured)?;
            let path = configured
                .canonicalize()
                .map_err(|error| Error::io("resolve managed tool read grant", error))?;
            if path.parent().is_none()
                || input.directory != path.is_dir()
                || (!input.directory && !path.is_file())
                || !paths.insert(path.clone())
            {
                return Err(Error::invalid(
                    "managed tool read grant is invalid or repeated",
                ));
            }
            read_roots.push(PathGrant {
                path,
                directory: input.directory,
            });
            if input.content.is_some() {
                let resource = if let Some(retained) = retained {
                    let bound = retained.get(resources.len()).ok_or_else(|| {
                        Error::new("identity_mismatch", "managed resource binding is missing")
                    })?;
                    bound.validate_binding_with_budget(input, &mut budget)?;
                    bound.clone()
                } else {
                    BoundManagedResource::capture(input, &mut budget)?
                };
                resources.push(resource);
            }
        }
        if retained.is_some_and(|retained| retained.len() != resources.len()) {
            return Err(Error::new(
                "identity_mismatch",
                "managed resource bindings differ",
            ));
        }
        Ok(Self {
            schema_version: if !resources.is_empty() {
                3
            } else {
                match policy.lifecycle {
                    ManagedLifecycle::ForegroundV1 => 1,
                    ManagedLifecycle::CheckBundleV1 => 2,
                }
            },
            policy: policy.clone(),
            policy_digest: canonical_digest(1, policy)?,
            tools,
            read_roots,
            resources,
        })
    }
    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }
    pub fn supports_check_bundles(&self) -> bool {
        matches!(self.schema_version, 2 | 3)
            && self.policy.lifecycle == ManagedLifecycle::CheckBundleV1
    }
    pub fn tools(&self) -> &[BinaryIdentity] {
        &self.tools
    }
    pub fn executables(&self) -> Vec<PathBuf> {
        self.tools.iter().map(|tool| tool.path.clone()).collect()
    }
    pub fn read_roots(&self) -> &[PathGrant] {
        &self.read_roots
    }
    pub(super) fn read_grants(&self) -> Vec<PathGrant> {
        let mut grants = self
            .read_roots
            .iter()
            .filter(|grant| {
                !self
                    .resources
                    .iter()
                    .any(|resource| resource.root() == grant.path)
            })
            .cloned()
            .collect::<Vec<_>>();
        grants.extend(
            self.resources
                .iter()
                .flat_map(|resource| resource.read_grants()),
        );
        grants
    }
    pub(super) fn executable_mappings(&self) -> impl Iterator<Item = &std::path::PathBuf> {
        self.resources
            .iter()
            .flat_map(|resource| resource.executable_mappings())
    }
    pub fn revalidate(&self) -> Result<()> {
        self.revalidate_binding()?;
        let mut budget = ResourceBudget::new();
        for resource in &self.resources {
            resource.revalidate(&mut budget)?;
        }
        Ok(())
    }
    /// Verify the retained content manifest and current path/tool bindings without
    /// re-reading SDK/library bytes in a serialized admission callback. Full
    /// content checks are explicit at preparation/cleanup; qualification stays
    /// closed until the complete prepared workload contract is implemented.
    pub(crate) fn revalidate_binding(&self) -> Result<()> {
        if !matches!(self.schema_version, 1..=3)
            || Self::resolve_with_resources(&self.policy, Some(&self.resources))? != *self
        {
            return Err(Error::new(
                "identity_mismatch",
                "managed inventory binding changed",
            ));
        }
        Ok(())
    }
    pub(super) fn generated_root(&self, plan: &IsolationPlan) -> Result<Option<PathBuf>> {
        match self.policy.generated_executables {
            None => Ok(None),
            Some(ManagedGeneratedExecutables::ScratchBuildV1) => {
                let root = plan.scratch.join("build");
                match std::fs::symlink_metadata(&root) {
                    Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                        return Err(Error::new(
                            "scope_violation",
                            "managed build root is linked or is not a directory",
                        ));
                    }
                    Ok(_) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
                    Err(error) => return Err(Error::io("inspect managed build root", error)),
                }
                if canonical_target(&root)? != root {
                    return Err(Error::new(
                        "scope_violation",
                        "managed build root was redirected",
                    ));
                }
                Ok(Some(root))
            }
        }
    }
    pub(super) fn validate_plan(&self, plan: &IsolationPlan) -> Result<()> {
        self.revalidate_binding()?;
        if matches!(
            plan.role,
            ExecutionRole::Coordination | ExecutionRole::IndependentReview
        ) {
            return Err(Error::new(
                "scope_violation",
                "this role cannot select managed execution",
            ));
        }
        let declared: BTreeSet<_> = self.tools.iter().map(|tool| &tool.path).collect();
        let admitted: BTreeSet<_> = plan.executables.iter().collect();
        if declared != admitted || declared.len() != plan.executables.len() {
            return Err(Error::new(
                "scope_violation",
                "executable grants differ from the managed inventory",
            ));
        }
        for tool in &self.tools {
            if overlap(&tool.path, &plan.repository)
                || overlap(&tool.path, &plan.scratch)
                || plan
                    .source_writes
                    .iter()
                    .chain(&plan.resource_writes)
                    .any(|grant| overlap(&tool.path, &grant.path))
            {
                return Err(Error::new(
                    "scope_violation",
                    "pinned tools overlap managed writable code or data",
                ));
            }
        }
        for grant in &self.read_roots {
            if plan
                .forbidden_roots
                .iter()
                .any(|path| overlap(path, &grant.path))
                || overlap(&grant.path, &plan.repository)
                || overlap(&grant.path, &plan.scratch)
                || plan
                    .source_writes
                    .iter()
                    .chain(&plan.resource_writes)
                    .any(|write| overlap(&grant.path, &write.path))
            {
                return Err(Error::new(
                    "scope_violation",
                    "managed tool read grant overlaps protected context or mutable execution data",
                ));
            }
        }
        for resource in &self.resources {
            if plan
                .read_roots
                .iter()
                .any(|grant| overlap(resource.root(), &grant.path))
            {
                return Err(Error::new(
                    "scope_violation",
                    "ordinary read grants overlap a closed managed content resource",
                ));
            }
        }
        self.generated_root(plan)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ManagedExecutable, ManagedLifecycle, ManagedReadRoot};
    use crate::digest::bytes_digest;
    use crate::isolation::repository_command_prerequisite;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        _temp: tempfile::TempDir,
        policy: ManagedExecution,
        plan: IsolationPlan,
        tool: PathBuf,
        sdk: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::TempDir::new().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let execution = root.join("runs");
            let repository = execution.join("repository");
            let scratch = execution.join("scratch");
            let sdk = root.join("sdk");
            let auth = root.join("auth");
            for path in [&repository, &scratch, &sdk, &auth] {
                fs::create_dir_all(path).unwrap();
            }
            let tool = root.join("tool");
            let bytes = b"#!/bin/sh\nexit 0\n";
            fs::write(&tool, bytes).unwrap();
            fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
            let policy = ManagedExecution {
                lifecycle: ManagedLifecycle::ForegroundV1,
                executables: vec![ManagedExecutable {
                    path: tool.to_str().unwrap().into(),
                    digest: bytes_digest(bytes),
                }],
                read_roots: vec![ManagedReadRoot {
                    path: sdk.to_str().unwrap().into(),
                    directory: true,
                    content: None,
                }],
                generated_executables: None,
            };
            let managed = BoundManagedExecution::resolve(&policy).unwrap();
            let plan = IsolationPlan {
                schema_version: 1,
                role: ExecutionRole::Child,
                execution_root: execution,
                cwd: repository.clone(),
                repository,
                scratch,
                read_roots: vec![],
                source_writes: vec![],
                resource_writes: vec![],
                forbidden_roots: vec![auth],
                executables: vec![tool.clone()],
                managed_execution: Some(managed),
            };
            Self {
                _temp: temp,
                policy,
                plan,
                tool,
                sdk,
            }
        }
    }
    #[test]
    fn unpublished_schema_four_is_not_reinterpreted_as_a_supported_binding() {
        let mut f = Fixture::new();
        f.policy.lifecycle = ManagedLifecycle::CheckBundleV1;
        let original = BoundManagedExecution::resolve(&f.policy).unwrap();
        assert!(original.supports_check_bundles());
        let mut stored = serde_json::to_value(original).unwrap();
        stored["schema_version"] = 4.into();
        let decoded: BoundManagedExecution = serde_json::from_value(stored).unwrap();
        assert!(!decoded.supports_check_bundles());
        assert_eq!(
            decoded.revalidate_binding().unwrap_err().code,
            "identity_mismatch"
        );
        assert_eq!(decoded.revalidate().unwrap_err().code, "identity_mismatch");
    }
    #[test]
    fn generic_command_rendering_preserves_argv_and_only_host_environment() {
        let mut f = Fixture::new();
        f.policy.generated_executables = Some(ManagedGeneratedExecutables::ScratchBuildV1);
        f.plan.managed_execution = Some(BoundManagedExecution::resolve(&f.policy).unwrap());
        let isolation = f.plan.validate().unwrap();
        let argv = vec![
            f.tool.to_string_lossy().into_owned(),
            "repository-selected-command".into(),
            "--project-option=unchanged".into(),
        ];
        // Pure preparation: the inert fixture executable is never launched.
        let rendered = crate::isolation::render_command(&isolation, &argv).unwrap();
        assert_eq!(rendered.argv, argv);
        assert_eq!(rendered.executable, f.tool);
        assert_eq!(rendered.profile, isolation.seatbelt_profile().unwrap());
        assert_eq!(
            rendered.environment,
            std::collections::BTreeMap::from([
                ("PATH".into(), isolation.command_path().unwrap()),
                ("HOME".into(), f.plan.scratch.to_string_lossy().into_owned()),
                (
                    "TMPDIR".into(),
                    f.plan.scratch.to_string_lossy().into_owned()
                ),
                ("LANG".into(), "C.UTF-8".into()),
            ])
        );
        assert_eq!(
            repository_command_prerequisite(&isolation)
                .unwrap_err()
                .code,
            "managed_profile_unqualified"
        );
    }
    #[test]
    fn check_bundle_policy_is_explicit_and_cannot_be_schema_downgraded() {
        let f = Fixture::new();
        let foreground = BoundManagedExecution::resolve(&f.policy).unwrap();
        assert!(!foreground.supports_check_bundles());
        assert_eq!(
            serde_json::to_value(&foreground).unwrap()["schema_version"],
            1
        );
        let mut policy = f.policy.clone();
        policy.lifecycle = ManagedLifecycle::CheckBundleV1;
        let bundle = BoundManagedExecution::resolve(&policy).unwrap();
        assert!(bundle.supports_check_bundles());
        assert_ne!(bundle.policy_digest(), foreground.policy_digest());
        bundle.revalidate().unwrap();
        let mut record = serde_json::to_value(bundle).unwrap();
        assert_eq!(record["schema_version"], 2);
        record["schema_version"] = 1.into();
        let changed: BoundManagedExecution = serde_json::from_value(record).unwrap();
        assert!(changed.revalidate().is_err());
    }
    #[test]
    fn executable_mapping_excludes_arbitrary_read_and_source_roots() {
        let mut f = Fixture::new();
        let profile = f.plan.validate().unwrap().seatbelt_profile().unwrap();
        assert!(profile.contains(&format!(
            "(allow file-read* file-map-executable (literal \"{}\"))",
            f.tool.display()
        )));
        for path in [&f.sdk, &f.plan.repository, &f.plan.scratch] {
            assert!(!profile.lines().any(|line| {
                line.starts_with("(allow")
                    && line.contains("file-map-executable")
                    && line.contains(&format!("\"{}\"", path.display()))
            }));
        }
        assert!(
            profile.contains(
                "(allow file-read* file-test-existence file-map-executable (subpath \"/System/Cryptexes/OS\"))"
            )
        );
        assert!(profile.contains(&format!(
            "(deny file-read* file-test-existence file-write* file-map-executable process-exec (subpath \"{}\"))",
            f.plan.forbidden_roots[0].display()
        )));
        assert!(!profile.contains("(import"));
        assert!(!profile.contains("(allow network"));
        assert!(!profile.contains("(allow mach"));
        f.policy.generated_executables = Some(ManagedGeneratedExecutables::ScratchBuildV1);
        f.plan.managed_execution = Some(BoundManagedExecution::resolve(&f.policy).unwrap());
        let generated = f.plan.validate().unwrap().seatbelt_profile().unwrap();
        assert!(generated.contains(&format!(
            "(allow file-map-executable (subpath \"{}\"))",
            f.plan.scratch.join("build").display()
        )));
    }
    #[test]
    fn content_bound_resources_preserve_read_scope_and_separate_mappings() {
        let mut f = Fixture::new();
        let library = f.sdk.join("compiler.dylib");
        let header = f.sdk.join("header.h");
        fs::write(&library, b"library fixture; never executed").unwrap();
        fs::write(&header, b"header fixture").unwrap();
        f.policy.read_roots[0].content = Some(crate::config::ManagedResourceContent {
            digest: crate::isolation::managed_resource_digest(&f.sdk, true).unwrap(),
            executable_mappings: vec!["compiler.dylib".into()],
        });
        let bound = BoundManagedExecution::resolve(&f.policy).unwrap();
        assert_eq!(serde_json::to_value(&bound).unwrap()["schema_version"], 3);
        assert!(!bound.supports_check_bundles());
        f.plan.managed_execution = Some(bound);
        let isolation = f.plan.validate().unwrap();
        isolation.revalidate_resource_contents().unwrap();
        let profile = isolation.seatbelt_profile().unwrap();
        assert!(profile.contains(&format!(
            "(allow file-read* (subpath \"{}\"))",
            f.sdk.display()
        )));
        for path in [&f.sdk, &library, &header] {
            assert!(!profile.contains(&format!(
                "(allow process-exec (literal \"{}\"))",
                path.display()
            )));
        }
        assert!(profile.contains(&format!(
            "(allow file-map-executable (literal \"{}\"))",
            library.display()
        )));
        assert!(!profile.contains(&format!(
            "(allow file-map-executable (literal \"{}\"))",
            header.display()
        )));
        assert_eq!(
            repository_command_prerequisite(&isolation)
                .unwrap_err()
                .code,
            "managed_profile_unqualified"
        );
        fs::write(&library, b"substituted library fixture").unwrap();
        // Structural admission checks do not pretend to observe resource bytes.
        isolation.revalidate().unwrap();
        assert!(isolation.revalidate_resource_contents().is_err());
        assert_eq!(
            repository_command_prerequisite(&isolation)
                .unwrap_err()
                .code,
            "managed_profile_unqualified"
        );
    }
    #[test]
    fn content_resources_reject_broader_grants_and_preserve_bundle_capability() {
        let mut f = Fixture::new();
        fs::write(f.sdk.join("file"), b"resource fixture").unwrap();
        f.policy.lifecycle = ManagedLifecycle::CheckBundleV1;
        f.policy.read_roots[0].content = Some(crate::config::ManagedResourceContent {
            digest: crate::isolation::managed_resource_digest(&f.sdk, true).unwrap(),
            executable_mappings: vec![],
        });
        let bound = BoundManagedExecution::resolve(&f.policy).unwrap();
        assert!(bound.supports_check_bundles());
        f.plan.managed_execution = Some(bound.clone());
        f.plan.validate().unwrap();
        f.plan.read_roots.push(PathGrant {
            path: f.sdk.clone(),
            directory: true,
        });
        assert_eq!(f.plan.validate().unwrap_err().code, "scope_violation");
        let mut serialized = serde_json::to_value(&bound).unwrap();
        serialized["schema_version"] = 2.into();
        let forged: BoundManagedExecution = serde_json::from_value(serialized).unwrap();
        assert!(forged.revalidate_binding().is_err());
        let mut serialized = serde_json::to_value(&bound).unwrap();
        serialized["resources"] = serde_json::json!([]);
        let forged: BoundManagedExecution = serde_json::from_value(serialized).unwrap();
        assert!(forged.revalidate_binding().is_err());
    }
    #[test]
    fn loader_bootstrap_reads_only_fixed_grafts_and_literal_ancestors() {
        use std::collections::BTreeSet;
        let f = Fixture::new();
        let boundary = f.plan.validate().unwrap();
        let profile = boundary.seatbelt_profile().unwrap();
        let ancestors = [
            "/",
            "/System",
            "/System/Volumes",
            "/System/Volumes/Preboot",
            "/System/Volumes/Preboot/Cryptexes",
            "/System/Volumes/Preboot/Cryptexes/App",
            "/System/Volumes/Preboot/Cryptexes/Incoming",
            "/System/Cryptexes",
        ];
        let mut expected: BTreeSet<String> = ancestors
            .iter()
            .map(|path| format!("(allow file-read* file-test-existence (literal \"{path}\"))"))
            .collect();
        for path in [
            "/System/Volumes/Preboot/Cryptexes/App/System",
            "/System/Volumes/Preboot/Cryptexes/Incoming/OS",
            "/System/Volumes/Preboot/Cryptexes/OS",
            "/System/Volumes/Preboot/Cryptexes/Rosetta",
            "/System/Cryptexes/App",
            "/System/Cryptexes/OS",
            "/System/Cryptexes/Rosetta",
        ] {
            expected.insert(format!(
                "(allow file-read* file-test-existence file-map-executable (subpath \"{path}\"))"
            ));
        }
        let actual: BTreeSet<String> = profile
            .lines()
            .filter(|line| line.starts_with("(allow") && line.contains("file-test-existence"))
            .map(str::to_owned)
            .collect();
        assert_eq!(actual, expected);
        for ancestor in ancestors {
            assert!(!profile.contains(&format!("(subpath \"{ancestor}\")")));
            for line in profile.lines().filter(|line| {
                line.starts_with("(allow") && line.contains(&format!("(literal \"{ancestor}\")"))
            }) {
                assert!(!line.contains("file-write"));
                assert!(!line.contains("file-map-executable"));
                assert!(!line.contains("process-exec"));
            }
        }
        assert!(profile.starts_with("(version 1)\n(deny default)\n"));
        for forbidden in [
            "(import",
            "(allow network",
            "(allow mach",
            "(allow sysctl-write",
        ] {
            assert!(!profile.contains(forbidden));
        }
        assert!(profile.contains(&format!(
            "(deny file-read* file-test-existence file-write* file-map-executable process-exec (subpath \"{}\"))",
            f.plan.forbidden_roots[0].display()
        )));
        assert_eq!(
            repository_command_prerequisite(&boundary).unwrap_err().code,
            "managed_profile_unqualified"
        );
    }
    #[test]
    fn managed_inventory_rejects_changed_bytes_aliases_and_substituted_records() {
        let f = Fixture::new();
        let bound = f.plan.managed_execution.as_ref().unwrap();
        let mut bad = f.policy.clone();
        bad.executables[0].digest = bytes_digest(b"different");
        assert_eq!(
            BoundManagedExecution::resolve(&bad).unwrap_err().code,
            "identity_mismatch"
        );
        let alias = f.tool.with_file_name("alias");
        std::os::unix::fs::symlink(&f.tool, &alias).unwrap();
        let mut duplicate = f.policy.clone();
        duplicate.executables.push(ManagedExecutable {
            path: alias.to_str().unwrap().into(),
            digest: f.policy.executables[0].digest.clone(),
        });
        assert!(BoundManagedExecution::resolve(&duplicate).is_err());
        let mut record = serde_json::to_value(bound).unwrap();
        record["tools"][0]["digest"] = serde_json::json!(bytes_digest(b"substitution"));
        let substituted: BoundManagedExecution = serde_json::from_value(record).unwrap();
        assert_eq!(
            substituted.revalidate().unwrap_err().code,
            "identity_mismatch"
        );
        fs::write(&f.tool, b"#!/bin/sh\nexit 1\n").unwrap();
        assert_eq!(bound.revalidate().unwrap_err().code, "identity_mismatch");
    }
    #[test]
    fn tool_reads_stay_command_only_and_do_not_establish_native_qualification() {
        let f = Fixture::new();
        let boundary = f.plan.validate().unwrap();
        assert!(boundary.plan().read_roots.is_empty());
        assert!(
            boundary
                .seatbelt_profile()
                .unwrap()
                .contains(f.sdk.to_str().unwrap())
        );
        let error = repository_command_prerequisite(&boundary).unwrap_err();
        assert_eq!(error.code, "managed_profile_unqualified");
        assert_eq!(
            error.details["profile"],
            boundary
                .plan()
                .managed_execution
                .as_ref()
                .unwrap()
                .policy_digest()
        );
        let mut legacy = f.plan.clone();
        legacy.managed_execution = None;
        let bytes = serde_json::to_vec(&legacy).unwrap();
        assert!(
            !String::from_utf8(bytes.clone())
                .unwrap()
                .contains("managed_execution")
        );
        let roundtrip: IsolationPlan = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            legacy.validate().unwrap().digest(),
            roundtrip.validate().unwrap().digest()
        );
        assert_ne!(legacy.validate().unwrap().digest(), boundary.digest());
        assert_eq!(
            repository_command_prerequisite(&legacy.validate().unwrap())
                .unwrap_err()
                .code,
            "process_tree_unverified"
        );
    }
    #[test]
    fn managed_profile_cannot_add_tools_or_read_protected_or_writable_context() {
        let f = Fixture::new();
        for role in [
            ExecutionRole::Coordination,
            ExecutionRole::IndependentReview,
        ] {
            let mut plan = f.plan.clone();
            plan.role = role;
            assert_eq!(plan.validate().unwrap_err().code, "scope_violation");
        }
        let mut plan = f.plan.clone();
        plan.executables.clear();
        assert_eq!(plan.validate().unwrap_err().code, "scope_violation");
        for path in [
            &f.plan.forbidden_roots[0],
            &f.plan.repository,
            &f.plan.scratch,
        ] {
            let mut policy = f.policy.clone();
            policy.read_roots[0].path = path.to_str().unwrap().into();
            let mut plan = f.plan.clone();
            plan.managed_execution = Some(BoundManagedExecution::resolve(&policy).unwrap());
            assert_eq!(plan.validate().unwrap_err().code, "scope_violation");
        }
        let mut plan = f.plan.clone();
        plan.resource_writes.push(PathGrant {
            path: f.sdk.clone(),
            directory: true,
        });
        plan.read_roots = plan.resource_writes.clone();
        assert_eq!(plan.validate().unwrap_err().code, "scope_violation");
    }
    #[test]
    fn generated_execution_is_explicit_and_bound_to_unredirected_scratch_build() {
        let mut f = Fixture::new();
        let before = f.plan.validate().unwrap().seatbelt_profile().unwrap();
        assert!(!before.contains("(allow process-exec (subpath"));
        f.policy.generated_executables = Some(ManagedGeneratedExecutables::ScratchBuildV1);
        f.plan.managed_execution = Some(BoundManagedExecution::resolve(&f.policy).unwrap());
        let profile = f.plan.validate().unwrap().seatbelt_profile().unwrap();
        assert!(profile.contains(&format!(
            "(allow process-exec (subpath \"{}\"))",
            f.plan.scratch.join("build").display()
        )));
        assert!(!profile.contains(&format!(
            "(allow process-exec (subpath \"{}\"))",
            f.plan.repository.display()
        )));
        std::os::unix::fs::symlink(&f.sdk, f.plan.scratch.join("build")).unwrap();
        assert_eq!(f.plan.validate().unwrap_err().code, "scope_violation");
        fs::remove_file(f.plan.scratch.join("build")).unwrap();
        std::os::unix::fs::symlink(f.sdk.join("not-present"), f.plan.scratch.join("build"))
            .unwrap();
        assert_eq!(f.plan.validate().unwrap_err().code, "scope_violation");
    }
}
