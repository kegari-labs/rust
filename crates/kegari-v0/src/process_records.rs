//! Application admission for durable process journals. The execution crate owns
//! persistence and actual process guards; worker and sealed host policy stay here.
use crate::host_process::LocalProcessPermit;
use kegari_execution::journal::{
    ProcessJournalStore, ProcessRecord, ProcessRecovery, SpawnBinding, SpawnGuard,
};
use kegari_platform::ProcessProbe;
use kegari_storage::PrivateDir;
use kegari_types::error::{Error, Result};
use std::sync::Arc;
use uuid::Uuid;

pub struct ProcessJournal {
    store: Arc<ProcessJournalStore>,
}
impl ProcessJournal {
    pub fn new(directory: PrivateDir, runtime: Uuid) -> Result<Self> {
        Ok(Self {
            store: Arc::new(ProcessJournalStore::new(directory, runtime)?),
        })
    }
    pub fn with_probe(
        directory: PrivateDir,
        runtime: Uuid,
        probe: Arc<dyn ProcessProbe>,
    ) -> Result<Self> {
        Ok(Self {
            store: Arc::new(ProcessJournalStore::with_probe(directory, runtime, probe)?),
        })
    }
    /// Persist before native spawn while holding the operation's current gate.
    /// Any record already present means recovery, never permission to replay.
    pub fn begin(self: &Arc<Self>, binding: SpawnBinding) -> Result<SpawnGuard> {
        if binding.control_owner.is_some() || binding.lease.is_none() {
            return Err(Error::new(
                "access_denied",
                "host-local process requires a sealed host permit",
            ));
        }
        self.store.begin(binding)
    }
    pub(crate) fn begin_control(
        self: &Arc<Self>,
        binding: SpawnBinding,
        permit: &LocalProcessPermit,
    ) -> Result<SpawnGuard> {
        permit.validate_journal_binding(&binding)?;
        self.store.begin(binding)
    }
    pub fn observe(&self, operation_id: Uuid) -> Result<Vec<ProcessRecovery>> {
        self.store.observe(operation_id)
    }
    /// Historical data only. No PID probe, adoption, signal or exit inference.
    pub(crate) fn retained_records(&self, operation_id: Uuid) -> Result<Vec<ProcessRecord>> {
        self.store.retained_records(operation_id)
    }
    pub fn operations(&self) -> Result<Vec<Uuid>> {
        self.store.operations()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kegari_execution::journal::{CheckBundleMember, CheckMemberKind, HostProcessIdentity};
    use kegari_platform::{BinaryIdentity, ProcessIdentity, ProcessObservation};
    use kegari_types::control::Lease;
    use std::{
        path::Path,
        sync::atomic::{AtomicUsize, Ordering},
    };

    struct NoProbe(Arc<AtomicUsize>);
    impl ProcessProbe for NoProbe {
        fn current(&self) -> Result<ProcessIdentity> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(Error::invalid("facade fixture forbids process probes"))
        }
        fn observe(&self, _: &ProcessIdentity) -> Result<ProcessObservation> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(Error::invalid("facade fixture forbids process probes"))
        }
        fn matching_executable(&self, _: &Path) -> Result<Vec<ProcessIdentity>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(Error::invalid("facade fixture forbids process probes"))
        }
    }

    fn fixture() -> (
        tempfile::TempDir,
        Arc<ProcessJournal>,
        Arc<AtomicUsize>,
        SpawnBinding,
    ) {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let runtime = Uuid::from_u128(1);
        let calls = Arc::new(AtomicUsize::new(0));
        let journal = Arc::new(
            ProcessJournal::with_probe(
                PrivateDir::create(&root.join("processes")).unwrap(),
                runtime,
                Arc::new(NoProbe(calls.clone())),
            )
            .unwrap(),
        );
        let binding = SpawnBinding {
            operation_id: Uuid::from_u128(2),
            effect_id: Uuid::from_u128(3),
            lease: Some(Lease {
                runtime_id: runtime,
                owner_generation: 1,
                owner_id: Uuid::from_u128(4),
            }),
            control_owner: None,
            check_bundle: None,
            work_key: "fixture work".into(),
            scope_digest: kegari_digest::bytes_digest(b"fixture scope"),
            cwd: root.clone(),
            // This absent executable must not be inspected before admission refusal.
            executable: BinaryIdentity {
                path: root.join("not-an-executable"),
                digest: kegari_digest::bytes_digest(b"fixture executable"),
                bytes: 0,
            },
            observed_executables: vec![],
            argv_digest: kegari_digest::bytes_digest(b"fixture argv"),
            kind: "fixture_only".into(),
        };
        (temporary, journal, calls, binding)
    }

    fn assert_refused(journal: &Arc<ProcessJournal>, calls: &AtomicUsize, binding: SpawnBinding) {
        let error = journal.begin(binding).err().unwrap();
        assert_eq!(
            error,
            Error::new(
                "access_denied",
                "host-local process requires a sealed host permit",
            )
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(journal.operations().unwrap().is_empty());
    }

    #[test]
    fn worker_facade_refuses_missing_lease_before_probe_or_journal_mutation() {
        let (_temporary, journal, calls, mut binding) = fixture();
        binding.lease = None;
        assert_refused(&journal, &calls, binding);
    }

    #[test]
    fn worker_facade_refuses_host_bindings_before_probe_or_journal_mutation() {
        let (_temporary, journal, calls, mut binding) = fixture();
        binding.control_owner = Some(HostProcessIdentity {
            runtime_id: Uuid::from_u128(1),
            owner_generation: 1,
            process: ProcessIdentity {
                pid: 100,
                parent_pid: 1,
                process_group: 100,
                uid: 501,
                boot_id: Uuid::from_u128(5),
                started_seconds: 1,
                started_microseconds: 0,
                executable: "/synthetic/host".into(),
            },
            operation_id: binding.operation_id,
            work_key: binding.work_key.clone(),
            scope_digest: binding.scope_digest.clone(),
            config_revision: "synthetic revision".into(),
            cleanup_receipt: None,
        });
        assert_refused(&journal, &calls, binding.clone());
        binding.lease = None;
        assert_refused(&journal, &calls, binding);
    }

    #[test]
    fn journal_digest_syntax_matches_backend_evidence_validation() {
        for digest in [
            format!("sha256:{}", "a".repeat(64)),
            format!("sha256:v1:{}", "0".repeat(64)),
            format!("sha256:{}", "A".repeat(64)),
            format!("sha256:{}", "g".repeat(64)),
            format!("sha256:{}", "a".repeat(63)),
            format!("sha256:{}", "a".repeat(65)),
            format!("sha256:v2:{}", "a".repeat(64)),
            "a".repeat(64),
            format!(" sha256:{}", "a".repeat(64)),
            format!("sha256:{} ", "a".repeat(64)),
            format!("sha256:{}", "é".repeat(32)),
        ] {
            let member = CheckBundleMember {
                bundle_effect: Uuid::from_u128(1),
                plan_digest: digest.clone(),
                ordinal: 0,
                kind: CheckMemberKind::Check,
                service: None,
            };
            assert_eq!(
                member.validate(),
                crate::evidence::validate_digest(&digest),
                "digest format: {digest}"
            );
        }
    }
}
