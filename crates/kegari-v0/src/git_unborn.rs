//! Fresh owned workspaces for an actual repository with no reference baseline.
use super::*;

#[derive(Clone, Debug)]
pub(crate) struct VerifiedUnbornClone {
    identity: crate::execution::DirectoryIdentity,
    branch: String,
    paths: Vec<CloneDirectoryStamp>,
    initial_files: BTreeMap<String, (String, String)>,
}
impl VerifiedUnbornClone {
    pub(crate) fn identity(&self) -> &crate::execution::DirectoryIdentity {
        &self.identity
    }
    pub(crate) fn branch(&self) -> &str {
        &self.branch
    }
    pub(crate) fn revalidate(&self) -> Result<()> {
        for stamp in &self.paths {
            if clone_directory_stamp(&stamp.path)? != *stamp {
                return Err(Error::new(
                    "execution_ownership",
                    "unborn workspace directory was replaced",
                ));
            }
        }
        let git = self.identity.repository_path.join(".git");
        let head = git.join("HEAD");
        let metadata =
            fs::symlink_metadata(&head).map_err(|e| Error::io("inspect unborn HEAD", e))?;
        if !metadata.is_file()
            || metadata.len() > 4096
            || fs::read(&head).map_err(|e| Error::io("read unborn HEAD", e))?
                != format!("ref: refs/heads/{}\n", self.branch).as_bytes()
        {
            return Err(Error::new(
                "stale_revision",
                "unborn workspace HEAD changed",
            ));
        }
        // Codex receives read-only access to .git. No ref may silently become a
        // fabricated admitted baseline, even if an outside actor mutates it.
        if git.join("packed-refs").exists() || directory_has_file(&git.join("refs"))? {
            return Err(Error::new(
                "stale_revision",
                "unborn workspace acquired a reference; re-admission is required",
            ));
        }
        Ok(())
    }
}
fn directory_has_file(path: &Path) -> Result<bool> {
    for entry in fs::read_dir(path).map_err(|e| Error::io("inspect unborn refs", e))? {
        let entry = entry.map_err(|e| Error::io("inspect unborn ref", e))?;
        let kind = entry
            .file_type()
            .map_err(|e| Error::io("inspect unborn ref kind", e))?;
        if !kind.is_dir() || directory_has_file(&entry.path())? {
            return Ok(true);
        }
    }
    Ok(false)
}
impl Git {
    pub(crate) fn unborn_branch(&self, source: &Path) -> Result<String> {
        if self.text(source, &["rev-parse", "--is-inside-work-tree"])? != "true" {
            return Err(Error::new(
                "source_unavailable",
                "bootstrap requires an actual local working repository",
            ));
        }
        let git = fs::canonicalize(source.join(".git"))
            .map_err(|e| Error::io("resolve unborn Git directory", e))?;
        if git != source.join(".git")
            || !fs::symlink_metadata(source.join(".git"))
                .map_err(|e| Error::io("inspect unborn Git directory", e))?
                .is_dir()
            || Path::new(&self.text(source, &["rev-parse", "--absolute-git-dir"])?) != git
            || git.join("objects/info/alternates").exists()
        {
            return Err(Error::new(
                "execution_ownership",
                "bootstrap source must be an independent ordinary Git repository",
            ));
        }
        let head = self.output(
            source,
            &[
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new("--quiet"),
                OsStr::new("HEAD"),
            ],
            None,
        )?;
        if head.status.code() != Some(1)
            || !self
                .text(source, &["for-each-ref", "--format=%(refname)"])?
                .is_empty()
        {
            return Err(Error::new(
                "baseline_present",
                "repository has a commit or reference; select its actual baseline",
            ));
        }
        let reference = self.text(source, &["symbolic-ref", "--quiet", "HEAD"])?;
        self.run(source, &["check-ref-format", &reference], None)?;
        reference
            .strip_prefix("refs/heads/")
            .map(str::to_owned)
            .ok_or_else(|| {
                Error::new(
                    "source_unavailable",
                    "unborn HEAD is not an ordinary branch",
                )
            })
    }
    pub(crate) fn prepare_unborn_execution_clone(
        &self,
        source: &Path,
        root: &Path,
        identity: &crate::execution::DirectoryIdentity,
    ) -> Result<VerifiedUnbornClone> {
        if self.host_scope.is_some() {
            return Err(Error::new(
                "scope_violation",
                "host metadata scope cannot create bootstrap source",
            ));
        }
        identity.repository.validate()?;
        identity.work.validate()?;
        let source =
            fs::canonicalize(source).map_err(|e| Error::io("resolve bootstrap source", e))?;
        let root = owner_directory(root)?;
        let destination = &identity.repository_path;
        if identity.schema_version != 1
            || identity.runtime_id.is_nil()
            || identity.directory_id.is_nil()
            || identity.operator_checkout != source
            || root.starts_with(&source)
            || source.starts_with(&root)
            || destination.parent() != Some(root.as_path())
            || destination.exists()
        {
            return Err(Error::new(
                "execution_ownership",
                "bootstrap requires a fresh disjoint owned workspace",
            ));
        }
        let branch = self.unborn_branch(&source)?;
        let before =
            self.raw_source_paths_snapshot(&source, self.observed_source_paths(&source, 100_000)?)?;
        fs::create_dir(destination).map_err(|e| Error::io("create unborn workspace", e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(destination, fs::Permissions::from_mode(0o700))
                .map_err(|e| Error::io("protect unborn workspace", e))?;
        }
        self.run(
            destination,
            &[
                "init",
                "--quiet",
                "--template=/dev/null",
                &format!("--initial-branch={branch}"),
            ],
            None,
        )?;
        for (name, file) in &before {
            let path = destination.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| Error::io("create bootstrap source directory", e))?;
            }
            if file.mode == "120000" {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStrExt;
                    std::os::unix::fs::symlink(OsStr::from_bytes(&file.bytes), &path)
                        .map_err(|e| Error::io("copy bootstrap source link", e))?;
                }
                #[cfg(not(unix))]
                {
                    return Err(Error::new(
                        "unsupported_platform",
                        "bootstrap links require Unix",
                    ));
                }
            } else {
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|e| Error::io("copy bootstrap source", e))?;
                output
                    .write_all(&file.bytes)
                    .map_err(|e| Error::io("write bootstrap source", e))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(
                        &path,
                        fs::Permissions::from_mode(if file.mode == "100755" {
                            0o700
                        } else {
                            0o600
                        }),
                    )
                    .map_err(|e| Error::io("protect bootstrap source", e))?;
                }
            }
        }
        if self.unborn_branch(&source)? != branch
            || self
                .raw_source_paths_snapshot(&source, self.observed_source_paths(&source, 100_000)?)?
                != before
            || self.unborn_branch(destination)? != branch
        {
            return Err(Error::new(
                "stale_revision",
                "bootstrap source changed while preparing; workspace retained",
            ));
        }
        let proof = VerifiedUnbornClone {
            identity: identity.clone(),
            branch,
            initial_files: before
                .iter()
                .map(|(path, file)| (path.clone(), (file.mode.clone(), file.oid.clone())))
                .collect(),
            paths: [
                root.as_path(),
                destination.as_path(),
                &destination.join(".git"),
            ]
            .into_iter()
            .map(clone_directory_stamp)
            .collect::<Result<_>>()?,
        };
        proof.revalidate()?;
        Ok(proof)
    }
}

/// Actual root objects produced from one checked unborn workspace. The absent
/// parent is structural: no synthetic baseline enters Revision or delivery.
#[derive(Debug)]
pub(crate) struct RootCommittedSource {
    revision: Revision,
    metadata: SourceCommitMetadata,
    files: usize,
}
impl RootCommittedSource {
    pub(crate) fn revision(&self) -> &Revision {
        &self.revision
    }
    pub(crate) fn metadata(&self) -> &SourceCommitMetadata {
        &self.metadata
    }
    pub(crate) fn files(&self) -> usize {
        self.files
    }
}
impl Git {
    pub(crate) fn commit_unborn_execution(
        &self,
        proof: &VerifiedUnbornClone,
        boundary: &kegari_execution::ValidatedIsolation,
        scope: &WriteScope,
        metadata: SourceCommitMetadata,
    ) -> Result<RootCommittedSource> {
        if self.host_scope.is_some()
            || boundary.plan().role != kegari_execution::ExecutionRole::Child
            || proof.identity.repository_path != boundary.plan().repository
            || proof.identity.work.child.is_none()
        {
            return Err(Error::new(
                "scope_violation",
                "root source sealing requires the actual scoped child workspace",
            ));
        }
        boundary.revalidate()?;
        proof.revalidate()?;
        scope.validate()?;
        metadata.validate()?;
        let path = &proof.identity.repository_path;
        if self.unborn_branch(path)? != proof.branch {
            return Err(Error::new("stale_revision", "bootstrap branch changed"));
        }
        if !self.run(path, &["ls-files", "--stage"], None)?.is_empty() {
            return Err(Error::new(
                "dirty_index",
                "bootstrap staging changed; preserve it for reconciliation",
            ));
        }
        let before =
            self.raw_source_paths_snapshot(path, self.observed_source_paths(path, 100_000)?)?;
        if before.is_empty() {
            return Err(Error::new(
                "bootstrap_empty",
                "bootstrap has no actual source files; no empty initial commit is manufactured",
            ));
        }
        for name in proof
            .initial_files
            .keys()
            .chain(before.keys())
            .collect::<BTreeSet<_>>()
        {
            if proof.initial_files.get(name).map(|(mode, oid)| (mode, oid))
                == before.get(name).map(|file| (&file.mode, &file.oid))
            {
                continue;
            }
            let absolute = path.join(name);
            if !scope.permits(name)
                || !boundary.plan().source_writes.iter().any(|grant| {
                    grant.path == absolute || grant.directory && absolute.starts_with(&grant.path)
                })
            {
                return Err(Error::new(
                    "scope_violation",
                    "bootstrap source changed outside its authorized child boundary",
                ));
            }
        }
        let index = path
            .join(".git")
            .join(format!("kegari-root-index-{}", proof.identity.directory_id));
        if index.symlink_metadata().is_ok()
            || index.with_extension("lock").symlink_metadata().is_ok()
        {
            return Err(Error::new(
                "reconciliation_required",
                "root source sealing has a retained index; never repeat an unknown attempt",
            ));
        }
        self.index_command(path, &index, &["read-tree", "--empty"], None)?;
        let mut update = Vec::new();
        for (name, file) in &before {
            let actual = String::from_utf8(self.run(
                path,
                &["hash-object", "-w", "--stdin", "--no-filters"],
                Some(&file.bytes),
            )?)
            .map_err(|_| Error::new("git_response", "root blob ID is not UTF-8"))?;
            if actual.trim_end() != file.oid {
                return Err(Error::new(
                    "provenance_mismatch",
                    "bootstrap bytes differ from Git object",
                ));
            }
            update.extend_from_slice(format!("{} {}\t{}\0", file.mode, file.oid, name).as_bytes());
        }
        self.index_command(
            path,
            &index,
            &["update-index", "-z", "--index-info"],
            Some(&update),
        )?;
        let tree = String::from_utf8(self.index_command(path, &index, &["write-tree"], None)?)
            .map_err(|_| Error::new("git_response", "root tree ID is not UTF-8"))?
            .trim_end()
            .to_owned();
        validate_oid(&tree)?;
        proof.revalidate()?;
        if self.raw_source_paths_snapshot(path, self.observed_source_paths(path, 100_000)?)?
            != before
        {
            return Err(Error::new(
                "stale_revision",
                "bootstrap source changed during root object creation",
            ));
        }
        let date = metadata
            .timestamp
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let mut command = self.command(path);
        command
            .env("GIT_AUTHOR_NAME", &metadata.identity.name)
            .env("GIT_AUTHOR_EMAIL", &metadata.identity.email)
            .env("GIT_COMMITTER_NAME", &metadata.identity.name)
            .env("GIT_COMMITTER_EMAIL", &metadata.identity.email)
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date)
            .args([
                "-c",
                "commit.gpgSign=false",
                "commit-tree",
                &tree,
                "-F",
                "-",
            ]);
        let output = self.finish_command(command, Some(metadata.message.as_bytes()))?;
        if !output.status.success() {
            return Err(Error::new(
                "git_operation_failed",
                "root commit creation failed; preserve retained files and index",
            ));
        }
        let commit = String::from_utf8(output.stdout)
            .map_err(|_| Error::new("git_response", "root commit ID is not UTF-8"))?
            .trim_end()
            .to_owned();
        let revision = self.exact_revision(path, &commit)?;
        if revision.tree != tree
            || self.text(path, &["rev-list", "--parents", "-n", "1", &commit])? != commit
        {
            return Err(Error::new(
                "provenance_mismatch",
                "bootstrap commit differs from the actual parentless tree",
            ));
        }
        proof.revalidate()?;
        self.run(
            path,
            &["update-ref", "--stdin"],
            Some(format!("create refs/heads/{} {}\n", proof.branch, commit).as_bytes()),
        )?;
        self.run(path, &["read-tree", &tree], None)?;
        if self.raw_source_paths_snapshot(path, self.observed_source_paths(path, 100_000)?)?
            != before
        {
            return Err(Error::new(
                "stale_revision",
                "bootstrap source changed while sealing; retain actual commit for reconciliation",
            ));
        }
        self.assert_clean(path)?;
        Ok(RootCommittedSource {
            revision,
            metadata,
            files: before.len(),
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn unborn_bootstrap_preserves_real_files_without_inventing_a_revision() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::TempDir::new().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let source = base.join("source");
        let root = base.join("owned");
        for path in [&source, &root] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let git = Git::new(crate::test_pinned_git::prepare(&base)).unwrap();
        git.run(&source, &["init", "--quiet", "--initial-branch=main"], None)
            .unwrap();
        fs::write(source.join("README.md"), b"new repository\n").unwrap();
        fs::write(source.join("staged.txt"), b"staged input\n").unwrap();
        git.run(&source, &["add", "staged.txt"], None).unwrap();
        let index = fs::read(source.join(".git/index")).unwrap();
        let digest = bytes_digest(b"bootstrap fixture");
        let identity = crate::execution::DirectoryIdentity {
            schema_version: 1,
            runtime_id: Uuid::from_u128(1),
            directory_id: Uuid::from_u128(2),
            repository: RepositoryId {
                hostname: "github.invalid".into(),
                id: "42".into(),
            },
            work: crate::evidence::WorkIdentity {
                linear_endpoint: "https://linear.invalid/graphql".into(),
                linear_workspace: Uuid::from_u128(3),
                parent: Uuid::from_u128(4),
                child: Some(Uuid::from_u128(5)),
                generation: crate::evidence::Generation {
                    authorization: digest.clone(),
                    definition: digest.clone(),
                    correction: digest.clone(),
                    scope: digest,
                },
            },
            repository_path: root.join("repository"),
            operator_checkout: source.clone(),
        };
        let proof = git
            .prepare_unborn_execution_clone(&source, &root, &identity)
            .unwrap();
        assert_eq!(proof.branch(), "main");
        proof.revalidate().unwrap();
        assert_eq!(
            fs::read(identity.repository_path.join("README.md")).unwrap(),
            b"new repository\n"
        );
        assert_eq!(
            fs::read(identity.repository_path.join("staged.txt")).unwrap(),
            b"staged input\n"
        );
        assert_eq!(fs::read(source.join(".git/index")).unwrap(), index);
        assert!(git.revision(&identity.repository_path, "HEAD").is_err());
        assert_eq!(
            git.prepare_unborn_execution_clone(&source, &root, &identity)
                .unwrap_err()
                .code,
            "execution_ownership"
        );
        // A tag-only source is not empty, even with an unborn default HEAD.
        let blob = String::from_utf8(
            git.run(
                &source,
                &["hash-object", "-w", "--stdin"],
                Some(b"tag-only object"),
            )
            .unwrap(),
        )
        .unwrap();
        git.run(
            &source,
            &["update-ref", "refs/tags/data", blob.trim()],
            None,
        )
        .unwrap();
        assert_eq!(
            git.unborn_branch(&source).unwrap_err().code,
            "baseline_present"
        );
        let scratch = root.join("scratch");
        fs::create_dir(&scratch).unwrap();
        let boundary = kegari_execution::IsolationPlan {
            schema_version: 1,
            role: kegari_execution::ExecutionRole::Child,
            execution_root: root.clone(),
            repository: identity.repository_path.clone(),
            cwd: identity.repository_path.clone(),
            scratch,
            read_roots: vec![],
            source_writes: vec![kegari_execution::PathGrant {
                path: identity.repository_path.join("README.md"),
                directory: false,
            }],
            resource_writes: vec![],
            forbidden_roots: vec![],
        }
        .validate(crate::BUILD_ID)
        .unwrap();
        fs::write(
            identity.repository_path.join("README.md"),
            b"actual scoped bootstrap output\n",
        )
        .unwrap();
        let metadata=SourceCommitMetadata {
            identity:CommitIdentity{name:"Fixture Operator".into(),email:"operator@example.invalid".into()},
            timestamp:chrono::DateTime::from_timestamp(1_700_000_000,0).unwrap(),
            message:"feat: initialize actual source\n\nSigned-off-by: Fixture Operator <operator@example.invalid>\n".into(),
        };
        let sealed = git
            .commit_unborn_execution(
                &proof,
                &boundary,
                &WriteScope {
                    paths: vec!["README.md".into()],
                },
                metadata,
            )
            .unwrap();
        assert_eq!(
            git.text(
                &identity.repository_path,
                &["rev-list", "--parents", "-n", "1", "HEAD"]
            )
            .unwrap(),
            sealed.revision().commit
        );
        assert_eq!(sealed.files(), 2);
        assert_eq!(
            git.revision(&identity.repository_path, "HEAD").unwrap(),
            *sealed.revision()
        );
        assert_eq!(
            git.run(&identity.repository_path, &["show", "HEAD:README.md"], None)
                .unwrap(),
            b"actual scoped bootstrap output\n"
        );
        assert_eq!(fs::read(source.join(".git/index")).unwrap(), index);
        assert_eq!(
            fs::read(source.join("README.md")).unwrap(),
            b"new repository\n"
        );
        assert!(git.revision(&source, "HEAD").is_err());
        assert_eq!(proof.revalidate().unwrap_err().code, "stale_revision");
    }
}
