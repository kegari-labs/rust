# Owned worker sessions

[SPEC 1.5](../SPEC.md) retains Kegari's ownership of its Codex app-server process
and workflow/session admission. Codex owns repository tools and internal command
execution. The custom repository executor has been removed; supported lifecycle
hooks use the Codex command interface. Remaining hook/lifecycle limitations and
focused evidence are tracked in [PROGRESS.md](../PROGRESS.md); the former custom
executor qualification campaign must not be resumed.

## Preparation and protocol admission

`worker_run::prepare` assembles configured workflow guidance, shipped role context,
current authority, source identity, Codex account/profile and supported permissions.
It compares configuration and workflow identities and starts no provider session.
`PreparedRun::contract()` supplies the immutable scope for durable run admission.
The production worker retains finite turn/time/attempt limits and cannot acquire
new authority from a fixture, raw session record or agent output.

Native app-server creation and registration of its actual cancellation handle
occur inside the fresh serialized permit gate. Executable identity and bounded
version output are verified, then revalidated at launch. The owned version probe
is terminated/reaped on error or timeout. Initialization, account/catalog/allowance
checks, permission verification and response waits run outside the control mutex.

Changing thread/turn/review requests use the production `RequestGate`. Kegari
persists the sent protocol effect and gates its bounded pipe write. Interleaved
workflow/provider callbacks and response waits remain outside that gate. Lost
responses stay uncertain; another transport or a matching request ID does not
authorize replay. Protected records retain safe identities and digests rather
than raw prompts or credentials.

The worker's admitted role selects native Codex repository capabilities. Required
hooks use supported Codex execution interfaces with their existing timing and
failure semantics. Independent PR review cannot execute repository code or hooks.
The migration removes Kegari's hook/command executor rather than adding another
wrapper around it. Missing capabilities block the affected action specifically.

## Cancellation and joined completion

The host retains `OwnedWorkerRun`, fences mutation/admission before `cancel_fenced`
and awaits `join`. Actual owned handles govern app-server cleanup; recovered
numeric PIDs cannot be adopted as owned children. Native launch still uses the
host journal/guard and shared process interlock needed for Kegari's own processes.
See [process recovery](process-recovery.md).

Workflow completion must distinguish actual owned host exit, supported Codex
terminal/writer observations, result acceptance and unresolved effects. A joined
Rust task, EOF or app-server exit does not prove arbitrary descendant closure.
Request supported Codex interruption/cleanup, preserve affected workspaces and do
not replay unknown commands or adopt/restart arbitrary background services.

The existing completion registry, host-process evidence and retention paths must
migrate without discarding uncertainty. Old local-command closure fields are
legacy contract data; they must not silently become proof of Codex-internal
command cleanup. Change affected persisted identities explicitly and refuse
unsupported recovery while retaining original records.

## Runtime and recovery integration

The runtime coordinator owns selection, current authority, finite admission,
provider refresh, correction ingestion, result/review gates and supervisor
integration. The original worker contract remains immutable. Verified corrections
may steer the bounded session only within supported scope; processing is tied to
the actual turn and source generation.

Resume requires supported provider history plus compatible account, directory,
profile, instructions and source/workflow state. Opaque protected lineage and
actual receipt checks remain backend responsibilities; protocol data does not
issue admission authority. Failed results stay failed, and unknown outcomes retain
their holds. Later validation or repair requires its own actual workflow admission.

Evaluation and ordinary workers share actual app-server launch and protocol
mechanics. Evaluation permission alone cannot create an unowned subprocess or
release uncertain ownership. Fixture-based results do not establish live behavior.

## Focused development checks

Select existing worker/protocol/supervisor checks relevant to the changed path,
using the pinned toolchain/cache and `--offline --locked`. Do not routinely run
all suites or add new lifecycle qualification infrastructure during feature work.
Initial migration uses a simulated production flow and missing/stale-result
refusal. Native/provider qualification remains a separately authorized milestone
0.7 activity; no such operation is authorized by this guide.
