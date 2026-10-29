//! Bounded diagnostic publication through the existing metadata receipt path.
//! A blocker notice grants no worker, repository or delivery authority.
use super::*;
use crate::authority::IssueKey;
use crate::reconcile::{ParentProjection, RuntimeProjection};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
struct Notice {
    schema_version: u32,
    workspace: Uuid,
    affected: IssueKey,
    parent: IssueKey,
    generation: Option<crate::evidence::Generation>,
    code: String,
    reason: String,
    impacted_gates: Vec<String>,
    last_verified_evidence: Option<String>,
    attempted_recovery: String,
    next_action: String,
}
#[derive(Clone, Debug)]
pub(crate) struct BlockerPublication {
    notice: Notice,
    work: String,
    config_revision: String,
    source_digest: String,
    digest: String,
    body: String,
}
impl BlockerPublication {
    pub(crate) fn work_key(&self) -> &str {
        &self.work
    }
    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }
}
fn bounded(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || *c == ' ')
        .take(limit)
        .collect()
}
fn next_action(error: &Error) -> String {
    if let Some(action) = error
        .next_action
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        return bounded(action, 1024);
    }
    match error.code.as_str() {
        "attempts_exhausted" | "review_rounds_exhausted" => "Inspect the retained attempts, resolve the cause, then grant an explicit bounded retry.",
        "retry_wait" | "rate_limited" | "quota_exhausted" => "Wait for the recorded retry or account-refresh time; current exhaustion is not cleared by a timer alone.",
        "authorization_unpublished" => "Allow the coordinator to publish the current verified authorization; inspect any retained publication error.",
        "incompletely_prepared" | "invalid_metadata" | "invalid_config" => "Correct the affected issue metadata or operator configuration, then reconcile current scope.",
        "authorization_provenance_unavailable" | "renewed_authorization_required" => "Prepare the complete intended parent and child scope, then apply the configured human authorization label.",
        "provider_intervention" | "persistent_blocker" => "Inspect the retained failure and satisfy the provider or workflow resolution before requesting another attempt.",
        "reconciliation_required" | "operation_uncertain" => "Reconcile the original operation and provider result; do not repeat an uncertain write.",
        "scope_violation" | "access_denied" | "permission_fenced" => "Restore the intended scoped permission or explicitly revise operator configuration, then reconcile.",
        _ => "Inspect this work with `kegari-v0 show`, resolve the reported prerequisite, and reconcile before requesting more work.",
    }.into()
}
fn target<'a>(
    projection: &'a RuntimeProjection,
    workspace: Uuid,
    parent_id: Uuid,
    issue: &IssueKey,
) -> Result<&'a ParentProjection> {
    let observed = projection
        .workspaces
        .get(&workspace)
        .filter(|w| w.complete)
        .ok_or_else(|| {
            Error::new(
                "incomplete_snapshot",
                "blocker publication requires complete workspace observations",
            )
        })?;
    let parent = observed
        .parents
        .get(&parent_id)
        .filter(|p| {
            p.included
                && p.issue
                    .as_ref()
                    .is_some_and(|i| i.id == parent_id && i.parent_id.is_none())
        })
        .ok_or_else(|| {
            Error::new(
                "scope_violation",
                "blocker parent is not currently included and accessible",
            )
        })?;
    if issue.endpoint != parent.key.endpoint || issue.workspace != parent.key.workspace {
        return Err(Error::new(
            "scope_violation",
            "blocker target crosses its observed provider namespace",
        ));
    }
    if issue != &parent.key
        && !(parent.required_children.contains(&issue.issue)
            && parent.children.get(&issue.issue).is_some_and(|child| {
                &child.key == issue
                    && child
                        .issue
                        .as_ref()
                        .is_some_and(|i| i.id == issue.issue && i.parent_id == Some(parent_id))
            }))
    {
        return Err(Error::new(
            "scope_violation",
            "blocker target is outside the current included hierarchy",
        ));
    }
    let now = Utc::now();
    if parent.observed_at > now + Duration::seconds(5)
        || now - parent.observed_at > Duration::seconds(30)
    {
        return Err(Error::new(
            "stale_observation",
            "blocker publication needs fresh provider observations",
        ));
    }
    Ok(parent)
}
impl AuthorizationPublisher {
    pub(crate) fn blocker_publications(
        &self,
        errors: &BTreeMap<String, Error>,
    ) -> Result<Vec<BlockerPublication>> {
        let config = self.configuration.snapshot()?;
        let projection = self.observations.admission_snapshot()?;
        let revision = crate::config::config_revision(&config)?;
        if projection.runtime_id != config.runtime.id || projection.config_revision != revision {
            return Err(Error::new(
                "stale_config",
                "blocker projection differs from current configuration",
            ));
        }
        let fences = self.configuration.authority_fences()?;
        let mut notices = Vec::new();
        for workspace in projection.workspaces.values().filter(|w| w.complete) {
            let Some(configured) = config
                .workspaces
                .values()
                .find(|w| w.id == workspace.workspace_id && w.evaluation.is_none())
            else {
                continue;
            };
            if fences.check_workspace(configured.id).is_err() {
                continue;
            }
            for parent in workspace.parents.values().filter(|p| p.included) {
                for issue in std::iter::once(&parent.key)
                    .chain(parent.children.values().map(|child| &child.key))
                {
                    let work = issue.identity()?;
                    let Some(error) = errors.get(&work) else {
                        continue;
                    };
                    // Independent PR jobs, absent issues and excluded/unconfigured
                    // scopes remain local; an error hash alone grants no target.
                    if target(&projection, workspace.workspace_id, parent.key.issue, issue).is_err()
                    {
                        continue;
                    }
                    if error.code.is_empty()
                        || error.code.len() > 128
                        || !error
                            .code
                            .bytes()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
                    {
                        continue;
                    }
                    let actual = parent
                        .blockers
                        .iter()
                        .chain(
                            parent
                                .children
                                .get(&issue.issue)
                                .into_iter()
                                .flat_map(|child| child.blockers.iter()),
                        )
                        .find(|blocker| blocker.affected == *issue && blocker.code == error.code);
                    let notice = Notice {
                        schema_version: 1,
                        workspace: workspace.workspace_id,
                        affected: issue.clone(),
                        parent: parent.key.clone(),
                        generation: parent.work.as_ref().map(|work| work.generation.clone()),
                        code: error.code.clone(),
                        reason: bounded(&error.message, 1024),
                        impacted_gates: actual.map_or_else(
                            || vec!["admission".into(), "completion".into(), "release".into()],
                            |blocker| blocker.impacted_gates.clone(),
                        ),
                        last_verified_evidence: actual
                            .and_then(|b| b.last_verified_evidence.clone()),
                        attempted_recovery:
                            "Current provider reconciliation and durable attempt/receipt inspection"
                                .into(),
                        next_action: actual
                            .map_or_else(|| next_action(error), |b| bounded(&b.next_action, 1024)),
                    };
                    // No timestamps or raw comment inventory: our own diagnostic
                    // must not turn the next observation into another publication.
                    let digest =
                        canonical_digest(1, &json!({"notice":notice,"configuration":revision}))?;
                    let body = format!(
                        "Kegari is blocked for Linear issue {}.\n\nReason: `{}` — {}\n\nImpacted gates: {}.\nLast verified evidence: {}.\nAttempted recovery: {}.\n\nNext action: {}\n\n<!-- kegari:v0:scheduling_blocker:{} -->",
                        issue.issue,
                        notice.code,
                        notice.reason,
                        notice.impacted_gates.join(", "),
                        notice
                            .last_verified_evidence
                            .as_deref()
                            .unwrap_or("no passing evidence inferred"),
                        notice.attempted_recovery,
                        notice.next_action,
                        digest
                    );
                    notices.push(BlockerPublication {
                        notice,
                        work,
                        config_revision: revision.clone(),
                        source_digest: parent.source_digest.clone(),
                        digest,
                        body,
                    });
                }
            }
        }
        Ok(notices)
    }
    pub(crate) fn next_blocker_publication(&self, notice: &BlockerPublication) -> Result<Uuid> {
        let config = self.configuration.snapshot()?;
        Ok(metadata_publication_attempt(
            "scheduling_blocker",
            config.runtime.id,
            &notice.digest,
            &notice.work,
            &self.control.operations()?,
            &config.runtime,
            Utc::now(),
        )?
        .operation)
    }
    fn current_blocker(&self, notice: &BlockerPublication, lease: &Lease) -> Result<()> {
        (self.owner)(lease)?;
        self.configuration
            .authority_fences()?
            .check_workspace(notice.notice.workspace)?;
        if crate::config::config_revision(&self.configuration.snapshot()?)?
            != notice.config_revision
        {
            return Err(Error::new(
                "stale_config",
                "blocker configuration changed before publication",
            ));
        }
        let projection = self.observations.admission_snapshot()?;
        let parent = target(
            &projection,
            notice.notice.workspace,
            notice.notice.parent.issue,
            &notice.notice.affected,
        )?;
        if parent.source_digest != notice.source_digest
            || parent.config_revision != notice.config_revision
            || parent.work.as_ref().map(|work| &work.generation)
                != notice.notice.generation.as_ref()
        {
            return Err(Error::new(
                "stale_authorization",
                "blocker target observations changed before publication",
            ));
        }
        Ok(())
    }
    pub(crate) async fn publish_blocker(
        self: &Arc<Self>,
        lease: &Lease,
        notice: BlockerPublication,
    ) -> Result<()> {
        let config = self.configuration.snapshot()?;
        let plan = metadata_publication_attempt(
            "scheduling_blocker",
            config.runtime.id,
            &notice.digest,
            &notice.work,
            &self.control.operations()?,
            &config.runtime,
            Utc::now(),
        )?;
        let (linear, writer, runtime) = self.client_for_workspace(notice.notice.workspace)?;
        let prior = self
            .control
            .operations()?
            .into_iter()
            .find(|r| r.operation_id == plan.operation);
        let receipt = if let Some(receipt) = prior {
            receipt
        } else {
            self.current_blocker(&notice, lease)?;
            let permit = self.control.admit_metadata(
                lease,
                MetadataAdmission {
                    request_id: plan.admission,
                    work_key: notice.work.clone(),
                    scope_digest: notice.digest.clone(),
                    reference: plan.reference,
                },
                || self.current_blocker(&notice, lease),
            )?;
            let request = MutationRequest {
                request_id: plan.operation,
                runtime_id: runtime,
                actor: Actor {
                    identity: format!("owner:{}", lease.owner_id),
                    roles: Default::default(),
                    principal: None,
                },
                kind: "scheduling_blocker".into(),
                canonical_targets: vec![notice.work.clone()],
                expected: BTreeMap::from([
                    ("preparation".into(), "scheduling_blocker".into()),
                    ("snapshot".into(), notice.digest.clone()),
                    ("workspace".into(), notice.notice.workspace.to_string()),
                    ("parent".into(), notice.notice.parent.issue.to_string()),
                    ("body_digest".into(), bytes_digest(notice.body.as_bytes())),
                    ("writer".into(), writer.clone()),
                    ("blocker_body".into(), notice.body.clone()),
                    (
                        "blocker_issue".into(),
                        notice.notice.affected.issue.to_string(),
                    ),
                    ("blocker_work".into(), notice.work.clone()),
                ]),
                payload: json!({}),
            };
            let this = self.clone();
            let checked = notice.clone();
            let lease = lease.clone();
            let result = ProviderBroker {
                linear: Some(linear.clone()),
                github: None,
            }
            .mutate(
                self.control.clone(),
                request,
                Some(permit),
                None,
                ProviderMutation::LinearComment {
                    issue: notice.notice.affected.issue,
                    body: notice.body.clone(),
                },
                move || this.current_blocker(&checked, &lease),
            )
            .await;
            self.control.retire_local(plan.admission, json!({"phase":"blocker_publication","result":if result.is_ok(){"provider_acknowledged"}else{"blocked"},"error":result.as_ref().err()}))?;
            result?
        };
        self.recover_blocker_receipt(lease, receipt.operation_id)
            .await
    }
    /// Exact readback only. Persisted diagnostic text never authorizes a resend.
    pub(crate) async fn recover_blocker_receipt(
        &self,
        lease: &Lease,
        operation: Uuid,
    ) -> Result<()> {
        (self.owner)(lease)?;
        let receipt = self.control.operation(operation)?;
        let value = |key: &str| {
            receipt.expected.get(key).cloned().ok_or_else(|| {
                Error::new(
                    "record_corrupt",
                    "blocker receipt lacks its original bounded publication",
                )
            })
        };
        let workspace: Uuid = value("workspace")?
            .parse()
            .map_err(|_| Error::new("record_corrupt", "blocker workspace is invalid"))?;
        let issue: Uuid = value("blocker_issue")?
            .parse()
            .map_err(|_| Error::new("record_corrupt", "blocker issue is invalid"))?;
        let body = value("blocker_body")?;
        let digest = value("snapshot")?;
        if body.is_empty() || body.len() > 16384 || issue.is_nil() {
            return Err(Error::new(
                "record_corrupt",
                "blocker body or target is invalid",
            ));
        }
        let (linear, writer, runtime) = self.client_for_workspace(workspace)?;
        let target = format!(
            "linear:{}:{}:{}",
            linear.endpoint(),
            linear.workspace_id(),
            issue
        );
        if receipt.runtime_id != runtime
            || receipt.kind != "linear_comment"
            || receipt.canonical_targets != vec![target]
            || receipt.expected.get("preparation").map(String::as_str) != Some("scheduling_blocker")
            || receipt.expected.get("provider_body_digest") != Some(&bytes_digest(body.as_bytes()))
            || receipt.expected.get("provider_writer") != Some(&writer)
        {
            return Err(Error::new(
                "ownership_conflict",
                "blocker receipt differs from its exact original publication",
            ));
        }
        let admission_id: Uuid = value("admission_operation")?
            .parse()
            .map_err(|_| Error::new("record_corrupt", "blocker admission is invalid"))?;
        let admission = self.control.operation(admission_id)?;
        let reference = MetadataReference::from_receipt(&admission)?
            .ok_or_else(|| Error::new("record_corrupt", "blocker has no metadata admission"))?;
        if reference.purpose != "scheduling_blocker"
            || reference.generation != digest
            || admission.runtime_id != runtime
            || admission.canonical_targets != vec![value("blocker_work")?]
        {
            return Err(Error::new(
                "ownership_conflict",
                "blocker admission differs from original work",
            ));
        }
        if receipt.phase == ReceiptPhase::Reconciled
            && receipt
                .observed_result
                .as_ref()
                .is_some_and(|r| r["blocker_digest"] == digest)
        {
            return Ok(());
        }
        if let Some(error) = receipt
            .observed_result
            .as_ref()
            .filter(|r| r["effect"] == "not_sent")
            .and_then(|r| r.get("error"))
        {
            return Err(serde_json::from_value(error.clone())
                .map_err(|_| Error::new("record_corrupt", "invalid retained blocker error"))?);
        }
        let comments = linear.comments(issue).await?;
        let matches: Vec<_> = comments
            .iter()
            .filter(|comment| {
                comment.issue_id == issue
                    && comment.body == body
                    && !comment.bot
                    && !comment.delegated
                    && comment.user_id.is_some_and(|id| id.to_string() == writer)
            })
            .collect();
        if matches.len() != 1 {
            return Err(Error::new(
                if matches.is_empty() {
                    "operation_uncertain"
                } else {
                    "ownership_conflict"
                },
                "blocker requires one exact current provider record; unknown writes are not resent",
            ));
        }
        let comment = matches[0];
        if receipt
            .remote_ids
            .get("comment_id")
            .is_some_and(|id| id != &comment.id.to_string())
        {
            return Err(Error::new(
                "ownership_conflict",
                "blocker provider result changed identity",
            ));
        }
        (self.owner)(lease)?;
        self.control.reconcile_owned(
            &receipt,
            Reconciliation::UniqueVerified {
                remote_ids: BTreeMap::from([("comment_id".into(), comment.id.to_string())]),
                result: json!({"blocker_digest":digest,"url":comment.url}),
                payload_digest: receipt.payload_digest.clone(),
                actor: receipt.actor.clone(),
            },
        )?;
        Ok(())
    }
}
