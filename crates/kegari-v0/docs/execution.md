# Local execution and evidence

`execution.rs` implements local hooks, named checks, restricted Git inspection,
scoped source writes, and validation of a video captured by an integrated check.
It calls `isolation::run_command` through `SandboxBoundary`. The adapter has no
unsandboxed fallback and never injects provider credentials into subprocesses.

## Host integration

Create an isolated clone, validate its `IsolationPlan`, then register an
`OwnedDirectory`. Retain the returned identity digest in host control storage.
Recovery uses `OwnedDirectory::reopen` with that retained digest; a directory
marker by itself does not authorize adoption. Source and operator checkouts must
be disjoint. Every action revalidates the current boundary, runtime/work/repository
identity and admitted Git revision.

`LocalExecutor` accepts the trusted command boundary, pinned `Git`, finite limits,
and owned directory capability. The command boundary is a host dependency, never
a dynamic tool argument. Only child and validation roles can execute repository
commands or hooks. Parent, review and independent-review roles cannot do so.
`GitInspection` exposes only fixed status, diff and object-file reads. Trusted
delivery code separately owns commits, branches, publication and merge effects.

`write_source` requires a child role, exact permitted path, expected prior content
digest and finite bytes. It persists an incomplete receipt before writing, uses a
retained parent-directory descriptor with no-follow exclusive creation and an
atomic rename, rechecks identity and prior bytes, then persists completion. It
rejects linked files, traversal, `.git`, stale writes and paths outside the scope.
Failed writes preserve owned temporary files for explicit reconciliation.

## Hooks and durable attempts

`HookLedger` persists a started/unknown record before dispatch. `after_create`
runs once and only success initializes the directory. `before_run` is keyed to an
admitted session. The host calls `mark_launched` only after actual launch, and
`mark_ended` after actual termination, before `after_run` becomes eligible.
An `after_run` failure remains diagnostic and never makes failed work successful.

`before_remove` requires the digest of an explicitly authorized cleanup plan and
no launched session or unknown hook. A failure leaves all files in place. A fresh
explicit cleanup plan can retry a known failure while retaining the earlier
record; the same plan cannot replay. The host must recheck every preservation
predicate after the hook and immediately before removal. This adapter never
deletes an execution directory. Timed-out or identity-mismatched hooks remain
unknown and require reconciliation.

## Named checks

`run_check` resolves the exact named local or integrated workflow command. It
preserves argv and uses the validated repository cwd. Hosted CI cannot be created
through this API. Integrated checks require the candidate-vector digest. The host
fixes any additional output paths, such as a recording, before dispatch.

Checks persist `CheckJournalEntry::Started` before execution and a finished or
uncertain outcome afterward. The host must store these records durably, prevent
duplicate operation dispatch, and reconcile outstanding started records after a
crash. Persistence failure never grants a successful result.

For supervised cancellation, use the split API: `prepare_check` performs source
inspection without launching a process; `start_check` synchronously creates the
owned process under the serialized control gate; `complete_check` waits outside
that gate. Register the returned `CommandCancellation` capability before leaving
the start gate. Persist and fence operator cancellation before invoking it. The
host finalizes the durable effect and rechecks current authority before exposing
passed evidence. Cancellation holds an actual unreaped child handle, never an
adopted numeric PID. Full source hashing and process waiting do not hold the
control mutex. `run_check_complete` also returns a private failed capability so
failed result bytes can be retained before a corrected rerun.

Source snapshots bind HEAD, tree, regular-file bytes, executable modes and symlink
targets. The pinned Git adapter rejects tracked/index changes and undeclared
untracked files, so dirty source cannot claim a clean commit. Source and revision
must remain unchanged while checks run. Each declared artifact must be a fresh,
bounded regular file under an allowed output root. Existing output requires a
private `RetainedArtifact` capability: `retain_outputs` copies and verifies its
bytes in content-addressed chunks in owner-only host storage outside every worker
read/write root. A digest alone does not authorize overwriting prior evidence.

`VerifiedCheckExecution::production_evidence` rejects deterministic fixture
results. The host must additionally apply current work/correction authority,
workflow freshness and the exact required check inventory before constructing a
delivery gate. A local successful check does not imply hosted CI, review, staging,
acceptance, release or production success.

## Video provenance

A `CaptureOrigin` can only be obtained from a successful integrated check that
declared and freshly produced the video artifact. It binds the executed check,
source snapshot, candidate vector and bytes. An arbitrary existing MP4 path cannot
be promoted into a capture origin.

`validate_video` revalidates those bindings, runs the explicitly admitted ffprobe
and ffmpeg binaries with fixed arguments, verifies MP4 container metadata,
H.264/AAC streams, bounded duration and dimensions, and requires a complete decode.
Optional conversion reads the original captured flow, uses fixed codec settings
and a new output, and preserves the original. It does not generate substitute
frames, truncate the flow or invent capture success. Media receipts bind tool
binary digests and command outcomes. Upload, attachment to the parent, retrieval
and playback authorization are separate provider gates.

## Validation and remaining qualification

Run the deterministic integration suite with:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache \
  cargo test -p kegari-v0 --test execution --test git_delivery
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache \
  cargo clippy -p kegari-v0 --lib --test execution --test git_delivery -- -D warnings
```

The execution suite uses real disposable Git repositories and filesystem writes,
with a clearly marked deterministic subprocess boundary. It verifies hook order
and failure preservation, write-ahead persistence, role denial, dirty source and
stale artifacts, prior-artifact retention, exact source writes, codec/decode
failure, full-flow conversion provenance, directory identity and hardlink denial.
These fixtures do not qualify actual OS sandbox enforcement, live ffmpeg capture,
provider upload/playback, or real-agent behavior. The current host's outer policy
blocked the earlier OS sandbox qualification attempt; this adapter does not
retry or bypass that restriction. Actual qualification remains separately gated.

Limits cover command time, stdout/stderr, source scans, individual artifact bytes
and media duration. They do not implement a filesystem quota for all scratch
files written by a subprocess. The host still owns disk-pressure admission and
preservation policy, durable operation recovery and actual worker/provider wiring.

Hooks also expose `prepare_hook`, `start_hook`, and `complete_hook`: preparation
persists the attempt before any process starts, the short admitted start returns
an owned cancellation handle, and completion waits outside the control mutex.
Skipped hooks have no process handle. Failed or interrupted starts retain
`StartedUnknown` and cannot silently replay. The host must finalize the control
effect against current authority before admitting the following phase.

Trusted Git sequences use `Git::start_operation` with finite per-command and
whole-operation deadlines, bounded stdin/stdout/stderr, and an owned process
group cancellation handle. Every actual subprocess start passes the supplied
short authority gate; a fence between commands prevents the next command.
Ordinary disposable-process tests verify timeout, cancellation, and reaping;
these tests make no claim about OS sandbox qualification.

`prepare_execution_clone` copies committed local objects into an independently
owned exact-revision checkout; the operator's staged, unstaged and untracked work
is never input. Its retained receipt sits outside the worker repository. Reuse
requires matching identity and HEAD and preserves all dirty files and staging.
Independent PR reviews use `prepare_independent_review_clone` with a separate
workspace/repository/PR/head/base identity, without a fabricated Linear parent.
The review role still receives no repository execution or write permission.

`commit_execution_changes` accepts only an owned child directory and its exact
execution binding. It rejects ambient staging and changes outside both declared
scope and actual source grants, constructs a separate retained index, writes raw
binary/symlink/mode objects, and creates a single-parent signed-off commit before
advancing HEAD by comparison with the old revision. Source writers must be ended
before this host phase; subsequent checks need fresh admission for the new head.
Partial indexes and objects are retained on interruption. External Git filters,
configuration includes, sparse checkouts and Gitlinks are explicitly blocked by
the supported execution profile. SHA-1 is used only for Git object interoperability;
evidence authentication remains SHA-256/HMAC. The object/index mechanics follow
the official [Git index](https://git-scm.com/docs/git-update-index),
[raw object](https://git-scm.com/docs/git-hash-object) and
[commit object](https://git-scm.com/docs/git-commit-tree) interfaces.

Owned process groups are established from actual freshly spawned Child handles,
never restored numeric PIDs. The host observes terminal leaders with
`waitid(WNOWAIT)`, finishes their group while the leader remains unreaped, and only
then consumes the exit status. The nonconsuming observation follows the
[POSIX wait status contract](https://pubs.opengroup.org/onlinepubs/9699919799/functions/V2_chap02.html).
Group cleanup does not prove that a descendant could not escape into another
session/group. Actual execution-boundary qualification must establish that
containment separately; unexpected group identity or pipe retention remains
unknown and cannot release evidence gates.

The new-session escape is a concrete open implementation finding, not merely a
missing test. `process_group` now includes a finite owned fixture which creates a
new session, closes inherited streams, and writes its owned artifact after the
original group has completed. The current Seatbelt profile has no verified
mechanism preventing this escape. Native `OwnedCommand::wait` therefore returns
`process_tree_unverified` after group completion instead of producing passing
check evidence. `group_exited` is diagnostic only; the stronger `exited` capability
also refuses completion. Callers retain their registry entry, Sent receipt and
permit/hold. An actual descendant containment or tracking mechanism remains
required before native repository commands can complete safely. The deterministic
execution boundary continues to test the surrounding workflow without claiming
that missing native guarantee.

Native execution now requires `JournaledSandboxBoundary`, bound to the current
operation permit. Every actual subprocess persists Starting and an inherited
locked guard before spawn, captures its native child identity when available,
and records GroupExited only after owned group cleanup and reap. Recovery never
adopts the stored PID and still reports all-descendants-exited as false. Plain
`SandboxBoundary` rejects native starts without this journal.

`Git::from_pinned_executable` inspects tool bytes without launching a process;
`start_journaled_operation` performs its version probe under the same current
gate and journal. `GitExecutionScope` covers trusted metadata reads during check
preparation/completion and structured Git inspection. Each read registers a
cancellation handle within the spawn gate, waits outside ControlHost, and removes
that handle only after its owned process has exited. Preparation and model runs
use different immutable permits even when reusing one owned directory.

The observed Homebrew Git path has a group-writable ancestor and is rejected by
the strict executable ownership check. Tests use an exact-byte Git copy under
their own disposable protected directory, not a host installation. Native boot
identity remains unavailable in the outer development sandbox; journaled Git
fixtures explicitly inject a synthetic boot UUID while observing their actual
owned PID/start/path and Git effects. Neither fixture arrangement qualifies the
native production process-identity boundary.

Repository commands now fail the `process_tree_unverified` prerequisite before
native spawn. A known inability to prove descendant containment must not create
an avoidable permanent unknown operation. The prebuilt host cleanup guard uses
the same prerequisite and exact executable/profile/argument binding. Trusted
fixed Git metadata remains a separate journaled operation. A passing fixture
boundary does not remove this native production prerequisite.
