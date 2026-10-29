# Scoped worker orchestration

[SPEC 1.5](../SPEC.md) defines the worker as an orchestrator of Codex app-server.
Codex supplies repository inspection, editing, command execution, sandboxing and
supported terminal management. Kegari supplies the admitted task, role, context,
account and permissions, then interprets actual results through workflow gates.

**Implementation status:** `ScopedWorker`, production context assembly, workflow
and provider brokers, request gating and session recovery already exist. The
ordinary child path now uses native Codex repository tools; the competing
read/edit/command/resource tools and executor have been removed. Runtime
instruction migration began with version 8; the current catalog is version 9,
including honest blocked-child outcomes. Remaining integration
and deferred qualification are recorded in [PROGRESS.md](../PROGRESS.md).

## Authority and tool boundary

The host prepares an immutable contract from assembled instructions, actual
scope, current authority, account/profile and exact source identity. Admission
binds that contract to a durable operation and its finite budget. A caller-created
contract or serialized record cannot manufacture a permit or transport principal.

Kegari retains narrow tools for supplied workflow context, outcome proposals and
authorized provider actions. Provider writes still require current authority,
exact target/source bindings and recoverable receipts. Operator administration,
arbitrary remote mutations and release authority do not become agent tools.

Repository reads, edits, commands and hooks use supported Codex interfaces within
the selected role's permissions. Kegari must not provide competing execution
adapters or require an environment/resource inventory for each command. Independent
PR review must not execute repository code; read-only filesystem permissions alone
are insufficient. Unsupported role/capability combinations remain specific
blockers, without a custom or unsandboxed fallback.

Structured command/file events retain their operation, thread, turn, item and
source associations and available results. They are observations, not proof of
host-owned processes or complete effect coverage. The backend retains required
validation obligations, failed attempts, review findings and acceptance rules.
A completion message, an exit status or an outcome proposal does not independently
satisfy them. Hosted CI remains required where configured.

`context_read` with `key: execution_results` lists completed native results in
bounded pages. Follow `next_offset` with the returned `results_digest`; a changed
inventory refuses a stale cursor. An entry's `output_cursor` selects its complete
redacted command output using byte `offset` and `next_offset`. Output pages bind
the exact thread, turn, item, observed source and full-output digest, preserving
UTF-8 boundaries. Redaction occurs before slicing. Reading pages does not accept
a check or make historical output evidence of the current source.

## Requests, corrections and outcomes

The production request gate checks current authority and persists sent protocol
effects before bounded writes. Responses and dynamic workflow/provider callbacks
run outside the control mutex. A cancelled or timed-out write can leave an
uncertain effect; its receipt and target hold survive. Unknown outcomes are
reconciled rather than blindly repeated.

A correction does not silently replace the original authority, profile or scope.
The host classifies the current correction, retains its exact input and verified
transition, and associates steering/processing with the actual turn. A successful
steer acknowledgement alone is not consumption of the correction. Pending
classification fences dependent Kegari effects. Old checks remain attached to their
original work generation and cannot pass the corrected outcome without valid
compatibility/carry evidence.

Provider policy or verification interventions remain durable holds. Later history
mismatch, timeout, process exit or a replacement account/thread cannot erase them.
Unsupported tools/input requests receive explicit outcomes; they do not grant
broader permissions or an interactive unattended exception.

## Owned sessions and recovery

[`worker-run.md`](worker-run.md) documents app-server launch, bounded protocol
writes and owned host cleanup. These mechanics remain distinct from Codex-internal
commands. On interruption, fence Kegari mutations first, request supported Codex
cancellation/terminal cleanup and retain uncertain effects and workspaces. Neither
EOF nor app-server exit proves every repository writer or detached descendant
stopped.

Protected retained sessions combine the actual journal, original receipt,
instruction/source binding and joined host completion. Read-only protocol data
cannot reconstruct those capabilities. Metadata recovery verifies account,
permissions, exact submitted inputs and complete supported history before any
resumed session is admitted. The final admission rechecks current work, source,
profile and remaining budgets; a fresh provider readback prevents stale recovery
observations from being reused.

The current protocol recovery supports only its declared compatible histories.
Cold workflow reconstruction, broader continuation and uncertain-result
reconciliation remain in the milestone queue. Repeated recovery preserves ordered
prior lineage, original attempt caps and usage attribution; missing or incompatible
records stay ineligible rather than gaining inferred authority.

## Focused development checks

Use the smallest existing worker, protocol or recovery fixture for the changed
behavior. Initial native-tool migration needs one simulated ordinary production
flow and one missing/stale-result refusal. Preserve applicable authorization,
correction, uncertainty and source-binding assertions; remove tests whose sole
purpose is enforcing the discarded executor. Full regression and authorized
actual-agent/provider qualification belong to milestone 0.7.
