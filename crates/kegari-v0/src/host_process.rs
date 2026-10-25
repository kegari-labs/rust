//! Sealed control-host process authority for bounded read-only recovery. This
//! authority never grants model work, repository mutation or an orchestration lease.
use crate::{
    app::ConfigurationControl,
    control::{Actor, ControlHost, HostProcessToken, MutationRequest, Receipt},
    digest::canonical_digest,
    error::{Error, Result},
    execution::CommandCancellation,
    git::VerifiedHostReadRoot,
    platform::{BinaryIdentity, ProcessIdentity, Registration},
    process_records::{ProcessJournal, SpawnBinding, SpawnGuard},
    worker::ExecutionRegistry,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostProcessIdentity {
    pub runtime_id: Uuid,
    pub owner_generation: u64,
    pub process: ProcessIdentity,
    pub operation_id: Uuid,
    pub work_key: String,
    pub scope_digest: String,
    pub config_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_receipt: Option<String>,
}
#[derive(Clone)]
pub struct HostProcessServices {
    pub control: Arc<ControlHost>,
    pub registration: Arc<Registration>,
    pub configuration: Arc<dyn ConfigurationControl>,
    pub processes: Arc<ProcessJournal>,
    pub executions: Arc<ExecutionRegistry>,
}
pub struct MetadataRequest {
    pub request_id: Uuid,
    pub work_key: String,
    pub roots: Vec<VerifiedHostReadRoot>,
    pub git_binary: BinaryIdentity,
}
#[derive(Default)]
struct OwnedState {
    closing: bool,
    handles: BTreeMap<Uuid, Arc<dyn CommandCancellation>>,
}
pub struct LocalProcessPermit {
    services: HostProcessServices,
    identity: HostProcessIdentity,
    token: Arc<HostProcessToken>,
    roots: Vec<VerifiedHostReadRoot>,
    binary: BinaryIdentity,
    owned: Mutex<OwnedState>,
    cleanup: Option<Arc<crate::housekeeping::VerifiedCleanupExecution>>,
    cleanup_root: Option<crate::housekeeping::VerifiedCleanupReadRoot>,
}
impl LocalProcessPermit {
    pub fn metadata(services: HostProcessServices, request: MetadataRequest) -> Result<Arc<Self>> {
        if request.request_id.is_nil()
            || request.work_key.is_empty()
            || request.work_key.len() > 512
            || request.roots.is_empty()
            || request.roots.len() > 64
        {
            return Err(Error::invalid(
                "metadata requires bounded exact roots, work and request identity",
            ));
        }
        services.registration.verify_control_owner()?;
        let owner = services.registration.snapshot()?;
        let process = owner.control_process.ok_or_else(|| {
            Error::new("ownership_conflict", "registered control process is absent")
        })?;
        let revision = services.configuration.snapshot()?.revision()?;
        let mut roots = BTreeSet::new();
        for root in &request.roots {
            if root.config_revision() != revision || !roots.insert(root.root().to_path_buf()) {
                return Err(Error::new(
                    "stale_config",
                    "metadata root belongs to another configuration or is duplicated",
                ));
            }
            root.revalidate_filesystem()?;
        }
        request.git_binary.revalidate(true)?;
        let scope_digest = canonical_digest(
            1,
            &json!({"kind":"read_only_git_metadata","roots":request.roots.iter().map(VerifiedHostReadRoot::digest).collect::<Result<Vec<_>>>()?,"binary":request.git_binary}),
        )?;
        let identity = HostProcessIdentity {
            runtime_id: owner.runtime_id,
            owner_generation: owner.owner_generation,
            process,
            operation_id: request.request_id,
            work_key: request.work_key,
            scope_digest,
            config_revision: revision,
            cleanup_receipt: None,
        };
        let actor = host_actor(&identity)?;
        let mutation = MutationRequest {
            request_id: identity.operation_id,
            runtime_id: identity.runtime_id,
            actor: Actor {
                identity: actor,
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: "host_metadata".into(),
            canonical_targets: vec![identity.work_key.clone()],
            expected: BTreeMap::from([
                ("scope".into(), identity.scope_digest.clone()),
                ("host_identity".into(), canonical_digest(1, &identity)?),
            ]),
            payload: json!({"identity":identity,"read_only":true}),
        };
        let token =
            services
                .control
                .begin_host_process(mutation, identity.owner_generation, |_| {
                    verify_identity(&services, &identity)?;
                    for root in &request.roots {
                        root.revalidate_filesystem()?;
                    }
                    Ok(())
                })?;
        Ok(Arc::new(Self {
            services,
            identity,
            token: Arc::new(token),
            roots: request.roots,
            binary: request.git_binary,
            owned: Mutex::new(OwnedState::default()),
            cleanup: None,
            cleanup_root: None,
        }))
    }
    pub(crate) fn cleanup(
        services: HostProcessServices,
        proof: Arc<crate::housekeeping::VerifiedCleanupExecution>,
        root: crate::housekeeping::VerifiedCleanupReadRoot,
        binary: BinaryIdentity,
    ) -> Result<Arc<Self>> {
        proof.validate_current()?;
        services.registration.verify_control_owner()?;
        let owner = services.registration.snapshot()?;
        let configuration = services.configuration.snapshot()?;
        if proof.plan().runtime_id != owner.runtime_id
            || root.operation_id() != proof.plan().operation_id
            || (root.root() != proof.plan().source && root.root() != proof.plan().retained)
        {
            return Err(Error::new(
                "scope_violation",
                "cleanup native scope differs from sealed plan",
            ));
        }
        let read = crate::git::cleanup_host_read_root(&root, &configuration)?;
        binary.revalidate(true)?;
        let identity = HostProcessIdentity {
            runtime_id: owner.runtime_id,
            owner_generation: owner.owner_generation,
            process: owner
                .control_process
                .ok_or_else(|| Error::new("ownership_conflict", "control process absent"))?,
            operation_id: proof.receipt().operation_id,
            work_key: format!("cleanup:{}", proof.plan().operation_id),
            scope_digest: canonical_digest(
                1,
                &json!({"plan":proof.plan().digest(),"root":read.digest()?,"binary":binary}),
            )?,
            config_revision: configuration.revision()?,
            cleanup_receipt: Some(proof.receipt().payload_digest.clone()),
        };
        Ok(Arc::new(Self {
            services,
            identity,
            token: proof.token(),
            roots: vec![read],
            binary,
            owned: Mutex::new(OwnedState::default()),
            cleanup: Some(proof),
            cleanup_root: Some(root),
        }))
    }
    fn current(&self, state: &crate::control::ControlSnapshot) -> Result<()> {
        verify_identity(&self.services, &self.identity)?;
        if let Some(cleanup) = &self.cleanup {
            cleanup.validate_snapshot(state)?;
        }
        if let Some(root) = &self.cleanup_root {
            root.revalidate_filesystem()?;
        }
        Ok(())
    }
    pub fn identity(&self) -> &HostProcessIdentity {
        &self.identity
    }
    pub fn binary(&self) -> &BinaryIdentity {
        &self.binary
    }
    pub fn root(&self) -> &Path {
        self.roots[0].root()
    }
    pub fn validate_root(&self, path: &Path) -> Result<()> {
        let root = self
            .roots
            .iter()
            .find(|root| root.root() == path)
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "metadata cwd is outside exact authenticated read roots",
                )
            })?;
        root.revalidate_filesystem()
    }
    /// Actual native start and owned handle registration are atomic with force,
    /// service shutdown and owner checks. The callback may spawn, never wait.
    pub(crate) fn start<T>(
        &self,
        cwd: &Path,
        argv: &[String],
        cancellation: Arc<dyn CommandCancellation>,
        launch: impl FnOnce(SpawnGuard) -> Result<T>,
    ) -> Result<T> {
        let version = validate_metadata_argv(argv)?;
        if version {
            if self.binary.path.parent() != Some(cwd) {
                return Err(Error::new(
                    "scope_violation",
                    "Git version probe cwd differs from pinned binary",
                ));
            }
        } else if argv.iter().any(|value| value == "check-ref-format") && cwd == Path::new("/") {
            // This fixed syntax-only command reads no repository.
        } else {
            self.validate_root(cwd)?;
        }
        self.binary.revalidate(true)?;
        let effect = Uuid::new_v4();
        let result = self.services.control.with_host_process(
            &self.token,
            |state| self.current(state),
            || {
                let mut owned = self.owned.lock().map_err(|_| {
                    Error::new("ownership_conflict", "host process handles unavailable")
                })?;
                if owned.closing || owned.handles.len() >= 4096 {
                    return Err(Error::new(
                        "admission_closed",
                        "metadata process scope closed or exhausted",
                    ));
                }
                if !version
                    && !(argv.iter().any(|value| value == "check-ref-format")
                        && cwd == Path::new("/"))
                {
                    self.validate_root(cwd)?;
                }
                let binding = SpawnBinding {
                    operation_id: self.identity.operation_id,
                    effect_id: effect,
                    lease: None,
                    control_owner: Some(self.identity.clone()),
                    work_key: self.identity.work_key.clone(),
                    scope_digest: self.identity.scope_digest.clone(),
                    cwd: cwd.to_path_buf(),
                    executable: self.binary.clone(),
                    observed_executables: vec![],
                    argv_digest: canonical_digest(1, argv)?,
                    kind: "host_git_metadata".into(),
                };
                let guard = self.services.processes.begin_control(binding, self)?;
                owned.handles.insert(effect, cancellation.clone());
                self.services.executions.register(
                    self.identity.operation_id,
                    effect,
                    cancellation,
                )?;
                launch(guard)
            },
        );
        if result.is_err() && self.services.control.fence_host_process(&self.token)? {
            self.services
                .executions
                .cancel_fenced(&self.services.control, self.identity.operation_id)?;
        }
        result
    }
    pub(crate) fn start_cleanup_command(
        &self,
        isolation: &crate::isolation::ValidatedIsolation,
        argv: &[String],
        timeout: std::time::Duration,
        output_limit: usize,
    ) -> Result<crate::isolation::OwnedCommand> {
        crate::isolation::repository_command_prerequisite()?;
        let cleanup = self.cleanup.as_ref().ok_or_else(|| {
            Error::new(
                "scope_violation",
                "metadata authority cannot execute removal hooks",
            )
        })?;
        let (expected, expected_argv, expected_timeout) = cleanup.command_identity()?;
        if expected.digest() != isolation.digest()
            || expected_argv != argv
            || timeout != expected_timeout
            || output_limit > 16 * 1024
        {
            return Err(Error::new(
                "scope_violation",
                "removal hook differs from sealed cleanup workflow",
            ));
        }
        let launch_identity = crate::isolation::command_launch_identity(isolation, argv)?;
        let effect = Uuid::new_v4();
        let slot = Arc::new(CleanupCancellation::default());
        let result = self.services.control.with_host_process(
            &self.token,
            |state| self.current(state),
            || {
                let mut owned = self.owned.lock().map_err(|_| {
                    Error::new("ownership_conflict", "cleanup handle registry unavailable")
                })?;
                if owned.closing || owned.handles.len() >= 4096 {
                    return Err(Error::new(
                        "admission_closed",
                        "cleanup scope closed or exhausted",
                    ));
                }
                let binding = SpawnBinding {
                    operation_id: self.identity.operation_id,
                    effect_id: effect,
                    lease: None,
                    control_owner: Some(self.identity.clone()),
                    work_key: self.identity.work_key.clone(),
                    scope_digest: self.identity.scope_digest.clone(),
                    cwd: launch_identity.cwd,
                    executable: launch_identity.launcher,
                    observed_executables: vec![launch_identity.target],
                    argv_digest: launch_identity.argv_digest,
                    kind: "repository_command".into(),
                };
                let guard = self.services.processes.begin_control(binding, self)?;
                owned.handles.insert(effect, slot.clone());
                self.services.executions.register(
                    self.identity.operation_id,
                    effect,
                    slot.clone(),
                )?;
                let command = crate::isolation::start_guarded_command(
                    isolation,
                    argv,
                    timeout,
                    output_limit,
                    &[],
                    guard,
                )?;
                *slot.0.lock().map_err(|_| {
                    Error::new("ownership_conflict", "cleanup child handle unavailable")
                })? = Some(Arc::new(command.cancellation()));
                Ok(command)
            },
        );
        if result.is_err() && self.services.control.fence_host_process(&self.token)? {
            self.services
                .executions
                .cancel_fenced(&self.services.control, self.identity.operation_id)?;
        }
        result
    }
    /// Close further starts before inspecting actual owned handles. Unknown exit
    /// preserves the durable pending local receipt and never acknowledges it.
    pub fn finish(&self, observation: Value) -> Result<Receipt> {
        // Finishing a fenced process is allowed; no new execution is launched.
        let handles = {
            let mut owned = self.owned.lock().map_err(|_| {
                Error::new("ownership_conflict", "host process handles unavailable")
            })?;
            owned.closing = true;
            owned.handles.values().cloned().collect::<Vec<_>>()
        };
        for handle in handles {
            if !handle.exited()? {
                return Err(Error::new(
                    "operation_uncertain",
                    "host metadata process has not actually exited",
                ));
            }
        }
        self.services
            .executions
            .observe_exited(self.identity.operation_id)?;
        if self
            .services
            .executions
            .active_operations()?
            .contains(&self.identity.operation_id)
        {
            return Err(Error::new(
                "operation_uncertain",
                "host process registry retains an unresolved handle",
            ));
        }
        let receipt = if let Some(cleanup) = &self.cleanup {
            cleanup.validate_current()?;
            cleanup.receipt().clone()
        } else {
            self.services.control.complete_host_process(&self.token, || self.services.registration.verify_control_owner(),
                json!({"metadata_observation":observation,"delivery_evidence":false,"owned_handles_joined":true}))?
        };
        self.owned
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "host process handles unavailable"))?
            .handles
            .clear();
        Ok(receipt)
    }
    pub(crate) fn validate_journal_binding(&self, binding: &SpawnBinding) -> Result<()> {
        if binding.control_owner.as_ref() != Some(&self.identity)
            || binding.lease.is_some()
            || binding.operation_id != self.identity.operation_id
            || binding.work_key != self.identity.work_key
            || binding.scope_digest != self.identity.scope_digest
        {
            return Err(Error::new(
                "scope_violation",
                "host journal differs from sealed authority",
            ));
        }
        if binding.kind == "host_git_metadata"
            && binding.executable == self.binary
            && binding.observed_executables.is_empty()
        {
            return Ok(());
        }
        if binding.kind == "repository_command"
            && let Some(cleanup) = &self.cleanup
        {
            let (isolation, argv, _) = cleanup.command_identity()?;
            let expected = crate::isolation::command_launch_identity(&isolation, &argv)?;
            if binding.cwd == expected.cwd
                && binding.executable == expected.launcher
                && binding.observed_executables == [expected.target]
                && binding.argv_digest == expected.argv_digest
            {
                return Ok(());
            }
        }
        Err(Error::new(
            "scope_violation",
            "host process journal command differs from its exact grant",
        ))
    }
}
#[derive(Default)]
struct CleanupCancellation(Mutex<Option<Arc<dyn CommandCancellation>>>);
impl CommandCancellation for CleanupCancellation {
    fn cancel(&self) -> Result<()> {
        if let Some(handle) = self
            .0
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "cleanup child handle unavailable"))?
            .as_ref()
        {
            handle.cancel()?;
        }
        Ok(())
    }
    fn exited(&self) -> Result<bool> {
        match self
            .0
            .lock()
            .map_err(|_| Error::new("ownership_conflict", "cleanup child handle unavailable"))?
            .as_ref()
        {
            Some(handle) => handle.exited(),
            None => Ok(false),
        }
    }
}
pub(crate) fn host_actor(identity: &HostProcessIdentity) -> Result<String> {
    Ok(format!(
        "control:{}",
        canonical_digest(1, &identity.process)?
    ))
}
fn verify_identity(services: &HostProcessServices, identity: &HostProcessIdentity) -> Result<()> {
    services.registration.verify_control_owner()?;
    let owner = services.registration.snapshot()?;
    if owner.runtime_id != identity.runtime_id
        || owner.owner_generation != identity.owner_generation
        || owner.control_process.as_ref() != Some(&identity.process)
        || services.configuration.snapshot()?.revision()? != identity.config_revision
    {
        return Err(Error::new(
            "ownership_conflict",
            "registered metadata process owner or configuration changed",
        ));
    }
    Ok(())
}
/// Exactly the trusted metadata grammar used for retained Git provenance. Fixed
/// defensive -c settings cannot be replaced with repository-controlled commands.
pub(crate) fn validate_metadata_argv(argv: &[String]) -> Result<bool> {
    if argv == ["--version"] {
        return Ok(true);
    }
    const SETTINGS: &[&str] = &[
        "core.hooksPath=/dev/null",
        "core.fsmonitor=false",
        "core.autocrlf=false",
        "core.filemode=true",
        "core.symlinks=true",
        "core.quotePath=true",
        "color.ui=false",
        "diff.external=",
        "diff.orderFile=/dev/null",
        "apply.ignoreWhitespace=false",
        "protocol.allow=never",
        "protocol.file.allow=always",
    ];
    let prefix: Vec<String> = SETTINGS
        .iter()
        .flat_map(|setting| ["-c".to_string(), setting.to_string()])
        .collect();
    if !argv.starts_with(&prefix) {
        return Err(Error::new(
            "scope_violation",
            "metadata Git lacks the exact defensive configuration",
        ));
    }
    let args = &argv[prefix.len()..];
    let oid = |v: &str| crate::git::validate_oid(v).is_ok();
    let revision = |v: &str| {
        v == "HEAD"
            || oid(v)
            || ["^{commit}", "^{tree}"].iter().any(|suffix| {
                v.strip_suffix(suffix)
                    .is_some_and(|v| v == "HEAD" || oid(v))
            })
    };
    let valid = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["rev-parse", "--verify", value] | ["rev-parse", "--verify", "--end-of-options", value] => {
            revision(value)
        }
        ["merge-base", "--is-ancestor", base, head] => oid(base) && oid(head),
        ["rev-list", "--parents", "-n", "1", head] => oid(head),
        [
            "diff",
            "--binary",
            "--full-index",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            base,
            head,
            "--",
        ] => oid(base) && oid(head),
        [
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
        ] => true,
        ["status", "--porcelain=v1", "--untracked-files=all"]
        | ["for-each-ref", "--format=%(objectname)", "refs/heads"]
        | ["fsck", "--full", "--no-reflogs"] => true,
        ["ls-tree", "-r", "-z", head] => oid(head),
        ["cat-file", "commit" | "blob", object] => oid(object),
        ["config", "--local", "--get", "user.name" | "user.email"] => true,
        ["check-ref-format", branch] => {
            branch.starts_with("refs/heads/")
                && branch.len() <= 4096
                && !branch.chars().any(char::is_control)
        }
        _ => false,
    };
    if !valid {
        return Err(Error::new(
            "scope_violation",
            "host metadata scope refuses this Git operation",
        ));
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        control::{Intent, Lifecycle, ReceiptPhase},
        storage::PrivateDir,
    };
    fn request(runtime: Uuid, id: Uuid, kind: &str) -> MutationRequest {
        MutationRequest {
            request_id: id,
            runtime_id: runtime,
            actor: Actor {
                identity: "fixture-control-owner".into(),
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: kind.into(),
            canonical_targets: vec!["exact-owned-work".into()],
            expected: BTreeMap::new(),
            payload: json!({"read_only":true}),
        }
    }
    fn lifecycle_request(runtime: Uuid) -> MutationRequest {
        let mut request = request(runtime, Uuid::new_v4(), "fixture");
        request.actor.roles.insert("lifecycle".into());
        request.canonical_targets = vec![runtime.to_string()];
        request
    }
    #[test]
    fn stopped_host_metadata_is_separate_from_worker_authority_and_blocks_service_stop() {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Uuid::new_v4();
        let host = ControlHost::setup(
            PrivateDir::create(&temp.path().canonicalize().unwrap().join("host")).unwrap(),
            runtime,
            Uuid::new_v4(),
        )
        .unwrap();
        let operation = Uuid::new_v4();
        let token = host
            .begin_host_process(
                request(runtime, operation, "host_metadata"),
                7,
                |snapshot| {
                    assert_eq!(snapshot.intent.desired, Intent::Stopped);
                    assert!(snapshot.active_operations.is_empty());
                    Ok(())
                },
            )
            .unwrap();
        assert!(host.snapshot().unwrap().active_operations.is_empty());
        assert_eq!(
            host.snapshot().unwrap().pending_local_processes,
            vec![operation]
        );
        assert_eq!(
            host.stop_service(lifecycle_request(runtime), || panic!(
                "live local metadata must prevent service reservation"
            ))
            .unwrap_err()
            .code,
            "stop_in_progress"
        );
        assert_eq!(
            host.begin_host_process(request(runtime, operation, "host_metadata"), 7, |_| Ok(()))
                .err()
                .unwrap()
                .code,
            "operation_uncertain"
        );
        let mut recover = request(runtime, Uuid::new_v4(), "fixture").actor;
        recover.roles.insert("recover".into());
        assert_eq!(
            host.reconcile_operation_observed(&recover, operation, |_, _| panic!(
                "live host handle cannot be reconciled"
            ))
            .unwrap_err()
            .code,
            "operation_active"
        );
        host.lifecycle(lifecycle_request(runtime), Lifecycle::ForceStop, 0)
            .unwrap();
        assert!(
            host.snapshot()
                .unwrap()
                .fenced_operations
                .contains(&operation)
        );
        assert_eq!(
            host.with_host_process::<()>(
                &token,
                |_| Ok(()),
                || panic!("fenced host process must never spawn")
            )
            .unwrap_err()
            .code,
            "access_denied"
        );
        host.complete_host_process(&token, || Ok(()), json!({"fixture_actual_join":true}))
            .unwrap();
        assert!(host.snapshot().unwrap().pending_local_processes.is_empty());
        assert_eq!(
            host.operation(operation).unwrap().phase,
            ReceiptPhase::Acknowledged
        );
    }
    #[test]
    fn recovered_local_metadata_remains_pending_without_recreating_a_permit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap().join("host");
        let runtime = Uuid::new_v4();
        let operation = Uuid::new_v4();
        let host = ControlHost::setup(PrivateDir::create(&path).unwrap(), runtime, Uuid::new_v4())
            .unwrap();
        let _token = host
            .begin_host_process(request(runtime, operation, "host_metadata"), 1, |_| Ok(()))
            .unwrap();
        drop(host);
        let recovered = ControlHost::recover(PrivateDir::open(&path).unwrap(), runtime).unwrap();
        assert_eq!(
            recovered.snapshot().unwrap().pending_local_processes,
            vec![operation]
        );
        assert!(
            recovered
                .snapshot()
                .unwrap()
                .unresolved_operations
                .contains_key(&operation)
        );
        assert_eq!(
            recovered
                .stop_service(lifecycle_request(runtime), || panic!(
                    "unknown prior local process blocks shutdown"
                ))
                .unwrap_err()
                .code,
            "stop_in_progress"
        );
    }
    #[test]
    fn metadata_grammar_rejects_writes_external_helpers_paths_and_config_substitution() {
        let settings = [
            "core.hooksPath=/dev/null",
            "core.fsmonitor=false",
            "core.autocrlf=false",
            "core.filemode=true",
            "core.symlinks=true",
            "core.quotePath=true",
            "color.ui=false",
            "diff.external=",
            "diff.orderFile=/dev/null",
            "apply.ignoreWhitespace=false",
            "protocol.allow=never",
            "protocol.file.allow=always",
        ];
        let prefix: Vec<String> = settings
            .iter()
            .flat_map(|value| ["-c".into(), value.to_string()])
            .collect();
        let check = |args: &[&str]| {
            let mut argv = prefix.clone();
            argv.extend(args.iter().map(|v| v.to_string()));
            validate_metadata_argv(&argv)
        };
        let oid = "a".repeat(40);
        assert!(validate_metadata_argv(&["--version".into()]).unwrap());
        for args in [
            vec!["rev-parse", "--verify", "HEAD^{tree}"],
            vec!["cat-file", "blob", &oid],
            vec!["merge-base", "--is-ancestor", &oid, &oid],
            vec!["check-ref-format", "refs/heads/exact-derived-name"],
        ] {
            assert!(!check(&args).unwrap());
        }
        for args in [
            vec!["checkout", "HEAD"],
            vec!["update-ref", "refs/heads/main", &oid],
            vec!["cat-file", "--filters", &oid],
            vec!["diff", "--no-index", "/etc/passwd", "/tmp/output"],
            vec!["rev-parse", "--git-path", "/etc/passwd"],
            vec!["submodule", "update"],
            vec!["fetch", "https://example.invalid/repo"],
        ] {
            assert_eq!(check(&args).unwrap_err().code, "scope_violation");
        }
        let mut changed = prefix;
        changed[1] = "core.hooksPath=/arbitrary".into();
        changed.extend(["status".into()]);
        assert!(validate_metadata_argv(&changed).is_err());
    }
}
