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
delivery verification and supervisor integration. Changed authority invalidates
the immutable worker contract. Resume requires reconciled prior state and exact
effective binding compatibility; it cannot silently rebind a changed instruction
manifest or correction generation. A fresh thread may be necessary. The complete
application continuation/readmission path and live behavioral qualification are
separate remaining integration work.

Deterministic validation uses the real assembly/control/broker/hook path and
explicit disposable subprocess fixtures. Those fixtures never execute the
installed Codex model or contact provider accounts:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test -p kegari-v0 --test worker --test codex_protocol --test supervisor
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy -p kegari-v0 --lib --test worker --test codex_protocol --test supervisor -- -D warnings
```

Native sandbox enforcement, live account/model behavior and required human
evaluation calibration remain unqualified. This module does not clear those
gates or authorize coordinator activation.
