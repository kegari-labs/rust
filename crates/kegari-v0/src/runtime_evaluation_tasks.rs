//! Host-owned evaluation task slots. This table supplies no model authority.
use crate::{
    control::Receipt,
    error::{Error, Result},
};
use std::{collections::BTreeMap, future::Future, sync::Mutex};
use tokio::{sync::Notify, task::JoinHandle};
use uuid::Uuid;

struct Slot {
    identity: String,
    handle: Option<JoinHandle<Result<Receipt>>>,
    joining: bool,
    result: Option<Result<Receipt>>,
}
#[derive(Default)]
pub(super) struct EvaluationTasks {
    slots: Mutex<BTreeMap<Uuid, Slot>>,
    wake: Notify,
}
impl EvaluationTasks {
    /// Caller holds the actual command's serialized admission gate. Starting a
    /// future never replaces a prior command, including failed or unknown work.
    pub fn start(
        &self,
        operation: Uuid,
        identity: String,
        run: impl Future<Output = Result<Receipt>> + Send + 'static,
    ) -> Result<()> {
        let mut slots = self.slots.lock().map_err(|_| unavailable())?;
        if operation.is_nil() {
            return Err(Error::invalid("evaluation task has no operation"));
        }
        if let Some(old) = slots.get(&operation) {
            return if old.identity == identity {
                Ok(())
            } else {
                Err(Error::new(
                    "request_conflict",
                    "evaluation operation already names another exact request",
                ))
            };
        }
        if slots.len() >= 65_536 || slots.values().filter(|s| s.result.is_none()).count() >= 32 {
            return Err(Error::new(
                "capacity",
                "evaluation task retention or active command bound reached",
            ));
        }
        slots.insert(
            operation,
            Slot {
                identity,
                handle: Some(tokio::spawn(run)),
                joining: false,
                result: None,
            },
        );
        self.wake.notify_waiters();
        Ok(())
    }
    pub fn pending(&self) -> Result<usize> {
        Ok(self
            .slots
            .lock()
            .map_err(|_| unavailable())?
            .values()
            .filter(|s| s.result.is_none())
            .count())
    }
    /// Take only already-finished handles, retaining an explicit joining slot
    /// across await. A concurrent drain cannot lose ownership or report clear.
    pub async fn drain_finished(&self) -> Result<()> {
        let finished = {
            let mut slots = self.slots.lock().map_err(|_| unavailable())?;
            let ids: Vec<_> = slots
                .iter()
                .filter(|(_, s)| s.handle.as_ref().is_some_and(JoinHandle::is_finished))
                .map(|(id, _)| *id)
                .collect();
            let mut finished = Vec::new();
            for id in ids {
                let slot = slots.get_mut(&id).expect("selected present slot");
                slot.joining = true;
                finished.push((id, slot.handle.take().expect("selected finished handle")));
            }
            finished
        };
        for (id, handle) in finished {
            let result = handle.await.unwrap_or_else(|_| {
                Err(Error::new(
                    "operation_uncertain",
                    "owned evaluation task did not return its exact terminal result",
                ))
            });
            let mut slots = self.slots.lock().map_err(|_| unavailable())?;
            let slot = slots.get_mut(&id).ok_or_else(|| {
                Error::new(
                    "ownership_conflict",
                    "owned evaluation join slot disappeared",
                )
            })?;
            if !slot.joining || slot.handle.is_some() || slot.result.is_some() {
                return Err(Error::new(
                    "ownership_conflict",
                    "evaluation join was already consumed",
                ));
            }
            slot.joining = false;
            slot.result = Some(result);
            self.wake.notify_waiters();
        }
        Ok(())
    }
    pub fn joined_operations(&self) -> Result<Vec<Uuid>> {
        Ok(self
            .slots
            .lock()
            .map_err(|_| unavailable())?
            .iter()
            .filter(|(_, s)| s.result.is_some() && !s.joining && s.handle.is_none())
            .map(|(id, _)| *id)
            .collect())
    }
    pub fn joined_result(&self, operation: Uuid) -> Result<Option<Result<Receipt>>> {
        let slots = self.slots.lock().map_err(|_| unavailable())?;
        Ok(slots
            .get(&operation)
            .filter(|slot| !slot.joining && slot.handle.is_none())
            .and_then(|slot| slot.result.clone()))
    }
    pub async fn wait(&self, operation: Uuid) -> Result<Receipt> {
        loop {
            let wake = self.wake.notified();
            self.drain_finished().await?;
            if let Some(result) = self
                .slots
                .lock()
                .map_err(|_| unavailable())?
                .get(&operation)
                .ok_or_else(|| {
                    Error::new(
                        "operation_missing",
                        "evaluation task has no retained host owner",
                    )
                })?
                .result
                .clone()
            {
                return result;
            }
            // Wake also from task completion. A bounded local tick guarantees
            // join even when no model/provider notification accompanies exit.
            tokio::select! { _=wake=>(), _=tokio::time::sleep(std::time::Duration::from_millis(100))=>() }
        }
    }
}
fn unavailable() -> Error {
    Error::new(
        "runtime_unavailable",
        "evaluation task registry lock failed",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    #[tokio::test]
    async fn one_owned_task_survives_waiter_loss_and_joins_once() {
        let tasks = Arc::new(EvaluationTasks::default());
        let operation = Uuid::new_v4();
        let count = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let count_task = count.clone();
        let release_task = release.clone();
        tasks
            .start(operation, "same-request".into(), async move {
                count_task.fetch_add(1, Ordering::SeqCst);
                release_task.notified().await;
                Err(Error::new(
                    "fixture_terminal",
                    "actual component task returned",
                ))
            })
            .unwrap();
        let waiter_tasks = tasks.clone();
        let waiter = tokio::spawn(async move { waiter_tasks.wait(operation).await });
        tokio::task::yield_now().await;
        waiter.abort();
        let _ = waiter.await;
        assert_eq!(tasks.pending().unwrap(), 1);
        assert!(tasks.joined_result(operation).unwrap().is_none());
        let duplicate = count.clone();
        tasks
            .start(operation, "same-request".into(), async move {
                duplicate.fetch_add(100, Ordering::SeqCst);
                Err(Error::new("invalid", "duplicate task must never run"))
            })
            .unwrap();
        assert_eq!(
            tasks
                .start(operation, "changed-request".into(), async {
                    Err(Error::invalid("never polled"))
                })
                .unwrap_err()
                .code,
            "request_conflict"
        );
        release.notify_one();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), tasks.wait(operation))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(result.code, "fixture_terminal");
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(tasks.pending().unwrap(), 0);
        assert_eq!(
            tasks
                .joined_result(operation)
                .unwrap()
                .unwrap()
                .unwrap_err()
                .code,
            "fixture_terminal"
        );
        assert_eq!(tasks.joined_operations().unwrap(), vec![operation]);
        assert_eq!(
            tasks.wait(operation).await.unwrap_err().code,
            "fixture_terminal"
        );
    }
    fn component_panic() -> Result<Receipt> {
        panic!("bounded component panic")
    }
    #[tokio::test]
    async fn panic_is_joined_uncertainty_not_success() {
        let tasks = EvaluationTasks::default();
        let operation = Uuid::new_v4();
        tasks
            .start(operation, "owned-panic".into(), async { component_panic() })
            .unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(5), tasks.wait(operation))
                .await
                .unwrap()
                .unwrap_err()
                .code,
            "operation_uncertain"
        );
        assert_eq!(tasks.pending().unwrap(), 0);
        // This count means the task joined. It says nothing about a model,
        // execution registry, Control permit or native local-exit capability.
    }
}
