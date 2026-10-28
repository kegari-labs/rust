//! Test-only runtime composition with actual protected registration and journals.
//! Callers supply clearly labeled provider/process fixtures; no synthetic
//! completion, acceptance or delivery capability is installed by this builder.
use crate::{
    app::{ConfigurationControl, HostFuture},
    configuration::{AuthorityFences, ConfigurationInputs},
    control::{Actor, Receipt},
    evidence::AttestationKey,
    host::RuntimeDependencies,
    housekeeping::{Housekeeping, LogSink},
    observations::ObservationCache,
    platform::Registration,
    process_records::ProcessJournal,
    runtime::RuntimeCoordinator,
    secrets::SecretStore,
};
use kegari_platform::{BinaryIdentity, HostIdentity, ProcessProbe};
use kegari_storage::PrivateDir;
use kegari_types::error::{Error, Result};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

pub(crate) struct FixtureConfiguration {
    pub config: Mutex<kegari_config::Config>,
    pub fences: Mutex<AuthorityFences>,
}
impl ConfigurationControl for FixtureConfiguration {
    fn snapshot(&self) -> Result<kegari_config::Config> {
        Ok(self.config.lock().unwrap().clone())
    }
    fn authority_fences(&self) -> Result<AuthorityFences> {
        Ok(self.fences.lock().unwrap().clone())
    }
    fn execute(
        &self,
        _: Actor,
        _: kegari_types::api::CommandRequest,
        _: Option<crate::secrets::Secret>,
        _: Receipt,
    ) -> HostFuture<'_, Receipt> {
        Box::pin(async {
            Err(Error::new(
                "fixture_unexpected_mutation",
                "runtime fixture does not authorize configuration writes",
            ))
        })
    }
    fn check(&self, _: Option<Uuid>) -> HostFuture<'_, Value> {
        Box::pin(async {
            Err(Error::new(
                "fixture_unexpected_probe",
                "runtime fixture does not launch configuration probes",
            ))
        })
    }
}
pub(crate) struct FixtureInputs {
    pub root: PathBuf,
    pub configuration: kegari_config::Config,
    pub inputs: Arc<dyn ConfigurationInputs>,
    pub secrets: Arc<dyn SecretStore>,
    pub key: Arc<AttestationKey>,
    pub probe: Arc<dyn ProcessProbe>,
    pub git: PathBuf,
}
pub(crate) struct RuntimeFixture {
    pub runtime: Arc<RuntimeCoordinator>,
    pub configuration: Arc<FixtureConfiguration>,
    pub processes: Arc<ProcessJournal>,
}
pub(crate) async fn build(input: FixtureInputs) -> Result<RuntimeFixture> {
    let process = input.probe.current()?;
    let host = HostIdentity {
        host_id: input.configuration.runtime.host_id.clone(),
        boot_id: process.boot_id,
        uid: process.uid,
        user: "disposable-fixture".into(),
        home: input
            .root
            .canonicalize()
            .map_err(|error| Error::io("resolve runtime fixture root", error))?,
    };
    let binary = BinaryIdentity::inspect(
        &std::env::current_exe().map_err(|error| Error::io("read fixture executable", error))?,
        false,
    )?;
    let (registration, control) = Registration::bootstrap_with_probe(
        &host,
        input.configuration.runtime.id,
        binary,
        Uuid::new_v4(),
        input.probe.clone(),
    )?;
    registration.complete_setup(&input.configuration)?;
    let registration = Arc::new(registration);
    let control = Arc::new(control);
    let configuration = Arc::new(FixtureConfiguration {
        config: Mutex::new(input.configuration),
        fences: Mutex::new(AuthorityFences::default()),
    });
    let observations = Arc::new(
        ObservationCache::with_inputs(
            configuration.clone(),
            input.secrets.clone(),
            input.key.clone(),
            registration.root().child("references")?,
            input.inputs.clone(),
            control.clone(),
        )
        .await?,
    );
    let processes = Arc::new(ProcessJournal::with_probe(
        registration.root().child("worker-processes")?,
        configuration.snapshot()?.runtime.id,
        input.probe.clone(),
    )?);
    let housekeeping = Arc::new(Housekeeping::new(
        PrivateDir::open(registration.root().path())?,
        control.clone(),
        input.probe,
        input.git,
    )?);
    let logs = Arc::new(LogSink::new(registration.root().child("logs")?));
    let runtime = RuntimeCoordinator::new(RuntimeDependencies {
        configuration: configuration.clone(),
        control,
        registration,
        observations,
        secrets: input.secrets,
        key: input.key,
        housekeeping,
        logs,
        inputs: input.inputs,
        wake: Arc::new(tokio::sync::Notify::new()),
    })?;
    Ok(RuntimeFixture {
        runtime,
        configuration,
        processes,
    })
}
