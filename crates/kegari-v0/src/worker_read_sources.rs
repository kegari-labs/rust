//! Named repository reads use original host resources and existing exact grants.
//! A selector is a repository identity, never a filesystem root supplied by a tool.
use super::*;

impl WorkerContract {
    pub(super) fn read_source_root(&self, repository_id: Option<&str>) -> Result<PathBuf> {
        let Some(id) = repository_id else {
            return Ok(self.isolation.plan().repository.clone());
        };
        if id.is_empty() || id.len() > 256 || id.contains('\0') {
            return Err(Error::new(
                "scope_violation",
                "read source requires one bounded host repository identity",
            ));
        }
        if self
            .repository
            .as_ref()
            .is_some_and(|repository| repository.id == id)
        {
            return Ok(self.isolation.plan().repository.clone());
        }
        let mut roots = self.read_resources.iter().filter_map(|(root, resource)| {
            matches!(resource, ReadResource::Repository { id: actual } if actual == id)
                .then_some(root)
        });
        let root = roots.next().ok_or_else(|| {
            Error::new(
                "scope_violation",
                "repository is not an original host-bound read source",
            )
        })?;
        if roots.next().is_some() {
            return Err(Error::new(
                "scope_violation",
                "repository read identity has ambiguous retained roots",
            ));
        }
        let plan = self.isolation.plan();
        let overlaps = |a: &Path, b: &Path| a.starts_with(b) || b.starts_with(a);
        if !plan
            .read_roots
            .iter()
            .any(|grant| grant.path == *root && grant.directory)
            || overlaps(root, &plan.repository)
            || overlaps(root, &plan.scratch)
            || plan
                .source_writes
                .iter()
                .chain(&plan.resource_writes)
                .any(|grant| overlaps(root, &grant.path))
            || plan
                .forbidden_roots
                .iter()
                .any(|forbidden| overlaps(root, forbidden))
            || kegari_execution::canonical_target(root)? != *root
        {
            return Err(Error::new(
                "scope_violation",
                "selected repository is not an exact immutable read grant",
            ));
        }
        Ok(root.clone())
    }
    pub(super) fn observed_source_root(
        &self,
        observed: &Path,
    ) -> Result<(PathBuf, Option<String>)> {
        if observed.starts_with(&self.isolation.plan().repository) {
            return Ok((self.isolation.plan().repository.clone(), None));
        }
        let mut selected = None;
        for (root, resource) in &self.read_resources {
            let ReadResource::Repository { id } = resource else {
                continue;
            };
            if observed.starts_with(root) {
                let original = self.read_source_root(Some(id))?;
                if original != *root || selected.is_some() {
                    return Err(Error::new(
                        "validation_read",
                        "observed source is ambiguous in retained repository grants",
                    ));
                }
                selected = Some((original, Some(id.clone())));
            }
        }
        selected.ok_or_else(|| {
            Error::new(
                "validation_read",
                "source observation is outside original repository identities",
            )
        })
    }
}
impl ScopedWorker {
    pub(super) fn read_sources(&self) -> Result<Value> {
        let mut repositories = BTreeMap::new();
        if let Some(primary) = &self.contract.repository {
            repositories.insert(
                primary.id.clone(),
                json!({"repository_id":primary.id,"primary":true}),
            );
        }
        for resource in self.contract.read_resources.values() {
            if let ReadResource::Repository { id } = resource {
                // Listing uses the same selector checks as reads; a conflicting
                // host resource is not silently advertised as usable authority.
                self.contract.read_source_root(Some(id))?;
                repositories
                    .entry(id.clone())
                    .or_insert_with(|| json!({"repository_id":id,"primary":false}));
            }
        }
        bounded_value(
            json!({"schema_version":1,"repositories":repositories.into_values().collect::<Vec<_>>()}),
        )
    }
}
