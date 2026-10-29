//! Independent quota observation for exhausted accounts. A bounded, admitted
//! metadata process never starts a thread/turn or consumes a model attempt.
use super::*;
use crate::codex_host::OwnedProviderCancellation;
use crate::control::Permit;
use crate::execution::CommandCancellation;
use kegari_config::{Connection, ProviderKind};
use kegari_execution::journal::{ProcessState, SpawnBinding};
use kegari_execution::{ExecutionRole, IsolationPlan};
use kegari_platform::BinaryIdentity;
use kegari_scheduler::Quota;
use std::path::Path;

const REFRESH_SECONDS: i64 = 60;
const PHASE: &str = "account_quota_refresh";

fn due(quota: &Quota, next: Option<chrono::DateTime<Utc>>, now: chrono::DateTime<Utc>) -> bool {
    // A reset time is an estimate, not evidence of availability. Recheck on the
    // finite cadence even before that time so manual account resets are noticed.
    next.is_none_or(|next| now >= next) && matches!(quota, Quota::Exhausted { .. })
}

impl RuntimeDriver {
    pub(super) fn launch_account_refresh(&self, lease: &Lease) -> Result<()> {
        let config = self.runtime.configuration.snapshot()?;
        let now = Utc::now();
        let selected = {
            let state = self.state()?;
            // One bounded metadata task at a time; unrelated worker slots remain
            // available and no scheduler allocation/model budget is consumed.
            if state.tasks.values().any(|task| task.record.phase == PHASE)
                || state.tasks.values().filter(|task| !task.worker).count()
                    >= self.limits.authorization_tasks
            {
                return Ok(());
            }
            self.runtime
                .exhausted_accounts()?
                .into_iter()
                .find_map(|(account, quota)| {
                    if !due(
                        &quota,
                        state.account_refresh_after.get(&account).copied(),
                        now,
                    ) {
                        return None;
                    }
                    config
                        .connections
                        .values()
                        .find(|connection| {
                            connection.kind == ProviderKind::Codex
                                && connection.account_id == account
                        })
                        .cloned()
                        .map(|connection| (account, connection))
                })
        };
        let Some((account, connection)) = selected else {
            return Ok(());
        };
        self.fresh(lease)?;
        let operation = Uuid::new_v4();
        let revision = crate::config::config_revision(&config)?;
        let work = format!("account-quota:{}", canonical_digest(1, &account)?);
        let scope = canonical_digest(
            1,
            &json!({"purpose":PHASE,"configuration":revision,"connection":connection}),
        )?;
        let mut record = DispatchRecord {
            schema_version: 1,
            session_recovery: None,
            correction: None,
            metadata: None,
            runtime: lease.runtime_id,
            action: canonical_digest(1, &json!({"purpose":PHASE,"operation":operation}))?,
            operation,
            work: work.clone(),
            phase: PHASE.into(),
            lease: lease.clone(),
            state: DispatchState::Dispatched,
            observed_at: now,
            error: None,
        };
        // Persist selection before admission. An interrupted/uncertain operation
        // retains its ordinary receipt/journal hold instead of being replayed.
        self.record(&record)?;
        self.state()?.account_refresh_after.insert(
            account.clone(),
            now + chrono::Duration::seconds(REFRESH_SECONDS),
        );
        let admitted = self
            .runtime
            .control
            .admit(lease, operation, work, scope, || {
                self.runtime.registration.verify_active_owner(lease)?;
                if crate::config::config_revision(&self.runtime.configuration.snapshot()?)?
                    != revision
                {
                    return Err(Error::new(
                        "stale_config",
                        "account refresh configuration changed",
                    ));
                }
                Ok(())
            });
        let permit = match admitted {
            Ok(permit) => permit,
            Err(error) => {
                record.state = DispatchState::Blocked;
                record.error = Some(error.clone());
                self.record_observation(&record)?;
                self.runtime.record_quota_refresh(&account, &Err(error))?;
                return Ok(());
            }
        };
        let runtime = self.runtime.clone();
        let processes = self.processes.clone();
        let task_record = record.clone();
        let join = tokio::spawn(async move {
            let result = refresh(&runtime, &processes, &permit, &connection, &revision).await;
            runtime.services.wake.notify_one();
            result
        });
        self.state()?.tasks.insert(
            operation,
            PendingTask {
                record: task_record,
                worker: false,
                join,
            },
        );
        Ok(())
    }
}

struct RefreshCancellation(OwnedProviderCancellation);
impl CommandCancellation for RefreshCancellation {
    fn cancel(&self) -> Result<()> {
        self.0.request_cancel()
    }
    fn exited(&self) -> Result<bool> {
        self.0.exited()
    }
}

fn fresh(runtime: &RuntimeCoordinator, permit: &Permit, revision: &str) -> Result<()> {
    runtime.registration.verify_active_owner(&permit.lease)?;
    if crate::config::config_revision(&runtime.configuration.snapshot()?)? != revision {
        return Err(Error::new(
            "stale_config",
            "account refresh configuration changed",
        ));
    }
    Ok(())
}

fn binding(
    permit: &Permit,
    executable: &BinaryIdentity,
    cwd: &Path,
    kind: &str,
    argv: &[String],
) -> Result<SpawnBinding> {
    Ok(SpawnBinding {
        operation_id: permit.operation_id,
        effect_id: Uuid::new_v4(),
        lease: Some(permit.lease.clone()),
        control_owner: None,
        check_bundle: None,
        work_key: permit.work_key.clone(),
        scope_digest: permit.scope_digest.clone(),
        cwd: cwd.into(),
        executable: executable.clone(),
        observed_executables: vec![],
        argv_digest: canonical_digest(1, &argv)?,
        kind: kind.into(),
    })
}

async fn refresh(
    runtime: &RuntimeCoordinator,
    processes: &Arc<ProcessJournal>,
    permit: &Permit,
    connection: &Connection,
    revision: &str,
) -> Result<()> {
    let mut owned = Vec::<OwnedProviderCancellation>::new();
    let previous = runtime.quota_observation_digest(&connection.account_id)?;
    let observed = tokio::time::timeout(Duration::from_secs(30), async {
        fresh(runtime, permit, revision)?;
        if connection.argv.len() != 4
            || connection.argv[1..] != ["app-server", "--listen", "stdio://"]
        {
            return Err(Error::new(
                "unsupported_profile",
                "account refresh requires the supported Codex stdio profile",
            ));
        }
        let executable = BinaryIdentity::inspect(
            &crate::configuration::resolve_executable(&connection.argv[0])?,
            false,
        )?;
        let auth_home = std::path::PathBuf::from(
            connection
                .auth_home
                .as_deref()
                .ok_or_else(|| Error::invalid("Codex auth home is missing"))?,
        );
        let root = runtime
            .registration
            .root()
            .child("account-observation")?
            .child(&canonical_digest(1, &connection.account_id)?.replace(':', "-"))?;
        let cwd = root.child("empty-context")?;
        let scratch = root.child("scratch")?;
        let isolation = IsolationPlan {
            schema_version: 1,
            role: ExecutionRole::Coordination,
            execution_root: root.path().into(),
            repository: cwd.path().into(),
            cwd: cwd.path().into(),
            scratch: scratch.path().into(),
            read_roots: vec![],
            resource_writes: vec![],
            source_writes: vec![],
            forbidden_roots: vec![auth_home.clone()],
        }
        .validate(crate::BUILD_ID)?;
        let permissions = crate::codex_host::permission_configuration(&isolation, &auth_home)?;
        let mut probe = None;
        runtime.control.validate_permit(permit, || {
            fresh(runtime, permit, revision)?;
            let binding = binding(
                permit,
                &executable,
                cwd.path(),
                "codex-version",
                &["--version".into()],
            )?;
            let effect = binding.effect_id;
            let guard = processes.begin(binding)?;
            let started = crate::codex_host::start_version_probe(&executable, Some(guard))?;
            let cancellation = started.cancellation();
            owned.push(cancellation.clone());
            runtime.executions.register(
                permit.operation_id,
                effect,
                Arc::new(RefreshCancellation(cancellation)),
            )?;
            probe = Some(started);
            Ok(())
        })?;
        let verified = probe
            .ok_or_else(|| Error::new("ownership_conflict", "account version probe did not start"))?
            .verify(Duration::from_secs(5))
            .await?;
        let mut client = None;
        runtime.control.validate_permit(permit, || {
            fresh(runtime, permit, revision)?;
            let binding = binding(
                permit,
                &executable,
                cwd.path(),
                "codex-app-server",
                &kegari_codex::app_server_argv(&permissions)?,
            )?;
            let effect = binding.effect_id;
            let guard = processes.begin(binding)?;
            let started = crate::codex_host::start_verified_guarded(
                &verified,
                &auth_home,
                &permissions,
                Duration::from_secs(5),
                guard,
            )?;
            let cancellation = started.cancellation();
            owned.push(cancellation.clone());
            runtime.executions.register(
                permit.operation_id,
                effect,
                Arc::new(RefreshCancellation(cancellation)),
            )?;
            client = Some(started);
            Ok(())
        })?;
        let mut client = client.ok_or_else(|| {
            Error::new(
                "ownership_conflict",
                "account metadata process did not start",
            )
        })?;
        // No thread, turn, tool, login or account switching is requested.
        client.client_mut().initialize().await?;
        client
            .client_mut()
            .verify_account(&connection.account_id)
            .await?;
        let result = client.client_mut().rate_limits().await?;
        client.terminate().await?;
        Ok(result)
    })
    .await
    .map_err(|_| Error::new("timeout", "account quota refresh exceeded thirty seconds"))
    .and_then(|result| result);

    // Timeout/cancellation drops the in-flight read before joining all actual
    // original handles. Never use EOF or absent handles as proof of exit.
    let mut cleanup_error = None;
    for process in &owned {
        if let Err(error) = process.join_termination().await {
            cleanup_error.get_or_insert(error);
        }
    }
    runtime.executions.observe_exited(permit.operation_id)?;
    let records = processes.retained_records(permit.operation_id)?;
    let closed = owned
        .iter()
        .all(|process| matches!(process.exited(), Ok(true)))
        && records.len() == owned.len()
        && records.iter().all(|record| {
            matches!(
                record.state,
                ProcessState::GroupExited {
                    guard_release_verified: true,
                    ..
                }
            )
        });
    if let Some(error) = cleanup_error {
        runtime.control.fence(permit.operation_id)?;
        runtime.record_quota_refresh(&connection.account_id, &Err(error.clone()))?;
        return Err(error);
    }
    if !closed {
        runtime.control.fence(permit.operation_id)?;
        let error = Error::new(
            "operation_uncertain",
            "account metadata process closure is unresolved",
        );
        runtime.record_quota_refresh(&connection.account_id, &Err(error.clone()))?;
        return Err(error);
    }
    let observed = observed.and_then(|value| {
        runtime
            .control
            .validate_permit(permit, || fresh(runtime, permit, revision))?;
        runtime.apply_quota_refresh(&connection.account_id, &value, &previous)?;
        Ok(value)
    });
    // Record only the diagnostic here. A second successful write would bypass
    // the freshness comparison and could overwrite a newer worker notification.
    if let Err(error) = &observed {
        runtime.record_quota_refresh(&connection.account_id, &Err(error.clone()))?;
    }
    runtime.control.finish(
        permit.operation_id,
        json!({
            "purpose":PHASE,"account_id":connection.account_id,"local_execution":"exited",
            "owned_handles_joined":true,"error":observed.as_ref().err(),
        }),
    )?;
    observed.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exhausted_accounts_refresh_without_assuming_reset_or_consuming_model_admission() {
        let now = chrono::DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let exhausted = Quota::Exhausted {
            observed_at: now,
            reset_at: Some(now + chrono::Duration::seconds(30)),
        };
        assert!(due(&exhausted, None, now));
        assert!(due(&exhausted, None, now + chrono::Duration::seconds(30)));
        assert!(!due(
            &exhausted,
            Some(now + chrono::Duration::seconds(60)),
            now + chrono::Duration::seconds(30)
        ));
        assert!(due(
            &Quota::Exhausted {
                observed_at: now,
                reset_at: None
            },
            None,
            now
        ));
        assert!(!due(&Quota::Unknown, None, now));
        assert!(!due(&Quota::Available { observed_at: now }, None, now));
    }
}
