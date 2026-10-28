//! Shared runtime-host contracts used by controller composition and backend implementations.
//! These declarations grant no authority beyond their existing domain interfaces.
use crate::authority::IssueKey;
use crate::configuration::ConfigurationInputs;
use crate::control::ControlHost;
use crate::control::ControlSnapshot;
use crate::control::Receipt;
use crate::housekeeping::Housekeeping;
use crate::housekeeping::LogSink;
use crate::observations::ObservationCache;
use crate::platform::Registration;
use crate::reconcile::Activity;
use crate::reconcile::DependencyOutcomes;
use crate::reconcile::HistoryRow;
use crate::reconcile::ParentDelivery;
use crate::reconcile::RuntimeProjection;
use kegari_config::Config;
use kegari_control_api::identity::Actor;
use kegari_secrets::{AttestationKey, Secret, SecretStore};
use kegari_types::api::CommandRequest;
use kegari_types::control::Lease;
use kegari_types::error::{Error, Result};
use serde_json::Value;
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};
use tokio::sync::Notify;
use uuid::Uuid;

pub type HostFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub trait ConfigurationControl: Send + Sync {
    fn authority_fences(&self) -> Result<crate::configuration::AuthorityFences>;
    fn snapshot(&self) -> Result<Config>;
    fn execute(
        &self,
        actor: Actor,
        request: CommandRequest,
        secret: Option<Secret>,
        parent: Receipt,
    ) -> HostFuture<'_, Receipt>;
    fn check(&self, workspace: Option<Uuid>) -> HostFuture<'_, Value>;
}

/// A single revision of observed providers and verified delivery state. This is
/// transient memory; provider records and immutable artifacts remain authoritative.
#[derive(Clone)]
pub struct RuntimeInspection {
    pub projection: RuntimeProjection,
    pub delivery: BTreeMap<IssueKey, ParentDelivery>,
    pub outcomes: DependencyOutcomes,
    pub activities: Vec<Activity>,
    pub history: Vec<HistoryRow>,
    pub release_details: BTreeMap<(Uuid, IssueKey), Value>,
    pub release_history: Vec<crate::inspection::ReleaseInspectionRow>,
    pub no_release_history: Vec<crate::inspection::NoReleaseInspectionRow>,
}
pub trait WorkflowControl: Send + Sync {
    /// Read-only refresh. May run while STOPPED; must never admit or publish work.
    fn inspect(&self) -> HostFuture<'_, RuntimeInspection>;
    fn health(&self) -> Result<Value>;
    fn command_targets(&self, request: &CommandRequest) -> Result<Vec<String>>;
    fn execute(
        &self,
        actor: Actor,
        request: CommandRequest,
        parent: Receipt,
    ) -> HostFuture<'_, Receipt>;
    fn cleanup_plan(&self, operation: Uuid) -> HostFuture<'_, Value>;
}
pub trait LocalControlOwner: Send + Sync {
    fn status(&self) -> Result<Value>;
    /// Called inside ControlHost's serialized STOPPED/no-permit shutdown gate.
    /// Must verify OS process/lock ownership and durably prevent service restart.
    fn reserve_shutdown(&self) -> Result<()>;
}
impl LocalControlOwner for crate::platform::Registration {
    fn status(&self) -> Result<Value> {
        serde_json::to_value(self.snapshot()?)
            .map_err(|_| Error::new("ownership_conflict", "cannot encode registered owner state"))
    }
    fn reserve_shutdown(&self) -> Result<()> {
        self.reserve_service_shutdown()
    }
}

pub struct RuntimeDependencies {
    pub configuration: Arc<dyn ConfigurationControl>,
    pub control: Arc<ControlHost>,
    pub registration: Arc<Registration>,
    pub observations: Arc<ObservationCache>,
    pub secrets: Arc<dyn SecretStore>,
    pub key: Arc<AttestationKey>,
    pub housekeeping: Arc<Housekeeping>,
    pub logs: Arc<LogSink>,
    pub inputs: Arc<dyn ConfigurationInputs>,
    pub wake: Arc<Notify>,
}
#[derive(Clone)]
pub struct RuntimeBackend {
    pub workflow: Arc<dyn WorkflowControl>,
    pub driver: Arc<dyn SupervisorDriver>,
}
pub trait HostRuntimeFactory: Send + Sync {
    fn build(&self, dependencies: RuntimeDependencies) -> HostFuture<'_, RuntimeBackend>;
}

pub type SupervisorFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrainState {
    pub local_clear: bool,
    pub remote_reconciled: bool,
}
/// Methods operate outside the control mutex and must use actual owned workers,
/// provider facts and receipt recovery. poll schedules bounded operations; it
/// never waits for the lifetime of a model turn or repository command.
pub trait SupervisorDriver: Send + Sync {
    fn reconcile_before_start(&self) -> SupervisorFuture<'_, ()>;
    fn recover<'a>(&'a self, lease: &'a Lease) -> SupervisorFuture<'a, ()>;
    fn poll<'a>(&'a self, lease: &'a Lease, sequence: u64) -> SupervisorFuture<'a, ()>;
    fn maintain<'a>(
        &'a self,
        lease: &'a Lease,
        snapshot: &'a ControlSnapshot,
    ) -> SupervisorFuture<'a, DrainState>;
    /// Called once per generation/operation after durable fencing. Cancellation
    /// results must remain pending/unknown until actually observed.
    fn cancel_fenced<'a>(&'a self, operations: &'a [Uuid]) -> SupervisorFuture<'a, ()>;
    fn exited<'a>(&'a self, lease: &'a Lease) -> SupervisorFuture<'a, DrainState>;
}

/// Construct the existing evaluation-aware native inputs without performing I/O.
/// Reserved evaluation namespaces retain their strict fixture routing.
pub fn configuration_inputs() -> Arc<dyn ConfigurationInputs> {
    Arc::new(crate::evaluation_provider::EvaluationConfigurationInputs::new())
}
