//! One bounded capture supplies both fixture provenance and reset contents.
use super::{FileSnapshot, MAX_EVIDENCE_BYTES, MAX_FILES, file_mode, relative};
use kegari_digest::bytes_digest;
use kegari_types::error::{Error, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_TREE_ENTRIES: usize = MAX_FILES * 2;
const MAX_TREE_DEPTH: usize = 128;

pub(super) struct CapturedFile {
    bytes: Vec<u8>,
    mode: u32,
}

impl CapturedFile {
    fn snapshot(&self) -> FileSnapshot {
        FileSnapshot {
            digest: bytes_digest(&self.bytes),
            mode: self.mode,
        }
    }

    pub(super) fn mode(&self) -> u32 {
        self.mode
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(super) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub(super) fn apply_mode(&self, path: &Path) -> Result<()> {
        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::Permissions::from_mode(self.mode)
        };
        #[cfg(not(unix))]
        let permissions = {
            let mut permissions = std::fs::metadata(path)
                .map_err(|e| Error::io("inspect reset file", e))?
                .permissions();
            permissions.set_readonly(self.mode & 0o222 == 0);
            permissions
        };
        std::fs::set_permissions(path, permissions)
            .map_err(|e| Error::io("preserve fixture file mode", e))
    }
}

fn validate_file(metadata: &std::fs::Metadata, limit: usize) -> Result<()> {
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err(Error::new(
            "evidence_limit",
            "evaluation input is not a bounded regular file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(Error::new(
                "scope_violation",
                "evaluation files may not alias another file",
            ));
        }
    }
    Ok(())
}

fn unchanged(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        before.dev() == after.dev()
            && before.ino() == after.ino()
            && before.mode() == after.mode()
            && before.uid() == after.uid()
            && before.gid() == after.gid()
            && before.nlink() == after.nlink()
            && before.len() == after.len()
            && before.mtime() == after.mtime()
            && before.mtime_nsec() == after.mtime_nsec()
            && before.ctime() == after.ctime()
            && before.ctime_nsec() == after.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        before.len() == after.len()
            && file_mode(before) == file_mode(after)
            && before.modified().ok() == after.modified().ok()
    }
}

pub(super) fn read_file(path: &Path, limit: usize) -> Result<CapturedFile> {
    read_file_guarded(path, limit, &mut |_| Ok(()))
}

pub(super) fn read_file_guarded(
    path: &Path,
    limit: usize,
    before_read: &mut dyn FnMut(usize) -> Result<()>,
) -> Result<CapturedFile> {
    let limit = limit.min(MAX_EVIDENCE_BYTES);
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NONBLOCK ensures a file replaced with a FIFO cannot hang before
        // the descriptor's regular-file metadata is checked.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    #[cfg(not(unix))]
    if std::fs::symlink_metadata(path)
        .map_err(|e| Error::io("inspect evaluation input", e))?
        .file_type()
        .is_symlink()
    {
        return Err(Error::new("scope_violation", "evaluation symlink rejected"));
    }
    let mut file = options
        .open(path)
        .map_err(|e| Error::io("open evaluation input", e))?;
    read_open_file(&mut file, limit, before_read)
}

/// Captures the same descriptor bytes while the caller reserves bounded work.
pub(super) fn read_open_file(
    file: &mut std::fs::File,
    limit: usize,
    before_read: &mut dyn FnMut(usize) -> Result<()>,
) -> Result<CapturedFile> {
    let limit = limit.min(MAX_EVIDENCE_BYTES);
    let before = file
        .metadata()
        .map_err(|e| Error::io("inspect evaluation input", e))?;
    validate_file(&before, limit)?;
    let mut bytes = Vec::new();
    // Reserve before each actual read; an attempted read is never refunded.
    // The extra byte detects growth and is charged even on a short/empty read.
    while bytes.len() <= limit {
        let amount = (before.len() as usize + 1 - bytes.len()).min(64 * 1024);
        before_read(amount)?;
        let mut chunk = vec![0; amount];
        let read = file
            .read(&mut chunk)
            .map_err(|e| Error::io("read evaluation input", e))?;
        bytes.extend_from_slice(&chunk[..read]);
        if read == 0 || bytes.len() > before.len() as usize {
            break;
        }
    }
    let after = file
        .metadata()
        .map_err(|e| Error::io("reinspect evaluation input", e))?;
    validate_file(&after, limit)?;
    if bytes.len() > limit {
        return Err(Error::new(
            "evidence_limit",
            "evaluation file grew beyond limit",
        ));
    }
    if bytes.len() as u64 != before.len() || !unchanged(&before, &after) {
        return Err(Error::new(
            "fixture_changed",
            "evaluation file changed during capture",
        ));
    }
    Ok(CapturedFile {
        bytes,
        mode: file_mode(&before),
    })
}

pub(super) struct CapturedTree {
    files: BTreeMap<String, CapturedFile>,
    // Topology is separate from the legacy file-only digest.
    topology: BTreeSet<PathBuf>,
}

impl CapturedTree {
    pub(super) fn capture(root: &Path) -> Result<Self> {
        Self::capture_with_limits(root, MAX_TREE_ENTRIES, MAX_TREE_DEPTH)
    }

    fn capture_with_limits(root: &Path, entry_limit: usize, depth_limit: usize) -> Result<Self> {
        Self::capture_guarded(root, entry_limit, depth_limit, &mut |_| Ok(()))
    }

    pub(super) fn capture_observed(
        root: &Path,
        before_read: &mut dyn FnMut(usize) -> Result<()>,
    ) -> Result<Self> {
        Self::capture_guarded(root, MAX_TREE_ENTRIES, MAX_TREE_DEPTH, before_read)
    }

    pub(super) fn entries(&self) -> impl Iterator<Item = (&str, &CapturedFile)> {
        self.files.iter().map(|(path, file)| (path.as_str(), file))
    }

    fn capture_guarded(
        root: &Path,
        entry_limit: usize,
        depth_limit: usize,
        before_read: &mut dyn FnMut(usize) -> Result<()>,
    ) -> Result<Self> {
        struct Budget {
            bytes: usize,
            entries: usize,
            entry_limit: usize,
            depth_limit: usize,
        }
        fn visit(
            root: &Path,
            path: &Path,
            files: &mut BTreeMap<String, CapturedFile>,
            topology: &mut BTreeSet<PathBuf>,
            budget: &mut Budget,
            depth: usize,
            before_read: &mut dyn FnMut(usize) -> Result<()>,
        ) -> Result<()> {
            // Empty directories do not affect the legacy digest, but still
            // consume traversal work and recursion depth when grading a tree.
            if budget.entries >= budget.entry_limit || depth > budget.depth_limit {
                return Err(Error::new(
                    "evidence_limit",
                    "fixture topology exceeds bound",
                ));
            }
            budget.entries += 1;
            if path != root {
                topology.insert(
                    path.strip_prefix(root)
                        .map_err(|_| Error::invalid("fixture escaped root"))?
                        .to_owned(),
                );
            }
            // Also check the caller's original deadline on directory-only work.
            before_read(0)?;
            let metadata = std::fs::symlink_metadata(path)
                .map_err(|e| Error::io("inspect fixture tree", e))?;
            if metadata.is_dir() {
                for entry in
                    std::fs::read_dir(path).map_err(|e| Error::io("read fixture directory", e))?
                {
                    let entry = entry.map_err(|e| Error::io("read fixture entry", e))?;
                    // Preserve snapshot's existing root-only Git exclusion.
                    // Nested .git files remain part of the fixture digest.
                    if path == root && entry.file_name() == ".git" {
                        topology.insert(".git".into());
                        continue;
                    }
                    visit(
                        root,
                        &entry.path(),
                        files,
                        topology,
                        budget,
                        depth + 1,
                        before_read,
                    )?;
                }
            } else {
                if files.len() >= MAX_FILES {
                    return Err(Error::new("evidence_limit", "fixture tree exceeds bound"));
                }
                let data = read_file_guarded(path, MAX_EVIDENCE_BYTES - budget.bytes, before_read)?;
                budget.bytes += data.bytes.len();
                let key = path
                    .strip_prefix(root)
                    .map_err(|_| Error::invalid("fixture escaped root"))?
                    .to_str()
                    .ok_or_else(|| Error::invalid("fixture path not UTF-8"))?
                    .to_owned();
                if files.insert(key, data).is_some() {
                    return Err(Error::new(
                        "fixture_changed",
                        "duplicate captured fixture path",
                    ));
                }
            }
            Ok(())
        }
        let mut files = BTreeMap::new();
        let mut topology = BTreeSet::new();
        let mut budget = Budget {
            bytes: 0,
            entries: 0,
            entry_limit,
            depth_limit,
        };
        visit(
            root,
            root,
            &mut files,
            &mut topology,
            &mut budget,
            0,
            before_read,
        )?;
        Ok(Self { files, topology })
    }

    pub(super) fn topology(&self) -> &BTreeSet<PathBuf> {
        &self.topology
    }

    pub(super) fn snapshot(&self) -> BTreeMap<String, FileSnapshot> {
        self.files
            .iter()
            .map(|(path, file)| (path.clone(), file.snapshot()))
            .collect()
    }

    pub(super) fn verify_reset(
        &self,
        repository: &Path,
        bound_workflow: &[u8],
    ) -> Result<BTreeMap<String, FileSnapshot>> {
        let mut expected: BTreeMap<_, _> = self
            .repository_files()
            .into_iter()
            .map(|(path, file)| (path.to_owned(), file.snapshot()))
            .collect();
        expected
            .get_mut("WORKFLOW.md")
            .ok_or_else(|| Error::invalid("fixture workflow missing from capture"))?
            .digest = bytes_digest(bound_workflow);
        let actual = Self::capture(repository)?.snapshot();
        if actual != expected {
            return Err(Error::new(
                "evaluation_fidelity",
                "prepared repository differs from captured fixture reset",
            ));
        }
        Ok(actual)
    }

    pub(super) fn file(&self, path: &str) -> Result<&CapturedFile> {
        relative(path)?;
        self.optional_file(path)
            .ok_or_else(|| Error::invalid("required fixture input is missing from capture"))
    }

    pub(super) fn optional_file(&self, path: &str) -> Option<&CapturedFile> {
        self.files.get(path)
    }

    pub(super) fn repository_files(&self) -> BTreeMap<&str, &CapturedFile> {
        self.files
            .iter()
            .filter_map(|(path, file)| {
                let relative = path.strip_prefix("repository/")?;
                // Previous reset copied snapshot(repository), which excludes
                // its own Git administration, even though the outer fixture
                // digest includes that nested inventory.
                if relative == ".git" || relative.starts_with(".git/") {
                    None
                } else {
                    Some((relative, file))
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::{Bounds, Seed, Variant, prepare_captured_fixture, tree_digest};
    use kegari_context::instructions::AgentRole;
    use kegari_digest::canonical_digest;
    use serde_json::json;

    fn variant(capture: &CapturedTree) -> Variant {
        Variant {
            case_id: "V0-E07".into(),
            id: "capture".into(),
            requirements: vec![],
            role: AgentRole::Implementation,
            phase: "implementation".into(),
            profile: "fixture".into(),
            bounds: Bounds {
                repetitions: 1,
                max_turns: 1,
                max_seconds: 1,
            },
            fixture: "fixture".into(),
            fixture_digest: canonical_digest(1, &capture.snapshot()).unwrap(),
            scorer: "scorer.json".into(),
            scorer_digest: bytes_digest(b"scorer"),
            canary_diagnostic: None,
            initial_state: "fixture".into(),
            seed: Seed::DeclaredPrior {
                artifact: "prior.json".into(),
                digest: bytes_digest(capture.file("prior.json").unwrap().bytes()),
            },
            expected_outcomes: vec![],
            prohibited_effects: vec![],
            required_artifacts: vec![],
            qualitative: vec![],
        }
    }

    #[test]
    fn change_and_restore_cannot_substitute_any_prepared_input() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let suite = root.join("suite");
        let source = suite.join("fixture");
        let grader = root.join("grader");
        std::fs::create_dir_all(source.join("repository")).unwrap();
        std::fs::create_dir(&grader).unwrap();
        let context = json!({
            "workspace":{"id":{"$kegari":"workspace_id"}},
            "operation":{"id":{"$kegari":"operation_id"}},
            "profile":{"$kegari":"profile"},"phase":{"$kegari":"phase"},
            "task":"original task covered by declared digest"
        });
        let binding = json!({"schema_version":1,"source_write_paths":["file.txt"],"slots":[
            {"pointer":"/operation/id","source":"operation_id"},
            {"pointer":"/phase","source":"phase"},
            {"pointer":"/profile","source":"profile"},
            {"pointer":"/workspace/id","source":"workspace_id"}
        ]});
        let prior = json!({"repository_files":{"file.txt":"declared old guidance"}});
        for (name, value) in [
            ("context.json", &context),
            ("context-binding.json", &binding),
            ("prior.json", &prior),
        ] {
            std::fs::write(source.join(name), serde_json::to_vec(value).unwrap()).unwrap();
        }
        std::fs::write(
            source.join("repository/file.txt"),
            b"declared current content",
        )
        .unwrap();
        std::fs::write(
            source.join("repository/WORKFLOW.md"),
            b"scratch: @KegariScratch@\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                source.join("repository/file.txt"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let capture = CapturedTree::capture(&source).unwrap();
        let variant = variant(&capture);

        // Controlled interleaving: replace every source after its sole capture,
        // prepare while those changes exist, then restore before the final
        // source hash check. Hash-before/hash-after would miss this substitution.
        for path in capture.files.keys() {
            std::fs::write(source.join(path), b"substituted and not valid JSON").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(source.join(path), std::fs::Permissions::from_mode(0o600))
                    .unwrap();
            }
        }
        std::fs::remove_file(source.join("context-binding.json")).unwrap();
        let prepared =
            prepare_captured_fixture(&suite, &variant, &capture, &root.join("trial"), &grader)
                .unwrap();
        for (path, file) in &capture.files {
            std::fs::write(source.join(path), file.bytes()).unwrap();
            file.apply_mode(&source.join(path)).unwrap();
        }
        assert_eq!(tree_digest(&source).unwrap(), variant.fixture_digest);
        assert_eq!(prepared.fixture_digest, variant.fixture_digest);
        assert_eq!(prepared.input, context);
        assert_eq!(prepared.prior, Some(prior));
        assert_eq!(
            serde_json::to_value(prepared.context_binding.as_ref().unwrap()).unwrap(),
            binding
        );
        assert_eq!(
            std::fs::read(prepared.repository.join("file.txt")).unwrap(),
            b"declared current content"
        );
        assert_eq!(
            prepared.before["file.txt"],
            capture.snapshot()["repository/file.txt"]
        );
        assert_eq!(
            std::fs::read_to_string(prepared.repository.join("WORKFLOW.md")).unwrap(),
            format!("scratch: {}\n", prepared.scratch.display())
        );
        prepared.verify_preparation().unwrap();
    }

    #[test]
    fn captured_bytes_are_hashed_with_legacy_modes_and_root_git_exclusion() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        std::fs::create_dir_all(root.join("repository/.git")).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/ignored"), b"root git administration").unwrap();
        std::fs::write(root.join("repository/.git/HEAD"), b"nested admin").unwrap();
        std::fs::write(root.join("repository/file"), b"actual file").unwrap();
        let expected = [
            ("repository/.git/HEAD", b"nested admin".as_slice()),
            ("repository/file", b"actual file".as_slice()),
        ]
        .into_iter()
        .map(|(path, bytes)| {
            (
                path.to_owned(),
                FileSnapshot {
                    digest: bytes_digest(bytes),
                    mode: file_mode(&std::fs::metadata(root.join(path)).unwrap()),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
        let capture = CapturedTree::capture(root).unwrap();
        assert_eq!(capture.snapshot(), expected);
        assert_eq!(
            tree_digest(root).unwrap(),
            canonical_digest(1, &expected).unwrap()
        );
        assert_eq!(
            capture
                .repository_files()
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec!["file"]
        );
    }

    #[test]
    fn descriptor_read_enforces_remaining_total_byte_budget() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file");
        std::fs::write(&file, b"12345").unwrap();
        assert_eq!(read_file(&file, 5).unwrap().bytes(), b"12345");
        assert!(matches!(read_file(&file, 4), Err(e) if e.code == "evidence_limit"));
        let oversized = temp.path().join("oversized");
        std::fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_EVIDENCE_BYTES as u64 + 1)
            .unwrap();
        assert!(matches!(read_file(&oversized, usize::MAX), Err(e) if e.code == "evidence_limit"));
    }

    #[test]
    fn reset_verification_rejects_destination_content_mode_and_inventory_substitution() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let source = root.join("source");
        let destination = root.join("destination");
        std::fs::create_dir_all(source.join("repository")).unwrap();
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(source.join("repository/file"), b"declared").unwrap();
        std::fs::write(source.join("repository/WORKFLOW.md"), b"@KegariScratch@").unwrap();
        let captured = CapturedTree::capture(&source).unwrap();
        for (path, file) in captured.repository_files() {
            std::fs::write(destination.join(path), file.bytes()).unwrap();
            file.apply_mode(&destination.join(path)).unwrap();
        }
        let bound = b"/actual/private/scratch";
        assert!(captured.verify_reset(&destination, bound).is_err());
        std::fs::write(destination.join("WORKFLOW.md"), bound).unwrap();
        captured.verify_reset(&destination, bound).unwrap();
        std::fs::write(destination.join("file"), b"substituted").unwrap();
        assert!(
            matches!(captured.verify_reset(&destination, bound), Err(e) if e.code == "evaluation_fidelity")
        );
        std::fs::write(destination.join("file"), b"declared").unwrap();
        std::fs::write(destination.join("extra"), b"undeclared").unwrap();
        assert!(
            matches!(captured.verify_reset(&destination, bound), Err(e) if e.code == "evaluation_fidelity")
        );
        std::fs::remove_file(destination.join("extra")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let original = captured.file("repository/file").unwrap().mode;
            std::fs::set_permissions(
                destination.join("file"),
                std::fs::Permissions::from_mode(original ^ 0o100),
            )
            .unwrap();
            assert!(
                matches!(captured.verify_reset(&destination, bound), Err(e) if e.code == "evaluation_fidelity")
            );
            captured
                .file("repository/file")
                .unwrap()
                .apply_mode(&destination.join("file"))
                .unwrap();
        }
        captured.verify_reset(&destination, bound).unwrap();
    }

    #[test]
    fn empty_directories_consume_entry_and_depth_budgets_without_changing_digest() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        for name in ["a", "b", "c"] {
            std::fs::create_dir(root.join(name)).unwrap();
        }
        let empty = CapturedTree::capture_with_limits(root, 4, 1).unwrap();
        assert!(empty.snapshot().is_empty());
        std::fs::create_dir(root.join("d")).unwrap();
        assert!(
            matches!(CapturedTree::capture_with_limits(root, 4, 1), Err(e) if e.code == "evidence_limit")
        );
        std::fs::create_dir_all(root.join("a/b/c")).unwrap();
        assert!(
            CapturedTree::capture_with_limits(root, 8, 3)
                .unwrap()
                .snapshot()
                .is_empty()
        );
        std::fs::create_dir(root.join("a/b/c/d")).unwrap();
        assert!(
            matches!(CapturedTree::capture_with_limits(root, 8, 3), Err(e) if e.code == "evidence_limit")
        );
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_read_rejects_links_and_fifo_without_blocking() {
        use std::os::unix::ffi::OsStrExt;
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("file");
        std::fs::write(&file, b"secret").unwrap();
        let link = temp.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(read_file(&link, MAX_EVIDENCE_BYTES).is_err());
        let hard = temp.path().join("hard");
        std::fs::hard_link(&file, &hard).unwrap();
        assert!(
            matches!(read_file(&hard, MAX_EVIDENCE_BYTES), Err(e) if e.code == "scope_violation")
        );
        let fifo = temp.path().join("fifo");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // Owned disposable fixture; no service, child process or native provider.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(
            matches!(read_file(&fifo, MAX_EVIDENCE_BYTES), Err(e) if e.code == "evidence_limit")
        );
    }
}
