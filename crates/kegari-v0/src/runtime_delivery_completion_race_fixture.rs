//! Actual first inventory against an actual delayed completion commit. The new
//! runtime shares the registered ControlHost/disk, not historical cache values.
//! No provider request, native worker, synthetic proof, or raw state reset occurs.
use super::*;
use std::sync::mpsc::{self, SyncSender};
use std::time::Duration as StdDuration;

pub(super) struct ColdCompletionReaders {
    reader: Arc<RuntimeDelivery>,
    writer: Arc<RuntimeDelivery>,
}

/// Prepare before the original facade writes its first genuine seal. Both new
/// facades restore the actual retired attempt while the seal namespace is empty.
pub(super) fn prepare(
    original: &Arc<RuntimeDelivery>,
    operation: Uuid,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ColdCompletionReaders> + '_>> {
    Box::pin(async move {
        let previous = &original.runtime;
        let seal_leaf = format!("release-completion-{operation}.json");
        assert_eq!(
            original
                .records
                .read_bytes(&seal_leaf, 1024 * 1024)
                .unwrap_err()
                .code,
            "record_missing"
        );
        let configuration = Arc::new(crate::runtime_fixture::FixtureConfiguration {
            config: Mutex::new(previous.configuration.snapshot().unwrap()),
            fences: Mutex::new(previous.configuration.authority_fences().unwrap()),
        });
        let observations = Arc::new(
            crate::observations::ObservationCache::with_inputs(
                configuration.clone(),
                previous.services.secrets.clone(),
                previous.services.key.clone(),
                previous.registration.root().child("references").unwrap(),
                previous.services.inputs.clone(),
                previous.control.clone(),
            )
            .await
            .unwrap(),
        );
        let probe = Arc::new(crate::test_process_probe::FixtureBootProbe::new());
        let git = original.git_path().unwrap();
        let housekeeping = Arc::new(
            crate::housekeeping::Housekeeping::new(
                crate::storage::PrivateDir::open(previous.registration.root().path()).unwrap(),
                previous.control.clone(),
                probe.clone(),
                git.clone(),
            )
            .unwrap(),
        );
        let runtime = crate::runtime::RuntimeCoordinator::new(crate::host::RuntimeDependencies {
            configuration,
            control: previous.control.clone(),
            registration: previous.registration.clone(),
            observations,
            secrets: previous.services.secrets.clone(),
            key: previous.services.key.clone(),
            housekeeping,
            logs: Arc::new(crate::housekeeping::LogSink::new(
                previous.registration.root().child("logs").unwrap(),
            )),
            inputs: previous.services.inputs.clone(),
            wake: Arc::new(tokio::sync::Notify::new()),
        })
        .unwrap();
        assert!(!Arc::ptr_eq(&runtime, previous));
        assert!(!Arc::ptr_eq(&runtime.observations, &previous.observations));
        assert!(!Arc::ptr_eq(&runtime.executions, &previous.executions));
        let processes = Arc::new(
            crate::process_records::ProcessJournal::with_probe(
                previous
                    .registration
                    .root()
                    .child("completion-race-processes")
                    .unwrap(),
                runtime.configuration.snapshot().unwrap().runtime.id,
                probe,
            )
            .unwrap(),
        );
        let reader =
            RuntimeDelivery::with_fixture(runtime.clone(), processes.clone(), git.clone()).unwrap();
        let writer = RuntimeDelivery::with_fixture(runtime, processes, git).unwrap();
        for facade in [&reader, &writer] {
            assert!(
                facade
                    .runtime
                    .known_release_completion_anchors()
                    .unwrap()
                    .is_empty()
            );
            facade.restore_releases().unwrap();
            let state = facade.releases.state.lock().unwrap();
            assert!(state.restored);
            assert!(state.attempts[&operation].local_retired);
            assert!(state.attempts[&operation].attempt.completed());
            assert!(state.completions.is_empty());
            assert!(state.completion_anchors.is_empty());
        }
        assert!(
            reader
                .runtime
                .known_release_completion_anchors()
                .unwrap()
                .is_empty()
        );
        ColdCompletionReaders { reader, writer }
    })
}

struct ReleaseReader(Option<SyncSender<()>>);
impl ReleaseReader {
    fn release(&mut self) {
        if let Some(signal) = self.0.take() {
            let _ = signal.try_send(());
        }
    }
}
impl Drop for ReleaseReader {
    fn drop(&mut self) {
        self.release();
    }
}
struct RestoreSeal {
    records: Arc<crate::storage::PrivateDir>,
    leaf: String,
    bytes: Vec<u8>,
    restored: bool,
}
impl RestoreSeal {
    fn restore(&mut self) {
        self.records.write_bytes(&self.leaf, &self.bytes).unwrap();
        self.restored = true;
    }
}
impl Drop for RestoreSeal {
    fn drop(&mut self) {
        if !self.restored {
            // A failure still joins the two bounded threads before this guard
            // restores the fixture's exact original protected bytes.
            let _ = self.records.write_bytes(&self.leaf, &self.bytes);
        }
    }
}

pub(super) fn assert_serialized_first_reader(
    cold: &ColdCompletionReaders,
    original: &crate::runtime_delivery::VerifiedReleaseCompletion,
) {
    let operation = original.terminal().plan.operation_id;
    let seal_leaf = format!("release-completion-{operation}.json");
    assert!(
        cold.reader
            .runtime
            .known_release_completion_anchors()
            .unwrap()
            .is_empty()
    );
    assert!(
        cold.writer
            .runtime
            .known_release_completion_anchors()
            .unwrap()
            .is_empty()
    );
    assert!(Arc::ptr_eq(&cold.reader.runtime, &cold.writer.runtime));
    let original_bytes = cold
        .reader
        .records
        .read_bytes(&seal_leaf, 1024 * 1024)
        .unwrap();
    let commit = cold
        .writer
        .prepare_fixture_completion_commit(operation)
        .unwrap();
    assert!(
        cold.reader
            .runtime
            .known_release_completion_anchors()
            .unwrap()
            .is_empty()
    );
    let receipts = canonical_digest(1, &cold.reader.runtime.control.operations().unwrap()).unwrap();
    let mut restore = RestoreSeal {
        records: cold.reader.records.clone(),
        leaf: seal_leaf.clone(),
        bytes: original_bytes,
        restored: false,
    };
    let (observed_tx, observed_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let (commit_gate_tx, commit_gate_rx) = mpsc::sync_channel(1);
    let (completed_tx, completed_rx) = mpsc::sync_channel(1);
    cold.reader
        .fixture_pause_first_completion_anchor(operation, observed_tx, release_rx)
        .unwrap();
    cold.writer
        .fixture_signal_completion_commit(operation, commit_gate_tx)
        .unwrap();

    let (read_result, commit_result, missing) = std::thread::scope(|threads| {
        // This guard releases on every assertion/unwind path before scope joins.
        // The reader's own recv_timeout is a second finite cleanup bound.
        let mut release = ReleaseReader(Some(release_tx));
        let reader = threads.spawn(|| cold.reader.sync_released_completions());
        observed_rx
            .recv_timeout(StdDuration::from_secs(10))
            .expect("first actual authenticated anchor read was not reached");
        assert!(
            cold.reader
                .runtime
                .known_release_completion_anchors()
                .unwrap()
                .is_empty()
        );
        std::fs::remove_file(cold.reader.records.path().join(&seal_leaf)).unwrap();
        let missing = original.validate().unwrap_err();
        assert_eq!(missing.code, "record_missing");
        let writer = threads.spawn(move || {
            let result = commit();
            let _ = completed_tx.send(result.clone());
            result
        });
        commit_gate_rx
            .recv_timeout(StdDuration::from_secs(10))
            .expect("actual delayed commit did not reach its serialized boundary");
        assert!(
            matches!(
                completed_rx.recv_timeout(StdDuration::from_millis(200)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "commit must not pass the Control boundary while the first-reader callback is paused"
        );
        assert_eq!(
            cold.reader
                .records
                .read_bytes(&seal_leaf, 1024 * 1024)
                .unwrap_err()
                .code,
            "record_missing"
        );
        release.release();
        let read_result = reader.join().expect("actual inventory thread panicked");
        let commit_result = writer.join().expect("actual commit thread panicked");
        (read_result, commit_result, missing)
    });
    read_result.unwrap();
    let denied = commit_result.unwrap_err();
    assert_eq!(denied.code, missing.code);
    assert_eq!(denied.message, missing.message);
    assert_eq!(
        cold.reader
            .records
            .read_bytes(&seal_leaf, 1024 * 1024)
            .unwrap_err()
            .code,
        "record_missing"
    );
    let anchors = cold
        .reader
        .runtime
        .known_release_completion_anchors()
        .unwrap();
    assert_eq!(anchors.len(), 1);
    assert_eq!(anchors[0].operation(), operation);
    assert_eq!(anchors[0].digest(), original.digest());
    for facade in [&cold.reader, &cold.writer] {
        assert!(
            !facade
                .releases
                .state
                .lock()
                .unwrap()
                .completions
                .contains_key(&operation)
        );
    }
    assert_eq!(
        canonical_digest(1, &cold.reader.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
    restore.restore();
    // Exact restoration alone grants no outcome; successful real inventory must
    // reauthenticate the complete original record and all its references.
    cold.reader.sync_released_completions().unwrap();
    let inventory = cold
        .reader
        .release_completion_queue_inventory(&BTreeMap::new())
        .unwrap();
    assert_eq!(inventory.proofs[&operation].digest(), original.digest());
    assert_eq!(
        canonical_digest(1, &cold.reader.runtime.control.operations().unwrap()).unwrap(),
        receipts
    );
}
