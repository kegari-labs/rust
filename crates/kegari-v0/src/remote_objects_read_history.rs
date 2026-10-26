//! Immutable history of an actual already imported source. This grants no Git
//! execution scope, acquisition request, worker authority or freshness.
use super::*;
use crate::control::Receipt;

#[derive(Clone)]
pub(crate) struct RetainedObjectReadHistory {
    source: Arc<VerifiedObjectSource>,
    control: Arc<ControlHost>,
    key: Arc<AttestationKey>,
    files: BTreeMap<String, String>,
    receipts: BTreeMap<Uuid, String>,
}
impl VerifiedObjectSource {
    pub(crate) fn retain_read_history(
        self: &Arc<Self>,
        control: Arc<ControlHost>,
        key: Arc<AttestationKey>,
    ) -> Result<RetainedObjectReadHistory> {
        self.revalidate_filesystem()?;
        let mut files = BTreeMap::new();
        for (name, maximum) in [
            ("source.json", 256 * 1024),
            ("intent.json", 256 * 1024),
            ("objects.json", 8 * 1024 * 1024),
        ] {
            files.insert(
                name.into(),
                bytes_digest(&self.directory.read_bytes(name, maximum)?),
            );
        }
        let effect = control.operation(self.record.effect)?;
        let owner = effect
            .expected
            .get("admission_operation")
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or_else(|| invalid("import has no exact admitted owner"))?;
        let owner = control.operation(owner)?;
        require_closed_owner(&owner)?;
        if effect.runtime_id != owner.runtime_id
            || effect.owner_generation != owner.owner_generation
            || owner.canonical_targets.len() != 1
            || effect.canonical_targets.first() != owner.canonical_targets.first()
            || effect.expected.get("admitted_work") != owner.canonical_targets.first()
            || effect.expected.get("admitted_scope") != owner.expected.get("scope")
        {
            return Err(invalid(
                "import and closed owner differ in runtime or scope",
            ));
        }
        let value = RetainedObjectReadHistory {
            source: self.clone(),
            control,
            key,
            files,
            receipts: BTreeMap::from([
                (effect.operation_id, canonical_digest(1, &effect)?),
                (owner.operation_id, canonical_digest(1, &owner)?),
            ]),
        };
        value.validate()?;
        Ok(value)
    }
}
impl RetainedObjectReadHistory {
    pub(crate) fn digest(&self) -> Result<String> {
        canonical_digest(
            1,
            &json!({"source":self.source.digest()?,"files":self.files,"receipts":self.receipts}),
        )
    }
    /// Protected reads/filesystem identity only; safe inside an already held
    /// Control gate. No Git subprocess, snapshot or nested serialization.
    pub(crate) fn validate(&self) -> Result<()> {
        self.source.revalidate_filesystem()?;
        let mut files = BTreeMap::new();
        for (name, expected) in &self.files {
            let maximum = if name == "objects.json" {
                8 * 1024 * 1024
            } else {
                256 * 1024
            };
            let bytes = self.source.directory.read_bytes(name, maximum)?;
            if bytes_digest(&bytes) != *expected {
                return Err(invalid("retained import file changed"));
            }
            files.insert(name.as_str(), bytes);
        }
        // Parse the exact bytes whose retained identity was just checked, not
        // another filesystem read that could select a different record.
        let signed: Signed<SourceRecord> = serde_json::from_slice(&files["source.json"])
            .map_err(|_| invalid("retained source marker is malformed"))?;
        self.key.verify("verified_object_source", &signed)?;
        if canonical_digest(1, &signed.payload)? != self.source.digest()? {
            return Err(invalid("retained source marker differs from actual source"));
        }
        let intent: Signed<serde_json::Value> = serde_json::from_slice(&files["intent.json"])
            .map_err(|_| invalid("retained import intent is malformed"))?;
        self.key.verify("object_acquisition_intent", &intent)?;
        let record = &self.source.record;
        if intent.payload["request"] != record.request_digest
            || intent.payload["effect"] != record.effect.to_string()
        {
            return Err(invalid("retained import intent differs from actual source"));
        }
        let objects: BTreeSet<String> = serde_json::from_slice(&files["objects.json"])
            .map_err(|_| invalid("retained import inventory is malformed"))?;
        if canonical_digest(1, &objects)? != record.objects_digest {
            return Err(invalid(
                "retained import inventory differs from actual source",
            ));
        }
        let mut receipts = BTreeMap::new();
        for (operation, expected) in &self.receipts {
            let receipt = self.control.operation(*operation)?;
            if canonical_digest(1, &receipt)? != *expected {
                return Err(invalid("retained import receipt or owner changed"));
            }
            receipts.insert(*operation, receipt);
        }
        let effect = &receipts[&record.effect];
        let source_digest = self.source.digest()?;
        if effect.kind != "git/object-import"
            || !matches!(
                effect.phase,
                ReceiptPhase::Acknowledged | ReceiptPhase::Reconciled
            )
            || effect.expected.get("object_request") != Some(&record.request_digest)
            || effect.expected.get("pack_digest") != Some(&record.pack_digest)
            || effect.observed_result.as_ref().is_none_or(|observed| {
                observed["object_import"] != "verified"
                    || observed["source_digest"] != source_digest
            })
        {
            return Err(invalid("retained source lost its actual successful import"));
        }
        self.source.revalidate_filesystem()
    }
}
fn require_closed_owner(receipt: &Receipt) -> Result<()> {
    if receipt.kind != "admission"
        || receipt.phase != ReceiptPhase::Reconciled
        || receipt.observed_result.as_ref().is_none_or(|observed| {
            observed["local_execution"] != "exited"
                || observed["unresolved_effects"]
                    .as_array()
                    .is_none_or(|effects| !effects.is_empty())
        })
    {
        return Err(invalid(
            "retained import requires its actually closed owner",
        ));
    }
    Ok(())
}
fn invalid(message: &str) -> Error {
    Error::new("source_history_unavailable", message)
}
