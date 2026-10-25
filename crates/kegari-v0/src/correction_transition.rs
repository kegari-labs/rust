//! Native correction lineage for an already admitted child. These capabilities
//! are deliberately not deserializable and do not reserve a new operation.
use super::*;
use crate::reconcile::{DependencyOutcomes, FreshGate};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize)]
struct CurrentRoute {
    child: Uuid,
    comment: Uuid,
    digest: String,
    scope_digest: String,
    instructions: String,
    validation_expectations: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
struct CurrentClassification {
    source: SourceRevision,
    class: CommentClass,
    publication: String,
    historical_revisions: BTreeSet<String>,
    routes: Vec<CurrentRoute>,
}
/// A complete, fresh native classification inventory. Historical revisions are
/// retained separately from the latest source text; old instructions never
/// become current merely because their history digest is still retained.
#[derive(Clone, Debug)]
pub struct VerifiedCorrectionRoutes {
    workspace: Uuid,
    work: WorkIdentity,
    watermark: u64,
    scope_digest: String,
    source_digest: String,
    observed_at: DateTime<Utc>,
    history: BTreeSet<String>,
    checkpoints: BTreeMap<String, BTreeSet<String>>,
    publications: Vec<String>,
    current: Vec<CurrentClassification>,
    definitions: serde_json::Value,
}
impl VerifiedCorrectionRoutes {
    pub fn work(&self) -> &WorkIdentity {
        &self.work
    }
    pub fn watermark(&self) -> u64 {
        self.watermark
    }
    pub fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &serde_json::json!({"workspace":self.workspace,"work":self.work,"watermark":self.watermark,"scope":self.scope_digest,"source":self.source_digest,"history":self.history,"checkpoints":self.checkpoints,"publications":self.publications,"current":self.current,"definitions":self.definitions}),
        )
    }
    pub(crate) fn validate_successor(
        &self,
        prior: &FreshGate,
        next: &FreshGate,
        source_digest: &str,
        now: DateTime<Utc>,
        age: chrono::Duration,
    ) -> Result<()> {
        let before = prior.work();
        let after = next.work();
        let checkpoint = self
            .checkpoints
            .get(&before.generation.correction)
            .ok_or_else(|| {
                invalid("admitted correction generation is not an exact signed history predecessor")
            })?;
        if self.observed_at > now
            || now - self.observed_at > age
            || self.source_digest != source_digest
            || checkpoint.len() as u64 != prior.correction_watermark()
            || !checkpoint.is_subset(&self.history)
            || checkpoint == &self.history
            || canonical_digest(1, &self.history)? != after.generation.correction
            || self.history.len() as u64 != next.correction_watermark()
            || self.work != *after
            || self.workspace != next.workspace_id()
            || Some(self.scope_digest.as_str()) != next.child_scope_digest()
            || before.linear_endpoint != after.linear_endpoint
            || before.linear_workspace != after.linear_workspace
            || before.parent != after.parent
            || before.child != after.child
            || before.child.is_none()
            || before.generation.authorization != after.generation.authorization
            || before.generation.scope != after.generation.scope
            || prior.workspace_id() != next.workspace_id()
            || prior.config_revision() != next.config_revision()
            || prior.workflow_digest() != next.workflow_digest()
            || canonical_digest(1, prior.profile())? != canonical_digest(1, next.profile())?
            || prior.child_scope_digest() != next.child_scope_digest()
            || prior.required_children() != next.required_children()
            || prior.dependency_digest() != next.dependency_digest()
        {
            return Err(invalid(
                "correction is stale, lacks exact history lineage or changes admitted authority, scope, settings or dependencies",
            ));
        }
        Ok(())
    }
    fn input(&self, prior: &FreshGate) -> Result<String> {
        let history = self
            .checkpoints
            .get(&prior.work().generation.correction)
            .ok_or_else(|| invalid("correction predecessor disappeared"))?;
        let added: BTreeSet<_> = self.history.difference(history).cloned().collect();
        let child = self
            .work
            .child
            .ok_or_else(|| invalid("correction target is not a child"))?;
        let changed: Vec<_> = self
            .current
            .iter()
            .filter(|item| !item.historical_revisions.is_disjoint(&added))
            .collect();
        if changed.is_empty() {
            return Err(invalid(
                "new corrective history has no verified current source classification",
            ));
        }
        let affected = changed
            .iter()
            .any(|item| item.routes.iter().any(|route| route.child == child));
        if !affected && prior.work().generation.definition != self.work.generation.definition {
            return Err(invalid(
                "changed definition has no actionable route to this child",
            ));
        }
        let input = serde_json::to_string(&serde_json::json!({
            "kind":"verified_child_correction", "work":self.work, "watermark":self.watermark,
            "affected":affected, "current_definitions":self.definitions,
            "classifications":changed.iter().map(|item| serde_json::json!({
                "source_comment":item.source.comment,"source_revision":item.source.body_digest,
                "source_author":item.source.author,"edited_at":item.source.edited_at,
                "class":item.class,"publication_digest":item.publication,
                "routes":item.routes.iter().filter(|route|route.child==child).collect::<Vec<_>>()
            })).collect::<Vec<_>>(),
            "completion":"Use only these current native instructions and the existing child scope. Report the processed watermark and input identity after completing the relevant work. Earlier checks retain their original generation; rerun affected checks. A classification without a route grants no new source change or inferred reversal."
        })).map_err(|_| invalid("correction input encoding failed"))?;
        if input.len() > 65_536 {
            return Err(invalid("correction input exceeds finite provider bound"));
        }
        Ok(input)
    }
}

/// An immutable chain from the original admitted work to an effective corrected
/// generation. Original instruction/tool/isolation contracts remain untouched.
#[derive(Clone, Debug)]
pub struct VerifiedCorrectionTransition {
    original: FreshGate,
    previous: Option<Arc<VerifiedCorrectionTransition>>,
    current: FreshGate,
    routes: VerifiedCorrectionRoutes,
    input: String,
    digest: String,
    depth: u32,
}
impl VerifiedCorrectionTransition {
    fn create(
        original: FreshGate,
        prior: &FreshGate,
        previous: Option<Arc<Self>>,
        current: FreshGate,
        routes: VerifiedCorrectionRoutes,
    ) -> Result<Self> {
        let depth = previous.as_ref().map_or(1, |p| p.depth + 1);
        if depth > 128 {
            return Err(invalid("correction transition chain exceeds finite bound"));
        }
        routes.validate_successor(
            prior,
            &current,
            current.sources_digest(),
            Utc::now(),
            chrono::Duration::seconds(30),
        )?;
        let input = routes.input(prior)?;
        let digest = canonical_digest(
            1,
            &serde_json::json!({"original":original.digest()?,"previous":previous.as_ref().map(|p|p.digest()),"current":current.digest()?,"routes":routes.digest()?,"input":bytes_digest(input.as_bytes())}),
        )?;
        Ok(Self {
            original,
            previous,
            current,
            routes,
            input,
            digest,
            depth,
        })
    }
    pub fn original_authority(&self) -> &FreshGate {
        &self.original
    }
    pub fn current_authority(&self) -> &FreshGate {
        &self.current
    }
    pub fn original_work(&self) -> &WorkIdentity {
        self.original.work()
    }
    pub fn effective_work(&self) -> &WorkIdentity {
        self.current.work()
    }
    pub fn watermark(&self) -> u64 {
        self.current.correction_watermark()
    }
    pub fn input(&self) -> &str {
        &self.input
    }
    pub fn input_digest(&self) -> String {
        bytes_digest(self.input.as_bytes())
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub fn route_digest(&self) -> Result<String> {
        self.routes.digest()
    }
    pub fn previous(&self) -> Option<&Arc<Self>> {
        self.previous.as_ref()
    }
    pub(crate) fn revalidate(
        &self,
        observations: &crate::observations::ObservationCache,
        outcomes: &DependencyOutcomes,
    ) -> Result<()> {
        let prior = self
            .previous
            .as_ref()
            .map_or(&self.original, |previous| &previous.current);
        let current = observations.active_corrected_gate(prior, &self.routes, outcomes)?;
        self.current
            .compare_current(&current, Utc::now(), chrono::Duration::seconds(30))
    }
    pub(crate) fn record(&self) -> Result<serde_json::Value> {
        Ok(
            serde_json::json!({"schema_version":1,"original":self.original,"original_digest":self.original.digest()?,"current_digest":self.current.digest()?,"previous":self.previous.as_ref().map(|p|p.digest()),"current":self.current,"routes":{"workspace":self.routes.workspace,"work":self.routes.work,"watermark":self.routes.watermark,"scope":self.routes.scope_digest,"source":self.routes.source_digest,"history":self.routes.history,"checkpoints":self.routes.checkpoints,"publications":self.routes.publications,"current":self.routes.current,"definitions":self.routes.definitions},"routes_digest":self.routes.digest()?,"input":self.input,"input_digest":self.input_digest(),"digest":self.digest,"depth":self.depth}),
        )
    }
}
impl CorrectionPublisher {
    /// Fresh provider reads only. An existing strong active snapshot is required;
    /// a worker message or a serialized gate cannot manufacture that ownership.
    pub async fn current_routes(&self, admitted: &FreshGate) -> Result<VerifiedCorrectionRoutes> {
        (self.services.control_owner_check)()?;
        self.services.observations.refresh().await?;
        let (config, parent) = self
            .services
            .observations
            .active_correction_context(admitted)?;
        let configured = config
            .workspaces
            .values()
            .find(|workspace| workspace.id == admitted.workspace_id())
            .ok_or_else(|| invalid("correction workspace disappeared"))?;
        observe_routes(
            &config,
            configured,
            &parent,
            admitted,
            &self.services.key,
            &self.services.control.operations()?,
        )
    }
    /// The successor gate is minted from the same current native observation and
    /// current permission checks, while retaining the admitted execution settings.
    pub async fn transition(
        &self,
        original: &FreshGate,
        previous: Option<Arc<VerifiedCorrectionTransition>>,
        outcomes: &DependencyOutcomes,
    ) -> Result<VerifiedCorrectionTransition> {
        if let Some(previous) = &previous
            && previous.original.digest()? != original.digest()?
        {
            return Err(invalid(
                "correction predecessor belongs to another original admission",
            ));
        }
        let prior = previous.as_ref().map_or(original, |p| &p.current);
        let routes = self.current_routes(prior).await?;
        let current = self
            .services
            .observations
            .active_corrected_gate(prior, &routes, outcomes)?;
        VerifiedCorrectionTransition::create(
            original.clone(),
            prior,
            previous.clone(),
            current,
            routes,
        )
    }
}

fn observe_routes(
    config: &Config,
    configured: &Workspace,
    parent: &ParentProjection,
    admitted: &FreshGate,
    key: &AttestationKey,
    receipts: &[Receipt],
) -> Result<VerifiedCorrectionRoutes> {
    let child = admitted
        .work()
        .child
        .ok_or_else(|| invalid("correction route needs an admitted child"))?;
    if !parent.authorization_published
        || !parent.corrections.pending_ids().is_empty()
        || parent.workspace_id != admitted.workspace_id()
        || parent.key.issue != admitted.work().parent
        || !parent.required_children.contains(&child)
    {
        return Err(invalid(
            "correction parent is unpublished, incomplete or has pending direction",
        ));
    }
    let selected = parent
        .children
        .get(&child)
        .ok_or_else(|| invalid("correction child disappeared"))?;
    let work = selected
        .work
        .clone()
        .ok_or_else(|| invalid("correction child lacks current authorized work"))?;
    let scope_digest = canonical_digest(
        1,
        selected
            .scope
            .as_ref()
            .ok_or_else(|| invalid("correction child lacks explicit scope"))?,
    )?;
    let chain = read_chain(config, configured, parent, key, receipts)?;
    let history = parent.corrections.invalidations().corrective_revisions;
    if chain.last().map(|last| &last.signed.payload.history) != Some(&history)
        || canonical_digest(1, &history)? != work.generation.correction
        || history.len() as u64 != parent.corrections.watermark
    {
        return Err(invalid(
            "current correction history lacks its exact final signed publication",
        ));
    }
    let mut checkpoints = BTreeMap::new();
    let empty = BTreeSet::<String>::new();
    checkpoints.insert(canonical_digest(1, &empty)?, empty);
    let mut latest = BTreeMap::new();
    let mut revisions: BTreeMap<Uuid, BTreeSet<String>> = BTreeMap::new();
    let mut publications = Vec::new();
    for intake in &chain {
        let record = &intake.signed.payload;
        publications.push(intake.signed.digest()?);
        for checkpoint in [&record.prior_history, &record.history] {
            checkpoints.insert(canonical_digest(1, checkpoint)?, checkpoint.clone());
        }
        for item in &record.classifications {
            revisions
                .entry(item.source.comment)
                .or_default()
                .insert(item.source.revision().identity_digest()?);
            for route in &item.routes {
                verify_route(config, parent, record, item, route, key, receipts)?;
            }
            latest.insert(item.source.comment, (intake, item));
        }
    }
    let writer = &config.connections[&configured.linear_connection].account_id;
    let mut current = Vec::new();
    for (id, (intake, item)) in latest {
        // Deleted informational comments confer no actionable input. A missing
        // corrective source or route still fails exact native reconstruction.
        if item.class == CommentClass::Informational
            && !parent
                .observations
                .values()
                .any(|o| o.comments.iter().any(|c| c.id == id))
        {
            continue;
        }
        if source(parent, configured, writer, key, config.runtime.id, id)? != item.source {
            return Err(invalid(
                "native correction revision changed after final classification",
            ));
        }
        if item.class == CommentClass::Ambiguous {
            return Err(invalid(
                "ambiguous direction cannot become a correction transition",
            ));
        }
        let before = &intake.signed.payload.before.generation;
        if before.authorization != work.generation.authorization
            || before.scope != work.generation.scope
            || before.definition != work.generation.definition
        {
            return Err(invalid(
                "current classification belongs to superseded authority or definition",
            ));
        }
        let mut routes = Vec::new();
        for reference in &item.routes {
            let comment = native(
                parent,
                reference.child,
                reference.comment,
                &reference.body_digest,
                writer,
            )?;
            let signed: Signed<RouteRecord> =
                decode_signed_publication(&comment.body, "correction_route", key)?;
            routes.push(CurrentRoute {
                child: reference.child,
                comment: reference.comment,
                digest: signed.digest()?,
                scope_digest: reference.scope_digest.clone(),
                instructions: signed.payload.instructions,
                validation_expectations: signed.payload.validation_expectations,
            });
        }
        current.push(CurrentClassification {
            source: item.source.clone(),
            class: item.class,
            publication: intake.signed.digest()?,
            historical_revisions: revisions.remove(&id).unwrap_or_default(),
            routes,
        });
    }
    Ok(VerifiedCorrectionRoutes {
        workspace: parent.workspace_id,
        work,
        watermark: parent.corrections.watermark,
        scope_digest,
        source_digest: parent.source_digest.clone(),
        observed_at: parent.observed_at,
        history,
        checkpoints,
        publications,
        current,
        definitions: serde_json::json!({"parent":parent.issue,"child":selected.issue,"scope":selected.scope,"required_children":parent.required_children}),
    })
}
