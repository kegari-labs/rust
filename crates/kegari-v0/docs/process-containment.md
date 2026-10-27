# Process containment: managed workload and future descendant coverage

Current entitlement status, reported by the user on 2026-10-26: the user has
submitted an application to Apple for the Endpoint Security entitlement.
Approval has not been reported. Revisit Apple's response and the full descendant
coverage approach before the v1 / 1.0.0 production-readiness decision. This is a
future design/qualification follow-up, not a blocker on the agreed managed v0
profile or approval for signing, installation, activation or other host/live
operations. Full coverage remains a prerequisite to untrusted-context
qualification whenever that is pursued; the pre-v1 revisit does not defer that
gate for earlier public-PR processing.

On 2026-10-26 the user selected the managed-workload contract recorded in
[SPEC §1.2](../SPEC.md#12-deliverable-and-exclusions), retained and clarified by
version 1.4. The managed-workload contract covers a qualified generic backend
for foreground work within its observed lifecycle, plus explicitly
registered managed services with verified ownership and bounded stop/exit handling.
Actual tools and environment identities are observed per operation; a shipped
toolset catalog or user-authored executable inventory is not an admission prerequisite.
Registration does not grant additional execution, filesystem, network, credential
or role authority. Required writers must finish before their artifacts are
accepted as final.

Escaped or unregistered descendants are outside this shutdown guarantee. A tool
can violate a foreground-only policy by detaching; the policy does not technically
prevent that behavior or promise to discover every escape. The supported profile
must state its enforced backend capabilities and lifecycle assumptions, retaining
actual operation/service observations. Known or suspected
ownership/exit uncertainty still blocks affected completion, cleanup, replacement
work and handover. Independent resource isolation remains required.

The managed-workload v0 profile can be completed and qualified without an
arbitrary-descendant guarantee. Full descendant lifecycle coverage remains a
desired future capability and a prerequisite to claiming qualification for
processing untrusted external context, including public PR material. Controlled
adversarial E07 implementation and evaluations remain required; their finite
results do not establish that broader qualification. Independent PR review stays
read/feedback-only and gains no repository execution permission.

The feasibility research below records the earlier, stronger descendant contract
at checkpoint `a217df8`. Endpoint Security was investigated as a candidate, not
selected as an unconditional prerequisite. Neither this scope amendment nor that
research qualifies an implementation or automatically removes a code-level
refusal.

## Why the existing process group is insufficient

A shell or test runner can fork a child, the child can create another session
or process group and close inherited output streams, and the parent can exit.
The child may still write into the operation's allowed directory. The existing
finite `tests/process_group.rs` regression demonstrates exactly that late write.
It establishes a process-group limitation, not a filesystem-sandbox escape.

`src/isolation.rs::repository_command_prerequisite` currently rejects repository
commands without an actual host-produced command domain with `process_tree_unverified`.
A bound schema-5 domain still returns `managed_profile_unqualified`;
it cannot manufacture the missing native qualification. Successful group cleanup
and EOF cannot establish
arbitrary-descendant closure. PID enumeration alone also cannot establish complete
membership or safely signal a process whose numeric PID has been reused.

| Feature | Required lifecycle evidence for the supported managed workload |
| --- | --- |
| Workflow hooks and local/integrated checks | Execute under the operation's role, then establish actual completion before accepting results. |
| Codex app-server and Git subprocesses | Preserve actual ownership, bounded termination and completion observations through provider/Git cancellation handles. A group result does not prove arbitrary-descendant closure. |
| Pause, stop, timeout and cancellation | Preserve graceful drain; fence mutation authority before forced termination; distinguish requested cancellation from verified exit. |
| Restart and recovery | Preserve uncertain ownership and prevent duplicate replacement work; persisted PIDs, group-exit records and empty memory do not recreate live authority. |
| Candidate/evidence sealing and delivery gates | Settle every declared managed writer before treating artifacts as final; known or suspected outstanding writers block sealing. Remote CI/deployment outcomes require separate provider evidence. |
| Cleanup and handover | Establish supported managed-workload shutdown, including registered services, before removing owned directories or replacing the coordinator; retain every known uncertainty. |
| Actual-agent evaluations | Use the production lifecycle boundary and retain honest interrupted/unknown outcomes. |

Compilation, help/version output, configuration parsing and deterministic
fixtures can proceed while native qualification is incomplete. Completing those
checks does not qualify a managed profile or a broader context profile by itself.

## Current automatic command domain

The host derives the schema-5 domain from fixed OS backend resources, the actual
owned repository and scratch directories, and already-admitted resource grants.
Executable names, expected hashes and mapping lists are not configuration inputs.
Generated and unfamiliar programs can be resolved inside that domain without a
catalog entry; canonical targets outside it remain refused. External host tool
directories need actual generic resource authority. Discovery does not add home,
credential, network or source-write access. Coordination and independent review
receive no repository command domain.

The original preparation binds the backend policy before hooks; retained model
and cleanup isolation bind the actual domain and observed resource identities.
Immutable read resources are captured with bounded no-follow reads outside
Control. Links contribute their text, not an assertion about target bytes.
Later serialized gates check retained structure and root identity; full resource
recapture remains a separate outside-gate check. This is not an atomic filesystem
snapshot or an exhaustive transitive dependency inventory.

Legacy catalog/configuration fields and managed schemas 1–4 are refused. Schema 5
also binds the exact build's policy: a different build cannot reuse it merely
because it accepts schema 5. Retained cleanup therefore refuses incompatible
bindings instead of replacing their identities, migrating them, or deleting
owned state through a fallback. Current native qualification is still missing;
arbitrary worker-command invocation and the complete discovery/validation flow
also remain unfinished. These code paths do not establish first-run readiness
or qualification for public PRs.

## Guard inheritance during concurrent launches

Each managed launch retains its original locked guard descriptor. A concurrent
host fork must not accidentally prolong another operation's guard until the new
program starts. Spawn-return alone is insufficient: Darwin closes close-on-exec
descriptors individually, so the spawn error pipe can reach EOF before another
inherited descriptor closes. See the pinned [XNU descriptor-close loop](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_descrip.c#L783-L820).

The current correction registers the original guard descriptors under the host
spawn/retirement boundary. Before exec, a child closes only those registered
descriptors that remain close-on-exec, preserving its explicitly selected guard.
Guard lifetime and descriptor identity constrain callbacks retained by a reused
command. This does not unlock another owner's file description or turn a later
free lock into evidence that an observed survivor exited. The original owned
group, journal and output-closure requirements remain in force; arbitrary
detached-descendant containment is still unproven.

## Historical implementation and current boundary

The alpha.3/alpha.4 details below describe their actual historical source. SPEC 1.4
supersedes their operator-authored executable/toolchain catalog, now replaced by
the automatic domain above. M02-1–M02-4 still include unfinished generic command
discovery and validation work. These
older implementation details are not current setup instructions or qualification
of the replacement backend.

The alpha.3 increment adds an opaque owned-group exit observation, produced by
reaping the original child after the supported macOS group observation. A
per-spawn identity binds this observation to its launch guard; an unrelated exit
status or a reused numeric PID cannot satisfy that binding. Provider, Git and
command owners retain the same observation across persistence retries.

The group observation compares two bounded member inventories and repeated
process birth identities after all observed members are terminal. A changed,
unavailable or saturated inventory does not establish exit. These conservative
observations are not an atomic kernel snapshot or broader descendant guarantee.

New process journal records use schema 2. Legacy schema-1 group-exit rows remain
readable without promotion to the newer exit evidence. A remaining inherited
guard, changed/missing journal record or unresolved output stream preserves
uncertainty. Once an inherited guard holder is observed after group exit, later
descriptor release cannot clear that uncertainty. Shared observation locks let
readers coexist without being mistaken for the original inherited owner.
These records never claim arbitrary-descendant exit and do not
recreate a live process capability or retire an unfinished operation on recovery.
Handover metadata schema 2 explicitly requires process schema 2; older metadata
is refused rather than assumed compatible. No existing installation is migrated.

This is an implementation increment, not complete managed-profile support. The
alpha.4 declaration now pins foreground executable hashes, command-only toolchain
read grants and optional generated execution under owned `scratch/build`.
Preparation admission, model contracts and retained cleanup isolation bind that
inventory; profile changes fence workspace authority and require drained
configuration replacement. Model source-read grants do not expand with toolchain
reads. Handover metadata schema 3 requires explicit managed-schema-1 support.
The declaration does not qualify the native command path or provide registered
services. Command completion, the finite native qualification route and service
lifecycle remain required before the configured profile can execute work.

SPEC §§6.1, 7.1, 11.2–11.5 require admission before launch, independent resource
isolation, verified-owned termination, preserved uncertainty and safe recovery.
Under version 1.3 these lifecycle and cleanup obligations apply to the supported
managed workload and registered services. Implementations must establish that
declared lifecycle rather than relabel unknown shutdown as success. The broader
future descendant guarantee additionally needs complete membership through
fork/exec/reparent/session changes, event-loss detection and a defensible final
observation boundary. Existing process-group evidence is not that guarantee.

The specification does not require the kernel to kill every descendant after
simultaneous death of every observer. That failure may remain an explicit
reconciliation blocker: no successful shutdown or completion can be inferred,
and replacement work or cleanup for the affected operation remains blocked.
Existing resource isolation must continue to enforce
its boundary independently. This allowance does not waive actual force-stop
termination or permit a live observer to declare success from incomplete data.

## Historical stronger-contract candidate and unresolved questions

The installed macOS 27 SDK and Apple's current documentation expose
[`es_new_descendants_client`](https://developer.apple.com/documentation/endpointsecurity/es_new_descendants_client(_:_:)).
It observes the calling process and its descendants, requires Apple's Endpoint
Security entitlement, and does not require root or TCC approval for this client
type. Apple's DTS guidance confirms visibility across reparenting/session changes
and recommends audit-token signalling for known descendants. Signing requires
the client to be packaged in a bundle. See
[DTS lifecycle guidance](https://developer.apple.com/forums/thread/848882).

[`es_sync_client`](https://developer.apple.com/documentation/endpointsecurity/es_sync_client(_:_:))
provides a queue-processing barrier; its callback also runs on client destruction.
It is not a standalone complete-exit certificate. The unresolved design work is
how to establish final closure under concurrent creation, missing notifications,
identity transitions and observer failure. Fast handlers, multiple clients and
successful ordinary runs must not silently become a guarantee of losslessness.

`libproc.h` labels its interfaces private/subject to change, while Apple DTS
specifically endorses `proc_signal_with_audittoken` and
`proc_terminate_with_audittoken` as reasonable exceptions. Availability and
behavior still need qualification on the supported host. Identity-safe signalling
does not discover a descendant whose creation was never recorded.

Apple DTS also confirms that ES restrictions disappear with the client and that
macOS has no kernel-owned inherited boundary that terminates all members when
every custodian disappears. Those stronger lifetime guarantees must not be
attributed to this candidate. See
[client loss](https://developer.apple.com/forums/thread/850253) and
[custodian lifetime](https://developer.apple.com/forums/thread/851230).

No supported entitlement-free substitute for the stronger arbitrary-descendant
contract was established by this investigation.
Process groups/launchd group cleanup do not cover detached groups; ancestry scans
have races; denying subprocess creation breaks ordinary repository tools;
resource sandboxes do not themselves prove exit. This is a bounded feasibility
finding, not proof that no future platform facility could satisfy that contract,
and not a reason to require ES for the approved managed-workload profile.

## Historical preparation and current qualification limits

At the time of the investigation the user reported no existing Endpoint Security
entitlement and was renewing Apple Developer Program membership. Membership is
distinct from Apple's restricted
entitlement approval. Apple identifies non-security helper supervision as an
intended use case, but that is not approval for Kegari. The public request route
is [System Extensions and DriverKit](https://developer.apple.com/system-extensions/).
At that historical checkpoint, no request, signing, installation or client
activation had been performed. The user's subsequent application submission is
recorded above; it does not establish approval or implementation qualification.
No approval timeline is established; see
[Apple's request-status clarification](https://developer.apple.com/forums/thread/841639).

The existing platform test binary was prepared successfully, without running
tests, using `cargo test --locked -p kegari-v0 --test platform --no-run
--message-format=json` with the recorded stable toolchain and private Cargo cache.
Cargo reused the current artifact (`fresh: true`):

- Executable: `target/debug/deps/platform-8a42d593f3c5ceff`.
- SHA-256: `d45a9884f1e42762e6a0a2e9235ed318e128e4b353968d9b93e741b57b2a17fb`.
- Build record: `/private/tmp/kegari-v0-native-prerequisite-build-1.jsonl`.

This preparation predates 0.2.0-alpha.1. Refresh the executable identity before
using a native prerequisite request to qualify that prerelease.

The proposed first native pass remains unapproved: one host/birth-identity test,
then, only on success, one owned-child signalling test using `/bin/sleep 30`.
Neither establishes descendant containment. The second test's two-second poll
is not a hard whole-test deadline: Drop performs a direct-child kill and blocking
wait. A reviewed outer execution/cleanup bound and a permitted host environment
must be supplied before the exact request is ready to execute. Existing sandbox
and boot-identity denials must not be retried unchanged or bypassed.

The subsequent decision is now explicit: use the managed-workload contract while
retaining full descendant coverage as a future prerequisite for untrusted-context
qualification. No ES entitlement, application or activation is a prerequisite to
that managed-profile engineering work. A documented foreground-only policy still
does not enforce the restriction against arbitrary repository code. Any future
implementation change must retain honest unknowns and pass the applicable
profile, isolation and lifecycle checks; neither the scope decision nor a
prerelease executable supplies that evidence. The stronger-contract research and
old native prerequisite build remain historical evidence, not a current native
qualification result.
