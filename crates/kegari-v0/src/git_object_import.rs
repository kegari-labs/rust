//! Private quarantine operations; never available through host metadata scope.
use super::*;
use kegari_storage::PrivateDir;

impl Git {
    pub(crate) fn object_import_binary(&self) -> &kegari_platform::BinaryIdentity {
        &self.binary
    }
    pub(crate) fn import_verified_object_pack(
        &self,
        directory: &PrivateDir,
        bytes: &[u8],
        objects: &BTreeSet<String>,
        expected: &BTreeMap<String, String>,
    ) -> Result<()> {
        if self.host_scope.is_some() || (self.operation.is_none() && self.scope.is_none()) {
            return Err(Error::new(
                "scope_violation",
                "object import requires an admitted owned Git operation",
            ));
        }
        directory.revalidate()?;
        if fs::read_dir(directory.path())
            .map_err(|e| Error::io("inspect empty object quarantine", e))?
            .next()
            .is_some()
        {
            return Err(Error::new(
                "reconciliation_required",
                "object quarantine is not empty; it cannot be replayed",
            ));
        }
        let template = directory.child("empty-template")?;
        let repository = directory.child("repository.git")?;
        self.run(
            repository.path(),
            &[
                "init",
                "--bare",
                "--initial-branch=kegari-source",
                &format!("--template={}", template.path().display()),
            ],
            None,
        )?;
        self.run(
            repository.path(),
            &[
                "index-pack",
                "--stdin",
                "--strict",
                "--threads=1",
                &format!("--max-input-size={}", bytes.len()),
            ],
            Some(bytes),
        )?;
        self.verify_imported_objects(repository.path(), objects, expected)?;
        for commit in expected.keys() {
            self.run(
                repository.path(),
                &[
                    "update-ref",
                    &format!("refs/kegari/objects/{commit}"),
                    commit,
                    "0000000000000000000000000000000000000000",
                ],
                None,
            )?;
        }
        repository.revalidate()?;
        directory.revalidate()
    }
    pub(crate) fn verify_imported_objects(
        &self,
        root: &Path,
        objects: &BTreeSet<String>,
        expected: &BTreeMap<String, String>,
    ) -> Result<()> {
        if expected.is_empty() || expected.len() > 64 {
            return Err(Error::invalid("exact object revision inventory required"));
        }
        for (commit, tree) in expected {
            validate_oid(commit)?;
            validate_oid(tree)?;
            if self.exact_revision(root, commit)?.tree != *tree {
                return Err(Error::new(
                    "stale_result",
                    "imported Git tree differs from observed source",
                ));
            }
        }
        self.run(root, &["fsck", "--full", "--strict", "--no-reflogs"], None)?;
        let mut args = vec!["rev-list", "--objects", "--no-object-names"];
        args.extend(expected.keys().map(String::as_str));
        args.push("--");
        let actual: BTreeSet<String> = self.text(root, &args)?.lines().map(str::to_owned).collect();
        if &actual != objects {
            return Err(Error::new(
                "source_unavailable",
                "pack is not exactly the complete selected object closure",
            ));
        }
        Ok(())
    }
}

/// Only an authenticated, acknowledged immutable imported-source capability
/// reaches this constructor. It grants fixed metadata reads, never mutation.
pub(crate) fn retained_object_read_root(
    source: &crate::remote_objects::VerifiedObjectSource,
    config_revision: String,
) -> Result<VerifiedHostReadRoot> {
    source.revalidate_filesystem()?;
    host_read_root(
        source.repository().clone(),
        source.root(),
        config_revision,
        source.digest()?,
        Some(source.marker()),
    )
}
