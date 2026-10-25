# Behavior evaluation suite

The bounded trial loop, report CLI and 28 versioned variants are implemented. Deterministic
replay tests validate the graders and boundaries; they are **not actual-agent
qualification**. No live trial has been launched. The profile slot is unresolved,
the concrete fixture/native context and live authority composition is still
required, OS sandbox and
provider compatibility remain unqualified, and human review remains pending.

Suite version 2 uses grader `kegari-observable-v2`. It binds the complete trial
declaration and prepared context, rejects undeclared host-metadata hints and
counts nested `.git` paths in source/scope grading. Only the repository root's
Git administration is checked separately. Earlier grader results remain
historical evidence and need regrading before comparison under these rules.

`evaluations/manifest.json` declares all V0-E01–08 variants, their REQ/SC/V0
mappings, roles, context, initial state, outcomes, artifacts, prohibited effects,
scorer revisions and qualitative pass/fail examples. Every variant declares two
independent repetitions, three turns maximum and 180 seconds maximum. The exact
suite cap is 56 attempts and 10,080 seconds. These are engineering ceilings, not
an allowance to execute or spend. No model/account/profile was inferred from the
builder session. `profile-binding.template.json` records the missing selection.

## Fixture and scorer boundaries

`fixtures/<variant>/repository` contains ordinary source, tests, workflow and
root/scoped AGENTS.md guidance; `context.json` supplies synthetic task facts.
Continuation/recovery variants have an explicitly digested `prior.json`. The
reused-guidance variant stages its declared previous files in the same trial
directory, applies the current fixture, and then invokes production discovery.
The sole resource parameter `@KegariScratch@` in fixture workflow check commands
is bound to the actual private scratch directory before production loading; the
effective workflow digest and source snapshot record that substitution. Ordinary
Python 3 validation scripts write bounded status artifacts to scratch. The
testing fixture also checks that the proposed regression rejects a seeded mutant
and accepts the valid implementation. Provider
state in these seeds is simulated, not evidence of a live Linear/GitHub result.
The visual fixture is an HTML candidate; the real host must capture its actual
render and bind the artifact to that candidate. Replaying a reference image does
not exercise visual evidence capture.

`scorer/<variant>.json` contains private expected predicates and
`scorer/<variant>.trace.json` contains known-good **deterministic replay** inputs.
They never enter the worker directory or prompt. Traces are test controls, not
cached agent trials. All fixtures and grading material are independently
reviewable. Input/scorer changes require updating the declared digest and
regrading comparisons; changed rubrics do not preserve comparability by default.

`prepare_fixture` refuses an existing destination and captures bounded regular
files once, binding their bytes and modes to the declared digest. It rejects
hardlinks/symlinks, changing file descriptors, oversized directory inventories
and excessive nesting. Prior state, context, native bindings and reset files all
come from that same immutable capture. It verifies the final reset against those
captured bytes, allowing only the declared scratch substitution in `WORKFLOW.md`.
It never clears a
previous attempt. Its private preparation digest binds the input, declared prior
state, initial file snapshot and all prepared paths; replacing any of these
invalidates assembly, live preflight and grading. The schema-1 context is exactly
the fixture input with `timeline` removed and four canonical host fields added:
`workspace: {id, name: "synthetic"}`, `operation: {id}`, the actual bound `profile`
and `phase`. IDs must be the real admitted identities. Extra nested metadata and
undeclared guidance are rejected, including hints placed inside `workspace` or
`operation`. A native fixture mapping needs an explicit versioned declaration;
these comparisons must not be relaxed to make a native context pass.

Native context declarations can now include `context-binding.json` beside
`context.json`. This file is part of the declared fixture digest and private
preparation seal. Its schema version is `1`, with sorted unique `source_write_paths`
and `slots` entries containing a JSON `pointer` and typed `source`. Each declared
location in `context.json` must contain exactly `{"$kegari":"SOURCE"}`; every
placeholder must be declared exactly once. Required slots are `/workspace/id`,
`/operation/id`, `/profile` and `/phase`. Additional sources are limited to the
actual assembly's `work_generation`, `correction_revision` and
`repository_revision`. Digests and revision identities are validated. The binding
cannot copy arbitrary observed context, infer substitutions, or omit extra fields.
The complete rendered context, including all native metadata and assignment
payloads, must equal the production worker's context. Timeline input remains
outside the initial prompt. This matcher is implemented; migration of the suite's
native provider fixtures remains incomplete; the owned-clone bridge below still
requires complete runtime evaluation coverage.

For a native worker, `prepare_worker_fixture` binds the one declared workflow
scratch resource to the anticipated operation under the configured execution
root. It creates only a fresh seed fixture. It neither creates that worker's
directory nor grants admission. After ordinary production preparation/cloning
and admission, `AdmittedTrial::bind_fixture` consumes the seed and validates the
actual worker's operation, exact clone bytes/modes, workflow digest, complete
native context, role/phase, scope and private grader/report boundaries. Subsequent
grading reads the actual owned clone and scratch; the seed remains intact.
The private binding fixes the complete instruction manifest, refuses a second
worker/rebinding, and cannot be installed after the worker starts. An unbound
native seed cannot enter direct assembly or live execution. This bridge is local
implementation; it does not establish native execution qualification or complete
the suite's provider/context migration.

`verify_grader_boundary` checks actual validated filesystem
grants, repository, scratch, private scorer and report roots. The scorer and
report store must be explicit deny roots. The checked-in tests attempt grant
expansion, file aliasing and grader access, including executable paths through
private grading directories. These checks verify configured
boundaries and resolution; they do not replace the required live OS canary.

## Production integration contract

The runtime must explicitly invoke the suite; startup, polling, configuration
checks and profile edits must not call it. The API is in `src/evaluation.rs`:

1. Load `SuiteManifest`, preserving its unresolved profile until the operator
   supplies an exact allowed Codex profile, verified account and authorization
   reference. A profile file is configuration, not permission.
2. Create a new `ReportStore`, and invoke `run_suite`. For each trial, prepare a
   fresh fixture outside the suite/scorer/report roots. The runtime prepares a
   real disposable Git repository and captures the actual revision using its
   Git adapter; the runner never invents a commit identity or source revision.
3. Resolve actual executable/write grants and `ValidatedIsolation`; explicitly
   forbid scorer/report/auth/controller roots. Load that fixture's actual
   `WORKFLOW.md`. Use `assemble_trial` with the real `AssemblyInput`, concrete
   scoped tools, canonical operation identities and host-verified Git revision.
   It calls production `instructions::assemble` and verifies exact fixture
   context, workflow, role, cwd, profile and read boundaries.
4. Bind the complete dynamic tool definitions with `BoundToolContracts::bind`,
   including schemas and descriptions. Its immutable definitions and digest are
   tied to the exact assembly; the live runner uses and records only that bound
   contract. Same-named schema/description changes require a new binding and
   new trial provenance. The shared production path is
   `runtime_workers::prepare_admitted`, followed by consuming the returned worker
   with `into_trial`. The resulting opaque `AdmittedTrial::operation` provides
   the actual scoped worker, provider launch capability and hook lifecycle;
   callers cannot supply an unjournaled subprocess. The production `TrialHost`
   trait extends the same `codex::ToolBroker` used by product execution. Its
   `observe` projection must come from verified host receipts, actual gates,
   checks and artifacts. Worker claims belong only in `AgentSummary`. Its
   `reconcile_prior` consumes only declared synthetic earlier state. Its `rate_limits` feeds the actual provider snapshot into the runtime
   allowance gate. Its
   `checkpoint` durably records pending adapter requests before dispatch and
   current session state after observations; recovery must reconcile unknown
   requests, never resend them automatically.
5. Implement `LiveAuthority::acquire` through actual coordinator ownership,
   account capacity/allowance, explicit evaluation authority and independent
   review enablement. The non-deserializable `LiveLease` holds the reservation
   and rechecks authority at boundaries and tool calls. Its teardown must stop
   owned workers on cancellation; it cannot clear unresolved remote effects.
   No paid fallback or remote mutation authority is implied.
6. `run_live_trial` first checks the complete variant against the suite declaration
   and the actual worker role and phase against that variant,
   and the actual worker's entire bound context against the sealed fixture. It
   verifies the pinned executable, account, catalog/profile,
   rate-limit observation and concrete permission configuration, then uses
   real `RpcClient::start_thread`, native `review/start` for review roles, and
   bounded turn/event pumping. Declared edited/follow-up corrections use
   `turn/steer`, or queue a subsequent bounded turn after completion. A steer
   response does not count as correction consumption; only the host's verified
   correction watermark does. The timeout terminates the owned adapter and
   marks outstanding effects uncertain. The outer runner applies the lesser of
   each declared trial deadline and the remaining suite allowance to the entire
   executor, including fixture preparation/admission and final collection. A hung
   trial's future is dropped, invoking its owned cancellation guard, and its
   interruption is recorded before the next attempt is considered. The suite
   deadline records all remaining variants as skipped.
   A successful actual lifecycle can then yield `AdmittedTrial::finish` and
   `RuntimeCoordinator::finish_trial`; these consume the checked completion and
   do not reconstruct a permit or use a replay trace as production evidence.
7. Grade actual source snapshots and runtime observations, append the report,
   and preserve every failed/blocked/interrupted attempt. Grading compares
   exact changed paths and modes, required file contents, check status/receipt and
   candidate digest, gate state, findings at the actual revision, artifact
   provenance, and summary claims against host facts. Neither tool-call
   presence nor a generic success summary is a pass.

The API supports the production path without an evaluation-only replacement
broker. That concrete runtime composition has not yet been exercised; API and
fixture completion must not be described as end-to-end live validation.

Each live trial also requires the actual scoped worker's opaque
`WorkerProviderLaunch`. It binds the original admission, complete instructions,
isolation, permissions, executable and auth home. Evaluation uses the same
journaled version probe, serialized app-server spawn, owned process registry and
RPC request gate as ordinary workers. An evaluation lease alone cannot authorize
an unjournaled subprocess. Cancellation fences the actual operation; unconfirmed
exit retains its ownership and makes the report interrupted.

Final observations hash the actual candidate files and preserve each check's
original checked-file digest. A later edit or revision change makes an older
result unknown for the current candidate. Dirty source cannot be relabeled as
tested; the host must first establish the next exact validation revision.
Artifact observations revalidate their exact immutable retained bytes, and a
missing final observation is recorded as a failure explanation rather than
silently omitted grading. Fixture execution still cannot mint production evidence.
An outer suite timeout also fences/cancels the actual admitted operation before
the dropped trial releases its provider handles. An unconfirmed exit remains
interrupted with unresolved ownership; handle destruction is not proof of exit.

## Running permitted deterministic checks

From the Rust workspace:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test -p kegari-v0 --test evaluation
```

These tests load every fixture/scorer, replay each known-good trace, reject
missing/generic summaries, wrong revisions, incorrect state transitions,
missing/unsupported findings, false passed checks, wrong-candidate visuals,
forbidden effects and source-scope violations. They cover fresh reset semantics,
production workflow/asset assembly, hidden grader boundaries, finite duration,
all 56 blocked results when the profile is unresolved, immutable records,
crash/unknown records and duplicate/lucky-rerun rejection. These commands do not
start Codex model work and do not need credentials.

## Inspecting and comparing retained reports

The maintainer executable performs local read-only inspection. From the Rust
workspace, use absolute report-directory paths:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo run --locked -p kegari-v0 --bin kegari-v0-evaluation -- plan crates/kegari-v0/evaluations/manifest.json
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo run --locked -p kegari-v0 --bin kegari-v0-evaluation -- inspect /absolute/path/to/reports
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo run --locked -p kegari-v0 --bin kegari-v0-evaluation -- compare /absolute/path/to/baseline /absolute/path/to/candidate
```

These commands emit one JSON object and never start a trial. Inspection reads
every declared attempt, marks missing started results interrupted/unknown, and
lists unstarted attempts as skipped. These reconstructed observations are never
written into the original archive. Corrupt, mismatched, linked or undeclared
records produce an explicit error. Reads are bounded to 8 MiB per record and
256 MiB per archive. New archives have owner-only files and descriptor-relative
exclusive writes; a replaced directory cannot redirect them. Existing files are
never overwritten or automatically repaired.

Comparison preserves baseline failures even when a later attempt passes. It
reports runtime/build/instruction changes, both outcomes, actual versus replayed
evidence, elapsed time and available usage. Changed fixtures, rubrics, profiles,
provider versions, actual input, permission configuration, evidence class, tool
contracts or declared trial conditions are marked
incomparable. Historical schema-1 provider/grader versions remain inspectable;
they do not become executable under the current adapter. Comparability describes
conditions, not an improvement or qualification claim. Unknown usage stays null.
Missing original results are incomparable and retain the declared grader version;
unavailable execution build, timing and completion evidence are never inferred
from the inspecting binary. Successful actual-agent reports require a recorded
turn, activity and evidence references with ordered durable start/end times.

## Reports and human review

Owned worker binding and live entry exclude private grader/report paths from
global guidance, workflow and supplied/provider-discovered instruction inputs as
well as OS grants. Current fixture declarations require empty initial scratch,
including no hidden files or subdirectories. Binding and the final check before
trial hooks reject undeclared scratch state. Declared hook execution can then
create its observed outputs; arbitrary preexisting outputs are not reset silently.

A run directory is exclusive. The suite snapshot is written once; each attempt
gets an fsynced `*.started.json` before dispatch and an immutable
`*.result.json`. A started record without a result is interrupted/unknown after
restart. Partial/corrupt result files require inspection, not overwriting.
Reports include fixture/input/scorer/manifest and instruction provenance,
runtime version and exact build digest/provider/profile/configuration, full bound
tool definitions and digest, bounded outcomes, assertions, available
usage, artifact references and digest-only activity. Unknown token usage is
`null`, never zero. Full arbitrary prompts/provider/tool payloads are omitted;
reports disclose that they cannot reproduce byte-identical activity. Reports
must remain private and required evidence must survive cleanup.

The suite assessment includes every trial and per-case variability; no failure
is averaged away. A later passing attempt does not erase an earlier failure.
Material failures require a recorded cause/fix and all affected variants rerun
against the changed implementation/instructions. Keep earlier report directories
and explicit resolution references for independent review. Do not infer
monetary cost or statistical reliability from these small samples.

Even all deterministic/live predicates passing yields
`deterministic_pass_human_pending`, not qualification. A human must review a
representative sample covering every qualitative criterion (truthful summary,
actionable coordination/blocker/correction, supported review, boundary respect,
reconciled continuation) and every material/disputed failure. Record reviewer,
trial/artifact references, criterion verdicts, explanations and resolutions in
separate protected review evidence. The evaluator never grants human product
acceptance, release authority or runtime readiness.

## Supervised follow-up plan (not authorized or executed)

Select the evaluated profile/account and an explicit finite subset/full-suite
allowance within the declared maxima; identify a private, disposable auth home
and execution/report roots. Qualify the pinned provider and OS enforcement with
read/write/network/control-secret canaries, then wire the actual broker,
reconciler and host observations. Authorize actual-agent trials explicitly. The
56 trials may use synthetic provider resources only; any real remote actions
need separate authority and a concrete disposable-target/cleanup plan. Preserve
interrupted local/provider state for reconciliation. Complete human calibration
and independent review before claiming behavioral qualification.

The live `TrialOperation` now takes the concrete production `ScopedWorker` plus
its canonical admitted work key. Before dispatch it verifies the identical
instruction, complete tool contract and isolation, and rejects a fixture command
boundary. Declared correction events must pass the host's real authorized
correction ingestion callback before steering. Readmission after a correction
changes an immutable work/authority generation still needs application trial
coordinator composition; the existing operation is denied rather than silently
rebinding its permission or claiming that steering alone completed reconciliation.
