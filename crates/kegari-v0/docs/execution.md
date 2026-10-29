# Codex repository execution and Kegari evidence

[SPEC 1.5](../SPEC.md) defines the intended boundary: Codex app-server owns
repository inspection, file editing, shell execution, sandbox enforcement and
supported terminal management. Kegari selects and admits work, supplies context
and supported role permissions, observes results and decides workflow progression.

The ordinary admitted child path now uses native Codex tools and the custom
repository executor has been removed. Remaining caller integration and source
compatibility work are tracked in [MILESTONES.md](../MILESTONES.md) and
[PROGRESS.md](../PROGRESS.md). Focused simulated evidence does not establish
live/native qualification.

## Host integration

Kegari prepares an owned source directory and admits a bounded operation/session
using current work, correction, role, account and source identities. The selected
Codex profile supplies native repository tools and supported permissions. An
independent PR review cannot execute repository code; a read-only filesystem
profile alone does not establish that restriction. Missing required capabilities
produce a specific blocker, never a custom-executor or unsandboxed fallback.

Structured command/file observations retain their operation, thread, turn, item
and source associations. Available completion, output and artifact facts remain
separate from model interpretations. Kegari must not reconstruct executable or
environment inventories, generate repository sandbox rules, or require its own
admission and process journal for every Codex-internal command.

Kegari retains narrow machinery for launching and supervising its own app-server,
Git and control processes. Deterministic Git source preparation, candidate
assembly and privileged publication remain application operations with their
existing ownership and authority. See [host metadata operations](host-process.md)
and [process recovery](process-recovery.md).

## Hooks and durable attempts

Explicitly selected repository hooks use the supported Codex execution interface
and the existing workflow timing. Preparation failure prevents dependent work;
failed or uncertain cleanup hooks preserve the directory. Hook observations and
workflow attempts remain recoverable facts, not permission to replay an unknown
effect. No standalone Kegari shell runner is retained as a hook fallback.

On interruption, Kegari fences its mutations before requesting supported Codex
cancellation and terminal cleanup. Requested cancellation, confirmed completion
and uncertainty remain distinct. App-server exit or stream EOF does not prove
that every repository descendant stopped. Preserve affected files and block
conflicting work or acceptance until the relevant uncertainty is reconciled.

## Required checks and source-bound results

Agents discover appropriate commands from authorized repository guidance and
the task. Users do not need command catalogs or environment manifests. Kegari
retains required-check obligations, prior failures and reasons for corrections;
it does not parse target-toolset manifests or infer test counts from exit zero.

Result acceptance requires actual available execution results and the matching
work, correction and source/candidate identity. Missing, stale or substituted
results cannot satisfy a check. A completion message does not replace evidence.
Hosted CI, review, human acceptance and release decisions remain separate gates.
Source changes require fresh applicable evidence rather than relabeling old
results. Current named-check/custom-command mechanisms must migrate to this
boundary instead of becoming a second execution path.

## Artifacts and visual evidence

Protected artifact capture, storage, provenance and acceptance remain Kegari
responsibilities. Required writers must finish before their output is final;
retained artifacts cannot be overwritten merely because a rerun needs the path.
An arbitrary existing recording is not proof of the current candidate's behavior.
Visual capture, validation, publication and authenticated retrieval must retain
actual source/candidate and artifact identities through the supported Codex and
provider interfaces. Unsupported capabilities remain explicit limitations.

## Migration and validation

Remove replaced command rendering, sandbox generation, managed environment and
resource inventories, execution/edit adapters and command-level bookkeeping,
along with obsolete configuration, instructions, tests and dependencies. Preserve
shared host-process machinery and valid application evidence/acceptance rules.
Do not keep the replaced executor behind flags or compatibility wrappers.

Old records remain evidence for their original source and contract. They must
not silently become Codex execution evidence. Change affected persisted and
instruction identities explicitly and refuse unsupported recovery while
preserving the original data.

The initial increment compiles affected code and exercises one ordinary production
flow through simulated Codex transport plus a missing/stale-result refusal.
Further development uses the smallest relevant check. Full regression and
separately authorized live qualification belong to milestone 0.7; historical
custom-executor tests do not establish the new path's operation.
