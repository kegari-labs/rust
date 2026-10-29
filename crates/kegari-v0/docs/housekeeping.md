# Retention, diagnostics and explicit handover

[SPEC 1.5](../SPEC.md) retains Kegari's preservation, diagnostic and handover
responsibilities while repository execution belongs to Codex app-server. Explicit
cleanup hooks use its bounded command interface, with a sealed cleanup decision
and owned app-server lifetime. This source implementation is not live qualification.

Preservation is the default. `Housekeeping` never removes an execution directory
because a tracker state changed, an operation exited, a release succeeded or a
disk-space check failed. Completed receipts, evidence and qualification records
remain intact; automatic time-based deletion is not a workflow shortcut.

## Host integration

`LogSink` accepts bounded typed events with runtime/work/operation/session and
revision identities. Known secrets and authorization fields are redacted; full
prompts and arbitrary provider payloads are not log messages. Rotation is bounded,
and sink failure remains visible through `LogHealth` without authorizing a failed
receipt write or crashing otherwise safe scheduling.

Protected execution-directory registrations retain actual ownership and workflow
identities. A resumed operation joins a directory family only through the existing
authorized recovery path; a marker or serialized path cannot create authority.
Legacy records remain readable for preservation without becoming permission to
reuse a directory or execute work. Codex session, writer and terminal observations
must replace the discarded command/hook runner's lifecycle assumptions.

Cleanup requires a complete, verified retained copy and a current plan bound to
the actual directory, source, receipts, provider references and workflow. Dirty or
unpublished files, missing retained material, live owned processes, outstanding
writers, uncertain effects and unknown hook results prevent removal. Source and
retention inventories used to verify safe deletion are preservation checks, not
an execution environment catalog or per-Codex-command resource binding.

Applying a `CleanupPlan` requires its unchanged token and the authenticated
operator's current authority. Admission records the effect and hold before work;
provider/Git reads and filesystem traversal do not hold the control mutex. Each
unlink checks the applicable fence and actual file identity. Changed bytes or
modes stop deletion. A retained-copy tombstone and surviving files remain after
interruption; an unknown result is not blindly retried or marked successful.

Kegari's deterministic cleanup Git reads retain sealed host authority, bounded
commands, journals and original child handles. They do not create a worker lease
or expose arbitrary execution to an agent. Explicit repository removal hooks use
supported Codex execution with the same reviewed plan, timing and preservation
gates. Missing capability, failure or uncertainty preserves files. There is no
custom shell fallback and no requirement to qualify the discarded launcher first.

After cleanup, `retained_directory` verifies retained content before recovery uses
it. Background cleanup remains bound to the original authenticated command and
receipt. Neither an app-server exit nor a closed stream proves that all repository
writers stopped; only available supported observations may resolve that state.

## Handover

`__handover-info` provides state-free build, persisted-format and embedded-asset
metadata. `inspect_handover` examines an explicitly selected immutable replacement
under bounded time/output, then checks the registered owner, canonical locks,
STOPPED intent, supervisor-stop marker and absence of unresolved ownership/effects.
Unsupported schemas or changed metadata preserve source state.

The existing authenticated runtime qualification record binds exact replacement
bytes, metadata, instruction digests and retained evidence. Its signature proves
authenticity, not scientific validity or human calibration. Fixture signatures
and historical custom-executor passes cannot qualify the new Codex integration.
The local `service inspect-candidate`, `service record-qualification` and
`service handover` commands expose these existing mechanisms. Candidate inspection
provides exact identities without qualification; explicit human review records
are signed with the existing runtime key. Recording and replacement require the
stopped-host checks. Applying uses the reviewed token and preserves exact request
replay even after successful owner replacement. See the
[operator route](../../../docs/cli.md#candidate-inspection-and-handover).
Comprehensive authorized qualification belongs to 0.7 under
[MILESTONES.md](../MILESTONES.md).

`apply_handover` requires the current operator's configure/recover authority and
unchanged reviewed plan. It preserves prior registration and compatible state,
records the effect, replaces the fenced service registration and advances owner
generation. It does not start the service or automatically grant scheduling
permission. Interrupted replacement requires reconciliation. Building a new
candidate never replaces the live binary or activates its assets.

## Evidence and limits

Existing housekeeping fixtures exercise real disposable Git/filesystem/control
operations with synthetic provider, identity and qualification inputs where
specified. Their retained reports describe their original source; they are not
live qualification. Changed Codex integration receives the smallest relevant
focused check during implementation. Broad regression and actual-provider,
actual-agent, native and human qualification are deferred to the dedicated phase.

## Current session-reuse scope

Host Git checks and protected directory-family records establish the retained
source and receipt lineage. They remain distinct from Codex session recovery and
execution authority. Compatible session continuation reconciles actual provider
state and current instruction/work bindings before a new bounded admission.
Unknown effects, changed source or unavailable supported observations block the
affected reuse; they do not authorize adopting old PIDs or rerunning commands.

Current source recovery has incomplete broader/cold workflow reconstruction, as
recorded in the milestones. The migration must preserve valid source, artifact
and recovery checks without carrying forward custom environment/resource-domain
requirements or treating historical process proofs as Codex execution evidence.
