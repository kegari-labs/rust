# Codex lifecycle and retained host-process ownership

[SPEC 1.5](../SPEC.md) assigns repository execution, sandbox enforcement and
supported terminal management to Codex app-server. Kegari keeps ownership of its
own app-server, deterministic Git and control/service processes. The former
custom repository-executor containment project is superseded and must not drive
the new implementation queue.

## Supported lifecycle boundary

Kegari admits bounded operations/sessions with supported Codex permissions. It
records available command, file and terminal observations and distinguishes
completion, requested cancellation and unknown outcomes. It fences its own
mutations before requesting supported Codex interruption and cleanup.

A process-group exit, closed stream or app-server exit does not prove complete
descendant termination. Kegari does not promise to discover, adopt or automatically
restart arbitrary detached processes after app-server failure. Preserve uncertain
effects and files; block affected result acceptance, cleanup, conflicting work and
handover until reconciled. Required writers must finish before final artifacts
are accepted. Filesystem, network, credential and role restrictions remain required.

Independent PR review cannot execute repository code. Read-only filesystem access
alone is not a no-execution guarantee. Unsupported provider capabilities produce
specific role/workflow limitations, not a competing executor or unsandboxed fallback.

## Retained host mechanisms

Actual child handles, identity-safe observation/signalling, bounded termination,
joined output and protected journals remain useful for Kegari's own processes.
Saved PIDs are references, not reconstructed ownership or signal authority.
The shared spawn/retirement interlock and registered descriptor handling must
remain intact for app-server, Git and control operations. Known inherited holders,
missing observations and persistence failures preserve uncertainty.

These host mechanisms do not create a requirement to journal or independently
admit every command launched internally by Codex. Session and result boundaries
retain the applicable workflow authorization and source checks.

<a id="current-automatic-command-domain"></a>

## Migration status

The old repository command domain, rendering/sandbox/resource inventory and
blanket custom-executor qualification gate have been removed from the ordinary
child path. Native Codex integration checks the selected version, permissions
and required capabilities. Remaining integration and qualification limitations
are tracked in [PROGRESS.md](../PROGRESS.md).
Old records keep their original meaning; no historical execution proof becomes
Codex evidence. The preserved anchor above serves older evidence links only.

## Future untrusted-context qualification

The managed-v0 profile can be implemented without promising arbitrary-descendant
coverage. Full descendant lifecycle coverage remains a desired future capability
and a prerequisite to claiming qualification for arbitrary untrusted external
context, including public PR material. Controlled adversarial fixtures do not
establish that broader guarantee. This restriction does not mandate a particular
platform mechanism or block unrelated source implementation.

The user reported submitting an Apple Endpoint Security entitlement application
on 2026-10-26; approval has not been reported. Revisit its outcome and the broader
lifecycle requirement before the v1 production-readiness decision. The application
is not approval for signing, installation, activation or native testing, and is
not a prerequisite to the current Codex integration.

Earlier research at checkpoint `a217df8` concerned a stronger descendant contract.
The finite escape fixture established that process-group exit cannot establish
complete descendant exit. Subsequent results remain in the
[owned-group report](../validation/OWNED-GROUP-LIFECYCLE.md),
[native identity report](../validation/NATIVE-IDENTITY.md) and
[native command report](../validation/NATIVE-MANAGED-COMMAND.md), with their original
source identities and limitations. They are historical evidence, not an active
custom-launcher work plan. [Milestones](../MILESTONES.md) govern development and
later authorized qualification.
