//! Authenticated human decisions. Provider publications, fresh scope observations
//! and durable command lineage are required independently of orchestration intent.
use crate::authority::IssueKey;
use crate::configuration::ConfigurationInputs;
use crate::connections::ConnectionRegistry;
use crate::control::ControlHost;
use crate::control::ControlSnapshot;
use crate::control::Intent;
use crate::control::MutationRequest;
use crate::control::Receipt;
use crate::control::ReceiptPhase;
use crate::control::RunReference;
use crate::control_command::mutation_request;
use crate::control_command::verified_operator;
use crate::control_command::verify_current_actor;
use crate::evidence::AcceptanceActor;
use crate::evidence::AcceptanceRequest;
use crate::evidence::Publication;
use crate::evidence::VerifiedAcceptance;
use crate::evidence::WorkIdentity;
use crate::evidence::decode_signed_publication;
use crate::evidence::encode_signed_publication;
use crate::evidence::prepare_acceptance;
use crate::evidence::verify_acceptance;
use crate::housekeeping::Housekeeping;
use crate::observations::ObservationCache;
use crate::providers::ProviderBroker;
use crate::providers::ProviderMutation;
use crate::reconcile::DependencyOutcomes;
use crate::reconcile::GatePurpose;
use crate::reconcile::ParentDelivery;
use crate::reconcile::ParentProjection;
use crate::reconcile::RuntimeProjection;
use crate::reconcile::WorkspaceClients;
use crate::runtime_host::ConfigurationControl;
use crate::runtime_host::HostFuture;
use crate::scheduler::PhaseKey;
use chrono::{DateTime, Duration, Utc};
use kegari_config::Config;
use kegari_control_api::identity::Actor;
use kegari_control_api::identity::Principal;
use kegari_digest::{bytes_digest, canonical_digest};
use kegari_providers::{Comment, GithubClient};
use kegari_secrets::{AttestationKey, SecretStore, Signed};
use kegari_types::api::{CommandName, CommandRequest};
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use uuid::Uuid;

/// Candidate selection only. The manager independently checks these admissions,
/// current scope, resolved blockers, terminal ownership and finite phase limits.
#[derive(Clone, Debug)]
pub struct RetryTarget {
    pub phase: PhaseKey,
    pub admissions: BTreeSet<Uuid>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryBudget {
    pub max_attempts: u32,
    pub max_turns: u32,
    pub max_review_rounds: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryGrant {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub workspace_id: Uuid,
    pub command_id: Uuid,
    pub operation_id: Uuid,
    pub work: WorkIdentity,
    pub previous_phase: PhaseKey,
    pub prior_admissions: BTreeMap<Uuid, String>,
    pub budget: RetryBudget,
    pub control_actor: String,
    pub principal: Principal,
    pub linear_actor: Uuid,
    pub provider_writer: String,
    pub reason: String,
    pub granted_at: DateTime<Utc>,
}
#[derive(Clone, Debug)]
pub struct VerifiedRetryGrant {
    record: RetryGrant,
    publication: Publication,
    digest: String,
}
impl VerifiedRetryGrant {
    pub fn record(&self) -> &RetryGrant {
        &self.record
    }
    pub fn work(&self) -> &WorkIdentity {
        &self.record.work
    }
    pub fn phase(&self) -> &PhaseKey {
        &self.record.previous_phase
    }
    pub fn generation(&self) -> &crate::evidence::Generation {
        &self.record.work.generation
    }
    pub fn new_budget(&self) -> &RetryBudget {
        &self.record.budget
    }
    pub fn provenance(&self) -> &Publication {
        &self.publication
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn phase_key(&self) -> PhaseKey {
        let mut phase = self.record.previous_phase.clone();
        phase.retry_grant = Some(self.record.operation_id);
        phase
    }
    pub fn verify(
        signed: &Signed<RetryGrant>,
        publication: &Publication,
        current: &WorkIdentity,
        runtime: Uuid,
        key: &AttestationKey,
    ) -> Result<Self> {
        key.verify("operator_retry", signed)?;
        let record = &signed.payload;
        publication.verify(signed, "linear", &record.provider_writer)?;
        record.work.validate()?;
        if record.schema_version != 1
            || record.runtime_id != runtime
            || record.work != *current
            || record.command_id.is_nil()
            || record.operation_id
                != stage_id(
                    crate::control_command::command_effect_id(record.command_id),
                    "retry",
                )
            || record.workspace_id.is_nil()
            || record.prior_admissions.is_empty()
            || record.reason.trim().is_empty()
            || record.reason.len() > 4096
            || record.control_actor.is_empty()
            || record.linear_actor.is_nil()
            || record.provider_writer.is_empty()
            || record.granted_at > publication.observed_at
            || [
                record.budget.max_attempts,
                record.budget.max_turns,
                record.budget.max_review_rounds,
            ]
            .contains(&0)
            || record.previous_phase.generation != canonical_digest(1, &current.generation)?
            || record.previous_phase.work != work_key(current)?.identity()?
        {
            return Err(Error::new(
                "invalid_attestation",
                "retry grant does not bind current finite work/phase provenance",
            ));
        }
        for digest in record.prior_admissions.values() {
            crate::evidence::validate_digest(digest)?;
        }
        Ok(Self {
            record: record.clone(),
            publication: publication.clone(),
            digest: signed.digest()?,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplicitCorrection {
    pub schema_version: u32,
    pub runtime_id: Uuid,
    pub workspace_id: Uuid,
    pub parent: IssueKey,
    pub command_id: Uuid,
    pub operation_id: Uuid,
    pub control_actor: String,
    pub principal: Principal,
    pub linear_actor: Uuid,
    pub provider_writer: String,
    pub message_digest: String,
    pub created_at: DateTime<Utc>,
}
#[derive(Clone, Debug)]
pub struct VerifiedExplicitCorrection {
    record: ExplicitCorrection,
    publication: Publication,
    body_digest: String,
}
impl VerifiedExplicitCorrection {
    pub fn record(&self) -> &ExplicitCorrection {
        &self.record
    }
    pub fn publication(&self) -> &Publication {
        &self.publication
    }
    pub fn message_digest(&self) -> &str {
        &self.record.message_digest
    }
    pub fn body_digest(&self) -> &str {
        &self.body_digest
    }
    pub fn verify_comment(
        comment: &Comment,
        parent: &IssueKey,
        runtime: Uuid,
        workspace: Uuid,
        permitted: &BTreeSet<Uuid>,
        writer: &str,
        key: &AttestationKey,
    ) -> Result<Self> {
        let delimiter = "\n\n<!-- kegari:v0:explicit_correction -->\n";
        let (message, tail) = comment.body.rsplit_once(delimiter).ok_or_else(|| {
            Error::new(
                "invalid_attestation",
                "explicit correction framing is absent",
            )
        })?;
        check_message(message, 1024 * 1024)?;
        let marker = format!("<!-- kegari:v0:explicit_correction -->\n{tail}");
        let signed: Signed<ExplicitCorrection> =
            decode_signed_publication(&marker, "explicit_correction", key)?;
        let record = &signed.payload;
        if record.schema_version != 1
            || record.runtime_id != runtime
            || record.workspace_id != workspace
            || record.parent != *parent
            || record.command_id.is_nil()
            || record.operation_id
                != stage_id(
                    crate::control_command::command_effect_id(record.command_id),
                    "correction",
                )
            || record.control_actor.is_empty()
            || !permitted.contains(&record.linear_actor)
            || record.provider_writer != writer
            || record.message_digest != bytes_digest(message.as_bytes())
            || record.created_at > Utc::now() + Duration::seconds(5)
            || comment.issue_id != parent.issue
            || comment.bot
            || comment.delegated
            || comment.user_id.map(|id| id.to_string()).as_deref() != Some(writer)
            || comment.body_digest != bytes_digest(comment.body.as_bytes())
            || comment
                .created_at
                .is_none_or(|at| at < record.created_at - Duration::seconds(5))
        {
            return Err(Error::new(
                "invalid_attestation",
                "correction actor, content or provider provenance differs",
            ));
        }
        let publication = Publication {
            provider: "linear".into(),
            record_id: comment.id.to_string(),
            writer_account_id: writer.into(),
            payload_digest: canonical_digest(1, &signed)?,
            observed_at: Utc::now(),
            url: comment.url.clone(),
        };
        publication.verify(&signed, "linear", writer)?;
        Ok(Self {
            record: record.clone(),
            publication,
            body_digest: comment.body_digest.clone(),
        })
    }
}

/// Host composition must supply actual hydrated evidence/release/recovery state.
/// There are no default-success or caller-selected proof implementations.
pub trait DecisionEvidence: Send + Sync {
    fn current(&self, parent: &IssueKey) -> Result<ParentDelivery>;
    fn outcomes(&self) -> Result<DependencyOutcomes>;
    fn retry_target(&self, work: &WorkIdentity) -> Result<RetryTarget>;
    /// Revalidate the immutable prior target and sole exact published grant.
    /// Progress in that grant's new budget does not select a different target.
    fn published_retry_target(&self, expected: &VerifiedRetryGrant) -> Result<RetryTarget>;
    fn record_acceptance(&self, parent: &IssueKey, acceptance: VerifiedAcceptance) -> Result<()>;
    fn record_correction(&self, correction: VerifiedExplicitCorrection) -> Result<()>;
    fn retry_committed(&self, grant: VerifiedRetryGrant) -> Result<()>;
    fn release(&self, decision: AuthorizedDecision) -> HostFuture<'_, Receipt>;
    fn recover_release(&self, decision: AuthorizedDecision) -> HostFuture<'_, Receipt>;
    fn reconcile(&self, decision: AuthorizedOperationRecovery) -> HostFuture<'_, Receipt>;
}
#[derive(Clone)]
pub struct AuthorizedDecision {
    actor: Actor,
    request: CommandRequest,
    parent_command: Receipt,
    parent: IssueKey,
    workspace: Uuid,
    work: WorkIdentity,
    token: Option<String>,
    config_revision: String,
    control_generation: u64,
    authorization: Option<Receipt>,
}
impl AuthorizedDecision {
    pub fn actor(&self) -> &Actor {
        &self.actor
    }
    pub fn request(&self) -> &CommandRequest {
        &self.request
    }
    pub fn parent_command(&self) -> &Receipt {
        &self.parent_command
    }
    pub fn parent(&self) -> &IssueKey {
        &self.parent
    }
    pub fn workspace(&self) -> Uuid {
        self.workspace
    }
    pub fn work(&self) -> &WorkIdentity {
        &self.work
    }
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }
    /// Actual durable authorization stage, distinct from the still-running
    /// parent command. Backend effects cannot replace it with an asserted role.
    pub fn authorization_receipt(&self) -> Result<&Receipt> {
        self.authorization.as_ref().ok_or_else(|| {
            Error::new(
                "decision_pending",
                "decision authorization is not acknowledged",
            )
        })
    }
    /// Release adapters call this again inside every serialized mutation gate,
    /// after their own complete last-moment provider observation and evidence test.
    pub fn recheck(
        &self,
        config: &Config,
        projection: &RuntimeProjection,
        snapshot: &ControlSnapshot,
    ) -> Result<()> {
        verify_current_actor(
            config,
            &self.actor,
            kegari_control_api::api::command_role(&self.request.command, &self.request.args),
        )?;
        if crate::config::config_revision(config) != Ok(self.config_revision.clone())
            || snapshot.intent.generation != self.control_generation
            || !snapshot
                .background_commands
                .contains(&self.parent_command.operation_id)
            || snapshot.service_stopping
        {
            return Err(Error::new(
                "stale_generation",
                "decision configuration/control command ownership changed",
            ));
        }
        let current = projection.parent(self.workspace, self.parent.issue)?;
        require_work(
            current,
            &self.work,
            self.request.command == CommandName::Release,
        )?;
        if self.request.command == CommandName::Release
            && snapshot.intent.desired != Intent::Running
        {
            return Err(Error::new(
                "not_running",
                "new release admission requires RUNNING intent",
            ));
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct AuthorizedOperationRecovery {
    actor: Actor,
    request: CommandRequest,
    parent_command: Receipt,
    operation: Receipt,
    config_revision: String,
    control_generation: u64,
}
impl AuthorizedOperationRecovery {
    pub fn actor(&self) -> &Actor {
        &self.actor
    }
    pub fn request(&self) -> &CommandRequest {
        &self.request
    }
    pub fn parent_command(&self) -> &Receipt {
        &self.parent_command
    }
    pub fn operation(&self) -> &Receipt {
        &self.operation
    }
    pub fn recheck(&self, config: &Config, snapshot: &ControlSnapshot) -> Result<()> {
        verify_current_actor(config, &self.actor, "recover")?;
        if crate::config::config_revision(config) != Ok(self.config_revision.clone())
            || snapshot.intent.generation != self.control_generation
            || !snapshot
                .background_commands
                .contains(&self.parent_command.operation_id)
            || snapshot.service_stopping
        {
            return Err(Error::new(
                "stale_generation",
                "recovery command ownership/configuration changed",
            ));
        }
        Ok(())
    }
}

pub struct DecisionDependencies {
    pub configuration: Arc<dyn ConfigurationControl>,
    pub control: Arc<ControlHost>,
    pub observations: Arc<ObservationCache>,
    pub secrets: Arc<dyn SecretStore>,
    pub key: Arc<AttestationKey>,
    pub evidence: Arc<dyn DecisionEvidence>,
    pub housekeeping: Arc<Housekeeping>,
    pub inputs: Arc<dyn ConfigurationInputs>,
}
#[derive(Clone)]
pub struct DecisionManager {
    services: Arc<DecisionDependencies>,
}
#[derive(Clone)]
struct Context {
    actor: Actor,
    request: CommandRequest,
    parent_command: Receipt,
    config: Config,
    workspace: Uuid,
    parent: ParentProjection,
    work: WorkIdentity,
    clients: BTreeMap<Uuid, WorkspaceClients>,
    writer: String,
    linear_actor: Uuid,
    generation: u64,
}
impl DecisionManager {
    pub fn new(services: DecisionDependencies) -> Self {
        Self {
            services: Arc::new(services),
        }
    }
    pub fn command_targets(&self, request: &CommandRequest) -> Result<Vec<String>> {
        if matches!(
            request.command,
            CommandName::Cleanup | CommandName::OperationReconcile
        ) {
            let id = parse_uuid(text(&request.args, "operation")?)?;
            let receipt = self.services.control.operation(id)?;
            if receipt.runtime_id != request.runtime_id {
                return Err(Error::new(
                    "identity_mismatch",
                    "operation belongs to another runtime",
                ));
            }
            return Ok(vec![id.to_string()]);
        }
        let projection = self.services.observations.snapshot()?;
        let (workspace, key, child) = resolve(
            &projection,
            text(
                &request.args,
                if request.command == CommandName::Retry {
                    "work"
                } else {
                    "parent"
                },
            )?,
            request.command == CommandName::Retry,
        )?;
        let parent = projection.parent(workspace, key.issue)?;
        let mut targets = vec![key.identity()?];
        if let Some(child) = child {
            targets.push(IssueKey::new(&key.endpoint, key.workspace, child)?.identity()?);
        }
        if parent.workspace_id != workspace {
            return Err(Error::new(
                "identity_mismatch",
                "work projection ownership differs",
            ));
        }
        Ok(targets)
    }
    fn clients(&self, config: &Config) -> Result<BTreeMap<Uuid, WorkspaceClients>> {
        let transports = config
            .connections
            .keys()
            .filter_map(|name| {
                self.services
                    .inputs
                    .transport(name)
                    .map(|t| (name.clone(), t))
            })
            .collect();
        ConnectionRegistry::bind_with_transports(
            config,
            self.services.secrets.as_ref(),
            |name| self.services.inputs.environment(name),
            &transports,
        )?
        .clients(config)
        .cloned()
    }
    fn lineage(
        &self,
        actor: &Actor,
        request: &CommandRequest,
        parent: &Receipt,
    ) -> Result<MutationRequest> {
        let stored = self.services.control.operation(parent.request_id)?;
        let mut original = mutation_request(actor, request, stored.canonical_targets.clone());
        original.kind = format!("command/{:?}", request.command);
        original.payload = json!({"arguments":request.args,"secret_digest":null});
        let digest = canonical_digest(
            1,
            &json!({"kind":original.kind,"targets":original.canonical_targets,"expected":original.expected,"payload":original.payload,"principal":actor.principal}),
        )?;
        if stored.phase != ReceiptPhase::Prepared
            || stored.payload_digest != digest
            || parent.payload_digest != digest
            || stored.actor != actor.identity
            || stored.runtime_id != request.runtime_id
            || stored.kind != original.kind
            || request.request_id != crate::control_command::command_effect_id(parent.request_id)
        {
            return Err(Error::new(
                "request_conflict",
                "decision effect differs from its authenticated original command",
            ));
        }
        let mut effect = mutation_request(actor, request, stored.canonical_targets);
        effect.kind = format!("decision/{:?}", request.command);
        effect
            .expected
            .insert("parent_command_id".into(), parent.request_id.to_string());
        effect
            .expected
            .insert("parent_payload_digest".into(), digest);
        Ok(effect)
    }
    pub async fn execute(
        &self,
        actor: Actor,
        request: CommandRequest,
        parent: Receipt,
    ) -> Result<Receipt> {
        let config = self.services.configuration.snapshot()?;
        self.services
            .configuration
            .authority_fences()?
            .check_actor(
                &actor,
                kegari_control_api::api::command_role(&request.command, &request.args),
            )?;
        kegari_control_api::api::validate_command_request(&request, config.runtime.id)?;
        verify_current_actor(
            &config,
            &actor,
            kegari_control_api::api::command_role(&request.command, &request.args),
        )?;
        let effect = self.lineage(&actor, &request, &parent)?;
        match if request.command == CommandName::Cleanup {
            Err(Error::new(
                "record_missing",
                "cleanup replay belongs to its concrete helper",
            ))
        } else {
            self.services.control.operation(request.request_id)
        } {
            Ok(_) => {
                return self.services.control.mutate_observed(
                    effect,
                    None,
                    Some(kegari_control_api::api::command_role(
                        &request.command,
                        &request.args,
                    )),
                    |_| Err(Error::invalid("replay unexpectedly evaluated gate")),
                    |_| Err(Error::invalid("replay unexpectedly executed effect")),
                );
            }
            Err(e) if e.code == "record_missing" => (),
            Err(e) => return Err(e),
        }
        if request
            .expected_config_revision
            .as_ref()
            .is_some_and(|v| crate::config::config_revision(&config).as_ref() != Ok(v))
        {
            return Err(Error::new(
                "stale_config",
                "configuration changed before decision",
            ));
        }
        let snapshot = self.services.control.snapshot()?;
        if request
            .expected_control_generation
            .is_some_and(|g| g != snapshot.intent.generation)
            || !snapshot.background_commands.contains(&parent.operation_id)
        {
            return Err(Error::new(
                "stale_generation",
                "decision has no current owned background command",
            ));
        }
        if matches!(
            request.command,
            CommandName::Cleanup | CommandName::OperationReconcile
        ) {
            return self
                .operation(actor, request, parent, effect, config, snapshot)
                .await;
        }
        let projection = self.services.observations.refresh().await?;
        let (workspace, key, child) = resolve(
            &projection,
            text(
                &request.args,
                if request.command == CommandName::Retry {
                    "work"
                } else {
                    "parent"
                },
            )?,
            request.command == CommandName::Retry,
        )?;
        if self.command_targets(&request)? != parent.canonical_targets {
            return Err(Error::new(
                "scope_conflict",
                "decision target changed after command admission",
            ));
        }
        self.services
            .configuration
            .authority_fences()?
            .check_workspace(workspace)?;
        let p = projection.parent(workspace, key.issue)?.clone();
        let work = if let Some(id) = child {
            p.children.get(&id).and_then(|c| c.work.clone())
        } else {
            p.work.clone()
        }
        .ok_or_else(|| {
            Error::new(
                "incompletely_prepared",
                "current work generation is unavailable",
            )
        })?;
        let ws = config
            .workspaces
            .values()
            .find(|w| w.id == workspace)
            .ok_or_else(|| Error::new("scope_conflict", "workspace removed"))?;
        let operator = verified_operator(&config, &actor)?;
        let linear_actor = operator.linear_user_id;
        if request.command == CommandName::Correct && !ws.actors.correctors.contains(&linear_actor)
            || request.command == CommandName::Approve
                && !ws.actors.acceptors.contains(&linear_actor)
        {
            return Err(Error::new(
                "access_denied",
                "mapped Linear identity lacks the required workspace decision authority",
            ));
        }
        let writer = config.connections[&ws.linear_connection].account_id.clone();
        let ctx = Context {
            actor,
            request,
            parent_command: parent,
            config: config.clone(),
            workspace,
            parent: p,
            work,
            clients: self.clients(&config)?,
            writer,
            linear_actor,
            generation: snapshot.intent.generation,
        };
        if ctx.request.command == CommandName::Retry {
            let grant = self.retry(&ctx).await?;
            let signed = self.services.key.sign("operator_retry", grant.record())?;
            let comment = Uuid::parse_str(&grant.provenance().record_id)
                .map_err(|_| Error::new("invalid_attestation", "retry comment ID is invalid"))?;
            return self
                .services
                .control
                .record_retry_completion(effect, &ctx.parent_command, &grant, |snapshot| {
                    self.current_control(&ctx, snapshot)?;
                    let current = self.cached_fresh(&ctx)?;
                    let verified =
                        self.published_retry_candidate(&ctx, &current, snapshot, &signed, comment)?;
                    if verified.digest() != grant.digest() {
                        return Err(Error::new(
                            "stale_result",
                            "retry grant changed before command completion",
                        ));
                    }
                    Ok(())
                })
                .map_err(after_write);
        }
        let result = match ctx.request.command {
            CommandName::Correct => self.correct(&ctx).await,
            CommandName::Approve => self.approve(&ctx).await,
            CommandName::Release | CommandName::ReleaseRecover => self.release(&ctx).await,
            _ => Err(Error::invalid(
                "command is not an authenticated workflow decision",
            )),
        }?;
        self.finish(
            &ctx.actor,
            &ctx.request,
            &ctx.parent_command,
            effect,
            result,
        )
    }
    fn current_control(&self, ctx: &Context, snapshot: &ControlSnapshot) -> Result<()> {
        let fences = self.services.configuration.authority_fences()?;
        fences.check_actor(
            &ctx.actor,
            kegari_control_api::api::command_role(&ctx.request.command, &ctx.request.args),
        )?;
        fences.check_workspace(ctx.workspace)?;
        let config = self.services.configuration.snapshot()?;
        verify_current_actor(
            &config,
            &ctx.actor,
            kegari_control_api::api::command_role(&ctx.request.command, &ctx.request.args),
        )?;
        if crate::config::config_revision(&config)? != crate::config::config_revision(&ctx.config)?
            || snapshot.intent.generation != ctx.generation
            || snapshot.service_stopping
            || !snapshot
                .background_commands
                .contains(&ctx.parent_command.operation_id)
        {
            return Err(Error::new(
                "stale_generation",
                "decision configuration, principal or command ownership changed",
            ));
        }
        let ws = config
            .workspaces
            .values()
            .find(|w| w.id == ctx.workspace)
            .ok_or_else(|| Error::new("scope_conflict", "workspace removed"))?;
        let operator = verified_operator(&config, &ctx.actor)?;
        if operator.linear_user_id != ctx.linear_actor
            || ctx.request.command == CommandName::Correct
                && !ws.actors.correctors.contains(&operator.linear_user_id)
            || ctx.request.command == CommandName::Approve
                && !ws.actors.acceptors.contains(&operator.linear_user_id)
        {
            return Err(Error::new(
                "access_denied",
                "workspace decision permission was revoked",
            ));
        }
        Ok(())
    }
    async fn fresh(&self, ctx: &Context) -> Result<ParentProjection> {
        self.services.observations.refresh().await?;
        self.cached_fresh(ctx)
    }
    fn cached_fresh(&self, ctx: &Context) -> Result<ParentProjection> {
        self.services
            .configuration
            .authority_fences()?
            .check_workspace(ctx.workspace)?;
        let current = self.services.observations.admission_snapshot()?;
        if current.config_revision != crate::config::config_revision(&ctx.config)?
            || !current
                .workspaces
                .get(&ctx.workspace)
                .is_some_and(|w| w.complete)
        {
            return Err(Error::new(
                "observation_unavailable",
                "complete current configuration/provider observations are required",
            ));
        }
        let p = current.parent(ctx.workspace, ctx.parent.key.issue)?.clone();
        let now = Utc::now();
        if p.observed_at > now + Duration::seconds(5) || now - p.observed_at > Duration::seconds(30)
        {
            return Err(Error::new(
                "stale_result",
                "decision provider observations expired",
            ));
        }
        if p.workflow_digest != ctx.parent.workflow_digest {
            return Err(Error::new(
                "stale_generation",
                "workflow changed during decision",
            ));
        }
        if ctx.request.command == CommandName::Correct {
            if p.key != ctx.parent.key || p.work != ctx.parent.work || !p.authorization_published {
                return Err(Error::new(
                    "stale_generation",
                    "correction target authority changed before publication",
                ));
            }
        } else {
            require_work(
                &p,
                &ctx.work,
                ctx.request.command != CommandName::ReleaseRecover,
            )?;
        }
        Ok(p)
    }
    fn cached_same(&self, ctx: &Context, expected: &ParentProjection) -> Result<ParentProjection> {
        let current = self.cached_fresh(ctx)?;
        if current.source_digest != expected.source_digest {
            return Err(Error::new(
                "stale_result",
                "authoritative decision sources changed after preparation",
            ));
        }
        Ok(current)
    }
    fn stage_request(&self, ctx: &Context, stage: &str) -> Result<MutationRequest> {
        let mut request = self.lineage(&ctx.actor, &ctx.request, &ctx.parent_command)?;
        request.request_id = stage_id(ctx.request.request_id, stage);
        request.expected.insert(
            "config_revision".into(),
            crate::config::config_revision(&ctx.config)?,
        );
        request
            .expected
            .insert("control_generation".into(), ctx.generation.to_string());
        request.expected.insert(
            "decision_effect_id".into(),
            ctx.request.request_id.to_string(),
        );
        request
            .expected
            .insert("workspace".into(), ctx.workspace.to_string());
        request
            .expected
            .insert("parent".into(), ctx.parent.key.issue.to_string());
        request
            .expected
            .insert("work".into(), canonical_digest(1, &ctx.work)?);
        Ok(request)
    }
    async fn publish(
        &self,
        ctx: &Context,
        stage: &str,
        body: String,
        explicit: bool,
    ) -> Result<(Receipt, Comment)> {
        let mut request = self.stage_request(ctx, stage)?;
        if explicit {
            request
                .expected
                .insert("explicit_correction".into(), "true".into());
        }
        let linear = ctx
            .clients
            .get(&ctx.workspace)
            .ok_or_else(|| Error::new("missing_secret", "provider unavailable"))?
            .linear
            .clone();
        let this = self.clone();
        let checked = ctx.clone();
        let prepared = self.fresh(ctx).await?;
        let retry_record = if ctx.request.command == CommandName::Retry {
            Some(
                decode_signed_publication::<RetryGrant>(
                    &body,
                    "operator_retry",
                    &self.services.key,
                )?
                .payload,
            )
        } else {
            None
        };
        let target = ctx.work.child.unwrap_or(ctx.parent.key.issue);
        let corrective_operation = request.request_id;
        let corrective_digest = bytes_digest(body.as_bytes());
        let receipt = ProviderBroker {
            linear: Some(linear.clone()),
            github: None,
        }
        .mutate_observed(
            self.services.control.clone(),
            request,
            None,
            Some(
                kegari_control_api::api::command_role(&ctx.request.command, &ctx.request.args)
                    .into(),
            ),
            ProviderMutation::LinearComment {
                issue: target,
                body: body.clone(),
            },
            move |snapshot| {
                this.current_control(&checked, snapshot)?;
                let fresh = this.cached_same(&checked, &prepared)?;
                if checked.request.command == CommandName::Approve {
                    this.current_parent_evidence(&checked, &fresh)?;
                }
                if checked.request.command == CommandName::Retry {
                    let (target, admissions) = this.retry_candidate(&checked, &fresh, snapshot)?;
                    let record = retry_record.as_ref().ok_or_else(|| {
                        Error::new(
                            "invalid_attestation",
                            "retry publication has no immutable grant",
                        )
                    })?;
                    if target.phase != record.previous_phase
                        || admissions != record.prior_admissions
                    {
                        return Err(Error::new(
                            "stale_generation",
                            "retry phase or history changed before publication",
                        ));
                    }
                }
                if explicit {
                    this.services.observations.fence_explicit_correction(
                        checked.workspace,
                        &checked.parent.key,
                        corrective_operation,
                        &corrective_digest,
                        &checked.writer,
                    )?;
                }
                Ok(())
            },
        )
        .await?;
        if !matches!(
            receipt.phase,
            ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
        ) {
            return Err(Error::new(
                "operation_uncertain",
                "decision publication has no verified acknowledgement",
            ));
        }
        let comments = linear.comments(target).await.map_err(after_write)?;
        let matching: Vec<_> = comments
            .into_iter()
            .filter(|c| {
                c.issue_id == target
                    && c.body == body
                    && !c.bot
                    && !c.delegated
                    && c.user_id.map(|id| id.to_string()).as_deref() == Some(&ctx.writer)
            })
            .collect();
        if matching.len() != 1
            || receipt.remote_ids.get("comment_id")
                != matching.first().map(|c| c.id.to_string()).as_ref()
        {
            return Err(Error::new(
                "operation_uncertain",
                "published decision requires one exact acknowledged current provider comment",
            ));
        }
        Ok((receipt, matching.into_iter().next().unwrap()))
    }
    async fn correct(&self, ctx: &Context) -> Result<Value> {
        let message = text(&ctx.request.args, "message")?;
        check_message(message, 1024 * 1024)?;
        let record = ExplicitCorrection {
            schema_version: 1,
            runtime_id: ctx.config.runtime.id,
            workspace_id: ctx.workspace,
            parent: ctx.parent.key.clone(),
            command_id: ctx.parent_command.request_id,
            operation_id: stage_id(ctx.request.request_id, "correction"),
            control_actor: ctx.actor.identity.clone(),
            principal: ctx
                .actor
                .principal
                .clone()
                .ok_or_else(|| Error::new("access_denied", "operator principal absent"))?,
            linear_actor: ctx.linear_actor,
            provider_writer: ctx.writer.clone(),
            message_digest: bytes_digest(message.as_bytes()),
            created_at: Utc::now(),
        };
        let body = format!(
            "{message}\n\n{}",
            encode_signed_publication(
                "explicit_correction",
                &self.services.key.sign("explicit_correction", &record)?
            )?
        );
        let (receipt, comment) = self.publish(ctx, "correction", body, true).await?;
        let ws = ctx
            .config
            .workspaces
            .values()
            .find(|w| w.id == ctx.workspace)
            .unwrap();
        self.services
            .observations
            .refresh()
            .await
            .map_err(after_write)?;
        self.services
            .control
            .serialize_observation(|snapshot| {
                self.current_control(ctx, snapshot)?;
                let current = self.services.observations.admission_snapshot()?;
                let parent = current.parent(ctx.workspace, ctx.parent.key.issue)?;
                let live = current_decision_comment(
                    parent,
                    ctx.parent.key.issue,
                    comment.id,
                    &comment.body,
                    &ctx.writer,
                )?;
                let verified = VerifiedExplicitCorrection::verify_comment(
                    live,
                    &ctx.parent.key,
                    ctx.config.runtime.id,
                    ctx.workspace,
                    &ws.actors.correctors,
                    &ctx.writer,
                    &self.services.key,
                )?;
                self.services.evidence.record_correction(verified)
            })
            .map_err(after_write)?;
        Ok(
            json!({"comment_id":comment.id,"publication_operation":receipt.operation_id,"correction":"recorded","routing":"awaiting_permitted_coordination","linear_url":comment.url}),
        )
    }
    fn current_parent_evidence(
        &self,
        ctx: &Context,
        parent: &ParentProjection,
    ) -> Result<crate::evidence::VerifiedParent> {
        require_work(parent, &ctx.work, true)?;
        let delivery = self.services.evidence.current(&ctx.parent.key)?;
        let verified = delivery.verified_parent().cloned().ok_or_else(|| {
            Error::new(
                "evidence_missing",
                "current verified published parent evidence is required",
            )
        })?;
        if verified.manifest().work != ctx.work
            || text(&ctx.request.args, "result")? != verified.token()
        {
            return Err(Error::new(
                "stale_result",
                "supplied result token is not the exact current verified parent evidence",
            ));
        }
        Ok(verified)
    }
    async fn approve(&self, ctx: &Context) -> Result<Value> {
        let fresh = self.fresh(ctx).await?;
        let parent = self.current_parent_evidence(ctx, &fresh)?;
        let ws = ctx
            .config
            .workspaces
            .values()
            .find(|w| w.id == ctx.workspace)
            .unwrap();
        let permitted: BTreeSet<_> = ws.actors.acceptors.iter().map(Uuid::to_string).collect();
        let signed = prepare_acceptance(
            &parent,
            AcceptanceRequest {
                supplied_token: text(&ctx.request.args, "result")?,
                current_work: &ctx.work,
                pending_correction: !fresh.corrections.pending_ids().is_empty(),
                actor: AcceptanceActor {
                    control_actor: &ctx.actor.identity,
                    linear_actor: &ctx.linear_actor.to_string(),
                    provider_writer: &ctx.writer,
                    has_accept_role: true,
                    permitted_acceptors: &permitted,
                },
                runtime_id: ctx.config.runtime.id,
                operation_id: stage_id(ctx.request.request_id, "acceptance"),
                now: Utc::now(),
            },
            &self.services.key,
        )?;
        let (published, comment) = self
            .publish(
                ctx,
                "acceptance",
                encode_signed_publication("human_acceptance", &signed)?,
                false,
            )
            .await?;
        self.services
            .observations
            .refresh()
            .await
            .map_err(after_write)?;
        let fresh = self.fresh(ctx).await.map_err(after_write)?;
        self.current_parent_evidence(ctx, &fresh)
            .map_err(after_write)?;
        let publication = publication(&signed, &comment, &ctx.writer)?;
        self.current_acceptance(ctx, &fresh, &signed, &publication, &permitted)
            .map_err(after_write)?;
        let issue = fresh.issue.as_ref().ok_or_else(|| {
            Error::new(
                "observation_unavailable",
                "parent is missing after acceptance",
            )
        })?;
        let state = ws
            .linear
            .states
            .get(&issue.team_id.to_string())
            .ok_or_else(|| Error::new("scope_conflict", "parent team state mapping absent"))?
            .parent_accepted;
        let mut closure = None;
        if issue.state_id != state {
            let this = self.clone();
            let checked = ctx.clone();
            let prepared = self.fresh(ctx).await.map_err(after_write)?;
            let signed_gate = signed.clone();
            let publication_gate = publication.clone();
            let permitted_gate = permitted.clone();
            closure = Some(
                ProviderBroker {
                    linear: Some(ctx.clients[&ctx.workspace].linear.clone()),
                    github: None,
                }
                .mutate_observed(
                    self.services.control.clone(),
                    self.stage_request(ctx, "closure")?,
                    None,
                    Some("accept".into()),
                    ProviderMutation::LinearState {
                        issue: issue.id,
                        expected_state: issue.state_id,
                        state,
                    },
                    move |snapshot| {
                        this.current_control(&checked, snapshot)?;
                        let fresh = this.cached_same(&checked, &prepared)?;
                        this.current_acceptance(
                            &checked,
                            &fresh,
                            &signed_gate,
                            &publication_gate,
                            &permitted_gate,
                        )?;
                        Ok(())
                    },
                )
                .await
                .map_err(after_write)?,
            );
        }
        let final_parent = ctx.clients[&ctx.workspace]
            .linear
            .issue(ctx.parent.key.issue)
            .await
            .map_err(after_write)?;
        if final_parent.state_id != state {
            return Err(Error::new(
                "operation_uncertain",
                "accepted parent closure is not observed in the configured provider state",
            ));
        }
        self.services
            .observations
            .refresh()
            .await
            .map_err(after_write)?;
        let final_fresh = self.fresh(ctx).await.map_err(after_write)?;
        self.services
            .control
            .serialize_observation(|snapshot| {
                self.current_control(ctx, snapshot)?;
                let current = self.cached_same(ctx, &final_fresh)?;
                let verified =
                    self.current_acceptance(ctx, &current, &signed, &publication, &permitted)?;
                self.services
                    .evidence
                    .record_acceptance(&ctx.parent.key, verified)
            })
            .map_err(after_write)?;
        Ok(
            json!({"acceptance":signed.digest()?,"result":parent.token(),"publication":publication,"publication_operation":published.operation_id,"closure_operation":closure.map(|r|r.operation_id),"parent_state":state}),
        )
    }
    fn current_acceptance(
        &self,
        ctx: &Context,
        observed: &ParentProjection,
        signed: &Signed<crate::evidence::Acceptance>,
        prior: &Publication,
        permitted: &BTreeSet<String>,
    ) -> Result<VerifiedAcceptance> {
        let parent = self.current_parent_evidence(ctx, observed)?;
        let expected = encode_signed_publication("human_acceptance", signed)?;
        let comments = observed
            .observations
            .get(&ctx.parent.key.issue)
            .ok_or_else(|| {
                Error::new(
                    "evidence_missing",
                    "current acceptance publication was not observed",
                )
            })?;
        let matching: Vec<_> = comments
            .comments
            .iter()
            .filter(|c| c.body == expected)
            .collect();
        let comment = matching.first().ok_or_else(|| {
            Error::new(
                "evidence_missing",
                "acceptance publication disappeared or changed",
            )
        })?;
        if matching.len() != 1
            || comment.id.to_string() != prior.record_id
            || comment.issue_id != ctx.parent.key.issue
            || comment.bot
            || comment.delegated
            || comment.user_id.map(|id| id.to_string()).as_deref() != Some(&ctx.writer)
        {
            return Err(Error::new(
                "evidence_missing",
                "acceptance publication identity, native writer or uniqueness changed",
            ));
        }
        let current = publication(signed, comment, &ctx.writer)?;
        verify_acceptance(
            signed,
            &current,
            &parent,
            &ctx.work,
            !observed.corrections.pending_ids().is_empty(),
            permitted,
            &self.services.key,
        )
    }
    fn retry_candidate(
        &self,
        ctx: &Context,
        parent: &ParentProjection,
        snapshot: &ControlSnapshot,
    ) -> Result<(RetryTarget, BTreeMap<Uuid, String>)> {
        let target = self.services.evidence.retry_target(&ctx.work)?;
        self.retry_candidate_for_target(ctx, parent, snapshot, target, None)
    }
    fn retry_candidate_for_target(
        &self,
        ctx: &Context,
        parent: &ParentProjection,
        snapshot: &ControlSnapshot,
        target: RetryTarget,
        published: Option<&VerifiedRetryGrant>,
    ) -> Result<(RetryTarget, BTreeMap<Uuid, String>)> {
        parent.fresh_gate(
            ctx.work.child,
            GatePurpose::NewOperation,
            &self.services.evidence.outcomes()?,
            Utc::now(),
            Duration::seconds(30),
        )?;
        let receipts = self.services.control.operations()?;
        let mut successor = BTreeMap::new();
        if let Some(grant) = published {
            let workspace = ctx
                .config
                .workspaces
                .values()
                .find(|w| w.id == ctx.workspace)
                .ok_or_else(|| Error::new("stale_config", "retry workspace disappeared"))?;
            let account = &ctx.config.connections[&workspace.codex_connection].account_id;
            for receipt in &receipts {
                let binding = if let Some(run) = RunReference::from_receipt(receipt)? {
                    Some((run.phase, run.workspace, run.account))
                } else {
                    crate::control::PreparationReference::from_receipt(receipt)?
                        .map(|prep| (prep.phase, prep.workspace, prep.account))
                };
                if let Some((phase, workspace, successor_account)) = binding
                    && phase == grant.phase_key()
                {
                    if receipt.runtime_id != ctx.config.runtime.id
                        || workspace != ctx.workspace
                        || &successor_account != account
                    {
                        return Err(Error::new(
                            "identity_mismatch",
                            "published retry successor identity changed",
                        ));
                    }
                    successor.insert(receipt.operation_id, receipt);
                }
            }
        }
        let successor_effect = |id: &Uuid| -> bool {
            if successor.contains_key(id) {
                return true;
            }
            let Some(effect) = receipts.iter().find(|receipt| &receipt.operation_id == id) else {
                return false;
            };
            let Some(admission) = effect
                .expected
                .get("admission_operation")
                .and_then(|id| Uuid::parse_str(id).ok())
                .and_then(|id| successor.get(&id))
            else {
                return false;
            };
            effect.runtime_id == ctx.config.runtime.id
                && effect.expected.get("admitted_work") == Some(&target.phase.work)
                && effect.expected.get("admitted_scope") == admission.expected.get("scope")
                && effect.expected.contains_key("admitted_scope")
        };
        if target.phase.work != work_key(&ctx.work)?.identity()?
            || target.phase.generation != canonical_digest(1, &ctx.work.generation)?
            || target.admissions.is_empty()
            || snapshot.unresolved_operations.keys().any(|operation| {
                *operation != stage_id(ctx.request.request_id, "retry")
                    && !successor_effect(operation)
            })
            || snapshot.active_operations.iter().any(|p| {
                p.work_key == target.phase.work && !successor.contains_key(&p.operation_id)
            })
        {
            return Err(Error::new(
                "reconciliation_required",
                "retry requires reconciled inactive exact phase and current generation",
            ));
        }
        let mut found = BTreeMap::new();
        let mut ordinals = BTreeSet::new();
        for receipt in receipts {
            if let Some(run) = RunReference::from_receipt(&receipt)?
                && run.workspace == ctx.workspace
                && run.phase == target.phase
            {
                if !matches!(
                    receipt.phase,
                    ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
                ) || receipt.runtime_id != ctx.config.runtime.id
                    || !ordinals.insert(run.attempt)
                {
                    return Err(Error::new(
                        "reconciliation_required",
                        "prior phase admission is active, uncertain or inconsistent",
                    ));
                }
                found.insert(receipt.operation_id, receipt.payload_digest);
            }
        }
        if found.keys().copied().collect::<BTreeSet<_>>() != target.admissions
            || ordinals.iter().copied().ne(1..=ordinals.len() as u32)
        {
            return Err(Error::new(
                "reconciliation_required",
                "retry phase history is incomplete or changed",
            ));
        }
        Ok((target, found))
    }
    fn published_retry_candidate(
        &self,
        ctx: &Context,
        parent: &ParentProjection,
        snapshot: &ControlSnapshot,
        signed: &Signed<RetryGrant>,
        comment: Uuid,
    ) -> Result<VerifiedRetryGrant> {
        let live = current_decision_comment(
            parent,
            ctx.work.child.unwrap_or(ctx.parent.key.issue),
            comment,
            &encode_signed_publication("operator_retry", signed)?,
            &ctx.writer,
        )?;
        let verified = VerifiedRetryGrant::verify(
            signed,
            &publication(signed, live, &ctx.writer)?,
            &ctx.work,
            ctx.config.runtime.id,
            &self.services.key,
        )?;
        let target = self.services.evidence.published_retry_target(&verified)?;
        let (target, admissions) =
            self.retry_candidate_for_target(ctx, parent, snapshot, target, Some(&verified))?;
        if target.phase != signed.payload.previous_phase
            || admissions != signed.payload.prior_admissions
        {
            return Err(Error::new(
                "stale_result",
                "retry phase or history changed after exact grant publication",
            ));
        }
        Ok(verified)
    }
    async fn retry(&self, ctx: &Context) -> Result<VerifiedRetryGrant> {
        let reason = text(&ctx.request.args, "reason")?;
        check_message(reason, 4096)?;
        let fresh = self.fresh(ctx).await?;
        let (target, prior) =
            self.retry_candidate(ctx, &fresh, &self.services.control.snapshot()?)?;
        let record = RetryGrant {
            schema_version: 1,
            runtime_id: ctx.config.runtime.id,
            workspace_id: ctx.workspace,
            command_id: ctx.parent_command.request_id,
            operation_id: stage_id(ctx.request.request_id, "retry"),
            work: ctx.work.clone(),
            previous_phase: target.phase,
            prior_admissions: prior,
            budget: RetryBudget {
                max_attempts: ctx.config.runtime.max_attempts_per_phase,
                max_turns: ctx.config.runtime.max_turns,
                max_review_rounds: ctx.config.runtime.max_review_rounds,
            },
            control_actor: ctx.actor.identity.clone(),
            principal: ctx
                .actor
                .principal
                .clone()
                .ok_or_else(|| Error::new("access_denied", "operator principal missing"))?,
            linear_actor: ctx.linear_actor,
            provider_writer: ctx.writer.clone(),
            reason: reason.into(),
            granted_at: Utc::now(),
        };
        let signed = self.services.key.sign("operator_retry", &record)?;
        let (_receipt, comment) = self
            .publish(
                ctx,
                "retry",
                encode_signed_publication("operator_retry", &signed)?,
                false,
            )
            .await?;
        self.services
            .observations
            .refresh()
            .await
            .map_err(after_write)?;
        let fresh = self.fresh(ctx).await.map_err(after_write)?;
        self.published_retry_candidate(
            ctx,
            &fresh,
            &self.services.control.snapshot()?,
            &signed,
            comment.id,
        )
        .map_err(after_write)?;
        let verified = self
            .services
            .control
            .serialize_observation(|snapshot| {
                self.current_control(ctx, snapshot)?;
                let current = self.cached_same(ctx, &fresh)?;
                let verified =
                    self.published_retry_candidate(ctx, &current, snapshot, &signed, comment.id)?;
                self.services.evidence.retry_committed(verified.clone())?;
                Ok(verified)
            })
            .map_err(after_write)?;
        Ok(verified)
    }
    async fn release(&self, ctx: &Context) -> Result<Value> {
        let fresh = self.fresh(ctx).await?;
        if ctx.request.command == CommandName::Release {
            self.current_parent_evidence(ctx, &fresh)?;
        }
        let mut decision = AuthorizedDecision {
            actor: ctx.actor.clone(),
            request: ctx.request.clone(),
            parent_command: ctx.parent_command.clone(),
            parent: ctx.parent.key.clone(),
            workspace: ctx.workspace,
            work: ctx.work.clone(),
            token: ctx
                .request
                .args
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_owned),
            config_revision: crate::config::config_revision(&ctx.config)?,
            control_generation: ctx.generation,
            authorization: None,
        };
        let this = self.clone();
        let checked = ctx.clone();
        let prepared = self.fresh(ctx).await?;
        let proof = decision.clone();
        let control = self.services.control.clone();
        let request = self.stage_request(ctx, "release_admission")?;
        let role = kegari_control_api::api::command_role(&ctx.request.command, &ctx.request.args)
            .to_owned();
        let command = ctx.request.command.clone();
        let authorization = tokio::task::spawn_blocking(move || {
            let gate = |snapshot: &ControlSnapshot| {
                this.current_control(&checked, snapshot)?;
                let fresh = this.cached_same(&checked, &prepared)?;
                require_work(
                    &fresh,
                    &checked.work,
                    proof.request.command == CommandName::Release,
                )?;
                if proof.request.command == CommandName::Release
                    && snapshot.intent.desired != Intent::Running
                {
                    return Err(Error::new(
                        "not_running",
                        "new manual release requires RUNNING intent",
                    ));
                }
                Ok(())
            };
            if command == CommandName::ReleaseRecover {
                return control.record_release_recovery_intent(
                    request,
                    &checked.parent_command,
                    gate,
                );
            }
            control.mutate_observed(request, None, Some(&role), gate, |_| {
                Ok((
                    BTreeMap::new(),
                    json!({"decision":"authorized","command":command}),
                ))
            })
        })
        .await
        .map_err(|_| Error::new("control_failed", "release admission task failed"))??;
        decision.authorization = Some(authorization);
        let receipt = if ctx.request.command == CommandName::Release {
            self.services.evidence.release(decision).await?
        } else {
            self.services.evidence.recover_release(decision).await?
        };
        Ok(json!({"release_operation":receipt.operation_id,"receipt":receipt}))
    }
    fn finish(
        &self,
        actor: &Actor,
        request: &CommandRequest,
        parent: &Receipt,
        effect: MutationRequest,
        result: Value,
    ) -> Result<Receipt> {
        if matches!(
            request.command,
            CommandName::ReleaseRecover | CommandName::OperationReconcile
        ) {
            let current = |snapshot: &ControlSnapshot| {
                self.services
                    .configuration
                    .authority_fences()?
                    .check_actor(actor, "recover")?;
                verify_current_actor(&self.services.configuration.snapshot()?, actor, "recover")?;
                if !snapshot.background_commands.contains(&parent.operation_id) {
                    return Err(Error::new(
                        "ownership_conflict",
                        "recovery background task is no longer active",
                    ));
                }
                Ok(())
            };
            return if request.command == CommandName::ReleaseRecover {
                self.services
                    .control
                    .record_release_recovery_completion(effect, parent, result, current)
            } else {
                self.services
                    .control
                    .record_operation_reconciliation_completion(effect, parent, result, current)
            };
        }
        self.services.control.mutate_observed(
            effect,
            None,
            Some(kegari_control_api::api::command_role(
                &request.command,
                &request.args,
            )),
            |snapshot| {
                self.services
                    .configuration
                    .authority_fences()?
                    .check_actor(
                        actor,
                        kegari_control_api::api::command_role(&request.command, &request.args),
                    )?;
                verify_current_actor(
                    &self.services.configuration.snapshot()?,
                    actor,
                    kegari_control_api::api::command_role(&request.command, &request.args),
                )?;
                if !snapshot.background_commands.contains(&parent.operation_id) {
                    return Err(Error::new(
                        "ownership_conflict",
                        "decision background task is no longer active",
                    ));
                }
                Ok(())
            },
            |_| Ok((BTreeMap::new(), result)),
        )
    }
    pub async fn cleanup_plan(&self, operation: Uuid) -> Result<Value> {
        let config = self.services.configuration.snapshot()?;
        let github = self.cleanup_github(operation, &config)?;
        serde_json::to_value(
            self.services
                .housekeeping
                .inspect(operation, &github)
                .await?,
        )
        .map_err(|_| Error::invalid("cleanup plan encoding failed"))
    }
    fn cleanup_github(&self, operation: Uuid, config: &Config) -> Result<GithubClient> {
        let receipt = self.services.control.operation(operation)?;
        let workspace = receipt
            .expected
            .get("workspace")
            .or_else(|| receipt.expected.get("workspace_id"))
            .and_then(|id| Uuid::parse_str(id).ok())
            .or_else(|| {
                RunReference::from_receipt(&receipt)
                    .ok()
                    .flatten()
                    .map(|r| r.workspace)
            })
            .ok_or_else(|| {
                Error::new(
                    "scope_conflict",
                    "operation has no canonical workspace reference",
                )
            })?;
        self.clients(config)?
            .remove(&workspace)
            .map(|c| c.github)
            .ok_or_else(|| {
                Error::new(
                    "missing_secret",
                    "operation workspace has no verified provider connection",
                )
            })
    }
    async fn operation(
        &self,
        actor: Actor,
        request: CommandRequest,
        parent: Receipt,
        effect: MutationRequest,
        config: Config,
        snapshot: ControlSnapshot,
    ) -> Result<Receipt> {
        let operation = parse_uuid(text(&request.args, "operation")?)?;
        if request.command == CommandName::Cleanup {
            let github = self.cleanup_github(operation, &config)?;
            let this = self.clone();
            let revision = crate::config::config_revision(&config)?;
            let mut original = mutation_request(&actor, &request, parent.canonical_targets.clone());
            original.kind = "cleanup".into();
            return self
                .services
                .housekeeping
                .clone()
                .apply_for_command(
                    original,
                    operation,
                    text(&request.args, "plan")?.into(),
                    github,
                    Arc::new(move |actor| {
                        let config = this.services.configuration.snapshot()?;
                        this.services
                            .configuration
                            .authority_fences()?
                            .check_actor(actor, "recover")?;
                        verify_current_actor(&config, actor, "recover")?;
                        if crate::config::config_revision(&config)? != revision {
                            return Err(Error::new(
                                "stale_config",
                                "cleanup configuration changed",
                            ));
                        }
                        Ok(())
                    }),
                    parent,
                )
                .await;
        }
        let target = self.services.control.operation(operation)?;
        if target.runtime_id != config.runtime.id || target.request_id == parent.request_id {
            return Err(Error::new(
                "identity_mismatch",
                "recovery target is not another operation in this runtime",
            ));
        }
        let decision = AuthorizedOperationRecovery {
            actor: actor.clone(),
            request: request.clone(),
            parent_command: parent.clone(),
            operation: target,
            config_revision: crate::config::config_revision(&config)?,
            control_generation: snapshot.intent.generation,
        };
        decision.recheck(&config, &snapshot)?;
        let receipt = self.services.evidence.reconcile(decision).await?;
        self.finish(
            &actor,
            &request,
            &parent,
            effect,
            json!({"reconciled_operation":receipt.operation_id,"receipt":receipt}),
        )
    }
}
fn require_work(parent: &ParentProjection, work: &WorkIdentity, gated: bool) -> Result<()> {
    let current = work
        .child
        .and_then(|id| parent.children.get(&id).and_then(|c| c.work.as_ref()))
        .or_else(|| {
            if work.child.is_none() {
                parent.work.as_ref()
            } else {
                None
            }
        });
    if current != Some(work) || !parent.authorization_published {
        return Err(Error::new(
            "stale_result",
            "work authority or generation changed",
        ));
    }
    if gated
        && (!parent.corrections.pending_ids().is_empty()
            || !parent.blockers.is_empty()
            || work
                .child
                .and_then(|id| parent.children.get(&id))
                .is_some_and(|c| !c.blockers.is_empty()))
    {
        return Err(Error::new(
            "decision_blocked",
            "current correction, scope or authority blockers prevent this decision",
        ));
    }
    Ok(())
}
fn work_key(work: &WorkIdentity) -> Result<IssueKey> {
    IssueKey::new(
        &work.linear_endpoint,
        work.linear_workspace,
        work.child.unwrap_or(work.parent),
    )
}
fn resolve(
    projection: &RuntimeProjection,
    reference: &str,
    allow_child: bool,
) -> Result<(Uuid, IssueKey, Option<Uuid>)> {
    match projection.resolve_parent(reference) {
        Ok((ws, key)) => return Ok((ws, key, None)),
        Err(e) if allow_child && e.code == "not_found" => (),
        Err(e) => return Err(e),
    }
    let pieces: Vec<_> = reference.split('/').collect();
    let (filter, id) = match pieces.as_slice() {
        [id] => (None, *id),
        [ws, id] => (Some(*ws), *id),
        _ => {
            return Err(Error::invalid(
                "work reference requires workspace/key or unique key",
            ));
        }
    };
    let mut matches = vec![];
    for ws in projection
        .workspaces
        .values()
        .filter(|w| filter.is_none_or(|f| f == w.workspace_name || f == w.workspace_id.to_string()))
    {
        for parent in ws.parents.values() {
            for child in parent.children.values() {
                if child.key.issue.to_string() == id
                    || child.issue.as_ref().is_some_and(|i| i.identifier == id)
                {
                    matches.push((ws.workspace_id, parent.key.clone(), Some(child.key.issue)));
                }
            }
        }
    }
    match matches.as_slice() {
        [found] => Ok(found.clone()),
        [] => Err(Error::new(
            "not_found",
            "work does not resolve in configured scope",
        )),
        _ => Err(Error::new(
            "ambiguous_reference",
            "work matches multiple configured scopes",
        )),
    }
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| Error::invalid(format!("decision requires {key}")))
}
fn parse_uuid(text: &str) -> Result<Uuid> {
    Uuid::parse_str(text).map_err(|_| Error::invalid("operation must be a canonical UUID"))
}
fn check_message(message: &str, max: usize) -> Result<()> {
    if message.trim().is_empty() || message.len() > max || message.contains('\0') {
        Err(Error::invalid(
            "operator text must be nonempty bounded UTF-8 without NUL",
        ))
    } else {
        Ok(())
    }
}
fn after_write(error: Error) -> Error {
    Error::new(
        "operation_uncertain",
        "published decision requires observation/reconciliation; do not repeat the command",
    )
    .detail(json!({"cause":error.code}))
}
fn publication<T: Serialize>(
    signed: &Signed<T>,
    comment: &Comment,
    writer: &str,
) -> Result<Publication> {
    Ok(Publication {
        provider: "linear".into(),
        record_id: comment.id.to_string(),
        writer_account_id: writer.into(),
        payload_digest: canonical_digest(1, signed)?,
        observed_at: Utc::now(),
        url: comment.url.clone(),
    })
}
fn stage_id(effect: Uuid, stage: &str) -> Uuid {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("kegari:v0:decision-stage:{effect}:{stage}").as_bytes());
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn current_decision_comment<'a>(
    parent: &'a ParentProjection,
    target: Uuid,
    id: Uuid,
    body: &str,
    writer: &str,
) -> Result<&'a Comment> {
    let observed = parent.observations.get(&target).ok_or_else(|| {
        Error::new(
            "evidence_missing",
            "decision target has no current comment observation",
        )
    })?;
    let matching: Vec<_> = observed
        .comments
        .iter()
        .filter(|c| c.body == body)
        .collect();
    let comment = matching.first().ok_or_else(|| {
        Error::new(
            "evidence_missing",
            "decision publication was deleted or edited",
        )
    })?;
    if matching.len() != 1
        || comment.id != id
        || comment.issue_id != target
        || comment.bot
        || comment.delegated
        || comment.user_id.map(|id| id.to_string()).as_deref() != Some(writer)
    {
        return Err(Error::new(
            "evidence_missing",
            "decision publication identity/native provenance is no longer exact and unique",
        ));
    }
    Ok(comment)
}
