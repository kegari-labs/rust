//! Process-free inspection of original worker read grants. Handles name retained
//! contract resources, never paths supplied as new authority by a tool.
use super::*;
use crate::validation::DiscoveryReadContent;
use std::fs::{File, Metadata};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::time::{Duration as StdDuration, Instant};

const MAX_RESOURCES: usize = 256;
const MAX_ENTRIES: usize = 256;
const MAX_READ: usize = 48_000;
const TOTAL_BYTES: usize = 8 * 1024 * 1024;
const TOTAL_ENTRIES: usize = 8192;

fn denied() -> Error {
    Error::new(
        "scope_violation",
        "resource is outside the original admitted inspection boundary",
    )
}
fn changed() -> Error {
    Error::new(
        "validation_read_changed",
        "original resource identity or observed contents changed",
    )
}
fn current(deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        Err(Error::new(
            "timeout",
            "original inspection deadline exhausted",
        ))
    } else {
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct Stamp {
    device: u64,
    inode: u64,
    mode: u32,
    links: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Stamp {
    fn of(m: &Metadata) -> Self {
        Self {
            device: m.dev(),
            inode: m.ino(),
            mode: m.mode(),
            links: m.nlink(),
            bytes: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        }
    }
    fn identity(&self) -> (u64, u64, u32) {
        (self.device, self.inode, self.mode & libc::S_IFMT as u32)
    }
}
#[derive(Clone)]
struct Resource {
    root: PathBuf,
    directory: bool,
    identity: (u64, u64, u32),
    binding: String,
    description: Value,
    immutable: bool,
}
#[derive(Default)]
struct Budget {
    deadline: Option<Instant>,
    bytes: usize,
    entries: usize,
}
#[derive(Clone)]
pub(super) struct ResourceState {
    roots: BTreeMap<String, Resource>,
    budget: Arc<Mutex<Budget>>,
}
impl ResourceState {
    pub(super) fn new(contract: &WorkerContract) -> Result<Self> {
        // Initial root identity is host preparation, not a deferred observation.
        // Bound this metadata-only phase separately; actual tool calls later use
        // the original worker/evaluation cutoff and cannot extend it.
        let preparation_deadline = Instant::now() + StdDuration::from_secs(5);
        let plan = contract.isolation.plan();
        let mut definitions = vec![(
            plan.repository.clone(),
            true,
            json!({"kind":"primary_repository","repository_id":contract.repository.as_ref().map(|r|&r.id),"writable":!plan.source_writes.is_empty()}),
            false,
        )];
        for (path, resource) in &contract.read_resources {
            let grant = plan
                .read_roots
                .iter()
                .find(|g| g.path == *path)
                .ok_or_else(denied)?;
            let writable = plan.resource_writes.iter().any(|g| g == grant);
            let description = match resource {
                ReadResource::Repository { id } => {
                    json!({"kind":"read_repository","repository_id":id,"writable":false})
                }
                ReadResource::TestResource { name, .. } => {
                    json!({"kind":"test_resource","name":name,"writable":writable})
                }
            };
            definitions.push((path.clone(), grant.directory, description, !writable));
        }
        if definitions.len() > MAX_RESOURCES {
            return Err(Error::new("tool_limit", "resource inventory exceeds bound"));
        }
        let mut roots = BTreeMap::new();
        for (root, directory, mut description, immutable) in definitions {
            check_path(contract, &root)?;
            current(preparation_deadline)?;
            let file = open(&root, Some(preparation_deadline))?;
            let metadata = file
                .metadata()
                .map_err(|e| Error::io("inspect original resource", e))?;
            if directory != metadata.is_dir()
                || (!directory && (!metadata.is_file() || metadata.nlink() != 1))
            {
                return Err(denied());
            }
            let identity = Stamp::of(&metadata).identity();
            let binding = canonical_digest(
                1,
                &json!({"instruction":contract.assembled.manifest_digest,"isolation":contract.isolation.digest(),"root":root,"directory":directory,"identity":identity,"resource":description}),
            )?;
            let handle = canonical_digest(1, &json!({"resource_binding":binding}))?;
            description["handle"] = json!(handle);
            description["directory"] = json!(directory);
            if roots
                .insert(
                    handle,
                    Resource {
                        root,
                        directory,
                        identity,
                        binding,
                        description,
                        immutable,
                    },
                )
                .is_some()
            {
                return Err(denied());
            }
        }
        current(preparation_deadline)?;
        Ok(Self {
            roots,
            budget: Arc::new(Mutex::new(Budget::default())),
        })
    }
    fn lower_deadline(&self, deadline: Instant) {
        if let Ok(mut b) = self.budget.lock() {
            b.deadline = Some(b.deadline.map_or(deadline, |prior| prior.min(deadline)));
        }
    }
    pub(super) fn deadline(&self) -> Result<Instant> {
        let b = self
            .budget
            .lock()
            .map_err(|_| Error::new("inspection_unavailable", "inspection budget lock failed"))?;
        let original = b.deadline.ok_or_else(|| {
            Error::new(
                "inspection_unavailable",
                "original worker cutoff is not bound",
            )
        })?;
        current(original)?;
        Ok(original.min(Instant::now() + StdDuration::from_secs(5)))
    }
    pub(super) fn check_deadline(&self, deadline: Instant) -> Result<()> {
        self.charge(0, 0, deadline)
    }
    fn charge(&self, bytes: usize, entries: usize, deadline: Instant) -> Result<()> {
        current(deadline)?;
        let mut b = self
            .budget
            .lock()
            .map_err(|_| Error::new("inspection_unavailable", "inspection budget lock failed"))?;
        current(b.deadline.ok_or_else(|| {
            Error::new(
                "inspection_unavailable",
                "original worker cutoff is not bound",
            )
        })?)?;
        b.bytes = b
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| Error::new("tool_limit", "inspection byte budget exhausted"))?;
        b.entries = b
            .entries
            .checked_add(entries)
            .ok_or_else(|| Error::new("tool_limit", "inspection entry budget exhausted"))?;
        if b.bytes > TOTAL_BYTES || b.entries > TOTAL_ENTRIES {
            return Err(Error::new(
                "tool_limit",
                "shared worker inspection budget exhausted",
            ));
        }
        Ok(())
    }
    pub(super) fn environment(
        &self,
        contract: &WorkerContract,
        deadline: Instant,
    ) -> Result<Value> {
        self.charge(0, self.roots.len(), deadline)?;
        contract.isolation.revalidate_inspection_until(deadline)?;
        for resource in self.roots.values() {
            resource.check(contract, deadline)?;
        }
        let environment =
            match kegari_execution::observe_environment_until(&contract.isolation, deadline) {
                Ok(value) => json!({"available":true,"observation":value}),
                Err(error) if error.code == "execution_environment_unavailable" => {
                    json!({"available":false,"error":error})
                }
                Err(error) => return Err(error),
            };
        let value = bounded_value(
            json!({"schema_version":1,"environment":environment,"resources":self.roots.values().map(|r|&r.description).collect::<Vec<_>>()}),
        )?;
        self.charge(encoded_len(&value)?, 0, deadline)?;
        Ok(value)
    }
    pub(super) fn observe(
        &self,
        contract: &WorkerContract,
        handle: &str,
        path: &str,
        list: bool,
        deadline: Instant,
    ) -> Result<(Value, DiscoveryReadContent)> {
        if handle.len() > 128 {
            return Err(denied());
        }
        let resource = self.roots.get(handle).ok_or_else(denied)?;
        contract.isolation.revalidate_inspection_until(deadline)?;
        if let Some(domain) = &contract.isolation.plan().managed_execution {
            domain.revalidate_resource_versions_until(deadline, crate::BUILD_ID)?;
        }
        resource.check(contract, deadline)?;
        let target = resource.target(path, list)?;
        check_path(contract, &target)?;
        self.charge(0, 1, deadline)?;
        let mut file = open(&target, Some(deadline))?;
        let before = file
            .metadata()
            .map_err(|e| Error::io("inspect resource descriptor", e))?;
        let stamp = Stamp::of(&before);
        let (value, digest, bytes, version) = if list {
            if !before.is_dir() {
                return Err(denied());
            }
            let entries = list_directory(&file, self, contract, &target, deadline)?;
            if entries != list_directory(&file, self, contract, &target, deadline)? {
                return Err(changed());
            }
            let digest = canonical_digest(
                1,
                &entries.iter().map(|(entry, _)| entry).collect::<Vec<_>>(),
            )?;
            let version = canonical_digest(1, &json!({"directory":stamp,"entries":entries}))?;
            let value = json!({"handle":handle,"path":path,"complete":true,"scope":"admitted_non_git_children","entries":entries.iter().map(|(entry,_)|entry).collect::<Vec<_>>(),"digest":digest});
            let bytes = encoded_len(&value)? as u64;
            (value, digest, bytes, version)
        } else {
            if !before.is_file() || before.nlink() != 1 || before.len() > MAX_READ as u64 {
                return Err(Error::new(
                    "tool_limit",
                    "resource read requires one bounded regular non-hardlinked file",
                ));
            }
            let mut data = Vec::with_capacity(before.len() as usize);
            let mut chunk = [0u8; 4096];
            loop {
                current(deadline)?;
                let attempted = chunk.len().min(MAX_READ + 1 - data.len());
                self.charge(attempted, 0, deadline)?;
                let n = file
                    .read(&mut chunk[..attempted])
                    .map_err(|e| Error::io("read resource bytes", e))?;
                if n == 0 {
                    break;
                }
                if data.len() + n > MAX_READ {
                    return Err(Error::new("tool_limit", "resource read grew beyond bound"));
                }
                data.extend_from_slice(&chunk[..n]);
            }
            let digest = bytes_digest(&data);
            let content = std::str::from_utf8(&data)
                .map_err(|_| Error::new("invalid_input", "resource read requires UTF-8 text"))?;
            if content.contains('\0') {
                return Err(Error::new("invalid_input", "resource text contains NUL"));
            }
            let value = json!({"handle":handle,"path":path,"digest":digest,"bytes":data.len(),"content":content});
            (
                value,
                digest,
                data.len() as u64,
                canonical_digest(1, &stamp)?,
            )
        };
        current(deadline)?;
        if Stamp::of(
            &file
                .metadata()
                .map_err(|e| Error::io("recheck resource descriptor", e))?,
        ) != stamp
            || Stamp::of(
                &open(&target, Some(deadline))?
                    .metadata()
                    .map_err(|e| Error::io("recheck resource path", e))?,
            ) != stamp
        {
            return Err(changed());
        }
        resource.check(contract, deadline)?;
        let value = bounded_value(value)?;
        self.charge(encoded_len(&value)?, 0, deadline)?;
        let content = if list {
            DiscoveryReadContent::Directory {
                handle: handle.into(),
                resource_binding: resource.binding.clone(),
                path: path.into(),
                digest,
                bytes,
                version_digest: version,
            }
        } else {
            DiscoveryReadContent::Resource {
                handle: handle.into(),
                resource_binding: resource.binding.clone(),
                path: path.into(),
                digest,
                bytes,
                version_digest: version,
            }
        };
        if !list
            && resource.immutable
            && let Some(domain) = &contract.isolation.plan().managed_execution
            && let DiscoveryReadContent::Resource { digest, bytes, .. } = &content
        {
            domain.verify_resource_file_until(
                &resource.root,
                &target,
                digest,
                *bytes,
                deadline,
                crate::BUILD_ID,
            )?;
        }
        self.check_deadline(deadline)?;
        Ok((value, content))
    }
    pub(super) fn verify(
        &self,
        contract: &WorkerContract,
        content: &DiscoveryReadContent,
        deadline: Instant,
    ) -> Result<()> {
        let (handle, path, list) = match content {
            DiscoveryReadContent::Resource { handle, path, .. } => (handle, path, false),
            DiscoveryReadContent::Directory { handle, path, .. } => (handle, path, true),
            _ => {
                return Err(Error::new(
                    "validation_read",
                    "reference is not a resource observation",
                ));
            }
        };
        let (_, actual) = self.observe(contract, handle, path, list, deadline)?;
        if &actual != content {
            return Err(changed());
        }
        Ok(())
    }
}
impl Resource {
    fn check(&self, contract: &WorkerContract, deadline: Instant) -> Result<()> {
        current(deadline)?;
        check_path(contract, &self.root)?;
        let metadata = open(&self.root, Some(deadline))?
            .metadata()
            .map_err(|e| Error::io("recheck admitted resource", e))?;
        if Stamp::of(&metadata).identity() != self.identity {
            return Err(changed());
        }
        Ok(())
    }
    fn target(&self, path: &str, list: bool) -> Result<PathBuf> {
        if path == "." {
            if list && !self.directory {
                return Err(denied());
            }
            return Ok(self.root.clone());
        }
        if !self.directory || !normalized(path) {
            return Err(denied());
        }
        Ok(self.root.join(path))
    }
}
pub(super) fn normalized(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.contains(['\0', '\\'])
        && !Path::new(path).is_absolute()
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != ".." && !p.eq_ignore_ascii_case(".git"))
}
fn check_path(contract: &WorkerContract, path: &Path) -> Result<()> {
    if !path.is_absolute() || path.components().any(|c|matches!(c,Component::Normal(n) if n.to_str().is_some_and(|s|s.eq_ignore_ascii_case(".git"))))
        || contract.isolation.plan().forbidden_roots.iter().any(|f|path.starts_with(f))
        || !contract.isolation.plan().read_roots.iter().any(|g|path==g.path || g.directory && path.starts_with(&g.path)) {return Err(denied());}
    Ok(())
}
fn encoded_len(value: &Value) -> Result<usize> {
    Ok(serde_json::to_vec(value)
        .map_err(|_| Error::invalid("resource encoding"))?
        .len())
}
// O_NONBLOCK prevents a replaced final FIFO/device from blocking before fstat.
// Every component is descriptor-relative and nofollow, including the root.
fn open(path: &Path, deadline: Option<Instant>) -> Result<File> {
    let mut held = File::open("/").map_err(|e| Error::io("open inspection root", e))?;
    let parts = path
        .components()
        .filter_map(|p| {
            if let Component::Normal(n) = p {
                Some(n)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    for (i, name) in parts.iter().enumerate() {
        if let Some(deadline) = deadline {
            current(deadline)?;
        }
        let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| denied())?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if i + 1 < parts.len() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // SAFETY: held directory and one bounded NUL-terminated component remain live.
        let fd = unsafe { libc::openat(held.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(Error::io(
                "open original resource without links",
                std::io::Error::last_os_error(),
            ));
        }
        // SAFETY: openat returned a newly owned descriptor.
        held = unsafe { File::from_raw_fd(fd) };
    }
    Ok(held)
}
struct Directory(*mut libc::DIR);
impl Drop for Directory {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.0);
        }
    }
}
fn list_directory(
    file: &File,
    state: &ResourceState,
    contract: &WorkerContract,
    target: &Path,
    deadline: Instant,
) -> Result<Vec<(Value, Stamp)>> {
    // SAFETY: duplicate of a live owned directory; fdopendir owns the duplicate.
    let fd = unsafe {
        libc::openat(
            file.as_raw_fd(),
            c".".as_ptr(),
            libc::O_RDONLY
                | libc::O_DIRECTORY
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(Error::io(
            "duplicate resource directory",
            std::io::Error::last_os_error(),
        ));
    }
    let raw = unsafe { libc::fdopendir(fd) };
    if raw.is_null() {
        unsafe {
            libc::close(fd);
        }
        return Err(Error::io(
            "read resource directory",
            std::io::Error::last_os_error(),
        ));
    }
    let dir = Directory(raw);
    let mut entries = Vec::new();
    let mut count = 0usize;
    loop {
        current(deadline)?;
        // SAFETY: this thread owns the DIR stream and errno slot.
        unsafe {
            *errno() = 0;
        }
        let raw = unsafe { libc::readdir(dir.0) };
        if raw.is_null() {
            if unsafe { *errno() } != 0 {
                return Err(Error::io(
                    "enumerate resource directory",
                    std::io::Error::last_os_error(),
                ));
            }
            break;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*raw).d_name.as_ptr()) };
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        count += 1;
        state.charge(name.to_bytes().len(), 1, deadline)?;
        if count > MAX_ENTRIES {
            return Err(Error::new(
                "tool_limit",
                "complete directory listing exceeds entry bound",
            ));
        }
        let name_text = name
            .to_str()
            .map_err(|_| Error::new("invalid_input", "resource name is not UTF-8"))?;
        if name_text.eq_ignore_ascii_case(".git") {
            continue;
        }
        check_path(contract, &target.join(name_text))?;
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                file.as_raw_fd(),
                name.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(Error::io(
                "observe resource entry",
                std::io::Error::last_os_error(),
            ));
        }
        let m = unsafe { metadata.assume_init() };
        let kind = match m.st_mode as u32 & libc::S_IFMT as u32 {
            k if k == libc::S_IFDIR as u32 => "directory",
            k if k == libc::S_IFREG as u32 => "file",
            k if k == libc::S_IFLNK as u32 => "symlink",
            _ => "unsupported",
        };
        let stamp = stat_stamp(&m);
        let value = json!({"name":name_text,"kind":kind,"bytes":stamp.bytes});
        entries.push((value, stamp));
    }
    entries.sort_by(|a, b| a.0["name"].as_str().cmp(&b.0["name"].as_str()));
    if entries.windows(2).any(|w| w[0].0["name"] == w[1].0["name"]) {
        return Err(changed());
    }
    Ok(entries)
}
#[cfg(target_os = "macos")]
unsafe fn errno() -> *mut libc::c_int {
    unsafe { libc::__error() }
}
#[cfg(not(target_os = "macos"))]
unsafe fn errno() -> *mut libc::c_int {
    unsafe { libc::__errno_location() }
}
#[cfg(target_os = "macos")]
fn stat_stamp(m: &libc::stat) -> Stamp {
    Stamp {
        device: m.st_dev as u64,
        inode: m.st_ino,
        mode: m.st_mode as u32,
        links: m.st_nlink as u64,
        bytes: m.st_size as u64,
        modified: (m.st_mtime, m.st_mtime_nsec),
        changed: (m.st_ctime, m.st_ctime_nsec),
    }
}
#[cfg(not(target_os = "macos"))]
fn stat_stamp(m: &libc::stat) -> Stamp {
    Stamp {
        device: m.st_dev,
        inode: m.st_ino,
        mode: m.st_mode,
        links: m.st_nlink,
        bytes: m.st_size as u64,
        modified: (m.st_mtime, m.st_mtime_nsec),
        changed: (m.st_ctime, m.st_ctime_nsec),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    handle: String,
    path: String,
}
impl ScopedWorker {
    pub(crate) fn bind_inspection_deadline(&mut self, deadline: Instant) {
        self.resources.lower_deadline(deadline);
    }
    fn finish_inspection(&self, deadline: Instant) -> Result<()> {
        #[cfg(test)]
        if let Some(observed) = &self.inspection_observed {
            observed();
        }
        self.control.validate_permit(&self.permit, || {
            self.resources.check_deadline(deadline)?;
            self.fresh()?;
            self.resources.check_deadline(deadline)
        })
    }
    pub(super) fn execution_environment(&self) -> Result<Value> {
        let deadline = self.resources.deadline()?;
        let value = self.resources.environment(&self.contract, deadline)?;
        self.finish_inspection(deadline)?;
        Ok(value)
    }
    pub(super) fn inspect_resource(&mut self, call: &ToolCall, id: Uuid) -> Result<Value> {
        let args: Args = parse(&call.arguments)?;
        let deadline = self.resources.deadline()?;
        let (value, content) = self.resources.observe(
            &self.contract,
            &args.handle,
            &args.path,
            call.name == "resource_list",
            deadline,
        )?;
        self.finish_inspection(deadline)?;
        self.retain_resource_read(call, id, &value, content)?;
        Ok(value)
    }
}
