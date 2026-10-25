# Retention, diagnostics and explicit handover

Preservation is the default. `Housekeeping` never removes an execution directory
because a tracker state changed, an operation exited, a release succeeded or a
disk-space check failed. It never rotates receipts, evidence or qualification
artifacts. Completed receipts remain intact; there is no automatic receipt
compaction or time-based deletion.

## Host integration

Create `LogSink` with the private diagnostics directory. Supply a typed `LogEvent`
containing runtime/owner/workspace/work/operation/session/revision identities and
a concise event, outcome and reason. Register active credentials with
`register_secret`. Known credential bytes and recognizable authorization fields
are redacted, text fields are bounded, and arbitrary provider JSON or full prompts
are not accepted as log payloads. Each log is at most 10 MiB with ten retained
files including the current file. A smaller bound is available. Lock contention
waits at most 100 ms. Failure returns `false` and updates `LogHealth`, which the
host must expose through its independent status path; it does not panic or
authorize a failed receipt write.

Construct `Housekeeping` with the real control root, shared `ControlHost`, process
probe and pinned supported Git executable. Construction pins the executable without
launching it. The native factory installs `NativeCleanupProcesses` once, using the
same registered control owner, process journal and cancellation registry as the
runtime. Replacement adapters are rejected. At directory creation, call
`register_execution` with the actual `ExecutionBinding`, validated isolation,
`OwnedDirectory` capability and exact workflow. Record owned process identities
and actual hook ledgers as their lifecycle changes. These records contain small
identity, path and digest references, not issue descriptions or prompts.

Before offering cleanup, retain a complete separate owner-protected copy of the
self-contained clone, including its Git object database, and register that
`RetentionLocation` plus exact provider reference identities. The helper does not
silently make such a copy or invent a publication record. `inspect` reads actual
filesystem bytes, Git status/object connectivity, current provider refs, process
observations and all receipts. Missing retained material, changed provider refs,
unpublished local branch/detached commits, dangling commits, dirty/untracked
source, unresolved receipts, active work, live owned processes, a changed workflow
or unknown hook state prevents an eligible plan. Linked/special files, unsafe
ownership and inventories above 100,000 entries or 1 GiB also require preservation.

The returned `CleanupPlan` contains exact paths, file/directory identities and
digests, retained location, receipt/workflow references and a token. Applying it
requires the unchanged token and a callback that checks the current authenticated
operator's `recover` mapping and unchanged configuration revision. A short serialized
admission records Sent and a runtime-wide hold, then releases the control mutex.
Provider/Git reads, inventory hashing, hook waits and filesystem traversal run
outside that mutex. Each actual unlink uses a short serialized authority check
and effect, so a force fence prevents subsequent deletions without blocking status
behind a provider response or a complete tree scan.

Native Git reads use only fixed status, branch/HEAD and object-connectivity
commands through sealed registration-derived source/retention roots. Every child
has a durable process record, inherited guard and actual cancellation handle;
cleanup does not manufacture an orchestration lease. Post-admission reads and the
configured `before_remove` use the exact opaque cleanup effect. The hook command,
working directory, binaries, workflow and timeout must match that sealed plan.
Current native repository-command containment is unqualified, so a configured
removal hook fails its prerequisite **before** creating a cleanup effect receipt
or spawning the hook. The supported no-hook cleanup path is exercised with real
owned Git subprocesses in a disposable fixture.

After repeating preservation checks, cleanup writes a retained-copy tombstone
before descriptor-relative, no-follow deletion. Each regular file is hashed again
through its opened descriptor and must match the retained bytes and mode. Hashing
runs outside the control mutex; the unlink gate then compares descriptor and path
identity, size, mode, link count, owner and modification/change timestamps. Edits
observed after preservation or hashing stop removal and retain the remaining files.
Unknown outcomes stay Sent and
are never replayed. Exact request replay returns the original receipt. A canceled
or fenced partial deletion retains the tombstone, remaining files and complete
recovery copy; it does not claim a completed removal.

Recovery and future correction callers must use `retained_directory(operation)`
after explicit cleanup. It checks the complete retained content digest before
returning the replacement recovery path. The host must wire execution/process/hook
registration and this redirect into its actual operation factory and recovery;
calling the helper with an unregistered directory is an error. A background
cleanup command uses `apply_for_command` with the actual prepared parent receipt.
The helper verifies the original arguments, principal, runtime, derived effect ID
and active background ownership; arbitrary caller flags cannot waive those checks.

## Handover

`__handover-info` is a metadata-only entry point returning persisted schema/API
versions, build identity and the verified embedded role asset digest set.
`inspect_handover` invokes only that endpoint on an explicitly selected immutable
replacement binary, with a five-second/output bound. It then holds the canonical
control/runtime locks and verifies the registered host, STOPPED intent, durable
supervisor-stop marker, complete process inventory, no pending owner, no live old
process, and no Prepared/Sent receipt. Unsupported schemas or altered metadata
preserve all source state.

A `VerifiedRuntimeQualification` requires a host-authenticated qualification
record bound to the exact replacement bytes, metadata and instruction digests,
with retained deterministic, actual-agent, real-integration and human-calibration
artifact references. Verification checks the signature and retained artifact
bytes. Scientific validity and human calibration remain the explicit human
reviewer's responsibility: this helper does not turn a signature, synthetic test
or agent's own assessment into operational qualification. No real qualification
record was created by the builder conversation.

`apply_handover` requires the current local OS principal's configure and recover
roles and the unchanged reviewed plan. Under the locks, it preserves the plan and
prior service registration, records a Sent receipt, replaces the fenced service
plist/registered immutable binary, increments owner generation and acknowledges.
Config, intent, previous receipts, attestation-key identity and required evidence
remain retained. It does not invoke launchctl or start the host/orchestrator; the
supervisor stays explicitly stopped until subsequent local service start.
Interrupted replacement requires reconciliation, not a second automatic effect.

## Evidence and limits

`tests/housekeeping.rs` uses actual disposable Git repositories, private control
storage, exact content/inode checks and typed receipts. Provider responses,
process probes and qualification signing keys are labeled synthetic fixtures.
Tests cover successful preservation/removal, replay, dirty/missing/linked/stale
files, changed provider facts, live processes, uncertainty, role revocation,
rotation/redaction/failure visibility, and handover fencing/schema/signature/
retention behavior. They do not call native Keychain or launchd, run an agent, or
establish that the current build has passed live/human qualification.

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --test housekeeping --test transport
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --all-targets -- -D warnings
```

Read-only worker preparation has a separate preservation registration. It binds
an actual preparation permit and opaque worker authority to a coordination parent,
a reasoning-only child, or a verified independent PR clone. It verifies the pinned
workflow, canonical operation/work identity, owner-only directory inodes and absence
of repository execution/source/resource-write grants. Independent clones use Git's
read-only ownership observer; a missing clone or marker is never created by inspection.
A registration can retain actual observed process identities and report its current
preservation state. These records do not fabricate Linear work for independent
reviews and do not grant cleanup permission. Explicit safe cleanup for these
read-only preparation classes remains a separate integration requirement; preserve
the directories and records meanwhile.

The deterministic cleanup regressions cover provider-read suspension with
responsive status/force-stop, preservation after that force fence, immutable
process-adapter binding, native journaled Git reads using the same cleanup
receipt, and the unsupported-hook prerequisite producing no cleanup receipt.
The focused `cleanup_unlink_tests` additionally change same-inode, same-length
bytes and file modes after inventory and after the final hash; each change prevents
unlink and preserves both the edited source and the original retained bytes.
These fixtures do not qualify native descendant containment, live credentials,
launchd installation or real provider mutations.
