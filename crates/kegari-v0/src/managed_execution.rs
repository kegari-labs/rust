//! Host-owned managed foreground declarations bound to observed tool identities.
//! A declaration describes the intended boundary; it is not native qualification.
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
}

impl BoundManagedExecution {
    pub fn resolve(policy: &ManagedExecution) -> Result<Self> {
        policy.validate()?;
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
        }
        Ok(Self {
            schema_version: match policy.lifecycle {
                ManagedLifecycle::ForegroundV1 => 1,
                ManagedLifecycle::CheckBundleV1 => 2,
            },
            policy: policy.clone(),
            policy_digest: canonical_digest(1, policy)?,
            tools,
            read_roots,
        })
    }
    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }
    pub fn supports_check_bundles(&self) -> bool {
        self.schema_version == 2 && self.policy.lifecycle == ManagedLifecycle::CheckBundleV1
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
    pub fn revalidate(&self) -> Result<()> {
        if !matches!(self.schema_version, 1 | 2) || Self::resolve(&self.policy)? != *self {
            return Err(Error::new(
                "identity_mismatch",
                "managed execution inventory changed after preparation",
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
        self.revalidate()?;
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
                "(allow file-read* file-map-executable (subpath \"/System/Cryptexes/OS\"))"
            )
        );
        assert!(profile.contains(&format!(
            "(deny file-read* file-write* file-map-executable process-exec (subpath \"{}\"))",
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
