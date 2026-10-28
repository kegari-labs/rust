//! Host-produced generic command domains. No target-tool catalog or operator
//! environment manifest is accepted. A bound domain is not native qualification.
use super::managed_resources::{BoundManagedResource, ResourceBudget};
use super::{
    ExecutionRole, IsolationPlan, PathGrant, canonical_target, contained_path, overlap,
    safe_absolute,
};
use kegari_digest::canonical_digest;
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

const SCHEMA: u32 = 5;
const MAX_GRANTS: usize = 256;
const MAX_PATH_BYTES: usize = 4096;
// These are OS backend resources, not target-toolchain inventories. PATH grants
// no access by itself. Only these fixed roots and actual admitted scopes grant
// read/exec/map access. Missing OS grafts are never synthesized as observations.
pub(super) const OS_READ_ROOTS: &[&str] = &[
    "/usr/lib",
    "/System/Library",
    "/System/Volumes/Preboot/Cryptexes/App/System",
    "/System/Volumes/Preboot/Cryptexes/Incoming/OS",
    "/System/Volumes/Preboot/Cryptexes/OS",
    "/System/Volumes/Preboot/Cryptexes/Rosetta",
    "/System/Cryptexes/App",
    "/System/Cryptexes/OS",
    "/System/Cryptexes/Rosetta",
];
const OS_COMMAND_ROOTS: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/usr/libexec"];
const OS_PATH: &[&str] = &["/usr/bin", "/bin", "/usr/sbin", "/sbin"];

fn changed() -> Error {
    Error::new("identity_mismatch", "generic execution domain changed")
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DomainContext {
    role: ExecutionRole,
    execution_root: PathBuf,
    repository: PathBuf,
    scratch: PathBuf,
    reads: Vec<PathGrant>,
    writable_resources: Vec<PathGrant>,
    forbidden: Vec<PathBuf>,
}
impl DomainContext {
    fn from_plan(plan: &IsolationPlan) -> Self {
        Self {
            role: plan.role,
            execution_root: plan.execution_root.clone(),
            repository: plan.repository.clone(),
            scratch: plan.scratch.clone(),
            reads: plan.read_roots.clone(),
            writable_resources: plan.resource_writes.clone(),
            forbidden: plan.forbidden_roots.clone(),
        }
    }
    fn validate(&self, budget: &mut ResourceBudget) -> Result<()> {
        budget.check()?;
        if matches!(
            self.role,
            ExecutionRole::Coordination | ExecutionRole::IndependentReview
        ) {
            return Err(Error::new(
                "scope_violation",
                "this role has no repository command domain",
            ));
        }
        if self.reads.len() > MAX_GRANTS
            || self.writable_resources.len() > MAX_GRANTS
            || self.forbidden.len() > MAX_GRANTS
        {
            return Err(Error::invalid("execution domain exceeds scope bounds"));
        }
        for path in std::iter::once(&self.execution_root)
            .chain([&self.repository, &self.scratch])
            .chain(self.reads.iter().map(|g| &g.path))
            .chain(self.writable_resources.iter().map(|g| &g.path))
            .chain(&self.forbidden)
        {
            budget.check()?;
            safe_absolute(path)?;
            if path.as_os_str().len() > MAX_PATH_BYTES || canonical_target(path)? != *path {
                return Err(Error::new(
                    "scope_violation",
                    "execution domain path is noncanonical or excessive",
                ));
            }
        }
        for path in [&self.repository, &self.scratch] {
            budget.check()?;
            if !path.is_dir()
                || contained_path(&self.execution_root, path)? != *path
                || self.forbidden.iter().any(|f| overlap(f, path))
            {
                return Err(changed());
            }
        }
        if overlap(&self.repository, &self.scratch) {
            return Err(changed());
        }
        for grant in &self.reads {
            budget.check()?;
            if grant.path.parent().is_none()
                || self.forbidden.iter().any(|f| overlap(f, &grant.path))
            {
                return Err(changed());
            }
            observe_root(grant)?;
        }
        for write in &self.writable_resources {
            budget.check()?;
            if !self.reads.contains(write)
                || overlap(&write.path, &self.repository)
                || overlap(&write.path, &self.scratch)
            {
                return Err(changed());
            }
        }
        for grant in &self.reads {
            budget.check()?;
            if (overlap(&grant.path, &self.repository) || overlap(&grant.path, &self.scratch))
                && !self.owned_scope_covers(grant)
            {
                return Err(Error::new(
                    "scope_violation",
                    "read-only resource parent contains an owned mutable repository or scratch directory; use disjoint read grants",
                ));
            }
            if !self.owned_scope_covers(grant)
                && self
                    .writable_resources
                    .iter()
                    .any(|write| overlap(&write.path, &grant.path))
                && !self
                    .writable_resources
                    .iter()
                    .any(|write| grant_covers(write, grant))
            {
                return Err(Error::new(
                    "scope_violation",
                    "read-only resource parent contains a writable subtree; use disjoint read and writable resource grants",
                ));
            }
        }
        budget.check()
    }
    fn owned_scope_covers(&self, grant: &PathGrant) -> bool {
        grant.path.starts_with(&self.repository) || grant.path.starts_with(&self.scratch)
    }
    fn immutable_reads(&self) -> Vec<&PathGrant> {
        self.reads
            .iter()
            .filter(|grant| {
                !self.owned_scope_covers(grant)
                    && !self
                        .writable_resources
                        .iter()
                        .any(|write| grant_covers(write, grant))
            })
            .collect()
    }
}
fn grant_covers(parent: &PathGrant, child: &PathGrant) -> bool {
    parent.path == child.path || (parent.directory && child.path.starts_with(&parent.path))
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootObservation {
    grant: PathGrant,
    device: u64,
    inode: u64,
}
fn observe_root(grant: &PathGrant) -> Result<RootObservation> {
    let metadata = std::fs::symlink_metadata(&grant.path)
        .map_err(|e| Error::io("observe generic execution root", e))?;
    if metadata.file_type().is_symlink()
        || (grant.directory && !metadata.is_dir())
        || (!grant.directory && !metadata.is_file())
    {
        return Err(changed());
    }
    Ok(RootObservation {
        grant: grant.clone(),
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}
fn domain_roots(
    context: &DomainContext,
    budget: &mut ResourceBudget,
) -> Result<Vec<RootObservation>> {
    let mut roots = Vec::new();
    let mut seen = BTreeSet::new();
    for path in OS_READ_ROOTS.iter().chain(OS_COMMAND_ROOTS) {
        budget.check()?;
        let path = match Path::new(path).canonicalize() {
            Ok(path) => path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(Error::io("observe fixed OS resource", e)),
        };
        if seen.insert(path.clone()) {
            roots.push(observe_root(&PathGrant {
                path,
                directory: true,
            })?);
        }
    }
    for grant in [
        PathGrant {
            path: context.repository.clone(),
            directory: true,
        },
        PathGrant {
            path: context.scratch.clone(),
            directory: true,
        },
    ]
    .iter()
    .chain(&context.reads)
    {
        budget.check()?;
        if seen.insert(grant.path.clone()) {
            roots.push(observe_root(grant)?);
        }
    }
    for root in &roots {
        budget.check()?;
        if context
            .forbidden
            .iter()
            .any(|path| overlap(path, &root.grant.path))
        {
            return Err(Error::new(
                "scope_violation",
                "execution domain overlaps a forbidden root",
            ));
        }
    }
    budget.check()?;
    Ok(roots)
}
fn search_path(context: &DomainContext) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    OS_PATH
        .iter()
        .map(PathBuf::from)
        .chain([context.repository.clone(), context.scratch.clone()])
        .chain(
            context
                .reads
                .iter()
                .filter(|grant| grant.directory)
                .map(|grant| grant.path.clone()),
        )
        // A colon cannot be encoded as one PATH component. Such a resource
        // remains available by explicit path; it never splits into extra roots.
        .filter(|path| path.to_str().is_some_and(|value| !value.contains(':')))
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundManagedExecution {
    schema_version: u32,
    policy_digest: String,
    context: DomainContext,
    roots: Vec<RootObservation>,
    search_path: Vec<PathBuf>,
    resources: Vec<BoundManagedResource>,
}
impl BoundManagedExecution {
    /// Exact-build compatibility is deliberate: a different backend build must
    /// not silently adopt a retained schema-5 domain. Such recovery needs a
    /// separately authorized/revalidated preparation, not a rewritten old digest.
    pub fn automatic_policy_digest(runtime_build: &str) -> Result<String> {
        canonical_digest(
            SCHEMA,
            &serde_json::json!({
                "backend":"macos_managed_command_domain_v1", "build":runtime_build,
                "os_read_roots":OS_READ_ROOTS, "os_command_roots":OS_COMMAND_ROOTS,
                "path":OS_PATH, "scope":"owned_repository_scratch_and_admitted_reads",
                "lifecycle":"owned_groups_and_registered_check_services",
                "environment":"private_home_tmp_and_no_inherited_environment_v1"
            }),
        )
    }
    /// Full resource observation is allowed only in preparation outside Control.
    pub fn automatic(plan: &IsolationPlan, runtime_build: &str) -> Result<Self> {
        let context = DomainContext::from_plan(plan);
        let mut budget = ResourceBudget::new();
        context.validate(&mut budget)?;
        let roots = domain_roots(&context, &mut budget)?;
        let resources = context
            .immutable_reads()
            .into_iter()
            .map(|grant| BoundManagedResource::capture(grant, &mut budget))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            schema_version: SCHEMA,
            policy_digest: Self::automatic_policy_digest(runtime_build)?,
            roots,
            search_path: search_path(&context),
            context,
            resources,
        })
    }
    pub fn policy_digest(&self) -> &str {
        &self.policy_digest
    }
    pub fn supports_check_bundles(&self) -> bool {
        self.schema_version == SCHEMA
    }
    /// Bounded structure and metadata checks only; no resource content reads.
    fn revalidate_binding(&self, runtime_build: &str) -> Result<()> {
        self.revalidate_binding_with_budget(&mut ResourceBudget::new(), runtime_build)
    }
    fn revalidate_binding_with_budget(
        &self,
        budget: &mut ResourceBudget,
        runtime_build: &str,
    ) -> Result<()> {
        budget.check()?;
        if self.schema_version != SCHEMA
            || self.policy_digest != Self::automatic_policy_digest(runtime_build)?
        {
            return Err(changed());
        }
        self.context.validate(budget)?;
        if self.roots != domain_roots(&self.context, budget)?
            || self.search_path != search_path(&self.context)
        {
            return Err(changed());
        }
        let reads = self.context.immutable_reads();
        if reads.len() != self.resources.len() {
            return Err(changed());
        }
        for (grant, resource) in reads.into_iter().zip(&self.resources) {
            resource.validate_binding_with_budget(grant, budget)?;
        }
        budget.check()
    }
    /// Original resource versions are checked at the dispatch boundary. This
    /// reads bounded metadata only; full contents stay outside Control.
    pub fn revalidate_resource_versions_until(
        &self,
        deadline: std::time::Instant,
        runtime_build: &str,
    ) -> Result<()> {
        let mut budget = ResourceBudget::until(deadline);
        budget.check()?;
        self.revalidate_binding_with_budget(&mut budget, runtime_build)?;
        for resource in &self.resources {
            resource.revalidate_versions(&mut budget)?;
        }
        budget.check()
    }
    pub fn revalidate(&self, runtime_build: &str) -> Result<()> {
        self.revalidate_binding(runtime_build)?;
        let mut budget = ResourceBudget::new();
        for resource in &self.resources {
            resource.revalidate(&mut budget)?;
        }
        Ok(())
    }
    /// Compare a read with the original immutable resource snapshot. Current
    /// resource bytes must still be revalidated separately before acceptance.
    /// Legacy source-read callers retain their existing local metadata bound;
    /// this wrapper does not represent an original worker inspection cutoff.
    pub fn verify_resource_file(
        &self,
        root: &Path,
        path: &Path,
        digest: &str,
        bytes: u64,
        runtime_build: &str,
    ) -> Result<()> {
        self.verify_resource_file_until(
            root,
            path,
            digest,
            bytes,
            std::time::Instant::now() + std::time::Duration::from_secs(30),
            runtime_build,
        )
    }
    /// New inspection operations must retain their caller's absolute cutoff.
    pub fn verify_resource_file_until(
        &self,
        root: &Path,
        path: &Path,
        digest: &str,
        bytes: u64,
        deadline: std::time::Instant,
        runtime_build: &str,
    ) -> Result<()> {
        let mut budget = ResourceBudget::until(deadline);
        self.revalidate_binding_with_budget(&mut budget, runtime_build)?;
        let mut matching = self
            .resources
            .iter()
            .filter(|resource| resource.root() == root);
        let resource = matching.next().ok_or_else(changed)?;
        if matching.next().is_some() {
            return Err(changed());
        }
        resource.verify_file_with_budget(path, digest, bytes, &budget)?;
        budget.check()
    }
    pub(super) fn validate_plan(&self, plan: &IsolationPlan, runtime_build: &str) -> Result<()> {
        self.validate_plan_with_budget(plan, &mut ResourceBudget::new(), runtime_build)
    }
    pub(super) fn validate_plan_with_budget(
        &self,
        plan: &IsolationPlan,
        budget: &mut ResourceBudget,
        runtime_build: &str,
    ) -> Result<()> {
        budget.check()?;
        if self.context != DomainContext::from_plan(plan) {
            return Err(changed());
        }
        self.revalidate_binding_with_budget(budget, runtime_build)
    }
    pub(super) fn execution_grants(&self) -> impl Iterator<Item = &PathGrant> {
        self.roots.iter().map(|root| &root.grant)
    }
    pub(super) fn command_path(&self) -> Result<String> {
        self.search_path
            .iter()
            .map(|path| {
                path.to_str()
                    .filter(|s| !s.contains(':'))
                    .ok_or_else(|| Error::invalid("command search path is not representable"))
            })
            .collect::<Result<Vec<_>>>()
            .map(|parts| parts.join(":"))
    }
    /// The recorded search order only. Returning it does not grant access to a
    /// path or inspect, enumerate or execute any program within it.
    pub(super) fn search_roots(&self) -> &[PathBuf] {
        &self.search_path
    }
    pub(super) fn resolve(&self, cwd: &Path, name: &str) -> Result<PathBuf> {
        if name.is_empty() || name.len() > MAX_PATH_BYTES || name.contains('\0') {
            return Err(Error::invalid("command executable name is invalid"));
        }
        let path = Path::new(name);
        if path.is_absolute() || name.contains('/') {
            return self.selected(if path.is_absolute() {
                path.to_owned()
            } else {
                cwd.join(path)
            });
        }
        for directory in &self.search_path {
            let candidate = directory.join(name);
            match std::fs::metadata(&candidate) {
                Ok(metadata) if metadata.is_file() && metadata.mode() & 0o111 != 0 => {
                    return self.selected(candidate);
                }
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(Error::io("inspect command search candidate", error)),
            }
        }
        Err(Error::new(
            "executable_unavailable",
            "command is absent from the operation's ordered search path",
        ))
    }
    fn selected(&self, path: PathBuf) -> Result<PathBuf> {
        let resolved = path
            .canonicalize()
            .map_err(|e| Error::io("resolve selected command", e))?;
        let metadata =
            std::fs::metadata(&resolved).map_err(|e| Error::io("inspect selected command", e))?;
        if !metadata.is_file()
            || metadata.mode() & 0o111 == 0
            || self
                .context
                .forbidden
                .iter()
                .any(|f| resolved.starts_with(f))
            || !self.roots.iter().any(|root| {
                if root.grant.directory {
                    resolved != root.grant.path && resolved.starts_with(&root.grant.path)
                } else {
                    resolved == root.grant.path
                }
            })
        {
            return Err(Error::new(
                "scope_violation",
                "selected command escapes the enforced execution domain",
            ));
        }
        // Normalize dot components but preserve the actual final alias spelling:
        // argv[0]-sensitive tools must not silently become their symlink target.
        let parent = path
            .parent()
            .ok_or_else(|| Error::invalid("command has no parent"))?
            .canonicalize()
            .map_err(|e| Error::io("resolve selected command parent", e))?;
        Ok(parent.join(
            path.file_name()
                .ok_or_else(|| Error::invalid("command has no filename"))?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TEST_BUILD;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    struct Fixture {
        _temp: tempfile::TempDir,
        plan: IsolationPlan,
        resource: PathBuf,
        outside: PathBuf,
    }
    impl Fixture {
        fn new(role: ExecutionRole) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().canonicalize().unwrap();
            let runs = root.join("runs");
            let repository = runs.join("repository");
            let scratch = runs.join("scratch");
            let resource = root.join("resource");
            let outside = root.join("outside");
            let protected = root.join("protected");
            for path in [&repository, &scratch, &resource, &outside, &protected] {
                fs::create_dir_all(path).unwrap();
            }
            let plan = IsolationPlan {
                schema_version: 1,
                role,
                execution_root: runs,
                repository: repository.clone(),
                cwd: repository,
                scratch,
                read_roots: vec![PathGrant {
                    path: resource.clone(),
                    directory: true,
                }],
                source_writes: vec![],
                resource_writes: vec![],
                forbidden_roots: vec![protected],
                managed_execution: None,
            };
            Self {
                _temp: temp,
                plan,
                resource,
                outside,
            }
        }
        fn bind(&mut self) -> super::super::ValidatedIsolation {
            self.plan.managed_execution =
                Some(BoundManagedExecution::automatic(&self.plan, TEST_BUILD).unwrap());
            self.plan.validate(TEST_BUILD).unwrap()
        }
        fn program(path: &Path, body: &[u8]) {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    #[test]
    fn unfamiliar_and_generated_commands_need_no_predefined_inventory() {
        let mut fixture = Fixture::new(ExecutionRole::Child);
        let isolation = fixture.bind();
        // Created only after the domain was captured: actual invocation observes
        // each executable; neither these names nor digests were configured.
        for path in [
            fixture.plan.repository.join("custom/deep/unfamiliar-tool"),
            fixture.plan.scratch.join("any-layout/generated"),
        ] {
            Fixture::program(&path, b"#!/bin/sh\nexit 0\n");
            assert_eq!(
                isolation
                    .resolve_executable(path.to_str().unwrap())
                    .unwrap(),
                path
            );
        }
        assert_eq!(
            isolation
                .resolve_executable("./custom/deep/unfamiliar-tool")
                .unwrap(),
            fixture.plan.repository.join("custom/deep/unfamiliar-tool")
        );
        assert_eq!(
            isolation
                .resolve_executable("not-created-command")
                .unwrap_err()
                .code,
            "executable_unavailable"
        );
        assert_eq!(
            super::super::repository_command_prerequisite(&isolation)
                .unwrap_err()
                .code,
            "managed_profile_unqualified"
        );
    }
    #[test]
    fn path_order_and_alias_spelling_are_preserved_without_ambient_expansion() {
        let mut fixture = Fixture::new(ExecutionRole::Child);
        let name = "kegari-fixture-unfamiliar-command";
        for root in [
            &fixture.plan.repository,
            &fixture.plan.scratch,
            &fixture.resource,
        ] {
            Fixture::program(&root.join(name), b"#!/bin/sh\nexit 0\n");
        }
        let target = fixture.plan.repository.join(name);
        let alias = fixture.plan.repository.join("unfamiliar-alias");
        symlink(name, &alias).unwrap();
        let isolation = fixture.bind();
        assert_eq!(isolation.resolve_executable(name).unwrap(), target);
        assert_eq!(
            isolation.resolve_executable("unfamiliar-alias").unwrap(),
            alias
        );
        assert_eq!(
            isolation.command_path().unwrap(),
            format!(
                "/usr/bin:/bin:/usr/sbin:/sbin:{}:{}:{}",
                fixture.plan.repository.display(),
                fixture.plan.scratch.display(),
                fixture.resource.display()
            )
        );
        let rendered = super::super::render_command(
            &isolation,
            &["unfamiliar-alias".into(), "literal argument".into()],
        )
        .unwrap();
        assert_eq!(rendered.argv, ["unfamiliar-alias", "literal argument"]);
        assert_eq!(rendered.executable, alias);
        assert_eq!(rendered.environment.len(), 4);
        assert_eq!(
            rendered.environment["HOME"],
            fixture.plan.scratch.to_str().unwrap()
        );
        #[cfg(target_os = "macos")]
        {
            let observed = super::super::observe_rendered_command(&isolation, &rendered)
                .unwrap()
                .launch_identity();
            assert_eq!(observed.target.path, target);
            let direct = super::super::command_launch_identity(
                &isolation,
                &[name.into(), "literal argument".into()],
            )
            .unwrap();
            assert_ne!(observed.argv_digest, direct.argv_digest);
        }
    }
    #[test]
    fn canonical_escape_and_unadmitted_external_programs_are_refused() {
        let mut fixture = Fixture::new(ExecutionRole::Child);
        let outside = fixture.outside.join("external");
        let protected = fixture.plan.forbidden_roots[0].join("secret-tool");
        Fixture::program(&outside, b"outside");
        Fixture::program(&protected, b"secret");
        symlink(&outside, fixture.plan.repository.join("escape")).unwrap();
        symlink(&protected, fixture.plan.scratch.join("protected-alias")).unwrap();
        let isolation = fixture.bind();
        for name in [
            outside,
            fixture.plan.repository.join("escape"),
            fixture.plan.scratch.join("protected-alias"),
        ] {
            assert_eq!(
                isolation
                    .resolve_executable(name.to_str().unwrap())
                    .unwrap_err()
                    .code,
                "scope_violation"
            );
        }
        assert!(
            !isolation
                .command_path()
                .unwrap()
                .contains(fixture.outside.to_str().unwrap())
        );
    }
    #[test]
    fn exact_file_resource_is_not_a_sibling_directory_execution_grant() {
        let mut fixture = Fixture::new(ExecutionRole::Validation);
        let allowed = fixture.resource.join("allowed");
        let sibling = fixture.resource.join("sibling");
        Fixture::program(&allowed, b"allowed");
        Fixture::program(&sibling, b"sibling");
        fixture.plan.read_roots = vec![PathGrant {
            path: allowed.clone(),
            directory: false,
        }];
        let isolation = fixture.bind();
        assert_eq!(
            isolation
                .resolve_executable(allowed.to_str().unwrap())
                .unwrap(),
            allowed
        );
        assert_eq!(
            isolation
                .resolve_executable(sibling.to_str().unwrap())
                .unwrap_err()
                .code,
            "scope_violation"
        );
        let profile = isolation.seatbelt_profile().unwrap();
        assert!(profile.contains(&format!(
            "file-map-executable process-exec (literal \"{}\")",
            allowed.display()
        )));
        assert!(!profile.contains(&format!(
            "process-exec (subpath \"{}\")",
            fixture.resource.display()
        )));
    }
    #[test]
    fn immutable_inputs_are_observed_and_mutable_test_resources_are_not_frozen() {
        let mut fixture = Fixture::new(ExecutionRole::Validation);
        fs::write(fixture.resource.join("input"), b"before").unwrap();
        let isolation = fixture.bind();
        fs::write(fixture.resource.join("input"), b"after").unwrap();
        isolation.revalidate().unwrap(); // Structural checks do not read input bytes.
        assert!(isolation.revalidate_resource_contents().is_err());
        fixture.plan.managed_execution = None;
        fixture.plan.resource_writes = fixture.plan.read_roots.clone();
        let isolation = fixture.bind();
        fs::write(fixture.resource.join("input"), b"legitimate test mutation").unwrap();
        isolation.revalidate_resource_contents().unwrap();
    }
    #[test]
    fn mixed_read_parent_and_writable_child_refuses_without_hiding_read_only_siblings() {
        let mut fixture = Fixture::new(ExecutionRole::Validation);
        let input = fixture.resource.join("input");
        let mutable = fixture.resource.join("mutable");
        fs::write(&input, b"original read-only input").unwrap();
        fs::create_dir(&mutable).unwrap();
        let write = PathGrant {
            path: mutable.clone(),
            directory: true,
        };
        fixture.plan.read_roots.push(write.clone());
        fixture.plan.resource_writes.push(write.clone());
        // These are genuine, internally consistent filesystem grants. The
        // automatic observer must reject their unsupported mixed partition,
        // rather than exempt the whole parent and lose its read-only sibling.
        fixture.plan.validate(TEST_BUILD).unwrap();
        let error = BoundManagedExecution::automatic(&fixture.plan, TEST_BUILD).unwrap_err();
        assert_eq!(error.code, "scope_violation");
        assert!(error.message.contains("read-only resource parent"));
        // A disjoint representation retains writable semantics and real input
        // freshness; standard repository/scratch reads remain valid as well.
        fixture.plan.read_roots = vec![
            PathGrant {
                path: input.clone(),
                directory: false,
            },
            write,
            PathGrant {
                path: fixture.plan.repository.clone(),
                directory: true,
            },
            PathGrant {
                path: fixture.plan.scratch.clone(),
                directory: true,
            },
        ];
        let isolation = fixture.bind();
        fs::write(mutable.join("output"), b"permitted test output").unwrap();
        fs::write(fixture.plan.scratch.join("output"), b"private output").unwrap();
        isolation.revalidate_resource_contents().unwrap();
        fs::write(&input, b"changed read-only input").unwrap();
        assert!(isolation.revalidate_resource_contents().is_err());
    }
    #[test]
    fn parent_of_owned_scopes_cannot_hide_an_immutable_sibling() {
        let mut fixture = Fixture::new(ExecutionRole::Child);
        let input = fixture.plan.execution_root.join("shared-input");
        let repository_subpath = fixture.plan.repository.join("source");
        let scratch_subpath = fixture.plan.scratch.join("output");
        fs::write(&input, b"read-only sibling").unwrap();
        fs::create_dir(&repository_subpath).unwrap();
        fs::create_dir(&scratch_subpath).unwrap();
        fixture.plan.source_writes = vec![PathGrant {
            path: repository_subpath.clone(),
            directory: true,
        }];
        fixture.plan.read_roots = vec![PathGrant {
            path: fixture.plan.execution_root.clone(),
            directory: true,
        }];
        // Reachable through the ordinary path checks: the read parent is an
        // existing canonical directory, distinct from every forbidden root.
        fixture.plan.validate(TEST_BUILD).unwrap();
        let error = BoundManagedExecution::automatic(&fixture.plan, TEST_BUILD).unwrap_err();
        assert_eq!(error.code, "scope_violation");
        assert!(
            error
                .message
                .contains("owned mutable repository or scratch")
        );
        fixture.plan.read_roots = vec![
            PathGrant {
                path: input.clone(),
                directory: false,
            },
            PathGrant {
                path: fixture.plan.repository.clone(),
                directory: true,
            },
            PathGrant {
                path: repository_subpath.clone(),
                directory: true,
            },
            PathGrant {
                path: fixture.plan.scratch.clone(),
                directory: true,
            },
            PathGrant {
                path: scratch_subpath.clone(),
                directory: true,
            },
        ];
        let isolation = fixture.bind();
        fs::write(
            repository_subpath.join("changed"),
            b"owned source observation",
        )
        .unwrap();
        fs::write(scratch_subpath.join("result"), b"owned output").unwrap();
        isolation.revalidate_resource_contents().unwrap();
        fs::write(&input, b"changed sibling").unwrap();
        assert!(isolation.revalidate_resource_contents().is_err());
    }
    #[test]
    fn retained_domain_rejects_root_replacement_and_forged_or_legacy_bindings() {
        let mut fixture = Fixture::new(ExecutionRole::Child);
        let isolation = fixture.bind();
        let bound = fixture.plan.managed_execution.as_ref().unwrap();
        let original = serde_json::to_value(bound).unwrap();
        let roundtrip: BoundManagedExecution = serde_json::from_value(original.clone()).unwrap();
        roundtrip.revalidate(TEST_BUILD).unwrap();
        for schema in 1..5 {
            let mut value = original.clone();
            value["schema_version"] = schema.into();
            let forged: BoundManagedExecution = serde_json::from_value(value).unwrap();
            assert!(forged.revalidate_binding(TEST_BUILD).is_err());
        }
        let mut value = original;
        value["search_path"] = serde_json::json!([fixture.outside]);
        let forged: BoundManagedExecution = serde_json::from_value(value).unwrap();
        assert!(forged.revalidate_binding(TEST_BUILD).is_err());
        fs::rename(
            &fixture.plan.repository,
            fixture.plan.repository.with_extension("old"),
        )
        .unwrap();
        fs::create_dir(&fixture.plan.repository).unwrap();
        assert!(isolation.revalidate().is_err());
    }
    #[test]
    fn role_boundaries_and_forbidden_roots_remain_strict() {
        for role in [
            ExecutionRole::Coordination,
            ExecutionRole::IndependentReview,
        ] {
            let fixture = Fixture::new(role);
            assert_eq!(
                BoundManagedExecution::automatic(&fixture.plan, TEST_BUILD)
                    .unwrap_err()
                    .code,
                "scope_violation"
            );
            assert!(
                fixture
                    .plan
                    .validate(TEST_BUILD)
                    .unwrap()
                    .resolve_executable("sh")
                    .is_err()
            );
        }
        for role in [
            ExecutionRole::Review,
            ExecutionRole::Validation,
            ExecutionRole::Child,
        ] {
            let mut fixture = Fixture::new(role);
            let isolation = fixture.bind();
            let profile = isolation.seatbelt_profile().unwrap();
            assert!(profile.contains(&format!(
                "(allow file-write* (subpath \"{}\"))",
                fixture.plan.scratch.display()
            )));
            assert!(!profile.contains(&format!(
                "(allow file-write* (subpath \"{}\"))",
                fixture.plan.repository.display()
            )));
            assert!(profile.contains("(deny default)"));
            assert!(profile.contains(
                "(deny file-read* file-test-existence file-write* file-map-executable process-exec"
            ));
            assert!(
                isolation
                    .plan()
                    .managed_execution
                    .as_ref()
                    .unwrap()
                    .supports_check_bundles()
            );
        }
    }
    #[test]
    fn colon_resource_path_does_not_create_additional_search_components() {
        let mut fixture = Fixture::new(ExecutionRole::Child);
        let resource = fixture.outside.join("literal:component");
        fs::create_dir(&resource).unwrap();
        let program = resource.join("tool");
        Fixture::program(&program, b"literal");
        fixture.plan.read_roots = vec![PathGrant {
            path: resource.clone(),
            directory: true,
        }];
        let isolation = fixture.bind();
        assert!(
            !isolation
                .command_path()
                .unwrap()
                .contains(resource.to_str().unwrap())
        );
        assert_eq!(
            isolation
                .resolve_executable(program.to_str().unwrap())
                .unwrap(),
            program
        );
    }
}
