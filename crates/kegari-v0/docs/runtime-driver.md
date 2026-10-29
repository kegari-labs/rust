# Native scheduling and recovery driver

`RuntimeDriver` implements the supervisor's required driver against the concrete
`RuntimeCoordinator`. Its finite limits bound worker duration and concurrent
parent authorization tasks. It schedules only `ready_actions()` produced from
current complete provider observations and runtime-verified gates. Source
selection, isolated preparation, original phase budgets, current admission,
worker construction and process registration use the production paths in
`runtime_workers` and `worker_run`.

The synchronous scheduling implementation lives in `kegari-scheduler`, imported
through `kegari_scheduler`. It owns queue/capacity selection and retry helpers;
configuration-to-limit conversion and the real admission callback remain in this
backend. An allocation is not a permit, receipt or process owner. The existing
`Attempts` helper is retained by the scheduler crate without replacing the runtime's
receipt-based attempt accounting.

The scheduler reserves capacity for both preparation and model work, rotates
between workspaces and retains running reservations when configuration lowers
limits. The control host independently checks actual durable admissions at the
final serialized admission boundary. Authorization publication is a separate
bounded metadata task and does not consume a model slot. Long Git metadata and provider/session work run outside the control mutex and
the async listener's executor threads. Repository hooks and validation use the
selected Codex execution interfaces after migration.

Every action gets a retained dispatch record before execution. A worker action
binds semantic work, configuration, workflow, profile, scope, dependencies,
phase and original attempt ordinal. Observation timestamps, provider refresh
noise and the runtime's own comments cannot create a new preparation attempt.
A failed unchanged preparation is retained; a new source observation alone does
not silently grant another attempt. An explicit verified trigger or relevant
semantic change is needed. This dispatch history is not a delivery proof.

Joined launch results are retained before writing their terminal record, and a
storage failure cannot discard other completed joins. Failed launches fence any
remaining permit for the same canonical work before cancelling owned processes.
Completed worker/check capabilities remain retained by `RuntimeCoordinator`.
An empty in-memory registry never proves an earlier native worker exited. The
driver compares each retained process binding with its exact original admission,
including runtime, owner generation, owner identity, canonical work and scope.
Unresolved ownership of Kegari's own processes prevents their retirement. For
repository commands the migrated path consumes Codex's supported lifecycle facts;
it does not reconstruct command-level ownership or claim arbitrary descendant
closure. Unknown effects retain the affected workspace and block dependent work.

Recovery restores provider intervention holds from exact bound session journals.
An interrupted parent authorization publication can be reconciled from exactly
one unchanged, authenticated native comment and its original receipt, including
while STOPPED. Recovery never retransmits a missing comment or turns historical
publication into current authority. Failed or ambiguous recovery retains the
receipt's target hold and a diagnostic while unrelated authorized work can
continue. Recovery scans use finite bounds and rotate through pending receipts.

The driver does not manufacture child completion, hosted results, parent evidence,
human acceptance or release progress. Those phases require their separately
verified production capabilities. [SPEC 1.5](../SPEC.md) retains the current
control-host driver and lock/lease child arrangement. The custom repository
execution path remains pending replacement and deletion; no process relocation
or new executor qualification project is needed first. The exact-object source
broker is used by worker preparation, with missing or changed source objects
remaining explicit blockers. Current source and component evidence do not
establish live Codex operation, service installation or operational qualification.

Select the smallest relevant existing check when changing this boundary; do not
routinely run all of them. Focused references are `cargo test -p kegari-scheduler`,
`cargo test -p kegari-v0 --lib runtime_driver::tests`,
`cargo test -p kegari-v0 --test runtime_schedule` and
`cargo test -p kegari-v0 --test worker`, using the pinned toolchain/cache with
`--release --offline --locked`. Pure scheduler reload/fairness coverage lives in
the scheduler crate; admission binding, retained unknown exit, storage failure
during result retention and stable dispatch identity remain backend coverage.

Authorization retries distinguish actual admitted publication attempts from
failures before any admission. The publisher's durable planner owns admitted
attempts. The driver permits only a terminal retryable preparation error with
unchanged observed admission inventory and no provider effect, within the first
captured finite cap and exponential/provider backoff. Configuration increases do
not enlarge that cap. Dispatched or ambiguous attempts cannot be replayed.

Correction publication uses the same concrete publisher as host workflows.
STOPPED recovery verifies existing route/final receipts without writes. A RUNNING
continuation requires a strictly larger exact verified prefix, with every prior
stage unchanged and the original finite stage bound retained. Unchanged,
missing or ambiguous prefixes remain blocked; observations do not become source
classification authority by themselves.
