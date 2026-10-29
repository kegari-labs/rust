# Codex capability validation and later qualification

[SPEC 1.5](../SPEC.md) and [MILESTONES.md](../MILESTONES.md) replace the former
custom repository-executor qualification project. Codex app-server provides the
repository tools, sandbox and supported terminal lifecycle. Kegari qualifies its
integration with those capabilities, not a new execution backend or toolset catalog.

## Implementation boundary

The immediate work connects an ordinary admitted workflow to native Codex tools,
consumes structured results and deletes the replaced executor. Keep the pinned
Codex version initially and check the selected role's required capabilities.
Unsupported permissions, tool restrictions or observations cause specific
limitations. They do not authorize unsandboxed execution, a silent provider
upgrade, or restoring the custom launcher.

Independent PR review must remain unable to execute repository code; read-only
filesystem access alone is insufficient. Kegari retains workflow authority,
source-bound acceptance, provider mutations and release decisions. It must not
interpret project manifests, require environment/command specifications, or
recreate per-command resource inventories and process journals around Codex events.

## Focused validation while implementing

For the first migrated path, compile affected code and exercise one ordinary
production flow through simulated Codex transport and one missing/stale-result
refusal. Validate subsequent increments only enough to proceed. Preserve useful
assertions and remove tests that enforce the discarded architecture with a short
explanation. No new validation framework or broad qualification campaign is needed.

Implementation, focused validation and qualification are separate milestone
states. A permission-dependent demonstration blocks that demonstration and its
actual dependents, not unrelated source work. Operator-configured checks on target
projects remain required regardless of this development policy.

## Qualification boundary

Milestone 0.7 covers comprehensive regression, end-to-end/failure coverage and
separately authorized native, actual-agent, provider and human qualification.
Evidence must distinguish supported Codex permissions/tools/lifecycle, actual
session/result observations, and target-project validation or hosted CI. Bind it
to the tested source, build, assets and provider profile; skipped cases are not passes.
Historical command-launcher or resource-domain results do not qualify this integration.

Supported background work uses Codex lifecycle interfaces. Do not promise arbitrary
process adoption or service restart after app-server failure. Preserve unknown
outcomes and affected workspaces. Full descendant coverage remains a separate
future prerequisite to claiming qualification for arbitrary untrusted external
context; see [lifecycle limits](process-containment.md). It is not a requirement to
finish a custom repository launcher before implementing Codex-backed workflows.

The existing authenticated runtime qualification and handover mechanisms remain
application-owned. Their operator route belongs to milestone 0.5; actual
qualification and installation require separate authority. A build, configuration
check or documentation amendment does not install or activate a coordinator.
