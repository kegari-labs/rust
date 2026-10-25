# Scoped worker tools

The production broker composes `ControlHost`, opaque reconciliation gates,
`ProviderBroker`, `LocalExecutor`, the shipped instruction assembly and exact
Codex dynamic-tool definitions. Deterministic fixtures verify this composition;
no live model, provider write, sandbox qualification or human calibration has run.

[`worker-run.md`](worker-run.md) documents the concrete configured session and
hook factory, durable admission budgets, protocol write gate and owned process
completion boundary around this broker.

Host setup first creates a `ContractInput` from actual assembled instructions,
validated isolation, current `FreshGate` or `IndependentReviewGate`, selected
Codex account, exact repository, and configured external read-resource mappings.
`WorkerContract::new` verifies their identities and hashes the full tool schemas,
descriptions, account, permissions and authority into the admission scope. The
host admits that exact work key and scope through `ControlHost::admit`, then
constructs `ScopedWorker` with `WorkerServices`. Worker actors have no operator
roles or transport principal. Neither operator API commands nor arbitrary
provider mutations, shell commands, URLs or Git arguments are dynamic tools.

The host must supply concrete `WorkerHost` callbacks for provider refresh,
current OS process/start identity and held runtime lock, allowance enforcement,
prior-effect reconciliation, correction ingestion and authoritative observations.
The fresh-gate callbacks execute inside the serialized control gate and must not
reenter `ControlHost`; use the actual registration/owner proof instead. There is
no always-allow production implementation. Application-level composition of
these callbacks remains an integration responsibility; the deterministic test
host is not a production substitute.

The shipped definitions expose these capabilities according to role:

| Tool | Host behavior |
| --- | --- |
| `context_read` | Read one bounded entry of the supplied complete context. |
| `source_read` | Read bounded UTF-8 source through descriptor-relative paths; reject traversal, links, Git administration and control roots. |
| `source_write` | Child-only expected-digest replacement within exact authorized repository/path grants through `LocalExecutor`. |
| `git_inspect` | Restricted host Git inspection for executable roles; no arbitrary argv. |
| `validation_run` | Run a named configured local/integrated check with fixed host outputs. |
| `outcome_submit` | Retain an untrusted outcome/evidence proposal. |
| `correction_propose` | Retain a parent proposal for existing required children; never mark a correction consumed. |
| `review_feedback` | Selected exact-head PR feedback through `ProviderBroker`, or a pending no-change review proposal without a PR. |

Parent coordination and independent reviews expose no source-write, repository
execution or implementation tools. Independent PR work has a selected PR identity
and no fabricated Linear parent/child authorization. Feedback is neither a human
approval nor an authenticated host review attestation.

Tool arguments are closed-schema and bounded. Every tool call must match the
checkpointed account, thread and current turn. Same call IDs with changed inputs
are rejected; duplicate completed calls return their saved result. Pending calls
remain uncertain after interruption. Existing journals cannot be reopened as a
fresh operation or silently replayed. Journal and retained artifact directories
are owner-only, explicitly forbidden to workers and outside their read/write
roots. Session checkpoints persist pending provider methods before transmission.

Configured checks prepare their exact source snapshot before launch. The host
persists a sent effect and synchronously launches/registers the owned command
under its gate. Process waiting and full source/result verification happen after
the gate is released. `ExecutionRegistry::cancel_fenced` refuses cancellation
until the control host has fenced the operation without a persistence failure.
Force-stop therefore persists/fences first, cancels only the retained owned
process handle, then records its actual exit. Final `complete_effect` rechecks
current authority before a `VerifiedCheckExecution` can leave the broker. A
force/revocation race retains rejected evidence and cannot pass a gate. Both
successful and failed output bytes are retained before a later run replaces them.

Outcomes stay pending until the actual adapter reports completion for the
previously active turn, clears its active turn ID, and records that exact turn in
`completed_turns`. Current correction consumption, absence of intervention/input
and fresh authority are also required. `OutcomeSink::record` receives the exact
completed turn plus diagnostic claim/provenance; it must retain a pending claim,
not accept worker assertions as verified evidence. Delivery receives local gate
capabilities separately through `take_verified_checks`; fixture executions cannot
mint production evidence.

The evaluation runner accepts this concrete broker, checks the exact admitted
work/instruction/tool/isolation contract and rejects a deterministic command
boundary for actual-agent qualification. Its correction timeline must enter the
host's normal authorized correction path before steering. An active child may
advance through an opaque `VerifiedCorrectionTransition` only when actual native
route publications prove an exact signed predecessor/history chain and unchanged
authorization, scope, profile, workflow and dependency contracts. The original
assembly and session binding remain immutable. A later input carries the effective
work and exact revision; it is recorded against the acknowledged provider turn.
Steer acknowledgement does not consume a correction. The matching completed turn
must submit `processed_correction` with the actual watermark, input digest and
transition digest, and the native gate must still be current.

Pending native classification keeps effect/source gates closed while retaining
an existing bounded worker. A completed provider turn waits for host routing
within the original operation deadline; it does not run a new session or reset
its turn budget. Actual provider acknowledgements are retained against the exact
sent request even if native task generation changes between transmission and ACK.
Revoked ownership and fenced permits still reject further effects.

Corrected local checks use `EffectiveExecutionBinding`: the original owned
marker remains unchanged, new evidence names the effective work, and older checks
remain historical even if source bytes happen to be identical. Schema-2 outcome
recovery verifies the original assembly, signed transition chain and ordered
actual delivery/processing/completed-turn journal records. A historical record
alone cannot recreate a live execution capability.

`prepare_admitted` returns a consuming `AdmittedWorker` shared by production and
evaluation. Its trial path exposes the same ScopedWorker/provider launch and real
hook lifecycle. Trial completion is opaque and requires a current final checkpoint
and actual owned exits. The declared native evaluation fixture mapping and full
recovered-thread continuation remain incomplete. No completed fixture-only result
or skipped human calibration establishes live qualification.

Deterministic validation:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test -p kegari-v0 --test worker
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy -p kegari-v0 --test worker -- -D warnings
```

Coverage includes role/schema/path/account binding, actual scoped file changes,
revoked ownership and stale turns, duplicate tool calls, genuine adapter completion
notifications, unknown recovery, independent PR identity, correction scope,
failed-artifact retention, fixture-vs-production evidence and status/force-stop
responsiveness while an owned command waits. Native command confinement remains
a separate blocked qualification described in `protocol.md`.
