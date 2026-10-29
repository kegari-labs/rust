# Host metadata process authority

These are narrow Kegari-owned Git/control operations retained by
[SPEC 1.5](../SPEC.md). They are not a repository shell API or an admission
requirement for commands launched internally by Codex.

`LocalProcessPermit::metadata` authorizes bounded trusted Git reads while the
coordinator is STOPPED, without manufacturing an orchestration lease or worker
permit. Its private token binds the registered control process, owner generation,
configuration revision, canonical work target, pinned Git bytes and authenticated
read roots. Root proofs retain actual directory identities and signed candidate
markers; ordinary path strings cannot create this authority.

Admission records a Sent receipt and local process hold. Each command rechecks
current ownership, configuration, exact roots and a fixed read-only Git grammar
under the control mutex. Its journal guard and cancellation handle are registered
before spawning; waiting occurs outside that mutex. Force-stop fences these local
operations, and service shutdown cannot pass while they remain unresolved.
Only the original in-memory capability may acknowledge completion after observing
all of its actual owned handles. Restart restores the hold, never a launch token.

The grammar permits exact revision, object, ancestry, binary diff, status, tree,
branch-syntax and local human commit-identity reads. It denies arbitrary config,
external diff/filter commands, mutation commands and caller-controlled execution.
High-level clone/candidate/commit mutators also reject metadata scope before any
Rust filesystem mutation. Provider-facing models never receive this capability.

`cargo test -p kegari-v0 --lib host_process::tests` tests state/receipt invariants.
`cargo test -p kegari-v0 --test host_process` runs owned disposable Git children
with copied pinned Git bytes and an explicitly synthetic boot probe; it verifies
journal guards, STOPPED authority, force fencing, changed directory identity and
absence of a forbidden clone destination. This is not native service or sandbox
qualification. Cleanup authority is a separate capability and must consume a
verified cleanup plan; this metadata API does not authorize deletion or hooks.

Explicit cleanup also uses a sealed host authority. Its operator receipt retains
a runtime hold while asynchronous reads and deletion proceed; native process
records bind the exact cleanup payload digest. Registered source and retained
roots are revalidated, and post-admission reads reuse that cleanup token rather
than fabricating a worker permit. The fixed metadata grammar adds cleanup status,
local heads and `fsck --full --no-reflogs`; mutating fsck options remain forbidden.
Repository removal hooks must migrate to the selected Codex profile's supported
execution interface while retaining the cleanup plan and preservation checks.
The current custom hook adapter remains pending replacement; it is not a retained
host-shell fallback. Unavailable hook capabilities or uncertain outcomes preserve
the directory. Generic receipt reconciliation cannot clear any active host-local
process receipt.
