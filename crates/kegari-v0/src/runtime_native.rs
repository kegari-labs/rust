//! Concrete native composition. The control host and CLI share this installed
//! backend; constructing it never grants a worker lease or starts model work.
use crate::{
    app::HostFuture,
    corrections::{CorrectionDependencies, CorrectionPublisher},
    host::{HostRuntimeFactory, RuntimeBackend, RuntimeDependencies},
    runtime::RuntimeCoordinator,
    runtime_delivery::RuntimeDelivery,
    runtime_driver::{DriverLimits, RuntimeDriver, RuntimeDriverServices},
    runtime_workflow::RuntimeWorkflow,
    worker::WorkerHost,
};
use std::sync::Arc;

pub struct NativeRuntimeFactory;
impl HostRuntimeFactory for NativeRuntimeFactory {
    fn build(&self, dependencies: RuntimeDependencies) -> HostFuture<'_, RuntimeBackend> {
        Box::pin(async move {
            let runtime = RuntimeCoordinator::new(dependencies)?;
            let delivery = RuntimeDelivery::new(runtime.clone())?;
            let owner = runtime.clone();
            let registration = runtime.registration.clone();
            let corrections = Arc::new(CorrectionPublisher::new(CorrectionDependencies {
                configuration: runtime.configuration.clone(),
                control: runtime.control.clone(),
                observations: runtime.observations.clone(),
                secrets: runtime.services.secrets.clone(),
                key: runtime.services.key.clone(),
                inputs: runtime.services.inputs.clone(),
                owner_check: Arc::new(move |lease| owner.verify_owner(lease)),
                control_owner_check: Arc::new(move || registration.verify_control_owner()),
            }));
            let driver = RuntimeDriver::new(
                RuntimeDriverServices {
                    runtime: runtime.clone(),
                    delivery: delivery.clone(),
                    processes: delivery.processes(),
                    corrections,
                },
                DriverLimits {
                    // Final host watchdog; the captured session turn count and
                    // configured stream/stall bounds continue to apply within it.
                    worker_seconds: 86_400,
                    authorization_tasks: 16,
                },
            )?;
            let workflow = Arc::new(RuntimeWorkflow::new(runtime, delivery.clone(), delivery));
            Ok(RuntimeBackend { workflow, driver })
        })
    }
}
