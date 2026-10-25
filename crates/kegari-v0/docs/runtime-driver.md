# Native scheduling and recovery driver

`RuntimeDriver` implements the supervisor's required driver against the concrete
`RuntimeCoordinator`. Its finite limits bound worker duration and concurrent
parent authorization tasks. It schedules only `ready_actions()` produced from
current complete provider observations and runtime-verified gates. Source
selection, isolated preparation, original phase budgets, current admission,
worker construction and process registration use the production paths in
`runtime_workers` and `worker_run`.

The scheduler reserves capacity for both preparation and model work, rotates
between workspaces and retains running reservations when configuration lowers
limits. The control host independently checks actual durable admissions at the
final serialized admission boundary. Authorization publication is a separate
bounded metadata task and does not consume a model slot. Long Git metadata,
hook preflight and validation work run outside the control mutex and the async
listener's executor threads.

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
Unknown process-tree exit continues to prevent local drain and service shutdown.

Recovery restores provider intervention holds from exact bound session journals.
An interrupted parent authorization publication can be reconciled from exactly
one unchanged, authenticated native comment and its original receipt, including
while STOPPED. Recovery never retransmits a missing comment or turns historical
publication into current authority. Failed or ambiguous recovery retains the
receipt's target hold and a diagnostic while unrelated authorized work can
continue. Recovery scans use finite bounds and rotate through pending receipts.

The driver does not manufacture child completion, hosted results, parent evidence,
human acceptance or release progress. Those phases require their separately
verified production capabilities. Native process-tree containment remains
unqualified; recorded group exit and inherited guard release are insufficient to
claim that detached descendants exited. The remote-object importer is also absent:
exact missing source objects are an explicit prerequisite failure. This driver is
not evidence of a live model run, native service installation or operational
qualification. The native host factory must supply the complete production
backend before the executable entry point is enabled.

Focused checks are `cargo test -p kegari-v0 --lib runtime_driver::tests` and
`cargo test -p kegari-v0 --test worker`. Fixtures cover admission binding, retained
unknown exit, scheduler reload/fairness, storage failure during result retention,
and stable dispatch identity across harmless observation changes.

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
