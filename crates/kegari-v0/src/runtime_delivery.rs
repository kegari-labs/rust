//! Runtime delivery composition. Every reconstructed gate comes from current
//! providers plus authenticated retained execution records, never tracker state.
use crate::{
    config::Config,
    connections::ConnectionRegistry,
    delivery::{
        DeliveryCoordinator, DeliveryServices, GateCheck, ParentHydrationInputs, RepositoryContract,
    },
    deployment::ProductionObserver,
    error::{Error, Result},
    git::{Git, GitLimits, HostGitExecutionScope, RepositoryId},
    host_process::{HostProcessServices, LocalProcessPermit, MetadataRequest},
    process_records::ProcessJournal,
    reconcile::{FreshGate, GatePurpose, ParentProjection, RuntimeProjection},
    runtime::RuntimeCoordinator,
    storage::PrivateDir,
    workflow::Workflow,
};
use chrono::{Duration, Utc};
use serde_json::json;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use uuid::Uuid;

type PhaseSetupHolds =
    std::sync::Mutex<BTreeMap<Uuid, (crate::control::Permit, Arc<std::sync::atomic::AtomicBool>)>>;
pub struct RuntimeDelivery {
    runtime: Arc<RuntimeCoordinator>,
    processes: Arc<ProcessJournal>,
    records: Arc<PrivateDir>,
    refresh_lock: tokio::sync::Mutex<()>,
    children: child::Children,
    parents: parent::Parents,
    releases: release::Releases,
    phase_holds: PhaseSetupHolds,
    #[cfg(test)]
    fixture_git: Option<PathBuf>,
}
#[derive(Clone)]
struct WorkspaceDelivery {
    configuration: Config,
    coordinator: Arc<DeliveryCoordinator>,
    observer: Arc<ProductionObserver>,
    contracts: Vec<RepositoryContract>,
    workflow: Workflow,
    git: Arc<Git>,
    records: Arc<PrivateDir>,
}
impl RuntimeDelivery {
    /// Construction performs no provider request or child process launch. Native
    /// process identity prerequisites remain explicit and are never synthesized.
    pub fn new(runtime: Arc<RuntimeCoordinator>) -> Result<Arc<Self>> {
        let configuration = runtime.configuration.snapshot()?;
        let processes = Arc::new(ProcessJournal::new(
            runtime.registration.root().child("worker-processes")?,
            configuration.runtime.id,
        )?);
        let records = Arc::new(runtime.registration.root().child("delivery-runtime")?);
        let service = Arc::new(Self {
            runtime,
            processes,
            records,
            refresh_lock: tokio::sync::Mutex::new(()),
            children: child::Children::default(),
            parents: parent::Parents::default(),
            releases: release::Releases::default(),
            phase_holds: std::sync::Mutex::new(BTreeMap::new()),
            #[cfg(test)]
            fixture_git: None,
        });
        service
            .runtime
            .services
            .housekeeping
            .install_processes(Arc::new(
                crate::housekeeping::NativeCleanupProcesses::for_housekeeping(
                    service.metadata_services(),
                    &service.runtime.services.housekeeping,
                )?,
            ))?;
        Ok(service)
    }
    #[cfg(test)]
    fn with_fixture(
        runtime: Arc<RuntimeCoordinator>,
        processes: Arc<ProcessJournal>,
        git: PathBuf,
    ) -> Result<Arc<Self>> {
        let records = Arc::new(runtime.registration.root().child("delivery-runtime")?);
        Ok(Arc::new(Self {
            runtime,
            processes,
            records,
            refresh_lock: tokio::sync::Mutex::new(()),
            children: child::Children::default(),
            parents: parent::Parents::default(),
            releases: release::Releases::default(),
            phase_holds: std::sync::Mutex::new(BTreeMap::new()),
            fixture_git: Some(git),
        }))
    }
    pub(crate) fn processes(&self) -> Arc<ProcessJournal> {
        self.processes.clone()
    }
    fn git_path(&self) -> Result<PathBuf> {
        #[cfg(test)]
        if let Some(path) = &self.fixture_git {
            return Ok(path.clone());
        }
        resolve_git()
    }
    fn current_gate(&self) -> GateCheck {
        let runtime = self.runtime.clone();
        Arc::new(move |expected| {
            runtime
                .configuration
                .authority_fences()?
                .check_workspace(expected.workspace_id())?;
            if runtime.configuration.snapshot()?.revision()? != expected.config_revision() {
                return Err(Error::new("stale_config", "delivery configuration changed"));
            }
            let projection = runtime.observations.admission_snapshot()?;
            let outcomes = runtime.dependency_outcomes()?;
            let current = projection
                .parent(expected.workspace_id(), expected.work().parent)?
                .fresh_gate(
                    expected.work().child,
                    expected.purpose(),
                    &outcomes,
                    Utc::now(),
                    Duration::seconds(30),
                )?;
            expected.compare_current(&current, Utc::now(), Duration::seconds(30))
        })
    }
    fn retained_gate(&self) -> GateCheck {
        let runtime = self.runtime.clone();
        Arc::new(move |expected| {
            runtime
                .configuration
                .authority_fences()?
                .check_workspace(expected.workspace_id())?;
            runtime
                .observations
                .active_gate(expected, &runtime.dependency_outcomes()?)?;
            Ok(())
        })
    }
    fn workspace(
        &self,
        projection: &RuntimeProjection,
        workspace: Uuid,
    ) -> Result<WorkspaceDelivery> {
        let configuration = self.runtime.configuration.snapshot()?;
        if configuration.revision()? != projection.config_revision
            || configuration.runtime.id != projection.runtime_id
        {
            return Err(Error::new(
                "stale_config",
                "delivery projection differs from current configuration",
            ));
        }
        self.runtime
            .configuration
            .authority_fences()?
            .check_workspace(workspace)?;
        let selected = configuration
            .workspaces
            .values()
            .find(|w| w.id == workspace)
            .ok_or_else(|| Error::new("scope_violation", "delivery workspace was removed"))?;
        let observed = projection
            .workspaces
            .get(&workspace)
            .filter(|w| w.complete)
            .ok_or_else(|| {
                Error::new(
                    "incompletely_observed",
                    "delivery workspace observation is incomplete",
                )
            })?;
        if Utc::now() - observed.observed_at > Duration::seconds(30)
            || observed.observed_at > Utc::now() + Duration::seconds(5)
        {
            return Err(Error::new(
                "stale_result",
                "delivery provider projection expired",
            ));
        }
        let transports = configuration
            .connections
            .keys()
            .filter_map(|name| {
                self.runtime
                    .services
                    .inputs
                    .transport(name)
                    .map(|transport| (name.clone(), transport))
            })
            .collect();
        let registry = ConnectionRegistry::bind_with_transports(
            &configuration,
            self.runtime.services.secrets.as_ref(),
            |name| self.runtime.services.inputs.environment(name),
            &transports,
        )?;
        let clients = registry
            .clients(&configuration)?
            .get(&workspace)
            .cloned()
            .ok_or_else(|| {
                registry
                    .blocked()
                    .get(&workspace)
                    .cloned()
                    .unwrap_or_else(|| {
                        Error::new(
                            "missing_secret",
                            "delivery provider connection is unavailable",
                        )
                    })
            })?;
        let mut contracts = Vec::new();
        for (name, configured) in &selected.repositories {
            let repository_id: u64 = name.parse().map_err(|_| {
                Error::new(
                    "identity_mismatch",
                    "configured repository ID is not canonical",
                )
            })?;
            let repository = observed.repositories.get(&repository_id)
                .filter(|repository| repository.identity.id == repository_id && repository_id.to_string() == *name)
                .ok_or_else(|| Error::new("identity_mismatch", "configured delivery repository lacks its exact canonical provider identity"))?;
            let contract = RepositoryContract {
                configuration_name: name.clone(),
                repository: RepositoryId {
                    hostname: configured.hostname.clone(),
                    id: repository.identity.id.to_string(),
                },
                provider: repository.identity.clone(),
                configuration: configured.clone(),
            };
            contract.digest()?;
            contracts.push(contract);
        }
        let workflow = configuration
            .load_workflows()?
            .remove(&workspace)
            .ok_or_else(|| {
                Error::new("workflow_unavailable", "delivery workflow is unavailable")
            })?;
        if workflow.digest != observed.workflow_digest {
            return Err(Error::new(
                "stale_workflow",
                "delivery workflow changed after provider projection",
            ));
        }
        let git = Arc::new(Git::from_pinned_executable(self.git_path()?)?);
        let records = Arc::new(self.records.child(&workspace.to_string())?);
        let current_gate = self.current_gate();
        let coordinator = Arc::new(DeliveryCoordinator::new(
            DeliveryServices {
                host: self.runtime.control.clone(),
                broker: crate::providers::ProviderBroker {
                    linear: Some(clients.linear),
                    github: Some(clients.github.clone()),
                },
                records: records.clone(),
                key: self.runtime.services.key.clone(),
                git: git.clone(),
                retained_git_sources: contracts
                    .iter()
                    .map(|c| (c.repository.clone(), PathBuf::from(&c.configuration.source)))
                    .collect(),
                current_gate: current_gate.clone(),
                executions: self.runtime.executions.clone(),
                processes: self.processes.clone(),
            },
            contracts.clone(),
        )?);
        let observer = Arc::new(ProductionObserver::new(
            clients.github,
            self.runtime.control.clone(),
            self.runtime.services.key.clone(),
            records.child("deployment")?,
            current_gate,
        )?);
        Ok(WorkspaceDelivery {
            configuration,
            coordinator,
            observer,
            contracts,
            workflow,
            git,
            records,
        })
    }
    fn metadata_services(&self) -> HostProcessServices {
        HostProcessServices {
            control: self.runtime.control.clone(),
            registration: self.runtime.registration.clone(),
            configuration: self.runtime.configuration.clone(),
            processes: self.processes(),
            executions: self.runtime.executions.clone(),
        }
    }
    async fn readonly_coordinator_with_successor(
        &self,
        workspace: &WorkspaceDelivery,
        gate: &FreshGate,
        predecessor: Option<&KnownPredecessorBaseline>,
    ) -> Result<(Arc<DeliveryCoordinator>, Arc<LocalProcessPermit>)> {
        let mut roots = BTreeMap::new();
        for contract in &workspace.contracts {
            let root = crate::git::configured_host_read_root(
                &workspace.configuration,
                gate.workspace_id(),
                contract,
            )?;
            roots.insert(root.root().to_owned(), root);
        }
        for root in workspace.coordinator.retained_parent_read_roots(gate)? {
            if roots.insert(root.root().to_owned(), root).is_some() {
                return Err(Error::new(
                    "ownership_conflict",
                    "configured and retained repository roots overlap",
                ));
            }
        }
        if let Some(predecessor) = predecessor {
            for root in workspace
                .coordinator
                .retained_successor_host_roots(gate, predecessor)?
            {
                if let Some(existing) = roots.get(root.root()) {
                    if existing.digest()? != root.digest()? {
                        return Err(Error::new(
                            "ownership_conflict",
                            "predecessor read root overlaps another source",
                        ));
                    }
                } else {
                    roots.insert(root.root().to_owned(), root);
                }
            }
        }
        let read_coordinator = workspace.coordinator.clone();
        let read_configuration = workspace.configuration.clone();
        let read_workspace = gate.workspace_id();
        let imported = tokio::task::spawn_blocking(move || {
            read_coordinator.retained_object_read_roots(&read_configuration, read_workspace)
        })
        .await
        .map_err(|_| {
            Error::new(
                "object_validation",
                "retained source discovery did not finish",
            )
        })??;
        for root in imported {
            if roots.insert(root.root().to_owned(), root).is_some() {
                return Err(Error::new(
                    "ownership_conflict",
                    "retained object roots overlap another granted source",
                ));
            }
        }
        let registry = self.provider_broker(&workspace.configuration, gate.workspace_id())?;
        let authority = LocalProcessPermit::metadata(
            self.metadata_services(),
            MetadataRequest {
                request_id: Uuid::new_v4(),
                work_key: gate.work_key()?,
                roots: roots.into_values().collect(),
                git_binary: crate::platform::BinaryIdentity::inspect(
                    workspace.git.executable(),
                    true,
                )?,
            },
        )?;
        let git = workspace.git.clone();
        let scoped = authority.clone();
        let result = tokio::task::spawn_blocking(move || {
            git.with_host_execution_scope(HostGitExecutionScope {
                limits: GitLimits::default(),
                authority: scoped,
            })
        })
        .await
        .map_err(|_| Error::new("operation_uncertain", "read-only Git setup did not finish"))
        .and_then(|result| result);
        let git = match result {
            Ok(git) => Arc::new(git),
            Err(error) => {
                // A failed spawn may still own a process. finish independently
                // verifies actual handles and retains Sent if exit is unknown.
                authority.finish(json!({"error":error}))?;
                return Err(error);
            }
        };
        let coordinator = DeliveryCoordinator::new(
            DeliveryServices {
                host: self.runtime.control.clone(),
                broker: registry,
                records: workspace.records.clone(),
                key: self.runtime.services.key.clone(),
                git,
                retained_git_sources: workspace
                    .contracts
                    .iter()
                    .map(|c| (c.repository.clone(), PathBuf::from(&c.configuration.source)))
                    .collect(),
                current_gate: self.current_gate(),
                executions: self.runtime.executions.clone(),
                processes: self.processes.clone(),
            },
            workspace.contracts.clone(),
        );
        match coordinator {
            Ok(coordinator) => Ok((Arc::new(coordinator), authority)),
            Err(error) => {
                authority.finish(json!({"error":error}))?;
                Err(error)
            }
        }
    }
    fn provider_broker(
        &self,
        configuration: &Config,
        workspace: Uuid,
    ) -> Result<crate::providers::ProviderBroker> {
        let transports = configuration
            .connections
            .keys()
            .filter_map(|name| {
                self.runtime
                    .services
                    .inputs
                    .transport(name)
                    .map(|t| (name.clone(), t))
            })
            .collect();
        let registry = ConnectionRegistry::bind_with_transports(
            configuration,
            self.runtime.services.secrets.as_ref(),
            |name| self.runtime.services.inputs.environment(name),
            &transports,
        )?;
        let clients = registry
            .clients(configuration)?
            .get(&workspace)
            .cloned()
            .ok_or_else(|| {
                Error::new(
                    "missing_secret",
                    "delivery provider connection is unavailable",
                )
            })?;
        Ok(crate::providers::ProviderBroker {
            linear: Some(clients.linear),
            github: Some(clients.github),
        })
    }
    async fn refresh_parent(
        &self,
        workspace: &WorkspaceDelivery,
        parent: &ParentProjection,
    ) -> Result<crate::delivery::VerifiedDeliverySnapshot> {
        let outcomes = self.runtime.dependency_outcomes()?;
        let gate = parent.fresh_gate(
            None,
            GatePurpose::NewOperation,
            &outcomes,
            Utc::now(),
            Duration::seconds(30),
        )?;
        let mut child_gates = BTreeMap::new();
        for child in &parent.required_children {
            if let Ok(gate) = parent.fresh_gate(
                Some(*child),
                GatePurpose::NewOperation,
                &outcomes,
                Utc::now(),
                Duration::seconds(30),
            ) {
                child_gates.insert(*child, gate);
            }
        }
        let snapshot = self.runtime.observations.admission_snapshot()?;
        let predecessor = self.known_predecessor_baseline(&snapshot, &gate).await?;
        let observer = predecessor
            .as_ref()
            .map(|proof| workspace.observer.with_successor(proof.clone()))
            .transpose()?;
        let observer = observer.as_ref().unwrap_or(workspace.observer.as_ref());
        let (coordinator, authority) = self
            .readonly_coordinator_with_successor(workspace, &gate, predecessor.as_deref())
            .await?;
        let result = async {
            let mut integrations = Vec::new();
            if let Some(predecessor) = &predecessor {
                for baseline in predecessor.baselines() {
                    let production = baseline.production();
                    let observation = coordinator.observation(
                        gate.clone(),
                        crate::evidence::RevisionBinding {
                            repository: baseline.repository().clone(),
                            head: production.commit.clone(),
                            base: production.commit.clone(),
                            tested_commit: production.commit.clone(),
                            tested_tree: production.tree.clone(),
                        },
                    )?;
                    integrations.extend(
                        coordinator.retained_successor_integrations(&observation, predecessor)?,
                    );
                }
            }
            let contract = coordinator.parent_contract(gate, parent, &workspace.workflow)?;
            coordinator
                .refresh_delivery(
                    ParentHydrationInputs {
                        contract: &contract,
                        observer,
                        child_gates: &child_gates,
                        workflow: &workspace.workflow,
                        production_integrations: &integrations,
                    },
                    &workspace.configuration,
                )
                .await
        }
        .await;
        let completion = authority
            .finish(json!({"read_only":true,"parent":parent.key,"verified":result.is_ok()}));
        match (result, completion) {
            (Ok(snapshot), Ok(_)) => Ok(snapshot),
            (Err(error), Ok(_)) | (_, Err(error)) => Err(error),
        }
    }
    /// Replace each cached parent proof, including a verified empty projection.
    /// A failed read invalidates old readiness immediately and remains diagnostic.
    pub async fn refresh(&self, projection: &RuntimeProjection) -> Result<()> {
        let _refresh = self.refresh_lock.lock().await;
        self.refresh_current_delivery(projection).await
    }
    async fn refresh_for_inspection(&self, projection: &RuntimeProjection) -> Result<()> {
        let _refresh = self.refresh_lock.lock().await;
        self.refresh_current_delivery(projection).await
    }
    fn historical_delivery_blockers(
        &self,
        projection: &RuntimeProjection,
    ) -> Result<BTreeMap<crate::authority::IssueKey, Error>> {
        let mut blockers = BTreeMap::new();
        let mut blocked_releases = 0;
        for row in self.release_inspection_rows()? {
            // Historical observation errors on resolved records are not current
            // prerequisites. A successful record still needs its exact sealed
            // completion, even if it never had an ancestor hold. Missing removed
            // workspace prerequisites remain visible without restoring authority.
            if (row.active_hold
                || row.attempt.completed()
                || !projection.workspaces.contains_key(&row.workspace))
                && let Some(error) = row.diagnostic
            {
                blocked_releases += 1;
                blockers
                    .entry(row.parent)
                    .or_insert((row.attempt.plan.operation_id, error));
            }
        }
        // Only an authenticated seal identity makes a no-release reference
        // failure a strict queue blocker. Never-sealed deployed/legacy rows may
        // remain diagnostic without weakening their existing current gates.
        let no_release = self.no_release_completion_inventory()?;
        let mut blocked_no_release = 0;
        for row in no_release.rows {
            if no_release.sealed_operations.contains(&row.admission)
                && let Some(error) = row.diagnostic
            {
                blocked_no_release += 1;
                blockers.entry(row.parent).or_insert((row.admission, error));
            }
        }
        Ok(blockers.into_iter().map(|(parent, (operation, root_error))| {
            let error = Error::new("release_gates_unavailable",
                "Current delivery gates are unavailable while retained release evidence is unverified")
                .detail(json!({"blocked_parent":parent,"blocked_operation":operation,
                    "root_error":root_error,"blocked_release_count":blocked_releases,
                    "blocked_no_release_count":blocked_no_release}))
                .next("Inspect the per-parent release and no-release history and reconcile its exact unavailable references; do not repeat external effects.");
            (parent, error)
        }).collect())
    }
    async fn refresh_current_delivery(&self, projection: &RuntimeProjection) -> Result<()> {
        // Recover immutable dependency facts independently from strict queue
        // reconstruction; one unavailable historical proof cannot erase an
        // unrelated complete proof. This method owns no Control/release lock.
        self.sync_released_completions()?;
        self.sync_no_release_completions()?;
        let blockers = self.historical_delivery_blockers(projection)?;
        self.clear_no_release_observations()?;
        for observed in projection.workspaces.values() {
            let workspace = self.workspace(projection, observed.workspace_id);
            for parent in observed.parents.values() {
                // A retained failed identity never starts another observation
                // admission merely to rediscover its already known blocker.
                if let Some(error) = blockers.get(&parent.key) {
                    self.runtime
                        .invalidate_delivery(&parent.key, error.clone())?;
                    continue;
                }
                let result = match &workspace {
                    Ok(workspace) => self.refresh_parent(workspace, parent).await,
                    Err(error) => Err(error.clone()),
                };
                match result {
                    Ok(mut snapshot) => {
                        if let Ok(workspace) = &workspace
                            && let Err(error) = self
                                .hydrate_no_release(workspace, parent, &mut snapshot)
                                .await
                        {
                            self.runtime.invalidate_delivery(&parent.key, error)?;
                            continue;
                        }
                        if let Err(error) = self.runtime.retain_delivery_snapshot(snapshot) {
                            self.runtime.invalidate_delivery(&parent.key, error)?;
                        }
                    }
                    Err(error) => self.runtime.invalidate_delivery(&parent.key, error)?,
                }
            }
        }
        // Hydration may have sealed an actual newly observed no-release fact.
        self.sync_no_release_completions()?;
        Ok(())
    }
}
fn resolve_git() -> Result<PathBuf> {
    let path = std::env::var_os("PATH")
        .ok_or_else(|| Error::new("prerequisite", "Git search path is unavailable"))?;
    for directory in std::env::split_paths(&path).filter(|p| p.is_absolute()) {
        let candidate = directory.join("git");
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .map_err(|e| Error::io("resolve pinned Git executable", e));
        }
    }
    Err(Error::new("prerequisite", "Git executable is unavailable"))
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseIntent {
    schema_version: u32,
    runtime_id: Uuid,
    workspace_id: Uuid,
    request_id: Uuid,
    parent_command: Uuid,
    parent_command_digest: String,
    work: crate::evidence::WorkIdentity,
    parent: crate::authority::IssueKey,
    evidence_token: String,
    actor: crate::control::Actor,
    config_revision: String,
    queued_at: chrono::DateTime<Utc>,
}
/// An authenticated pending command reference. It grants no dispatch authority;
/// the orchestration driver must acquire a fresh actual lease admission.
pub struct PendingReleaseRequest {
    record: crate::evidence::Signed<ReleaseIntent>,
    receipt: crate::control::Receipt,
}
impl PendingReleaseRequest {
    pub fn request_id(&self) -> Uuid {
        self.record.payload.request_id
    }
    pub fn workspace_id(&self) -> Uuid {
        self.record.payload.workspace_id
    }
    pub fn parent(&self) -> &crate::authority::IssueKey {
        &self.record.payload.parent
    }
    pub fn evidence_token(&self) -> &str {
        &self.record.payload.evidence_token
    }
    pub fn receipt(&self) -> &crate::control::Receipt {
        &self.receipt
    }
}
fn delivery_id(runtime: Uuid, source: Uuid, phase: &str) -> Result<Uuid> {
    let digest = crate::digest::canonical_digest(
        1,
        &json!({"runtime":runtime,"source":source,"phase":phase}),
    )?;
    let value = digest
        .rsplit(':')
        .next()
        .filter(|value| value.len() == 64)
        .ok_or_else(|| Error::invalid("delivery phase digest invalid"))?;
    let bytes = hex::decode(value).map_err(|_| Error::invalid("delivery phase digest invalid"))?;
    let mut id = [0; 16];
    id.copy_from_slice(&bytes[..16]);
    id[6] = (id[6] & 15) | 0x80;
    id[8] = (id[8] & 63) | 0x80;
    Ok(Uuid::from_bytes(id))
}
impl RuntimeDelivery {
    pub async fn queue_release(
        &self,
        decision: crate::decisions::AuthorizedDecision,
    ) -> Result<crate::control::Receipt> {
        if decision.request().command != crate::api::CommandName::Release {
            return Err(Error::new(
                "access_denied",
                "release queue requires the authenticated release command",
            ));
        }
        let projection = self.runtime.observations.refresh().await?;
        self.refresh(&projection).await?;
        let configuration = self.runtime.configuration.snapshot()?;
        decision.recheck(
            &configuration,
            &projection,
            &self.runtime.control.snapshot()?,
        )?;
        let evidence = self.runtime.current_delivery(decision.parent())?;
        let parent = evidence.verified_parent().ok_or_else(|| {
            Error::new(
                "evidence_missing",
                "release requires current complete parent evidence",
            )
        })?;
        let acceptance = evidence.verified_acceptance().ok_or_else(|| {
            Error::new(
                "acceptance_required",
                "release requires current authenticated human acceptance",
            )
        })?;
        if decision.token() != Some(parent.token())
            || acceptance.acceptance().reviewed_token != parent.token()
            || parent.manifest().work != *decision.work()
        {
            return Err(Error::new(
                "stale_result",
                "release command does not name the current accepted evidence",
            ));
        }
        let intent = ReleaseIntent {
            schema_version: 1,
            runtime_id: configuration.runtime.id,
            workspace_id: decision.workspace(),
            request_id: delivery_id(
                configuration.runtime.id,
                decision.request().request_id,
                "manual-release-intent",
            )?,
            parent_command: decision.parent_command().operation_id,
            parent_command_digest: decision.parent_command().payload_digest.clone(),
            work: decision.work().clone(),
            parent: decision.parent().clone(),
            evidence_token: parent.token().into(),
            actor: decision.actor().clone(),
            config_revision: configuration.revision()?,
            queued_at: decision.parent_command().updated_at,
        };
        let signed = self
            .runtime
            .services
            .key
            .sign("manual_release_intent", &intent)?;
        let request = release_intent_request(&signed)?;
        let gate = self.current_gate();
        let current = projection
            .parent(decision.workspace(), decision.parent().issue)?
            .fresh_gate(
                None,
                GatePurpose::NewOperation,
                &self.runtime.dependency_outcomes()?,
                Utc::now(),
                Duration::seconds(30),
            )?;
        self.runtime.control.mutate_observed(request,None,Some("release"),|snapshot| {
            let config=self.runtime.configuration.snapshot()?;
            let projection=self.runtime.observations.admission_snapshot()?;
            self.runtime.configuration.authority_fences()?.check_actor(decision.actor(),"release")?;
            decision.recheck(&config,&projection,snapshot)?;
            gate(&current)
        }, |_| {
            let leaf=format!("release-intent-{}.json",intent.request_id);
            let _lock=self.records.section_lock("release-intents.lock")?;
            match self.records.read::<crate::evidence::Signed<ReleaseIntent>>(&leaf) {
                Ok(existing)=> {self.runtime.services.key.verify("manual_release_intent",&existing)?;if existing.digest()?!=signed.digest()? {return Err(Error::new("request_conflict","release request already names another immutable intent"));}},
                Err(error) if error.code=="record_missing"=>self.records.write(&leaf,&signed)?,
                Err(error)=>return Err(error),
            }
            Ok((BTreeMap::new(),json!({"queued":true,"dispatch_authorized":false,"request_id":intent.request_id,"parent":intent.parent,"evidence_token":intent.evidence_token})))
        })
    }
    /// Reconstruct queued intent only from its authenticated command and actual
    /// protected receipt. Serialized decisions and detached actor strings do not
    /// recreate a capability after restart.
    pub fn pending_release_requests(&self) -> Result<Vec<PendingReleaseRequest>> {
        let configuration = self.runtime.configuration.snapshot()?;
        let mut result = Vec::new();
        for leaf in self.records.records_bounded(65_536)? {
            let Some(id) = leaf
                .strip_prefix("release-intent-")
                .and_then(|s| s.strip_suffix(".json"))
            else {
                continue;
            };
            let signed: crate::evidence::Signed<ReleaseIntent> = self.records.read(&leaf)?;
            self.runtime
                .services
                .key
                .verify("manual_release_intent", &signed)?;
            let intent = &signed.payload;
            if intent.schema_version != 1
                || intent.runtime_id != configuration.runtime.id
                || intent.request_id.to_string() != id
                || intent.parent
                    != crate::authority::IssueKey::new(
                        &intent.work.linear_endpoint,
                        intent.work.linear_workspace,
                        intent.work.parent,
                    )?
                || intent.work.child.is_some()
            {
                return Err(Error::new(
                    "record_corrupt",
                    "release intent identity is inconsistent",
                ));
            }
            let receipt = self.runtime.control.operation(intent.request_id)?;
            let command = self.runtime.control.operation(intent.parent_command)?;
            if receipt.payload_digest != mutation_digest(&release_intent_request(&signed)?)?
                || !matches!(
                    receipt.phase,
                    crate::control::ReceiptPhase::Acknowledged
                        | crate::control::ReceiptPhase::Reconciled
                )
                || command.kind != "command/Release"
                || command.actor != intent.actor.identity
                || command.payload_digest != intent.parent_command_digest
            {
                return Err(Error::new(
                    "ownership_conflict",
                    "release queue lacks its actual authenticated command lineage",
                ));
            }
            result.push(PendingReleaseRequest {
                record: signed,
                receipt,
            });
        }
        result.sort_by_key(|r| (r.record.payload.queued_at, r.record.payload.request_id));
        Ok(result)
    }
}
fn release_intent_request(
    record: &crate::evidence::Signed<ReleaseIntent>,
) -> Result<crate::control::MutationRequest> {
    let intent = &record.payload;
    Ok(crate::control::MutationRequest {
        request_id: intent.request_id,
        runtime_id: intent.runtime_id,
        actor: intent.actor.clone(),
        kind: "manual_release_intent".into(),
        canonical_targets: vec![intent.parent.identity()?],
        expected: BTreeMap::from([
            ("parent_command".into(), intent.parent_command.to_string()),
            (
                "parent_command_digest".into(),
                intent.parent_command_digest.clone(),
            ),
            ("configuration".into(), intent.config_revision.clone()),
        ]),
        payload: serde_json::to_value(record)
            .map_err(|_| Error::invalid("encode release intent"))?,
    })
}

fn mutation_digest(request: &crate::control::MutationRequest) -> Result<String> {
    crate::digest::canonical_digest(
        1,
        &json!({"kind":request.kind,"targets":request.canonical_targets,"expected":request.expected,"payload":request.payload,"principal":request.actor.principal}),
    )
}

#[cfg(test)]
#[path = "runtime_delivery_tests.rs"]
pub(crate) mod tests;

#[path = "runtime_delivery_child.rs"]
mod child;

impl crate::runtime_driver::RuntimeProgression for RuntimeDelivery {
    fn reconcile(&self) -> crate::supervisor::SupervisorFuture<'_, ()> {
        Box::pin(async move {
            let projection = self.runtime.observations.refresh().await?;
            self.runtime.refresh_retry_references(&projection)?;
            self.runtime.recover_provider_holds()?;
            self.refresh(&projection).await
        })
    }
    fn reconcile_for_inspection(&self) -> crate::supervisor::SupervisorFuture<'_, ()> {
        Box::pin(async move {
            let projection = self.runtime.observations.refresh().await?;
            self.runtime.refresh_retry_references(&projection)?;
            self.runtime.recover_provider_holds()?;
            self.refresh_for_inspection(&projection).await
        })
    }
    fn advance<'a>(
        &'a self,
        lease: &'a crate::control::Lease,
    ) -> crate::supervisor::SupervisorFuture<'a, ()> {
        Box::pin(async move {
            self.runtime.registration.verify_active_owner(lease)?;
            let projection = self.runtime.observations.snapshot()?;
            let children = self.advance_children(lease, &projection).await;
            let parents = async {
                let current = self.runtime.observations.snapshot()?;
                self.advance_parents(lease, &current).await
            }
            .await;
            let releases = async {
                let current = self.runtime.observations.snapshot()?;
                self.advance_releases(lease, &current).await
            }
            .await;
            children.and(parents).and(releases)
        })
    }
    fn maintain(&self) -> crate::supervisor::SupervisorFuture<'_, ()> {
        Box::pin(async move {
            self.runtime.observe_local_exits()?;
            self.maintain_phase_setups()?;
            let projection = self.runtime.observations.snapshot()?;
            let children = self.maintain_child_phases(&projection).await;
            let parents = self.maintain_parent_phases().await;
            let releases = async {
                let current = self.runtime.observations.snapshot()?;
                self.maintain_releases(&current).await
            }
            .await;
            let refreshed = async {
                let current = self.runtime.observations.snapshot()?;
                self.refresh(&current).await
            }
            .await;
            children.and(parents).and(releases).and(refreshed)
        })
    }
    fn cancel_fenced<'a>(
        &'a self,
        operations: &'a [Uuid],
    ) -> crate::supervisor::SupervisorFuture<'a, ()> {
        Box::pin(async move {
            for operation in operations {
                self.runtime
                    .executions
                    .cancel_fenced(&self.runtime.control, *operation)?;
            }
            Ok(())
        })
    }
}

#[path = "runtime_delivery_parent.rs"]
mod parent;
pub(crate) use parent::ActiveParentContractInput;
pub use parent::ParentSources;

#[path = "runtime_delivery_release.rs"]
mod release;

pub use release::KnownPredecessorBaseline;
pub(crate) use release::{
    ReleaseCompletionAnchor, VerifiedNoReleaseCompletion, VerifiedReleaseCompletion,
    VerifiedSuccessorHolds,
};
#[path = "runtime_delivery_completion_binding.rs"]
mod completion_binding;

#[cfg(test)]
#[path = "runtime_delivery_worker_fixture.rs"]
pub(crate) mod worker_fixture;
