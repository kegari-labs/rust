//! One monotonic work deadline. Expiry fences work but never drops its owned
//! future or turns cancellation into joined/process-exit evidence.
use crate::error::{Error, Result};
use serde_json::json;
use std::{future::Future, time::Duration};
use tokio::time::Instant;

pub(super) struct EvaluationDeadline {
    at: Instant,
}
pub(super) struct SettledEvaluation<T> {
    pub(super) result: Result<T>,
    pub(super) deadline: Option<Error>,
}
impl EvaluationDeadline {
    pub(super) fn new(limit: Duration) -> Result<Self> {
        if limit.is_zero() {
            return Err(Error::invalid("evaluation deadline must be positive"));
        }
        Ok(Self {
            at: Instant::now()
                .checked_add(limit)
                .ok_or_else(|| Error::invalid("evaluation deadline overflows"))?,
        })
    }
    pub(super) fn instant(&self) -> Instant {
        self.at
    }
    pub(super) fn expired(&self) -> bool {
        Instant::now() >= self.at
    }
    pub(super) fn remaining(&self) -> Result<Duration> {
        let remaining = self.at.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(expired())
        } else {
            Ok(remaining)
        }
    }
    pub(super) async fn settle<T>(
        &self,
        work: impl Future<Output = Result<T>>,
        cancel: impl FnOnce() -> Result<()>,
    ) -> SettledEvaluation<T> {
        tokio::pin!(work);
        let cancellation = tokio::select! {
            biased;
            result = &mut work => {
                if !self.expired() { return SettledEvaluation { result, deadline: None }; }
                let error = expired().detail(json!({"settled_error":result.as_ref().err(),"cancellation_error":cancel().err()}));
                return SettledEvaluation { result, deadline: Some(error) };
            }
            _ = tokio::time::sleep_until(self.at) => cancel().err(),
        };
        // Keep polling the exact future until actual stage cleanup returns.
        // Stage-specific process/RPC bounds still apply. If cleanup is blocked,
        // the owned task remains pending and RuntimeDriver cannot claim clear.
        let settled = work.await;
        let error = expired().detail(
            json!({"settled_error":settled.as_ref().err(),"cancellation_error":cancellation}),
        );
        SettledEvaluation {
            result: settled,
            deadline: Some(error),
        }
    }
}
fn expired() -> Error {
    Error::new(
        "evaluation_deadline",
        "declared evaluation wall deadline expired; native cleanup is independent",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Notify;

    #[tokio::test]
    async fn expiry_fences_once_and_keeps_owned_cleanup_pending() {
        let deadline = EvaluationDeadline::new(Duration::from_millis(5)).unwrap();
        let cancelled = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let joined = Arc::new(AtomicUsize::new(0));
        let count = Arc::new(AtomicUsize::new(0));
        let task = {
            let cancelled = cancelled.clone();
            let release = release.clone();
            let joined = joined.clone();
            let count = count.clone();
            tokio::spawn(async move {
                deadline
                    .settle(
                        async move {
                            release.notified().await;
                            joined.fetch_add(1, Ordering::SeqCst);
                            Ok(42_u32)
                        },
                        move || {
                            count.fetch_add(1, Ordering::SeqCst);
                            cancelled.notify_one();
                            Ok(())
                        },
                    )
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(1), cancelled.notified())
            .await
            .unwrap();
        assert!(!task.is_finished());
        assert_eq!(joined.load(Ordering::SeqCst), 0);
        release.notify_one();
        let settled = task.await.unwrap();
        assert_eq!(settled.deadline.unwrap().code, "evaluation_deadline");
        assert_eq!(settled.result.unwrap(), 42);
        assert_eq!(joined.load(Ordering::SeqCst), 1);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stages_share_one_budget_and_late_success_is_not_success() {
        let deadline = EvaluationDeadline::new(Duration::from_millis(5)).unwrap();
        let before = deadline.remaining().unwrap();
        tokio::time::sleep(Duration::from_millis(8)).await;
        assert!(before <= Duration::from_millis(5));
        assert_eq!(
            deadline.remaining().unwrap_err().code,
            "evaluation_deadline"
        );
        let count = AtomicUsize::new(0);
        let result = deadline
            .settle(async { Ok(()) }, || {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .await;
        assert_eq!(result.deadline.unwrap().code, "evaluation_deadline");
        assert!(result.result.is_ok());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
