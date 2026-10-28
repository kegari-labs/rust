# Owned worker sessions

`worker_run::prepare` assembles the actual configured workflow, shipped role
instructions, opaque current authority, exact repository/read-resource grants,
Codex account/profile, full dynamic-tool contracts and named isolation policy.
It compares the current configuration revision and rereads workflow bytes. It
starts no provider session. `PreparedRun::contract()` supplies the immutable
scope needed for `ControlHost::admit_run`.

`worker_run::run` requires that durable admission, including the original account,
workspace, profile/instruction digests and finite turn budget. It accepts the
concrete `WorkerServices`; fixture command boundaries cannot enter a live run.
The host retains `OwnedWorkerRun`, calls `cancel_fenced` only after durable
fencing, and awaits `join`. Run observations are diagnostic worker claims; the
delivery coordinator still verifies every separate completion gate.

Native app-server process creation and installation of its owned cancellation
handle occur inside the serialized fresh permit gate. The executable is pinned
by actual bounded version output and file identity, then revalidated at launch.
The version probe bounds output and explicitly terminates/reaps its owned child
on error or timeout. Protocol initialization, account/catalog/rate-limit checks,
permission verification and response waits happen outside the control mutex.

Before each thread/start, thread/resume, turn/start, turn/steer or review/start
frame, the adapter calls the required production `RequestGate`. The control host
persists a sent protocol effect and gates the bounded pipe write. Its temporary
hold targets the exact Codex session so legitimate interleaved dynamic tools can
run before the start response. Fencing/recovery expands unresolved effects to
the work identity. Responses retain status and a digest, never raw prompts or
credential-bearing protocol text. Lost responses cannot be replayed on the
same transport.

The executable hook implementation requires a private retained `HookLedger`
from actual owned-directory preparation, including the after-create outcome.
Before/after-run hooks use the same `LocalExecutor`, immutable execution binding,
fresh authority and sent-effect registry as configured checks. Preparation and
waiting run outside the control mutex; synchronous start and cancellation-handle
registration occur inside it. Actual session launch and end are recorded
separately. A failed before-run hook prevents the session from launching. An
after-run failure remains diagnostic and preserves its files/unknown effects.
Nonexecuting coordination/review roles use a role-checked implementation that
verifies current authority and never executes repository hooks.

Cancellation signals the actual owned provider/process handles; it never adopts
recovered numeric PIDs. Unknown callback or process exits retain their registry
entries and sent receipts. `RunCompletion.local_exit_confirmed` is true only
after the owned provider and all registered local command handles confirm exit.
The host must not retire local ownership merely because the Rust task joined.
`ExecutionRegistry::observe_exited` can later remove proven exited handles; this
does not reconcile a remote/protocol outcome, validate evidence, or authorize
replaying an unknown hook. Host recovery must resolve those effects separately.

The native runtime coordinator remains responsible for selection, attempt
admission, provider refresh, current OS owner checks, correction ingestion,
delivery verification and supervisor integration. The initial worker contract
stays immutable. An active child can receive a separate, host-verified correction
chain with unchanged authority, permission scope, profile and dependency
contracts; its exact native input and processing report are tied to the actual
turn. Pending classification closes effects while the finite original session
waits. Historical checks remain attached to their original work generation.
Resume additionally requires actual provider history and exact effective binding
compatibility. It cannot silently rebind a changed instruction manifest,
directory, account or profile. The complete unfinished-work trigger and resumed
session readmission path remain separate implementation work; a generic summary
cannot authorize them. Live behavioral qualification remains unperformed.

Deterministic validation uses the real assembly/control/broker/hook path and
explicit disposable subprocess fixtures. Those fixtures never execute the
installed Codex model or contact provider accounts:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --offline --locked -p kegari-v0 --test worker --test codex_protocol
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --offline --locked -p kegari-controller --test supervisor
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --offline --locked --workspace --all-targets -- -D warnings
```

Native sandbox enforcement, live account/model behavior and required human
evaluation calibration remain unqualified. This module does not clear those
gates or authorize coordinator activation.

Worker services now require the host's durable process journal. Both the bounded
version probe and app-server start are gated and registered before the control
mutex is released. A separate owned probe handle must have exited before the
app-server handle is installed. The provider starts in the exact admitted private
cwd. See [process recovery](process-recovery.md) for crash/identity semantics and
the remaining whole-tree containment limitation.

Live evaluation and native scheduling share `launch_provider` through the sealed
`WorkerProviderLaunch` capability obtained from an actual `ScopedWorker`.
Construction validates without registering a fictitious active process; launch
registers ownership under the current serialized permit before the first native
probe. Probe and app-server starts each inherit a protected journal guard, and
all subsequent requests use the same durable protocol gate. The capability binds
the complete permission configuration, exact assembly, source isolation, account,
canonical work and original turn budget. Closing before launch prevents later
spawn; cancellation fences before invoking owned handles. Returning a process
error or dropping a trial does not release an unknown process hold.

Actual joined runs retain their original `WorkerContract` and optional
`WorkerExecution` capabilities for separately admitted delivery phases. A
historical JSON session record cannot manufacture checkout ownership.


Joined completions retain actual failed check capabilities alongside passing
checks, only after the corresponding effect has been observed under the current
authority. A failed check is never passing evidence; its capability can establish
an exact remaining validation obligation for a later host decision. An unknown
owned-process exit withholds both capability classes. Serialized diagnostics
cannot recreate either class.
