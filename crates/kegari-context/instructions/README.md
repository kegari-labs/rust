# Runtime instruction and repository workflow contract

[SPEC 1.5](../../kegari-v0/SPEC.md) makes Codex app-server the repository execution
layer. Kegari supplies task context, role/scope permissions, workflow obligations
and acceptance rules. It does not supply a competing repository command/edit
harness or require environment inventories and predefined command catalogs.

## Current assets and identity

The executable currently embeds six role/authority assets, the generic workflow
and their checked SHA-256 catalog. `instructions::assemble` validates the catalog,
role/profile/context bindings, selects applicable guidance and produces the prompt
and provenance manifest. Installed assets do not depend on a mutable checkout.

Asset identity `9` includes typed blocked-child reporting on the Codex-owned
execution path. The runtime
assets direct permitted repository work through native Codex tools. Kegari's
narrow tools supply context, obligation proposals, outcome proposals and provider
feedback; `validation_record` references an actual observed command item rather
than launching it. The previous `command_run`, `source_write`, `validation_run`
and resource-inventory recipes have been removed from runtime instructions.

The asset catalog records their exact hashes. The installed generic workflow
uses version `2`; a recorded version `1` workflow cannot silently become the new
execution contract. The generic installed workflow and
exported starter share the same bytes. This README is explanatory; the actual
runtime assets and callers are the implementation contract. Local assembly checks
do not establish successful live execution or qualification.

Historical instruction manifests and execution evidence retain their original
identities. Unsupported recovery must refuse without converting old proof into
Codex execution evidence. A rebuilt candidate is distinct from authorized
installation, qualification or coordinator handover.

## Context and workflow responsibilities

Authority, role, continuation and optional repository workflow guidance compose
with the admitted operation's task/context and supported Codex permissions.
Implementation, testing and documentation remain scoped child responsibilities;
parent coordination does not gain implementation rights. Independent PR review
cannot execute repository code; read-only filesystem access alone does not prove
that restriction.

Keep applicable repository guidance, bounded rendering, secret exclusion,
source/correction identities and instruction-resume compatibility. Distinguish
Kegari-supplied inputs from provider-confirmed context and unavailable internal
context. Repository content cannot broaden authority or select accounts.

The [starter workflow](../examples/WORKFLOW.md) is optional. Omitting a selected
workflow uses installed generic guidance; an invalid explicit selection refuses.
Export uses create-new and does not overwrite repository files. A file workflow
retains its path/digest; an installed default retains its embedded identity.

Codex retrieves project instructions, documentation, configuration and scripts to
derive appropriate in-scope commands, including for unfamiliar or empty projects.
Kegari does not interpret ecosystem manifests or require operators to prepare
command lists, executable catalogs or environment packages. Missing material
intent or unsupported permissions remain specific blockers.

Required project checks, hosted CI, independent review, human acceptance and
release authority remain separate gates. Structured Codex results must retain
available operation/thread/turn/item/source associations. Model summaries, exit
zero and missing results cannot establish passing validation. Corrections and
continuation preserve required obligations, failures and uncertain effects.

## Maintaining instructions

Use minimum sufficient focused checks during implementation, as required by the
[developer policy](../../../docs/development.md#feature-first-development-policy).
The initial migration needs one ordinary production flow through simulated Codex
transport and a missing/stale-result refusal. Do not create a new instruction or
evaluation framework. Broader regression and actual-agent/human qualification
follow working product features in the [milestones](../../kegari-v0/MILESTONES.md).

Runtime asset changes require explicit identities and catalog consistency.
Qualification claims require their corresponding authorized evidence; a new
prompt, simulated response or successful assembly test is not live operation.
