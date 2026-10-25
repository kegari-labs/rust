# Runtime worker preparation

`runtime_workers::launch` connects current sealed runtime authority to the actual
worker adapter. It accepts a canonical phase, owned lease, operation ID, finite
turn/duration bounds and, where applicable, a host-selected exact repository
commit/tree. It derives the role from the authorized child's responsibility or
independent-review gate. Parent coordination never receives source-write rights.

The runtime checks durable retry eligibility before preparation. The serialized
preparation gate checks the current native grant or sealed continuation cause
again, including the exact selected attempt, caps, trigger and spacing. Its
durable typed receipt binds the eventual worker ID, canonical preparation ID,
workspace, account and exact phase. A separate preparation admission reserves
the same canonical work identity. Owned Git clone
creation has a durable Sent effect, cancellation registration and a fresh gate
around every subprocess start. Waiting happens outside the control mutex.
Creation hooks use the real LocalExecutor and retained HookLedger. Failed or
unknown preparation preserves directories and records; an exit-observation
failure fences before requesting cancellation. An empty in-memory registry alone
is not recovery proof for work from a prior host process.

Only after observed local preparation exit does the runtime retire that
preparation claim, check retry eligibility again, assemble the exact installed
instructions and admit the model attempt. The durable admission binds account,
workspace, normalized tracker state, phase, profile, instruction digest and
original finite bounds. The coordinator checks current global/account/workspace/
state capacity atomically and retains the actual configuration/workflow snapshot
for admitted work. Every later effect still checks current authority and OS
ownership. Ordinary selection-filter changes may drain admitted work; authority
or resource-access revocations cannot.

Task context uses the production assembly's exact schema and selected parent
scope. It contains observed issue/required-child/correction data and only that
parent's verified evidence and history. Independent PR context contains its exact
repository/head/base and does not invent a Linear parent. Read-only children
without a repository use a separate empty owned substrate, retain their declared
reasoning responsibility, and expose no Git, source-write, validation or hook
execution. Repository-backed children use an owned clone; the operator checkout
is never reset or written.

Product source writes and disposable test-resource writes are distinct grants.
Each resource grant must match the actual configured path and writable/disposable
flags plus the child's authorized resource names. Resource writes cannot overlap
any configured operator checkout, product source, scratch, credentials or control
records; writable resource trees reject symlinks and hardlinks. The instruction
manifest, Codex permission configuration and local execution policy bind these
same grants. Built-in Codex shell execution is disabled for every role. Declared
repository checks, restricted Git inspection and source writes use host tools.

Deterministic coverage checks source/generation binding, configured resource
flags, forbidden resource aliases, role/tool narrowing and actual assembled
manifests. It does not qualify live Codex or native sandbox containment. The
native command boundary rejects repository execution with
`process_tree_unverified` before spawn where full descendant containment has not
been qualified. Retained preparation records do not authorize cleanup.

Source resolution reads the actual configured provider staging reference twice
around commit/tree metadata, or rereads the exact independent PR before and after
that metadata. Changed source/configuration/authority blocks preparation. It does
not update the operator checkout. Git metadata and clone commands use the preparation permit's
journaled process path. After model admission the execution boundary is rebuilt
with the model permit, so preparation authority is never reused for later checks.

Worker source preparation now uses the admitted exact-object acquisition broker
for initial configured staging and independent PR sources. The request binds the
preparation permit, current repository contract and sealed work authority. The
opaque imported source supplies actual head/base objects to the acquired-clone
APIs; its cache root is separate from the configured operator checkout retained
for attribution. Repository instruction discovery reads the resulting exact
commit clone. Owned delivery assignments continue to use their sealed local
source capability. Host metadata authority cannot perform cache-miss imports.

The common `prepare_admitted` / `AdmittedWorker` path stops after exact assembly
and final durable worker admission. Production consumes `.run()`; evaluation
consumes `.into_trial()`, which exposes actual `TrialOperation` references and
real hooks. Dropping either admitted capability fences before cancellation and
preserves uncertain process/effect ownership. This composition does not itself
satisfy actual-agent qualification or the pending native fixture mapping.

Unfinished child work requires an opaque delivery proof from an actual failed
required validation or completed exact-head review with findings. The proof
retains the original implementation phase, joined predecessor, actual failure
receipt, sealed source and precise remaining task. Summaries, serialized
references and old failed checks cannot issue this capability.

`next_attempt_for_unfinished` and final continued admission preserve the minimum
historical attempt, turn and review limits. The planner waits at least one second
after both predecessor and failure reconciliation, rejects any earlier unresolved
execution, and requires the original account and current strict admission
authority. Final admission rechecks the complete repair contract and receipt
digests under the control lock, then consumes the exact latest predecessor and
trigger in the same durable admission. Concurrent consumers cannot fork it;
repeating the same source and unmet obligation is `no_progress`.

An exhausted genuine obligation can be selected for an explicit operator retry.
The runtime retains the opaque proof, verifies its original and failed-session
receipts, current source/authority and actual joined completions, and binds the
new native grant to that exact obligation. The first session of the granted
budget requires `RetriedUnfinishedWork`; the ordinary implementation scheduler
cannot discard its source or findings. Final admission rereads the current
native grant and failure inventory, keeps the granted finite caps and the
one-second exit spacing, and uses a fresh clone/thread. Later sessions in that
same granted budget require their own actual trigger and retain its remaining
caps. Retaining a historical failed check alone never creates a retry target.

A completed repair can authorize another required validation or review only
through a separate opaque source-advanced proof. It binds the new completed
implementation, a materially changed source tree, the exact earlier phase
receipt and its original finite limits. Validation/review keep their original
phase names and attempt history. The final admission consumes that proof
atomically, waits for actual exits and reconciliation, and refuses repeated
source/obligation pairs or exhausted review rounds.

Both trigger types retain the complete relevant validation/review admission
inventory. The final gate rejects a stale proof after another check or review is
admitted, including a different retry-grant lineage. Mixed grant histories
require explicit verified lineage recovery; timestamps or old failure text do
not select authority.

Delivery assignments retain the complete native issue, parent, scope and
dependency context. Their exact host-created payload is embedded under
`evidence.delivery_assignment`; it does not replace the documented context
envelope. Assembly still verifies the actual operation, workspace, profile and
phase, and result readers bind both the complete protected context and assigned
payload. Native observations live under `evidence.native_context`. Typed public
generation digests use `authorization_digest` in model context; unrelated
credential-shaped fields still fail the unchanged secret guard.

Repair assignments use a fresh clone, assembly and provider thread because their
source and task context changed. This bounded path does not yet permit a repair
across a material correction-generation change, reconstruct opaque unfinished
proofs after restart, or establish the separate compatibility proof needed to
resume an older provider thread. Those cases remain held for verified recovery.

The assembler now retains versioned semantic component digests and a private
assembly seal for future compatible resume. Only typed operation ID/attempt
metadata is normalized; the actual rendered workflow, full task context,
guidance, permissions and all other manifest fields remain exact. A workflow
that renders a changed operation ID is incompatible. Legacy manifests retain
their original serialization and cannot establish this new proof. The native
resume admission/source-reuse bridge is still unfinished.
