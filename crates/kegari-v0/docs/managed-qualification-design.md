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
owned child/group, collectors and current process journal: allowed foreground and
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

The alpha.6 work implements finite check-owned service lifecycle alongside the
foreground supervisor; native qualification remains incomplete. The next package
is one closed macOS Rust/Cargo contract, not a general operator-profile issuer:

| Contract element | Required implementation and evidence |
| --- | --- |
| Tool inventory | Bind the supported target/OS, exact Cargo/rustc/rustdoc identities and the compiler, linker and archive tools required by the locked dependencies. Add clippy/rustfmt only with their own supported cases. Avoid ambient rustup resolution. |
| Offline inputs | Bind Cargo.lock and a complete dependency snapshot; prepare credential-free private Cargo configuration/cache, HOME and temporary state. Explicitly select compiler, linker, SDK/sysroot and dependency locations. |
| Resource identity | Bind bounded content manifests for toolchain libraries, compiler resources and SDK, rather than only directory paths/types. Distinguish ordinary readable inputs from specifically permitted executable mappings. |
| Generated executables | Keep build scripts, proc macros and generated test executables inside the authorized scratch/build root, with the original finite workload and resource bounds. |
| Runtime matching | Match the configured inventory to the shipped contract during managed-policy resolution, retain the contract/resource digest through preparation and the worker contract, and revalidate at command preflight and cleanup. Unknown tools or changed resources remain blockers. |
| Actual cases | Offline build/test, C dependency compilation/linking, build scripts/proc macros, generated tests and rustdoc; missing inputs, denied network/credentials/sibling paths, resource substitution, nonzero exit, timeout/cancellation, output limits and uncertain cleanup. Include an actual scoped Kegari build/test compatibility case. |

Alpha.8 adds optional bounded content manifests to managed read roots and keeps
executable mappings separately enumerated. Full SDK/library observations occur
outside serialized Control callbacks; admission verifies retained bindings. This
is a prerequisite, not the complete offline profile: a concurrent host update
can still invalidate a pre/post observation, and no immutable filesystem snapshot
is claimed. Legacy read roots still bind only path/type and cannot substitute for
the required toolchain content contract.

For the first offline profile, use a complete vendored dependency snapshot and
host-generated protected Cargo configuration, with writable cache/temp/build
state kept separate. Reject additional repository/ancestor Cargo configuration
and unsupported invocation overrides; a private Cargo home does not suppress
those inputs ([Cargo configuration](https://doc.rust-lang.org/stable/cargo/reference/config.html)).
Bind the complete rendered environment into launch identity. Route
doctest temporary output beneath the generated build root. Exact compiler,
linker, archive, SDK and toolchain identities remain required, and native cases
must establish their actual transitive compatibility.

Use a private prepared-launch object for that next package. It must bind the
original operation, selected profile, source/lock/vendor identities, protected
configuration, rendered argv/environment and original deadline. Both identity
hashing and spawn must consume the same rendered launch. Command-only input and
cache grants must stay separate from ordinary read/resource roots, because the
latter also enter Codex model permissions. Keep generated configuration outside
all writable scratch/cache parents; select the finite cache layout for the exact
Cargo version rather than assuming a stable universal Cargo-home layout.

The initial invocation grammar should cover built-in build/test/doc only, with
mandatory frozen operation and bounded jobs, rejecting external subcommands,
toolchain selectors, arbitrary configuration/compiler arguments, runners and
output relocation. Existing named checks also require a fresh result artifact.
The Cargo adapter must retain a truthful host-written invocation result derived
from the actual joined command outcome and prepared launch digest before result
acceptance; successful exit alone cannot manufacture per-test facts or counts.
Neither preparation nor missing vendor inputs may trigger automatic downloads
or native qualification in the running application.

The current command environment supplies PATH, HOME, TMPDIR, LANG and optional
CARGO_TARGET_DIR; that is not yet a complete offline Rust toolchain environment. The repository's
locked dependencies include `ring` and `cc`, so shell-only execution cannot
establish compatibility. Do not grant executable mapping to every read root as a
shortcut for supporting toolchain libraries.

Implement the parser/matcher, resource binding, private offline preparation,
environment/profile rendering and deterministic refusal cases first. Author the
native cases against that production path, then request separate bounded approval.
Neither these declarations nor NQ-02C's three passing foreground cases establish
a Rust pass; NQ-02A/B failures remain historical evidence.
Preserve the existing managed-profile prerequisite until its actual evidence
supports enabling the closed profile.

The original sequencing remains:

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
