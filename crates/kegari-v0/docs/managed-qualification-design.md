# Managed-profile qualification direction

This is a proposed implementation direction, not a qualification result. It was
traced against commit `354b27ad63672d8636c327fbe910bcaec5d30353`. No native run,
installation, credential access or service activation is authorized here.

## Chosen minimum

Use a closed, versioned supported managed-profile manifest shipped with the
runtime, explicit bounded native validation during release qualification, and
reviewed initial-install/whole-runtime handover gates. A new runtime qualification
command, issuer or proof registry is optional deferred architecture, not a SPEC
requirement or a blocker on the current implementation.

SPEC §1.2 requires the supported tools/services and lifecycle assumptions to be
identified. §§1.3 and 12.3 require qualification for that supported profile before
coordinating the lasting build. §12.4 requires explicit live-run authority and
rerunning affected checks when supported profiles change. None mandates an
online per-profile qualification service or a particular proof registry.

## Support contract and configuration

The shipped manifest should identify its schema/profile revision, foreground
lifecycle assumptions, supported tool versions/byte digests, OS/architecture and
runtime compatibility, permitted generated-build mode and the finite native case
manifest. Qualify actual transitive compiler/linker/interpreter use where claimed.
Read-root paths alone do not establish unchanged SDK/library content: relevant
input versions/identities need bounded evidence, or an explicit narrower claim.

Current `managed_execution` configuration accepts an explicit tool inventory and
`BoundManagedExecution` revalidates its observed paths/bytes. That prevents silent
substitution; it does not make every operator-declared tool supported or qualified.
The next integration must check the configured inventory and isolation shape
against the shipped closed support contract or equivalently precise retained
qualification evidence. Unknown tools, incompatible versions, unsupported grants
or absent qualification remain specific blockers. Do not add `qualified = true`,
treat a parsed report as native authority, or infer support from one passing shell
command. Changing the relevant tool/profile/runtime/OS inputs requires affected
validation and review; a configuration edit cannot assert that this occurred.

This profile covers cooperative foreground tools and their declared lifecycle.
It does not prove arbitrary repository scripts cannot detach or that every
possible tool invocation terminates. Known or suspected lifecycle uncertainty
still blocks affected completion, cleanup and handover. Full descendant coverage
remains required for broader untrusted-external-context qualification, including
public PR material. Controlled E07 obligations and independent filesystem,
network, credential and role isolation remain unchanged.

## Qualification and operational use

Deterministic component tests prove only their exercised behavior. Separately
approved native cases must exercise the actual production launcher, sandbox,
owned child/group, collectors and schema-2 journal: allowed foreground and
transitive execution, unsuccessful exit, timeout/cancellation, inherited-handle
uncertainty, denied resources/executables and generated-scratch rules as selected.
Bind results to the exact runtime binary/build, host/OS, support and case
manifests, tool inventory and retained observations. Missing, failed or skipped
cases must not become a successful aggregate. Preserve uncertain ownership and
failure evidence; no automatic rerun, budget reset or cleanup is implied.

Include this evidence in the runtime's integration/review artifacts. It does not
replace actual-agent evaluations, provider integration or human calibration.
`housekeeping::RuntimeQualification` remains the broader signed handover record;
its verifier is not an existing per-command execution grant.

Initial installation currently has operational qualification/review requirements,
not an automatic full-qualification verifier in `platform::install_service`.
SPEC requires qualification before lasting-build use, but does not prescribe that
particular automatic verifier. Keep that implementation limit explicit rather
than claiming an enforced installation gate. Existing handover checks, immutable
binary requirements, explicit service/lifecycle actions and operational authority
remain in force. Building the executable or running config check grants none of
those actions and must never launch native qualification as a side effect.

## Next coherent implementation package

1. Finish actual owned-command completion and cancellation first: original child
   reap, collector completion and exact guard/journal closure, with uncertainty
   retained. Keep the current qualification prerequisite closed during this work.
2. Add the shipped support/case manifest and validate configured policy against
   its closed contract at runtime preparation and command preflight. Preserve
   omission compatibility, role restrictions, current-authority checks, cleanup
   revalidation and declared tool/path identity binding.
3. Add bounded production-path native fixtures plus deterministic refusal,
   substitution, timeout and persistence-failure regressions. Execute native
   cases only under separate approval, then retain exact release evidence and
   operator qualification instructions before enabling the supported profile.

The prerequisite diagnostic should describe missing or incompatible supported
profile qualification evidence, not a missing runtime qualification API. For
example: `managed foreground profile lacks current supported-profile
qualification evidence`, with the actual policy digest in structured details.
The earlier authenticated issuer/registry proposal is deferred; it should be
revisited only if reusable operator-managed profiles require that additional
product capability.
