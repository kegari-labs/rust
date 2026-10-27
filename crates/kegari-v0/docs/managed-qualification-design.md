# Generic managed execution qualification

SPEC 1.4 and [MILESTONES.md](../MILESTONES.md) replace the earlier closed-toolset
qualification design. This document defines the remaining backend evidence;
it is not a qualification result or authority for native/live operations.
Historical results retain their original source, specification and limitations.

## Qualification boundary

Qualify the OS/provider execution backend and its enforced capabilities, not a
catalog of target languages, build systems or tool versions. Kegari must not
parse target manifests, rewrite ecosystem commands, supply a dedicated package
manager environment, or require users to author executable/digest inventories.
An unfamiliar executable is not itself an unsupported-workload result.

An agent discovers commands, relevant environment facts and validation obligations
from the authorized repository and task. The host binds actual argv, cwd,
non-secret environment observations, source/baseline, authority, deadlines and
resource scope before execution. Observations are automatically produced evidence;
they cannot become a second user-maintained environment specification. Available
resources do not grant permission to use credentials, networks or privileged
host capabilities. Specific missing authority/capability remains a blocker.

Generated programs, scripts and transitive tools use the same generic execution
boundary. Keep separate source writes, private outputs and explicit test resources.
Directory names, successful exit, agent assertions and a declaration that a tool
will remain in the foreground are not evidence of enforced isolation or closure.
Command results record actual exit/signal/timeout, bounded output and artifact
identities. Agent interpretations of project reports require provenance and review;
Kegari does not infer per-test counts or correctness through toolset parsers.

## Lifecycle and retained guarantees

The managed scope includes foreground work and explicitly registered finite
services owned by the admitted operation. Preserve original process handles,
identity-safe signalling, bounded cancellation, collected output and journal
closure. Known or suspected ownership/writer uncertainty blocks the affected
completion, cleanup, replacement work and handover.

Unregistered detached descendants remain outside the shutdown guarantee. Group
exit and EOF do not prove their termination. Full descendant coverage is still
required before qualification for arbitrary untrusted external context, including
public PR material. Controlled V0-E07 fixtures, independent role restrictions,
filesystem/network isolation and credential separation remain mandatory.

## Evidence needed for 0.2.0

M02-1 through M02-4 remove/rework the incompatible design and exercise the generic
production paths with deterministic tests. M02-5 then requires separately approved,
pinned and finite native cases covering:

- foreground, transitive and generated execution through the actual launcher;
- registered service start, readiness, bounded lifetime, cancellation and closure;
- unsuccessful exit, timeout, output bounds and interrupted/uncertain outcomes;
- allowed resources and denied source, sibling, control, credential and network access;
- relevant identity changes and stale/substituted invocation/result rejection.

Bind evidence to the actual source/build, backend/host version, case inputs,
observed environment and process records. Representative tools are test data,
not runtime admission catalog entries. Preserve failures and unknowns. Missing
or skipped cases remain incomplete; never promote a historical fixture pass to
qualification for a changed backend. Existing NQ-02A/B/C approvals are consumed,
and NQ-02C covers only three fixed alpha.7 foreground cases.

Full repeated actual-agent evaluations, real integrations and human calibration
remain distinct obligations, completed by milestone 0.7.0. Necessary evidence for
a claim made at 0.2.0 cannot be deferred merely to publish that version.

## Implementation checkpoint

The Cargo-specific parser, launch preparation and result adapter have been
removed. The executable catalog is replaced by the host-produced schema-5 domain
described in [configuration](configuration.md#generic-execution-migration): fixed
OS resources, owned execution paths and already-admitted generic resources.
Generic worker argv/cwd invocation now records actual commands through the
original authority, deadline and ownership gates. Its deterministic component
evidence is recorded in [the local command report](../validation/LOCAL-GENERIC-COMMANDS.md).
Complete discovery and validation-obligation integration remain unfinished;
named workflow checks remain a separate transitional validation route, and
generic command results cannot satisfy it. External host directories do not gain access
merely because a command needs them. The exact-build retained policy also refuses
incompatible cleanup rather than migrating evidence or deleting owned state.

The production qualification refusal remains closed while this backend's native
evidence is incomplete. Do not replace it with an unconfined fallback, a
`qualified = true` flag or a configuration assertion. Earlier catalog/schema
results remain historical evidence for their original implementations.

`housekeeping::RuntimeQualification` is the broader authenticated handover record,
not a per-command grant. Complete its route under milestone 0.5.0 while preserving
explicit installation/handover authority, immutable binary/assets and current
format compatibility. A build or `config check` does not install, activate or
qualify a coordinator, launch a test, or authorize the lasting implementation.
