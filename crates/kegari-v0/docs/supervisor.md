# Native orchestration supervision

`kegari_controller::supervisor::Supervisor::native` owns the actual registered `RunningOrchestration`
process and its inherited bounded channel. It requires concrete driver methods
for pre-start reconciliation, recovery, scheduling, worker maintenance, fenced
cancellation and exit reconciliation. No empty production driver, optimistic
recovery result or always-allow admission callback is supplied. The application
factory must connect these methods to the provider projection, scheduler, worker
registry and delivery engine before activating the service.

Saved STOPPED or PAUSED never spawns a new orchestration process. RUNNING performs
read-only pre-start reconciliation, rechecks current intent, reserves/starts the
registered binary, then reconstructs actual ownership/provider state before
opening admission. Every child poll is only a scheduling opportunity; the
concrete driver must obtain a fresh `ControlHost` permit for each operation.
Pause and stop continue maintenance of earlier admitted work. Normal stop exits
the orchestration process only after observed local drain, then reconciles its
registration against actual process/lock and remote outcomes.

The control host persists intent and force fences before supervisor cancellation.
The supervisor checks that persistence succeeded, remembers each cancellation
attempt per generation/operation and invokes only the driver's owned cancellation
capabilities. Unknown remote cancellation is not automatically repeated. A force
wake during a waiting scheduler/maintenance step invokes cancellation concurrently;
the driver must not hold a shared mutex across provider waits. Local cancellation
and remote reconciliation remain separate observations.

Heartbeats and current intent continue during bounded driver steps. Each step has
one deadline; cancellation has a maximum five-second bound. A timeout suspends
and fences the owner using a durable receipt without changing operator intent or
claiming its process exited. After suspension, the supervisor requests exit from the owned scheduling child,
performs bounded cancellation of fenced workers and continues observing them
while blocked. A later explicit force-stop still reaches cancellation. The
scheduling child's handle is joined on failed handshake/drop; no retained PID is
adopted. Metadata, active operation IDs and unknown effects remain until actual
reconciliation proves they may be cleared. The loop does not repeatedly restart
an owner under the same lifecycle command. A new explicit lifecycle command may
try recovery, which must reconcile the retained hold before any new admission.

`observe()` provides a watch channel with actual starting/recovering/running,
paused/draining/stopped/blocked/service-stopping state, lease, last poll sequence,
safe error and observation time. API status can consume it without waiting on a
provider or the supervisor's driver future. Service shutdown remains the separate
serialized STOPPED/no-owned-work reservation, followed by listener shutdown.

Deterministic tests inject only the process/channel boundary while retaining the
real `ControlHost` intent, permits, fences and receipts. They cover recovery
failure, pause/resume, drain, service stop, force during a blocked step, heartbeat
responsiveness, bounded timeout, unexpected exit and refusal to retry implicitly.
These fixtures do not prove native launchd, OS process identity or real-account
qualification. Native ownership tests remain explicitly blocked by the observed
outer sandbox boot-identity denial; no unconfined retry was attempted.

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --offline --locked -p kegari-controller --test supervisor --test orchestration
```

A control-host interrupt uses a separate shutdown signal: it preserves saved
operator intent, durably suspends/fences an active owner, cancels owned work and
observes actual exits. It does not manufacture a service-stopped reservation.
When startup failed before any owner was created, an observed restoration of
runtime prerequisites may retry that pre-start reconciliation without inventing
a new lifecycle command. The same signal does not reopen an owner that already
failed or clear any unresolved effect.
