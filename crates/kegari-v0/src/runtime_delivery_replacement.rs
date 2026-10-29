//! Fresh readback of the original replacement assignment before cold session
//! recovery. These records never supply a completed worker or adopted process.
use super::*;

impl RuntimeDelivery {
    pub(super) async fn restore_replacement_assignments(
        &self,
        projection: &RuntimeProjection,
    ) -> Result<()> {
        for leaf in self.records.records_bounded(65_536)? {
            let Some(id) = leaf
                .strip_prefix("replacement-work-")
                .and_then(|leaf| leaf.strip_suffix(".json"))
            else {
                continue;
            };
            let operation: Uuid = id.parse().map_err(|_| {
                Error::new("record_corrupt", "replacement checkpoint name is invalid")
            })?;
            if self.runtime.retained_replacement_work(operation)?.is_some() {
                let latest = self.runtime.latest_replacement_operation(operation)?;
                if self.runtime.retained_replacement_work(latest)?.is_none() {
                    self.runtime.retain_replacement_successor_chain(operation)?;
                }
                continue;
            }
            let signed: kegari_secrets::Signed<Value> = self.records.read(&leaf)?;
            self.runtime
                .services
                .key
                .verify("runtime_replacement_work", &signed)?;
            if signed.payload.get("schema_version").and_then(Value::as_u64) == Some(1) {
                // The historical checkpoint omitted exact source/publication
                // identities. Preserve it without inventing recovery authority.
                continue;
            }
            let saved: ReplacementCheckpoint = serde_json::from_value(signed.payload)
                .map_err(|_| Error::new("record_corrupt", "replacement checkpoint is malformed"))?;
            if saved.operation != operation || saved.runtime != projection.runtime_id {
                return Err(Error::new(
                    "record_corrupt",
                    "replacement checkpoint belongs to another runtime or operation",
                ));
            }
            let parent = match projection.parent(saved.workspace, saved.work().parent) {
                Ok(parent) => parent,
                Err(_) => continue,
            };
            let result = async {
                let latest = self.runtime.latest_replacement_operation(operation)?;
                match self.runtime.control.operation(latest) {
                    Ok(receipt) => {
                        let run = crate::control::run_reference_from_receipt(&receipt)?
                            .ok_or_else(|| {
                                Error::new(
                                    "record_corrupt",
                                    "replacement checkpoint does not name a worker",
                                )
                            })?;
                        if run.phase.phase != "child_replacement"
                            || run.workspace != saved.workspace
                        {
                            return Err(Error::new(
                                "record_corrupt",
                                "replacement checkpoint changed phase or workspace",
                            ));
                        }
                        if self.completed_replacement_is_current(&receipt, &run, projection)? {
                            return Ok(());
                        }
                    }
                    Err(error) if error.code == "record_missing" && operation == latest => (),
                    Err(error) => return Err(error),
                }
                let gate = parent.fresh_gate(
                    saved.work().child,
                    GatePurpose::NewOperation,
                    &self.runtime.dependency_outcomes()?,
                    Utc::now(),
                    Duration::seconds(30),
                )?;
                if gate.work() != saved.work() {
                    return Err(Error::new(
                        "stale_generation",
                        "replacement's original work generation changed",
                    ));
                }
                let workspace = self.workspace(projection, saved.workspace)?;
                let (reader, owned) = self
                    .readonly_coordinator_with_successor(&workspace, &gate, None)
                    .await?;
                let restored = reader
                    .restore_replacement_work(&gate, &saved, &workspace.workflow)
                    .await;
                let closed = owned
                    .finish(json!({"replacement_restore":operation,"verified":restored.is_ok()}));
                let work = match (restored, closed) {
                    (Ok(work), Ok(_)) => work,
                    (Err(error), _) | (_, Err(error)) => return Err(error),
                };
                (self.current_gate())(&gate)?;
                self.runtime.retain_replacement_work(operation, work)?;
                self.runtime.retain_replacement_successor_chain(operation)
            }
            .await;
            if let Err(error) = result {
                self.runtime.invalidate_delivery(&parent.key, error)?;
            }
        }
        Ok(())
    }
}
