# Runtime worker preparation

`runtime_workers::launch` connects current sealed runtime authority to the actual
worker adapter. It accepts a canonical phase, owned lease, operation ID, finite
turn/duration bounds and, where applicable, a host-selected exact repository
commit/tree. It derives the role from the authorized child's responsibility or
independent-review gate. Parent coordination never receives source-write rights.

The runtime checks durable retry eligibility before preparation. A separate
preparation admission reserves the same canonical work identity. Owned Git clone
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
native command boundary still retains `process_tree_unverified` where group exit
and pipe EOF cannot prove that detached descendants exited. Delivery-specific
sealed review/validation phases, continuation readmission, full descendant containment, startup reconciliation of retained native worker
process references and non-Linear cleanup eligibility are separate remaining
integration work; retained preparation records do not authorize cleanup.

Source resolution reads the actual configured provider staging reference twice
around commit/tree metadata, or rereads the exact independent PR before and after
that metadata. Changed source/configuration/authority blocks preparation. It does
not update the operator checkout. The exact selected objects must currently be
available there for a local owned clone; an authenticated owned remote-object
cache is pending. Git metadata and clone commands use the preparation permit's
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
