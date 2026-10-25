//! Bounded semantic fingerprints of provider-observed terminal turns. History
//! reconstructed from turn IDs or summary views is never complete evidence.
use super::*;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RecordedThreadHistory {
    turns: Vec<RecordedTurn>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedTurn {
    id: String,
    status: String,
    content_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transient_failure: Option<RecordedTransientFailure>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedTransientFailure {
    classification: TransientTurnFailure,
    terminal_digest: String,
    error_digest: String,
    policy_free: bool,
}
impl RecordedThreadHistory {
    pub(super) fn observe(&mut self, turn: &Value) -> Result<()> {
        let id = required_str(turn, "id")?;
        let status = required_str(turn, "status")?;
        let content_digest = full_turn_digest(turn)?;
        let transient_failure = match (&content_digest, turn.get("error")) {
            (Some(digest), Some(error))
                if status == "failed" && error.get("misalignment").is_none_or(Value::is_null) =>
            {
                let classification = match error.get("codexErrorInfo").and_then(Value::as_str) {
                    Some("serverOverloaded") => Some(TransientTurnFailure::ServerOverloaded),
                    Some("internalServerError") => Some(TransientTurnFailure::InternalServerError),
                    _ => None,
                };
                classification
                    .map(|classification| {
                        Ok::<_, Error>(RecordedTransientFailure {
                            classification,
                            terminal_digest: digest.clone(),
                            error_digest: canonical_digest(1, error)?,
                            policy_free: true,
                        })
                    })
                    .transpose()?
            }
            _ => None,
        };
        if let Some(existing) = self.turns.iter().find(|t| t.id == id) {
            if existing.status != status
                || existing.content_digest.is_some() && existing.content_digest != content_digest
            {
                return Err(Error::new(
                    "protocol_error",
                    "duplicate provider turn changed its retained contents",
                ));
            }
            return Ok(());
        }
        if self.turns.len() >= 4096 {
            return Err(Error::new(
                "history_limit",
                "provider history exceeds the finite turn inventory",
            ));
        }
        self.turns.push(RecordedTurn {
            id: id.into(),
            status: status.into(),
            content_digest,
            transient_failure,
        });
        Ok(())
    }
    pub(super) fn validate(
        &self,
        completed: &BTreeMap<String, String>,
        last: Option<&str>,
    ) -> Result<()> {
        let mut observed = BTreeMap::new();
        if self.turns.len() > 4096 || self.turns.last().map(|t| t.id.as_str()) != last {
            return Err(Error::new(
                "corrupt_receipt",
                "retained history order or bound differs",
            ));
        }
        for turn in &self.turns {
            if turn.id.is_empty()
                || !["completed", "failed", "interrupted"].contains(&turn.status.as_str())
                || observed
                    .insert(turn.id.clone(), turn.status.clone())
                    .is_some()
            {
                return Err(Error::new(
                    "corrupt_receipt",
                    "retained history contains invalid or duplicate turns",
                ));
            }
            if let Some(digest) = &turn.content_digest {
                crate::evidence::validate_digest(digest)?;
            }
            validate_failure(turn)?;
        }
        if &observed != completed {
            return Err(Error::new(
                "corrupt_receipt",
                "retained history differs from completed turns",
            ));
        }
        Ok(())
    }
    pub(super) fn terminal_transient_failure(
        &self,
        last: &str,
    ) -> Result<Option<TransientTurnFailure>> {
        let Some(turn) = self
            .turns
            .last()
            .filter(|turn| turn.id == last && turn.status == "failed")
        else {
            return Ok(None);
        };
        validate_failure(turn)?;
        Ok(turn
            .transient_failure
            .as_ref()
            .map(|failure| failure.classification.clone()))
    }
    pub(super) fn verify_readback(&self, turns: &[Value]) -> Result<()> {
        if turns.len() != self.turns.len() {
            return Err(Error::new(
                "reconciliation_required",
                "provider history inventory changed",
            ));
        }
        for (observed, retained) in turns.iter().zip(&self.turns) {
            let digest = full_turn_digest(observed)?;
            if required_str(observed, "id")? != retained.id
                || required_str(observed, "status")? != retained.status
                || retained.content_digest.is_none()
                || digest != retained.content_digest
            {
                return Err(Error::new(
                    "reconciliation_required",
                    "provider history is incomplete, reordered or changed",
                ));
            }
        }
        Ok(())
    }
}
fn validate_failure(turn: &RecordedTurn) -> Result<()> {
    if let Some(failure) = &turn.transient_failure {
        crate::evidence::validate_digest(&failure.terminal_digest)?;
        crate::evidence::validate_digest(&failure.error_digest)?;
        if turn.status != "failed"
            || !failure.policy_free
            || turn.content_digest.as_deref() != Some(failure.terminal_digest.as_str())
        {
            return Err(Error::new(
                "corrupt_receipt",
                "transient classification differs from its full failed terminal observation",
            ));
        }
    }
    Ok(())
}

/// Missing optional fields are normalized only according to the pinned Turn
/// schema. Every item/content field remains byte-semantically represented.
fn full_turn_digest(turn: &Value) -> Result<Option<String>> {
    match turn.get("itemsView") {
        None => (), // The pinned schema defaults only an absent field to full.
        Some(Value::String(view)) if view == "full" => (),
        Some(Value::String(view)) if matches!(view.as_str(), "summary" | "notLoaded") => {
            return Ok(None);
        }
        Some(_) => {
            return Err(Error::new(
                "protocol_error",
                "provider turn has an invalid items view",
            ));
        }
    }
    let Some(items) = turn.get("items").and_then(Value::as_array) else {
        return Ok(None);
    };
    if items.len() > 4096 {
        return Err(Error::new(
            "history_limit",
            "provider turn item inventory exceeds its bound",
        ));
    }
    let mut ids = BTreeSet::new();
    for item in items {
        if !ids.insert(required_str(item, "id")?) {
            return Err(Error::new(
                "protocol_error",
                "provider history contains duplicate item IDs",
            ));
        }
        required_str(item, "type")?;
    }
    let mut normalized = turn.clone();
    let object = normalized
        .as_object_mut()
        .ok_or_else(|| Error::new("protocol_error", "turn is not an object"))?;
    object.insert("itemsView".into(), json!("full"));
    for key in [
        "error",
        "rootTurnId",
        "startedAt",
        "completedAt",
        "durationMs",
    ] {
        object.entry(key).or_insert(Value::Null);
    }
    canonical_digest(1, &normalized).map(Some)
}
