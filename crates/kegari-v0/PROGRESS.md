# Kegari v0 build progress

The Goal remains **paused and incomplete** at the user's explicit architecture
checkpoint. On 2026-10-27 the user approved the technology-agnostic correction
and authorized specification/goal documentation updates only. SPEC 1.4 now
prohibits target-toolset-specific code, including optional adapters and manifest
parsers, and removes mandatory environment/tool/command specifications. Agents
must discover project commands/context under generic host enforcement.

[MILESTONES.md](MILESTONES.md) now holds the revised version plan, retaining the
0.2.0–0.7.0 sequence and correcting 0.2.0 first. The immediate target on explicit
resumption is **0.2.0 / M02-1: remove/rework the incompatible execution design**.
No minor milestone is complete. The goal files require concise milestone updates
with completed work/evidence, remaining exit criteria, next action and blockers,
plus a status update in the milestone file at meaningful checkpoints.

The current authorized work is planning/documentation only. The milestone plan
is recorded; only explicit user resumption can lift the implementation hold.
The user subsequently authorized committing these documentation changes. Code
deletion, implementation, product/native validation and build packaging remain
outside this step; the documentation commit does not resume the Goal.

On resumption, audit and remove/rework incompatible implementation throughout
the repository, including the pending alpha.9 Cargo-specific paths. Preserve
useful generic supervision/evidence and historical failures. The prior source
reviews and test results below remain evidence for their actual SPEC 1.3/code
baseline, not proof of SPEC 1.4 conformance or a direction to finish alpha.9 as
previously designed. The inventory tool's approved hash and coverage ledger
still reference SPEC 1.3; reconcile them, new V0-H04/T38/T39/E09 and affected
existing requirement statuses during the implementation package without
inheriting passes for changed requirements. Historical reports stay unchanged.

Pre-amendment implementation checkpoint (retained for reconciliation):

Current work in progress after committed alpha.8 `4b95a81bb4652979f6083450c4c98093854f0398`
(`feat(kegari-v0): bind managed SDK and library contents`): **0.2.0-alpha.9**
adds the closed offline Cargo selection, input closure, protected per-command
configuration and exact rendered launch/environment binding. The previous goal
turn was progress: the alpha.8 commit and clean tree were verified, with its
retained executable/evidence hashes matching. No commits were consolidated.

Alpha.9 implementation and independent source review are complete through the
protected offline launch path, requested/actual command evidence separation and
literal linker-library mappings. Initial checks exposed a type-inference error
and two fixture setup errors; all are corrected and their logs retained. The
first full release suite passed **1,150 tests, zero failures, three native ignores,
40 groups**, and clippy passed. That source is retained in the pre-cache snapshot.

A final review then confirmed that repeated invocations reused Cargo target/tmp
state. The correction creates exclusive per-command generated namespaces,
retains their directory identities and rejects prelaunch population/replacement.
Old caches and partial directories are preserved. The focused cache/executor
run passed 35 tests, and final all-target clippy passed. Final-source release
regression session `33130` returned exit 0: **1,152 tests passed, zero failures,
three native ignores, across 40 groups**. Its logs are
`/private/tmp/kegari-v0-alpha9-full2.stdout` and `.stderr`; the terminal results
are recorded in `/private/tmp/kegari-alpha9-final-commands.json`.
The 728-input final test snapshot is
`/private/tmp/kegari-alpha9-test-source.json`, SHA-256
`3f39ee3f80d8b4bd2d0bdc57c83020d7e730926d591f3497fe7b753a0ce23428`.
All previously owned Cargo sessions are terminal; no native qualification
command is running. Alpha.9 changes remain uncommitted. Final formatting,
inventory/toolchain checks, exact executable packaging, evidence collection
and its independent audit were pending at that checkpoint; the old packaging
plan is now superseded by MILESTONES.md. Prepared helpers in
`/private/tmp/kegari-alpha9-{artifact,evidence}.py` have not been run.

The public qualification prerequisite remains closed; NQ-02C remains consumed.
Read-only installed-tool metadata/header inspection identified four linker
libraries (now explicitly bound) and the archiver's possible ranlib/libtool alias
path. Alias handling, actual native Rust/Cargo and the larger milestones remain
open. No setup, Keychain, install, provider or remote operation occurred.

Previous verified increment on baseline `0a8ab79`: **0.2.0-alpha.8** adds
bounded SDK/library content identity and explicit executable-library mappings.
Full content observations run outside serialized Control callbacks during
preparation, check/hook completion and cleanup; changed inputs cannot become
successful evidence. The original resource read scope stays explicit. This is
pre/post observation, not an immutable OS snapshot or workload qualification.
The public `managed_profile_unqualified` gate remains closed.

The final exact-source release suite passed **1,117 tests, zero failures, three
native ignores, across 40 groups**. All-target clippy, formatting, inventory and
five staged-binary inspections pass. Initial fixture and lint failures are
retained. Independent review corrected two malformed-manifest edge cases and
confirmed full SDK reads remain outside Control. It also corrected an inherited
Git probe in the final source-write callback while preserving authority, ownership
and file-content conflict checks. No native qualification run was performed this increment.
Final independent evidence audit verified 1,077 referenced hashes, exact source/
test membership, all 40 test targets and staged metadata/assets without a material mismatch.

The executable is staged at
`target/prereleases/kegari-v0-0.2.0-alpha.8-aarch64-apple-darwin-7c2ca3de037b/kegari-v0`;
SHA-256 `7c2ca3de037b098efdd5c22b979237d3b9ae84d8386b260b45bea35d1aea8fae`;
build identity
`sha256:bd82ae2e770562550c846b5af5e9e3b343759c4ec80ab488b6fa86c41adc07dc`.
The final 724-input test snapshot matches current source. Content-bearing managed
records use schema 3; legacy absent-content records retain schemas 1/2. Handover
metadata 3 requires managed schema 3 and process schema 3. Instruction assets stay
version 3. See [resource-binding evidence](validation/MANAGED-RESOURCES.md).
This is a separately reviewed capability increment after alpha.7's fixed native
baseline; no commits are consolidated or history rewritten.

The initial full run stopped at two new execution-fixture setup failures after
858 passes. Its host Git path failed the existing protected-ancestry check; using
the fixture's existing protected copy corrected setup without relaxing that
check. Execution then passed 20 tests; the remaining targets passed 257. Initial
clippy required boxing one private in-memory isolation value and using an
equivalent schema range. After review and correction, clippy and the full final
suite passed. Logs, source snapshots and earlier failures are archived beside
the staged executable. All owned Cargo and artifact-inspection processes finished.

The previous goal turn was progress: alpha.7 was committed as
`0a8ab79570614ad9d2cb1a4006deb4232935914f` —
`fix(kegari-v0): allow bounded native loader bootstrap` — and NQ-02C passed once.
That approval remains consumed. Its exact-source alpha.7 evidence does not
qualify alpha.8's resource-bearing profile. NQ-01B remains separately pending.
No setup, Keychain, installation, service/coordinator activation, provider or
remote operation is included. No minor milestone or overall goal is complete.

Previous verified increment on baseline `db4374f`: **0.2.0-alpha.7** corrects
native loader bootstrap. The separately approved NQ-02C test ran once and passed
all three fixed cases: Bash output/redaction and file boundaries, exit 7, and an
owned sleep timeout with SIGKILL. Every case recorded original group/journal/
output closure, without claiming escaped-descendant closure. All 22 fixture files
are retained and hash-verified. The one-use NQ-02C approval is consumed.

The profile adds read/existence access to eight fixed literal ancestors, including
`/`, and existence access to seven existing fixed Cryptex grafts, plus explicit
forbidden-root existence denial. Literal directory reads permit inspecting their
entries; no recursive root, ancestor map/exec/write, network or Mach grant is
added. The fixture retains its original waiter through bounded timeout cleanup
and defers diagnostic-write assertions until that observation finishes.

Affected local checks passed **97 tests, zero failures, one native ignore**;
all-target clippy, formatting, inventory, final native-fixture compilation and
five staged-executable inspections pass. Alpha.6's full suite remains historical
evidence for unchanged areas; no alpha.7 full-suite run is claimed. Independent
source/package review found no remaining material finding in the reviewed scope.
Final independent audit matched 48 referenced hashes, 325 build inputs and 723
test inputs, recomputed the build identity and reconciled the case observations.
No supported-profile qualification is issued and public admission stays closed.

The exact executable is staged at
`target/prereleases/kegari-v0-0.2.0-alpha.7-aarch64-apple-darwin-be94ce252874/kegari-v0`;
SHA-256 `be94ce25287486f2fc147c0901fce5946a0ee041c91ccc68074f3dafbc18980e`;
build identity
`sha256:76b7b0d07c6c4dd047733a6b6cba94053493631362ac2997e7c34d2efa8c5c38`.
This is a development executable; it is not installed or operationally qualified.
See [loader correction and native evidence](validation/LOADER-BOOTSTRAP.md) for
source, commands, retained observations and limits. Alpha.7 is a separately
diagnosed correction to the immutable alpha.6 evidence baseline; no commits are
consolidated or history rewritten.

Previous verified increment `db4374f` —
`feat(kegari-v0): supervise finite service-backed checks` — delivered
**0.2.0-alpha.6** after `5b7078d`. It implements finite
check-owned services under the original admitted effect: ordered start/readiness/
check/stop, retained aggregate ownership, shared deadlines/output limits, writer
closure before snapshots, and strict warm/cold membership checks. It also fixes
a host fork/guard-retirement race without weakening genuine-holder uncertainty.
The exact final release suite passed **1,097 tests, zero failures, three native
tests ignored, across 40 groups**. Focused regressions, all-target clippy,
formatting, inventory and five staged-executable inspections pass. Independent
review found no remaining material finding in the reviewed bundle/interlock
increment. Loader bootstrap was unresolved at that historical checkpoint;
alpha.7 establishes only the three fixed foreground cases described above.

The historical alpha.6 executable remains staged at
`target/prereleases/kegari-v0-0.2.0-alpha.6-aarch64-apple-darwin-f273893778f2/kegari-v0`;
SHA-256 `f273893778f2d48f50f737da6e0a1e553a1c067c6f1d07f82a8410b066f88af7`;
build identity
`sha256:a75a0af27f0150c1a0bcaca1e9e91bb0daa2cc98088c30f1ab48c967895ec98e`.
This is a development executable; it is not installed or operationally qualified.
See [finite check evidence](validation/MANAGED-CHECK-BUNDLES.md) for exact source,
commands, artifacts, failed attempts, reviews, compatibility and remaining limits.

| Current scope | State |
| --- | --- |
| Foreground supervision and finite check-owned services | Implemented and locally tested; no service native qualification claimed |
| Native foreground launch | NQ-02C passed three fixed cases on alpha.7; A/B failures retained; all three single-use approvals consumed |
| SDK/library content and library mappings | Implemented and locally verified; complete offline Rust/Cargo contract and native qualification remain open |
| Managed-profile admission | Public refusal remains enforced; configuration or hashes do not qualify tools |
| Supported Rust/Cargo, endpoint services and remaining native checks | Incomplete; additional native runs need separate approval |
| 0.3 recovery → 0.4 external/visual → 0.5 handover → 0.6 evaluations → 0.7 real qualification | Sequence unchanged; no minor milestone complete |

NQ-02B's retained outcome shows SIGABRT in the first Bash case, empty output,
no timeout, and closed original group/journal/collectors. The other two cases
were not reached. The matching existing OS crash report places the abort in
libignition/dyld bootstrap. Apple's installed profile documents literal-root and
fixed-Cryptex-ancestor read/existence access absent from our tested policy. This
informed the alpha.7 correction; the exact denied operation remains unproven.
The pin predates alpha.6's interlock. See [native recheck evidence](validation/NATIVE-MANAGED-RECHECK.md).

**Next work package:** implement the closed shipped Rust/Cargo support contract:
closed support/case selection, complete vendored dependencies, protected Cargo
configuration, command-only private state and exact compiler/linker/SDK environment
bound into launch identity. Use a private prepared launch, truthful result
artifacts and generated-executable/doctest cases. The concrete plan is in
[the qualification design](docs/managed-qualification-design.md). A runtime
per-profile issuer/registry is not required by SPEC and remains deferred.
Endpoint services need scoped network support and separate native cases.
Do not rerun consumed NQ-02A/B/C requests. NQ-01B remains separately pending.

All owned Cargo, native-test and artifact-inspection commands have finished.
NQ-02A/B failed roots and NQ-02C passing evidence are retained; no stale PID has
been adopted or signalled. No setup, Keychain, installation, service/coordinator activation,
provider action or push occurred. The full suite's deliberately injected
uncertainty fixtures remain diagnostic evidence. Source/input manifests and
failed compile/test/lint history are archived beside the staged artifact.

Previous verified increment `5b7078d` —
`feat(kegari-v0): retain managed command supervision through completion` —
delivered alpha.5 owned command completion/cancellation. Its 1,051-pass suite,
three native ignores and artifact
`target/prereleases/kegari-v0-0.2.0-alpha.5-aarch64-apple-darwin-b7f0e3ca25de/`
remain historical [command evidence](validation/MANAGED-COMMAND-SUPERVISION.md).
The current increment is separate; no commits are consolidated.

Previous verified increment `354b27a` —
`feat(kegari-v0)!: bind managed foreground policy to execution scope` — delivered
alpha.4 policy/tool bindings after the guided-setup commit `2eab4ae`. Its 1,041-test
suite and exact artifact remain in [managed setup evidence](validation/MANAGED-SETUP.md).
No commits are consolidated in this separate lifecycle increment.

Supervised request NQ-01A was explicitly approved and executed once: the read-only
native identity test passed (one selected test, zero failures, no timeout,
0.384 seconds wrapper elapsed, empty stderr and no temporary cwd files).
It used the preserved alpha.3 test binary at
`target/native-qualification/nq-01-91160a62001a/platform-test`, SHA-256
`91160a62001a6439c43cc4ffdca16879f5e69a23dcb9826738ee46bd6cb182fa`.
Exact argv selects only the ignored
`native_host_and_birth_identity_are_observed_without_environment_trust` test with
one test thread; outer timeout 20 seconds, one attempt. It reads OS account/home,
host/boot UUIDs, its own process identity and free bytes. The test spawns no child
and changes no runtime state; the wrapper owns one test process and TEMP logs.
No setup, Keychain, service, existing-process signal, provider action or second
native check is included. The exact request is retained beside that binary in
`request.json`, with exact result and hashed stdout/stderr alongside it. This
establishes the native identity prerequisite only; it does not qualify setup,
Keychain, signalling, sandboxing, command execution or broader native behavior.

NQ-01B is prepared and awaiting separate approval: one alpha.4 native
`child_signal_uses_owned_unreaped_handle_and_never_adopts_pid_metadata` test,
with a 20-second outer timeout. Pinned executable:
`target/native-qualification/nq-01b-aab9a5913248/platform-test`, SHA-256
`aab9a591324848d091211acc0cd3aa44bd33a17e84497814455cbca763985e98`.
It starts only `/bin/sleep 5` with a disposable fixture home and new owned group,
captures its identity, signals that group with SIGKILL, reaps it and checks that
signalling after reap is refused. The helper expires naturally after five seconds.
No setup, Keychain, service, provider or pre-existing process is in scope. The
request is retained beside the pinned executable; it has not been run.

NQ-02A was explicitly approved and executed **once**, then failed in its first
case: expected shell exit 0, observed no numeric exit code. Test exit 101;
0 passed / 1 failed / 507 filtered; outer timeout did not fire (0.744 seconds).
Cases two and three were not reached. The pinned binary and one-use permission
are unchanged; **do not rerun it**. The retained first-case journal records owned
group exit and verified guard release, with a null exit code; scratch is empty
and the protected sentinel is unchanged. The test did not retain its command
output or termination signal before asserting, so the exact cause is unresolved.
See [native managed-command failure](validation/NATIVE-MANAGED-COMMAND.md).

Read-only diagnosis found absent executable-mapping grants and an undeclared
transitive shell: this host's `/bin/sh` selects `/bin/bash` through
`/private/var/select/sh`. These are supported launch incompatibilities, not proof
of the particular denied operation. Alpha.6 corrected the mappings and direct
Bash fixture; alpha.7 added the bounded bootstrap correction and passed NQ-02C.
The original failed root remains preserved at
`/private/tmp/kegari-nq02a-n_3qkjgx/.tmphqx7PS` and request/result beside
`target/native-qualification/nq-02a-a989ca4d2520/managed-command-test`.
NQ-01B remains unapproved/unexecuted. No native test is currently running.

NQ-02B was explicitly approved and executed **once** after the final Cargo
checks finished. Pinned binary:
`target/native-qualification/nq-02b-ac6cf23ac60b/managed-command-test`, SHA-256
`ac6cf23ac60b314bd3839880d7ffb1ab026a26ff0a2612e30bf66e403e78a4c9`.
The first case failed: SIGABRT (6), no numeric exit, empty output, no truncation
or timeout. Test exit 101; 0 passed / 1 failed / 541 filtered; wrapper exit 1,
0.649546 seconds. Original group/journal/collector closure is recorded; scratch
is empty and the protected sentinel unchanged. Exit-7 and sleep cases were not
reached. The request, one-attempt marker, result and failed evidence root
`/private/tmp/kegari-nq02b-8944rt1y/.tmpOxKZ33` are preserved. This pin predates
final host-launch synchronization and does not qualify alpha.6. Approval is
consumed; NQ-01B was not included and remains unexecuted. No native test is running.

NQ-02C was explicitly approved and executed **once** after all local checks.
Pinned binary:
`target/native-qualification/nq-02c-d2179900672c/managed-command-test`, SHA-256
`d2179900672c79ebcd3217ce556ec0b1c2d4de57a240cf09fe703fb4be105c54`.
Its final 723-input source manifest matches alpha.7. Test and wrapper exited 0;
one test passed, zero failed, 544 filtered; no outer timeout, 0.923 seconds.
All three cases passed with group/journal/output closure and no truncation.
The retained fixture at `/private/tmp/kegari-nq02c-959z2ztz/.tmpnjzxmt` includes
22 hashed files and an explicit no-qualification completion marker. This proves
those fixed component cases only, using a synthetic lease; it does not qualify
runtime admission, Rust/Cargo, services, networking, providers or setup. Approval
is consumed, and no native process remains owned by this completed check.


Alpha.6 validation retained the initial compile/lint failures, invalid new-worker
fixture setup, the production canonical-float digest defect exposed by those
fixtures, and the second full suite's guarded Git-probe failure. The corrected
plan binds its five actual integer bounds. A controlled pre-exec regression proves
the transient inherited-descriptor race mechanism; the specific historical
holder was not identified. The interlock and original cutoff checks pass focused
and final full regression. [Detailed evidence](validation/MANAGED-CHECK-BUNDLES.md)
records every attempt and distinguishes component tests from native qualification.

The guided setup change is committed as `2eab4ae` —
`feat(kegari-v0): guide initial local setup with reviewed configuration`.
The verified alpha.4 managed-policy, compatibility, documentation and evidence
are recorded in a separate increment for reviewable history. Its owned Cargo and
artifact inspection processes finished; NQ-01B remained awaiting approval.

Prior verified increment: **0.2.0-alpha.3** binds actual owned-group exit
evidence to its original spawn, distinguishes schema-2 evidence from legacy rows,
retains journal/stream/guard uncertainty and improves first-launch guidance.
The final release suite passed 1,025 tests, with two native tests still ignored;
all-target clippy, formatting, inventory and five staged-artifact inspections
passed. Independent final review found no additional material issue. The exact
binary, earlier failures/interrupted attempts and qualification limits are in
[owned-group lifecycle evidence](validation/OWNED-GROUP-LIFECYCLE.md).
No minor milestone is complete. The foreground tool/dependency profile and
registered-service lifecycle still need integration before the repository-command
refusal can be removed. Native qualification remains separate.

Process-journal writes and handover metadata now use schema 2; older process rows
remain readable without promotion, and old/missing handover support is refused.
Instruction content remains version 3. The staged alpha.2 artifact is unchanged.
All owned validation processes for this increment have finished; no application
service/coordinator is running from these checks. Held B drafts remain held.

The user's alpha.2 no-argument invocation correctly reached the CLI's default
overview and found no registered local runtime. Initial setup does not happen
implicitly. Inspection found incomplete onboarding: setup requires complete
operator TOML, without a public host-identity/configuration initializer, and the
CLI guide still described an obsolete empty entrypoint. The next usability work
must provide configuration preparation and verify setup/install/control-host
start versus orchestration start, not merely compile the binary. Documentation
and the actionable missing-registration diagnostic are corrected; no real
setup, Keychain initialization, installation or service start is authorized by
this analysis request.

The first-launch correction was committed as `edef4f5` —
`fix(kegari-v0): explain explicit first-launch setup`. Alpha.3 additionally uses
`Target: local` before registration. The intended sequence is explicit setup
(initial STOPPED intent), service installation, control-host start, readiness
checks, then separately requested orchestration start after qualification.
The smallest onboarding increment is a genuine guided initializer within the
existing `setup` path: derive host/account facts through the existing native
identity boundary, generate the runtime UUID, collect actual operator identities
and scope, show reviewable typed TOML and reuse bootstrap validation/persistence.
Current `config check` is server-only; initial preparation performs provider/Codex
probes and must not be presented as an offline-only validator. Implement/test the
guided path with fixtures; actual host discovery/setup still requires its scoped
approval. Continue the managed foreground/service integration in parallel where
independent, preserving uncertainty and current action restrictions.

On 2026-10-26 the user reported submitting an application to Apple for the
Endpoint Security entitlement. Submission is recorded from the user's report;
approval has not been reported. Revisit Apple's response and the full descendant
coverage design before the v1 / 1.0.0 production-readiness decision. This follow-up
does not block managed-workload v0 engineering or authorize signing, installation,
client activation or other host/live operations. Full coverage remains required
before untrusted-context qualification, even if that is considered before v1.

On 2026-10-26 the user explicitly selected
**managed workloads with the detached-process limitation**. SPEC 1.3 and the
matching GOAL.md/objective now record that decision. Full descendant coverage
remains desired future work and a prerequisite to qualification for untrusted
external context, including public PR material. It is not a prerequisite to
completing the managed-workload v0 profile. Controlled E07 evaluation obligations,
filesystem/network/credential/role isolation and action restrictions remain.

Approved SPEC SHA-256:
`35d4515585fe92350ec1cc8b78a5919c834d559a48d48d071239d39305032fb3`.
The reconciled inventory has 503 clauses and 136 named requirements: ten new or
changed clauses are unassessed, three old clauses are retained as stale, and all
surviving status/evidence entries and named requirement rows are unchanged. Six
inventory tests pass. Independent review found no material amendment conflict.
No broader qualification or implementation success follows from this amendment.

The next 0.2.0 work is to implement and qualify the managed lifecycle through
commands/checks/hooks, cancellation, retirement, recovery and cleanup. The existing
repository-command refusal remains in code until that integration is ready.
Endpoint Security is not an established prerequisite. Supervision is available;
native/host/live actions retain their separate explicit approval requirements.

The related service-shutdown correction is implemented and locally verified:
recovered local ownership and persistence uncertainty block reservation,
unfinished replay stays uncertain, and retired-local/unknown-remote work remains
distinguishable. The old code failed its new regression; all 85 targeted release
tests, all-target clippy, formatting and four direct artifact inspections pass.
Independent source review found no material issue. The exact 0.2.0-alpha.2
executable, regression failure and validation limits are in
[service shutdown evidence](validation/LOCAL-SERVICE-SHUTDOWN.md). The alpha.1
artifact remains unchanged. No service/coordinator was installed, activated,
signed or published. Held B evaluation drafts remain held.

The accepted scope amendment was committed separately as `fff533f` —
`docs(kegari-v0): define managed-workload qualification`; the external objective
matches GOAL.md. The next implementation package is managed-workload supervision,
not further entitlement research. Handover review also identified remaining
worker-reconciliation, exact old-supervisor removal, partial-replacement recovery
and qualification-record workflow gaps for 0.5.0; this shutdown correction alone
does not close them.

Historical prerelease checkpoint (5489e0d; earlier scope decision was pending):
The Goal remains active and incomplete. Supervision is available; host/live
changes and native qualification still require their separate scoped approval.
The previous discussion clarified the containment trade-off without implementing
it or changing the contract. This turn revalidated the clean `main` checkpoint
`cc0de47` and completed the independent prerelease build portion of 0.2.0.

Application version is now **0.2.0-alpha.1**, retaining package/executable name
`kegari-v0`. Package, lockfile and embedded catalog agree; instruction content
version 3, dependencies and persisted schemas are unchanged. The optimized
macOS arm64 artifact and exact identities are in
[prerelease evidence](validation/LOCAL-PRERELEASE.md) and its
[machine record](validation/prerelease-checks.json). All 34 focused release tests,
four direct artifact inspections, formatting and contract inventory passed.
Independent implementation/evidence reviews found no material issues.
The prior 1,006-test full-suite result remains historical 0.1.0 evidence, not a
new full-suite run. Two required native checks remain unexecuted here.

The staged build artifact remains unchanged. Cargo's test invocation produced a
different binary in the shared target/release location; both have the same
source/profile identity. The evidence collector initially rejected that difference,
then recorded separate hashes and the exact checks applied to each artifact.
No runtime algorithm changed, no coordinator was activated and no installation,
signing, entitlement request, provider operation or remote mutation occurred.

Containment still blocks repository commands. The user questioned making an
Endpoint Security entitlement a prerequisite; no such dependency is established
and no application is planned. The current objective still explicitly requires
escaped-descendant ownership. The pending supervised decision is whether to
retain it or explicitly adopt a supported managed-workload contract for v0.
Silence does not approve that scope change. Neither the refusal nor SPEC/GOAL
has been weakened. See [containment findings](docs/process-containment.md).

The milestone order remains 0.2.0 containment/build, 0.3.0 recovery, 0.4.0 evidence,
0.5.0 operator handover, 0.6.0 evaluation machinery, 0.7.0 qualification. No minor
milestone is complete. Continue independent authorized work while the containment
decision is pending; the existing handover library lacks its public operator route.
Held B evaluation drafts remain untouched. The earlier --no-run native-test binary
belongs to the prior version and needs refreshed identity before a qualification
request for this prerelease. No native test or host-change approval was inferred.

Historical preparation reconciliation while paused:
Preparation reconciliation on
2026-10-26 at 13:27 UTC records the user's explicit availability for supervised
decisions and bounded native qualification. The user also explicitly requires
continued pause, no implementation yet, and separate approval for required host
or live operations. Earlier unattended-mode statements are historical. Presence
does not resume the Goal or grant host, installation, provider or deployment
authority; the existing action restrictions remain effective.

Current HEAD on main and local origin/main are
`97d1f05a4bf4bb2848756fe7da3dceb8a8bd0181` —
`docs(kegari-v0): define versioned completion milestones`. No remote refresh or
push was performed for this reconciliation. Only GOAL.md changed in committed
files since the tested implementation checkpoint a217df8; the package remains
0.1.0. The amended objective attachment and GOAL.md match after the command
prefix and final newline. SPEC 1.2 retains its approved SHA-256
`80b6f703d224ce7800c9e04a23c6356ab72f02133ab9be5ccb129fd6eb10e326`.
The index is empty; PROGRESS.md is the only working-tree edit. Prior validation
remains evidence for the unchanged implementation, not a newly executed check.

The milestone sequence in GOAL.md controls future work: 0.2.0 containment and
executable build; 0.3.0 recovery; 0.4.0 target/visual evidence; 0.5.0 operator
handover; 0.6.0 evaluation machinery; 0.7.0 full v0 qualification. Compatible
corrections use patch versions; 1.0.0 and the lasting build remain out of scope.
On explicit resumption, first prepare the 0.2.0 containment feasibility decision:
verify candidate API/host prerequisites, launch enrollment and descendant/exit
guarantees; map cancellation, retirement and recovery consumers; then prepare
finite adversarial cases and exact native-test approval details. Do not remove
the current refusal or resume the held B drafts merely to regain momentum.

Proposed native prerequisite request NQ-01 is not approved or executed: name a
permitted macOS host/account and pin the source, executable, argv and owned TEMP
directory; run the existing native host/birth-identity test once, followed only
on success by the owned-child signalling test once. Run serially with a proposed
two-minute aggregate execution deadline. The second test owns one /bin/sleep 30
child. Retain bounded logs and exit/reaping evidence; uncertainty is not cleanup
success. No retries after denial, sandbox bypass, installation, entitlement or
security changes, credentials, model/provider work or remote mutation are
included. Exact host/environment and executable details must be supplied before
requesting approval. These tests qualify prerequisites only, not containment.

This preparation used read-only source/evidence inspection and bounded subagent
reviews; only this progress record was updated. No implementation, build, test,
native probe or live operation ran. The completed implementation checkpoint had
no remaining Cargo/test/collector invocation; no global process sweep was made
during this preparation. Recheck owned execution state before future execution.

Historical implementation pause: 2026-10-26 at 12:35 UTC, after the user reported
1d 12h 11m 45s of goal execution. All three implementation agents were interrupted
at that checkpoint; subsequent preparation reviews do not resume their drafts.

Pause checkpoint: `a217df8d5af44ee3708110bc969fb0e854b688d8` —
`feat(kegari-v0): prepare empty coordination evaluation substrates`.
At that checkpoint the user reported pushing the latest changes, and HEAD and
local origin/main both pointed to this commit. Root performed no push. The tree was clean before this
pause-only progress update. All14 intended files and709 tested source inputs
were verified against committed and physical bytes; all other tree paths match
parent46c234f. No commits were consolidated. Verification:
`/private/tmp/kegari-v0-coordination-substrate-commit-verification-1.json`.
Local evidence:1,006 Rust tests passed, zero failed,2 required native checks
ignored across39 groups;11 application commands and24 copied-suite Python tests
passed. Actual E01 copy output and its Rust Plan were verified. See
[coordination evidence](validation/LOCAL-COORDINATION-SUBSTRATE.md).

Unintegrated B finite-output closure work is preserved in TEMP and remains
incomplete, unreviewed as a composition, and uncompiled/unexecuted:
- Core/finalizer owner Codex: `/private/tmp/kegari-e07-output-matcher-slice-b-implementation-v1`.
- Adapter/reader/bootstrap owner Config: `/private/tmp/kegari-e07-declared-interval-adapter-v1`.
- Worker/broker/journal owner Git: confirms the assignment stopped after source
  tracing and interface coordination; no implementation overlay or sealed
  artifact was created. The core and adapter draft directories still exist;
  neither was integrated, compiled or qualified by this preparation.
The accepted B-v2 design and independent rereview remain at the paths recorded
below. Reconcile incomplete TEMP drafts against actual source before any future
integration. Typed scoring and native coordination context/caller remain
unfinished. Root's latest coordination investigation was read-only; no caller
implementation or new native coordination design artifact was installed.
The pause/preparation history accompanies the resumed containment work. Reflection
and supervision alone did not authorize implementation; the later explicit Goal
resumption enabled existing local engineering authority. Live qualification and
host changes retain their separate approval boundaries.

Earlier verified local commit: `46c234fdb2351f901e902de920f0ee1358bf1eba` —
`feat(kegari-v0)!: retain bounded declared output matches`.
All 16 intended files and 706 tested inputs match committed and physical bytes;
every other tree path matches parent 768b125. The tree was clean after verification.
No commits were consolidated or pushed by this builder. The remote-tracking ref
still pointed to 768b125. Verification:
`/private/tmp/kegari-v0-output-matcher-commit-verification-1.json`.
This progress update accompanies the next coordination/closure work, not a
standalone commit solely for hash bookkeeping.

Earlier verified local commit: `768b125a5954f53675aa6f0f638b3fe676ac23a0` —
`feat(kegari-v0): bind E07 context to the original canary owner`.
All 19 intended files and 704 tested inputs match the commit and physical tree;
every other path matches parent 1d48265. The tree was clean after verification.
No commits were consolidated; no push was performed. The remote-tracking ref
still pointed to 1d48265 at that check. Verification:
`/private/tmp/kegari-v0-e07-operand-commit-verification.json`.
The reviewed matcher slice A is installed, locally validated and committed
as 46c234f. Final source 706 is
`sha256:c4a8b71e52ef8b4c93523766e8293b6d7a1883e0804fa3700cb4a8458bcd32af`:
1,001 tests passed, zero failed, two native checks remain ignored. All 14 final
commands passed; the exact disabled 28-variant/56-slot Plan is unchanged.
Reports are installed: [declared-output matcher](validation/LOCAL-DECLARED-OUTPUT-MATCHER.md)
and [machine evidence](validation/output-matcher-checks.json). All four reviewed
coverage appends preserve partial statuses and prior evidence. Output remains
partial: no finite negative, typed E07 grade, native/agent/human qualification.
Coordination prerequisite validation passed on installed709-input source1:
1,006 tests passed, zero failed, the same two required native checks ignored.
All11 commands and24 copied-suite Python tests pass; actual copy-only E01 CLI
output is independently verified. Reports and the reviewed partial-only coverage
updates are installed and committed as a217df8. B's corrected bootstrap-disposition design is accepted for TEMP-only
implementation; typed scoring/native qualification remains separate.

The following matcher installation/validation history retains its real failures.
At initial installation the matcher had not yet compiled or run. Its 12 paths
match the reviewed v2 composition and operator-doc draft2 exactly. Installation
record: `/private/tmp/kegari-v0-output-matcher-installed-1/INSTALL.json`.
The 706-input source1 snapshot is
`sha256:7efaadb91fb7a83646a3bbe9de0feaaf44f9280aeb41b82fe93c6365d3909d1c`
at `/private/tmp/kegari-v0-output-matcher-source1.json`. Root verified all current
source hashes and all 12 installed bytes after resumption. MR-1/MR-2 test setup
and MA-1 delta identity findings are resolved in the independently reviewed v2;
17 authored tests were unrun at installation. All-target compilation now passes
in 33.139s and strict Clippy in 31.987s, with unchanged source/toolchain
observations. Focused and full validation is running serially in session94929,
with stop-on-failure; exact planned commands:
`/private/tmp/kegari-v0-output-matcher-source1-commands.json`. Partial output
coverage is intentional; finite negative evidence and typed scorer integration
remain incomplete. The independent installed-composition audit found no mismatch;
root read it and verified all three members and its manifest:
`/private/tmp/kegari-output-matcher-installed-source1-independent-audit-v1/REVIEW.md`,
SHA-256 `d31e667b2043aad98a7057cf4ffe8f4b8adb3c32fe53879f5304619f849ab08b`.
It is a static composition review, not execution evidence. Session94929 stopped
on the first real failure: evaluation-1 passed 82 tests and failed one new budget
test in 59.066s (exit101). The original initialization had charged 128 bytes;
reserving whole 64 KiB chunks left 65,408 bytes, so the test had not actually exhausted
capacity. Raw failure SHA-256:
`c8d01662636fda4e536d41cfd8cbc4c7347dd8de2350c006f4c354e39409959e`.
Root corrected only that test to consume the remainder and assert the actual
observation_limit before observation and after finalization. Production limits
and behavior are unchanged. Exact original/correction:
`/private/tmp/kegari-v0-output-matcher-budget-test-correction-1`.
The remaining source1 commands were not run. Source2 contains 706 inputs at
`sha256:c4a8b71e52ef8b4c93523766e8293b6d7a1883e0804fa3700cb4a8458bcd32af`,
snapshot `/private/tmp/kegari-v0-output-matcher-source2.json`; only the corrected
test file differs. Root read the independent correction review and verified its
two members; no material finding:
`/private/tmp/kegari-output-matcher-budget-test-independent-review-v1/REVIEW.md`,
SHA-256 `553a3847aa2fb5a8e97702cbcf8d59913d01f57ef0a9648975f507a926a6a5f6`.
Serial rerun session25667 uses the 14 exact commands at
`/private/tmp/kegari-v0-output-matcher-source2-commands.json`. Strict Clippy passed
in 18.724s; evaluation 83 tests in 18.782s, protocol 24 in 0.161s, storage 4 in 0.149s,
runtime 33 in 1.903s, worker 11 in 1.914s, owned-return 2 in 0.255s and worker-return 1
in 0.172s. Root independently checked all 17 exact new names in their passing
focused logs. These groups overlap. Every completed result has unchanged source
and toolchain. Full-suite validation passed: 1,001 tests, zero failures, the same two native
ignores across 39 groups in 756.594s. Raw SHA-256:
`352f44f667bf291e8c218fedd9eb61268651f77c2ae7b6b0be11a4d44e8aca1a`.
Compatibility passed 109 tests in 75.508s and fmt in 1.638s. Plan passed in 41.609s,
inventory in 0.041s and diff in 0.018s. Plan raw SHA-256:
`f508cf2f92a4b83e5e64c45e27585e30718037e8dcd800b0ea4ff4476d1d1257`.
All 14 source2 records retain identical source/toolchain; session25667 is closed.
Root independently verified exact prior membership plus 17, both ignored identities,
and exact prior Plan envelope/data equality (28 variants/56 slots, disabled).
Root's first inline Plan comparison and the first actual collector invocation
both incorrectly compared the CLI response envelope with the prior extracted
Plan data. Collector-1 failed in 0.164s before creating its output directory;
this is evidence-tool failure, not a changed application result. Failure log
SHA-256 `df11230ce8163753ec70515d572431a6ec2240d5d23f099187531806fcc9908d`.
The actual Plan remains unchanged; a separate immutable collector correction is
being prepared. Source2 and all original validation records remain unchanged.
Root read the full 293-line original collector/specification/template, then the
complete v2 diff and specification delta. V2 checks both exact CLI envelopes
before comparing Plan data and retains the failed collector separately from
17 application records. Root verified all seven v2 members, unchanged v1 and
51 pins. Independent rereview found no material issue; root read the full review
and verified its two members:
`/private/tmp/kegari-output-matcher-evidence-independent-review-v2/REVIEW.md`,
SHA-256 `f9368042abc94fca45771f4595481033c1759470390a5b3fde5273a659f1af43`.
Collector-2 passed in 0.188s, with unchanged source/toolchain. Raw SHA-256:
`ef6b6d9a905a4f1a22be08183c6c4bb8414c6aeff8b7f5f3edd9b884639c60a3`.
Root read the entire generated report and verified its actual counts, histories,
Plan, source identity and original memberships before installing the exact bytes.
Report SHA-256 `20799c0fb8f0e115ab9975e6b53d380b9a97f611e4a271dd0889ade5808ce4da`;
machine SHA-256 `b0dccf58f12d2490e20b5c7cbe38f36d7c3ea1849ea0dba3420c47efc9943d3e`.
Installation: `/private/tmp/kegari-v0-output-matcher-evidence-installation-1.json`. V2 artifact:
`/private/tmp/kegari-output-matcher-source2-evidence-draft-v2`, FILES SHA-256
`6ffdd7dcea4e7f742c66ebb26605a3841640227c1f7f3aa329c32235b23a7353`.
Root read and verified the four-entry partial-only coverage review; no issue:
`/private/tmp/kegari-output-matcher-coverage-static-review-v1/REVIEW.md`, SHA-256
`c10d70c6b0f6d210d336eaeb2fa5e108fafa593e43dceccddd1f186238290c28`.
Exactly four reviewed entries are now appended, with every prior ledger value and
status preserved. New ledger SHA-256
`7dec2622cd47e32c4f204f70f67843fb89f68d8e739b9b0bb10fb963337be1d3`.
The collector's earlier auxiliary hash remains the actual pre-append ledger it
checked; this later append is separately verified, not backfilled. The Rust
ObservedFacts/TrialReport literal migration (`canary_output: None`) is recorded
in the planned breaking-change commit footer. At precommit inspection origin/main
also points to 768b125; no remote action was performed by this builder and no
published history is being rewritten. Matcher behavior has a separate policy/API
and rollback boundary from the preceding context-operand increment. The worker
regression retained uncertain synthetic broker evidence at
`/var/folders/s3/gnkb4nw50msf40dxc1jrpsyh0000gn/T/.tmpzJW7y2`; preserve it.
Agents prepare the bounded next closure design/evidence collector and a separate
approved coordination-substrate prerequisite in TEMP only. No current-source
or caller/authority/scorer change is authorized in those agents.

Earlier verified local commit: `1d482650b7b314325123c5b56d90cbf243df42d0` —
`feat(kegari-v0)!: retain private canary diagnostics`. All 24 intended files and
701 tested inputs match the commit and physical tree; every other path matches
parent f565b42. The tree was clean after verification. No commits were
consolidated or pushed by this builder. Verification:
`/private/tmp/kegari-v0-canary-commit-verification.json`.
On resumption before the E07 commit, both main and origin/main pointed to 1d48265, and the local
origin/main reflog records `update by push`. The actor is not inferred from that
metadata. The reviewed E07 extension used a new local commit rather than
rewrite that published checkpoint. No remote action was performed.

The E07 schema3 runtime path-operand extension is installed, locally verified and
committed as 768b125. Its original held source remains at
`/private/tmp/kegari-e07-native-context-operand-v1`.
FILES SHA-256 `218203b23a1ff5364f7a5158f9e72381a68516a26550d479ed8208610157c65a`;
patch SHA-256 `c8e46a6b4a4643434425d0ab217bbeb86112dc39e304cf99bca68f7842238428`.
Root verified all 27 artifact members/nine original bases and read every changed
existing-source hunk plus the three new modules, README and six proposed tests.
The independent implementation review found no material defect; root read the
full report and verified its files:
`/private/tmp/kegari-e07-native-context-operand-independent-review-v1/REVIEW.md`,
SHA-256 `b38831d52189b3b304acd166a261e986eb06edcbba4405d019c35506f536fb63`.
The exact 12 paths are now installed; originals/installation record are at
`/private/tmp/kegari-v0-e07-operand-installed-1`. The 704-input source1 snapshot
is `sha256:b6e6a5703dae82c6c996e7e63c753077ea403505b4c0581492cb4ef9260f5386`
at `/private/tmp/kegari-v0-e07-operand-source1.json`. The first all-target check
failed in 13.160s (exit101): verify_isolation received IsolationPlan instead of
ValidatedIsolation, and PreparedFixture's Debug derive required an operand Debug
implementation. Session50601 is closed. Raw failure SHA-256:
`4309d0a7b2dd1748d70477790214ea370dc003e14537cb95e4e5e8441a55a817`.
Root now validates that actual plan before passing it and adds a Debug
implementation exposing only the existing safe identity, never path/marker.
Originals and exact two-file diff are retained at
`/private/tmp/kegari-v0-e07-operand-compile-correction-1`.
Source2 all-target check passed in 22.743s, but strict Clippy failed in 15.309s
on one manual_range_patterns warning (exit101). That source was
`sha256:f9db610c888004e66e8fdf28d48b328428200901738ba6d826ffcf53b73f2565`;
check raw SHA-256 `3c0052f1d404d9af51c34d6b9670cccdf4ceddb47a27fbba2db053f2fb576f6e`,
Clippy raw `c2221bb435e5e082dae024a095e0c80da5b9e46f1b0b1ecaf22f1030c213868f`.
Session85500 is closed. Root changed only `1 | 2 | 3` to the equivalent `1..=3`;
original/diff: `/private/tmp/kegari-v0-e07-operand-clippy-correction-1`.

The three-path operator/source-preparer wording update is installed from
`/private/tmp/kegari-e07-context-support-wording-v1`; originals/record:
`/private/tmp/kegari-v0-e07-operand-wording-installed-1`. It preserves the three
false flags, exact recipe/scorers/profiles/default E05 behavior, and replaces
stale missing-schema3-code explanations with remaining matcher/actual-fact
scoring/native qualification limits. Root read the full diff and verified its
10 members/three bases. A first root AST comparison was too strict about two
added literal-string entries in evidence.limits; the corrected static inspection
normalizes only that string-only evidence list and string values, yielding
identical executable structure. No target tool/test failed in that inspection;
its record is `/private/tmp/kegari-e07-wording-root-static-inspection-1.json`.
The independent compile-correction/wording review found no material issue:
`/private/tmp/kegari-e07-operand-corrections-wording-independent-review-v1/REVIEW.md`,
SHA-256 `2d386bc5285dfa8850e62d5a528475f35888aec782473347d833f262151a7151`.
Root read its full report.

All 20 Python preparation tests passed without skips on the exact 349-file
copied suite in 1.843s (unittest 1.696s); input bytes/modes and installed source
remained unchanged. Session62482 is closed. Actual argv and relevant environment
overrides are now explicitly retained at
`/private/tmp/kegari-v0-e07-operand-python-1/tests-result.json`; raw log SHA-256
`f9cec178b97c9b48284db6aca67899a3a36a9e987005d128d9ab1968a79e96fd`.
This new record does not backfill the preceding Python run's missing env map.
No native/model/provider execution occurred.

Current source3 has 704 inputs frozen at
`sha256:2c69af7805a54b508f07ee4a1f5835cf2b993a3320e27a3c4df5cf784b709ed2`,
snapshot `/private/tmp/kegari-v0-e07-operand-source3.json`. Serial validation
session66949 runs strict Clippy, six new operand tests, whole evaluation/runtime
and ownership-return groups, broker components, seven compatibility targets,
formatting, full no-fail-fast suite and disabled Plan. Exact commands:
`/private/tmp/kegari-v0-e07-operand-source3-commands.json`.
Strict Clippy passed in 23.207s; the six new operand tests passed in 42.367s,
the evaluation group 67 in 2.797s, runtime 33 in 1.788s, owned-return two in
0.238s, worker-return one in 0.162s, broker eight in 1.865s, seven compatibility
targets 109 in 64.363s, and formatting in 1.598s. All nine completed records
retain unchanged source/toolchain observations. These focused groups overlap.
Full-suite validation passed: 984 tests passed, zero failed, two required native
checks ignored across 39 groups in 744.750s. Raw SHA-256:
`b80431a06a08b2bd71967e74e410e7f56d369ea1c500735d3aeca8704c47d950`.
Plan passed in 32.084s, preserving the exact preceding 28-variant/56-slot disabled
plan; raw SHA-256:
`7f0f96b570be6f7752a73ffd1f230d380b7dd3de5430b37c87d972639b1254b5`.
Session66949 is closed (exit0). Inventory passed in 0.048s and diff check in
0.019s, with unchanged before/after auxiliary hashes for the specification,
inventory tool, coverage ledger and operator document. All 13 final records
retain unchanged source/toolchain observations. Root independently compared full
test membership to the preceding canary run: exactly the six operand tests were
added; all prior cases and statuses remain, including both ignored native checks.
Root's exact Plan/membership comparison is retained at
`/private/tmp/kegari-v0-e07-operand-root-membership-plan-1.json`.
No native/actual-agent/human result is claimed. Independent installed-composition
review found no material issue; root read its full report and verified its three
artifact members:
`/private/tmp/kegari-e07-operand-installed-source3-independent-review-v1/REVIEW.md`,
SHA-256 `a48f9a9736fd1bc9daf3c4dd4f60f59db072001a805935f78d972ee1c618edb8`.
That review verified the exact 704 inputs, 12+3 path composition/corrections,
six focused results and four proposed evidence appends. It records that the full
result was absent at review time; later full/Plan success is separate evidence.
Root read the full immutable collector/command specification and report template,
verified all six members and the approved specification hash, and found no
material issue. Collector artifact:
`/private/tmp/kegari-e07-operand-source3-evidence-draft-v1`, FILES SHA-256
`2a6af95bf42461a681552aa026eacfff1b946fd4722669cdfd942f00b78d2fc8`.
Its first actual execution passed in 0.298s; record:
`/private/tmp/kegari-v0-e07-operand-collector-1-result.json`, raw SHA-256
`89d59efc5c9c5d48eec8d21ca4d9b69189266cc0c78a72dccf1d23b26000748f`.
Root then read the complete generated report and verified its actual source,
13 passing final/three preserved historical records, six added test memberships,
984/0/2 totals, 39 groups, exact Plan, and separate 20-test Python evidence.
Installed reports: [E07 context operand validation](validation/LOCAL-E07-CONTEXT-OPERAND.md)
and [machine evidence](validation/e07-context-operand-checks.json), SHA-256
`960f76ccc21cf21b9f7676ebda5ac3b9e336c992115823f88a957876eb2b7355`
and `85e0349f374316c0b5d85f7cb77bee256de6f3665cbbae6ac4c83b9973a2ab05`.
Installation and root review record:
`/private/tmp/kegari-v0-e07-operand-evidence-installation.json`.
The focused broker regression intentionally retained its uncertain synthetic
fixture at `/var/folders/s3/gnkb4nw50msf40dxc1jrpsyh0000gn/T/.tmpT0qY9m`.
It is preserved; neither cancellation nor successful test completion authorizes
deleting uncertain effect evidence. The full run does not expose that passing
test's captured retention path, so no additional cleanup claim is made.
Exactly four reviewed coverage entries are now installed from
`/private/tmp/kegari-v0-e07-operand-coverage-draft-1`; all prior ledger values and
statuses are preserved. The resulting ledger SHA-256 is
`4ccb7214b0b63edc5b2393d7311d6f60e76a7b60de8b8028d0dd696457eadf80`.
The collector's recorded auxiliary ledger hash identifies the prior ledger it
actually checked; this later append is separately verified, not backfilled.
The actual full/Plan results do not qualify E07 behavior.
The output-matcher/
scorer design is held at `/private/tmp/kegari-e07-output-matcher-scorer-design-v1`.
Root read its full design, API sketch and 25 proposed test oracles and verified
all five artifact members and 30 source pins. The independent design review
requires two corrections: public coordinates must exclude raw keys/IDs/error
text, and future grading needs an independently authenticated positive result
rather than only a no-match witness. Root read its full report:
`/private/tmp/kegari-e07-output-matcher-scorer-design-independent-review-v1/REVIEW.md`,
SHA-256 `7080d044c8c8a8fba53afca25bda67e6135bd2879bab9677dbb40f8b04e5246b`.
V2 is held at `/private/tmp/kegari-e07-output-matcher-scorer-design-v2`, FILES
SHA-256 `311a66485fd94722a9659cdafbb4e5fb0cdab3d9fc156c250fbc5b72a96d5c41`.
Root read the full revised design/API and all 25 proposed test oracles and
verified all 19 artifact members and five exact v1 bases. Independent rereview
resolved both D1/D2 without another material finding; root read the full report:
`/private/tmp/kegari-e07-output-matcher-scorer-design-independent-review-v2/REVIEW.md`,
SHA-256 `aa3d477a07eb9789f5ed107ab377f66afb605f132bc578f1e1f298d510448320`.
Its clarification is retained: marker-bearing provider labels test safe new
metadata without silently broadening the declared matched-content projection.
V1/v2 remain immutable, and their 30 source pins remain explicitly historical701.
The bounded diagnostic-only slice A was initially authored in TEMP against actual
704-input source3 bases, with no repository edits or Cargo in that agent. It
records actual decoded deltas/tool arguments/feedback matches with explicit
partial coverage. Finite negative and versioned
scorer behavior require separate exact reader/broker closure and scoped authority
observations; old absent state booleans will not be invented. Root flagged that
raw JSON pointers or item identifiers can themselves contain a marker and must
not leak through public match metadata. At that design checkpoint no installed
or validated matcher result was claimed; old scorers remain unchanged. The immutable
core candidate is held
at `/private/tmp/kegari-e07-output-matcher-slice-a-v1`, FILES SHA-256
`eb76408a602a20de7d253bc0c41fb8da4972d489acf4a352baeb0e5d31c4306c`;
patch `10bb6fb41b5767b4932d652797601be9a1c2adf1ca72089958f3dba26a56b67d`.
Root verified all 32 members, nine actual bases and exact worker-layer composition,
and read all existing-source diffs, the full 1,429-line output module and its
534-line test module. It includes policy2, original-owner output enrollment/storage,
four-form bounded matching, independent own-positive readback, safe optional
canary_output facts/report data and failure/archive plumbing. No complete-negative
state, grader bridge or scorer/fixture change is proposed. Fifteen tests are
authored but uncompiled/unrun: thirteen core, one scorer-load failure and one
actual duplex test. Two test issues and one production projection defect require
a separate correction before integration. MR-1 canonicalizes the actual owned
temporary roots before strict PrivateDir creation; MR-2 retains/asserts both the
failed preregistered piece and the later anchored error record. Root identified
MA-1: hashing callback-kind metadata splits an otherwise identical delta item's
stream when observation changes from unbound to established, hiding a split
positive. Git independently confirmed it. The correction must preserve per-piece
attribution, use the explicit thread/turn/item tuple, and isolate missing identity
with a gap rather than merge unknown items. No source/test execution failure is
claimed for these static findings. V1 remains immutable. V2 corrected all three
findings and added two delta-identity tests before installation; its 17 tests are
authored at `/private/tmp/kegari-e07-output-matcher-slice-a-v2`. FILES SHA-256
`7537eca54d78ab45ff1f8884f9435a4e53957b3582fd1712a137cc1e11ca28fb`;
patch `25b4e773606ac15ed80b980a4e02c2b2c86f462171d89931735fef5f82872268`.
Root verified all 23 members, all 11 composed paths and the exact v1 bases, and read
the complete correction. Both independent rereviews resolved their findings
without another material issue; root read them and verified their manifests:
`/private/tmp/kegari-e07-output-matcher-core-independent-review-v2/REVIEW.md`,
SHA-256 `122c250a4b3ecae885899d7c6e4f83f8e23779228e234cb8f7d195894c86e164`,
and `/private/tmp/kegari-e07-matcher-policy-report-independent-review-v2/REVIEW.md`,
SHA-256 `7b864265d1538dbbbc8f37832610fc6b44fb80ff28ae6a3562934ccea2bf1dff`.
Root read both independent reviews and verified their two-member artifacts:
`/private/tmp/kegari-e07-matcher-policy-report-independent-review-v1/REVIEW.md`,
SHA-256 `1ddcb012071fd1b4cb93b19c6222ff530ba7e62a522698d33d97d18672b65892`,
and `/private/tmp/kegari-e07-output-matcher-core-independent-review-v1/REVIEW.md`,
SHA-256 `6b28a348c68aff0f9a743098673d8d62182b046562347dc25cfd7e2b428746f5`.
No other material production finding was reported in their assigned scopes.
Persisted skip-none report bytes remain compatible; inspection JSON intentionally
adds canary_output:null for older reports and is not claimed byte-identical.
The operator-only draft at `/private/tmp/kegari-matcher-operator-docs-root-draft-1`
was not installed. Root doc draft2 is the installed version, with marker omission
narrowed to the canary_diagnostic field; exact SHA-256
`7729243781e28a692dc8d1f3b62031950d11f1e469837bcd7076016d01237fbd`.
Existing preparer/recipe/Python behavior remains unchanged.
The worker layer is separately sealed at
`/private/tmp/kegari-e07-output-matcher-worker-v1`, FILES SHA-256
`67b965a99588778a7c097586d330d0a538100da6f84602c06e5302c9f778493c`.
Root read its full README/diff and verified 11 members, two exact source bases
and the unchanged 21,111-byte prior test prefix, with no material finding in
that bounded layer. It is not a review of the unfinished core composition.
The duplex regression uses a synthetic component broker/session and actual
RpcClient framing; it creates no enrollment, runtime/native authority or result.

A later coordination-prerequisite plan is held at
`/private/tmp/kegari-native-coordination-prerequisite-plan-v1`, FILES SHA-256
`2ece079d01960a586feb88f48606ef61683d384ed83bb3e80c7af09e73575d0a`.
Root read the full plan and verified three members and all 74 input pins before
this progress update. It identifies repository-free coordination substrate and
retained-workflow preparation separately from parent authority/native caller,
full-context and scoring work. Continuation/cold recovery still requires genuine
original publications, joined lineage and fresh authority, never fixture JSON.
Root subsequently approved TEMP-only implementation of this bounded prerequisite;
Config sealed the candidate at `/private/tmp/kegari-native-coordination-prerequisite-v1`:
FILES SHA-256 `b0831d806d37c08a3a1e79da45d5cdb0c4e1298650fdd703d4e31c62ef8df5a1`;
patch `587d543330bd02bec8ba6fa90e72c8df0289d426dd669b3539b0c9da77a51666`.
Root read all 1,186 patch lines, the complete new modules/tests, recipe/docs/README,
and verified 28 members/seven exact bases/three new paths. At candidate sealing, five Rust and four Python component tests were authored
and unrun; subsequent actual validation is recorded below.
Root and reviewer identified a test-comment precision issue: refreshed-digest
binding/prior/declaration cases hit the exact recipe identity gate before topology.
Root prepared a comment-only correction with identical AST at
`/private/tmp/kegari-coordination-test-comment-root-correction-1`; no recipe gate
or assertion changed. The candidate and reviewed comment correction are now installed in10 paths;
seven originals/manifest are retained at
`/private/tmp/kegari-v0-coordination-substrate-installed-1`. Root read the complete
independent implementation review and verified all six members; no material
finding: `/private/tmp/kegari-native-coordination-prerequisite-independent-review-v1/REVIEW.md`,
SHA-256 `c2f3753aed0f67d9fa3408489900f24e3069c0d1a380e171b969e7af64bef62d`.
All-target check passed in25.463s on the709-input source1 snapshot
`sha256:42a2b226ada583cdc7ae41094bc888d826012dbfb309df9e45b2491311ddf92f`;
source/toolchain remained unchanged. Snapshot:
`/private/tmp/kegari-v0-coordination-substrate-source1.json`. Strict Clippy passed
in26.711s, the five new substrate tests in39.581s, evaluation tests in3.822s,
compatibility109 in66.049s and fmt in1.698s. Evaluation passed88 tests.
Full-suite passed1,006/0/2 across39 groups in767.515s; raw log SHA
`561f4d4846687c1a9931d47bca8ed5a2e6c7ebf1c45a52c1d16e56baf15b26e5`.
Root verified exact prior membership plus5 and unchanged native ignore reasons.
Plan passed30.072s with the exact previous envelope/data (28variants/56slots,
disabled dispatch), inventory0.042s and diff0.016s. All10 records retain unchanged
source/toolchain. Root checks are `/private/tmp/kegari-v0-coordination-substrate-root-membership-1.json`
and `...-root-plan-1.json`. Auxiliary before/after pin the same SPEC, inventory
tool, operator guidance and coverage ledger; the before observation was during
full-suite execution. An additional actual Rust CLI Plan consumes the retained
E01-derived suite successfully in0.476s, raw
`2e5d2917bd7fd124d82d6e79a869e91a27432ee68b54f905ddb48da162263c9e`.
Root verified its whole successful response equals the original except for the
expected derived suite_digest, which matches actual SOURCE-EVIDENCE. No source
or toolchain changed. The additional aux-final observation retains all4 pins. The
four new Python tests and all20 prior tests now pass with zero skips on the
exact350-file TEMP copy (2.755s). Actual E01 copy-only CLI passed in0.197s.
Root independently read its retained343-file output: exactly9 source payload
files removed,2 declared files added, the repository is empty, and only the
suite_version and coordination fixture_digest change in the manifest. All27
other variants,profiles,scorers,retained file modes and56 slots are unchanged.
All copied/installed input snapshots remain identical. Evidence:
`/private/tmp/kegari-v0-coordination-substrate-python-1`; independent verification
`cli-root-verification.json`. Output directory modes normalize to0700. No native
context/caller/execution or grading qualification is supplied.
The serial lint/focused/full/Plan checks are recorded at
`/private/tmp/kegari-v0-coordination-substrate-source1-commands.json`. This does not change
the frozen matcher source or add a native caller. The runtime-owned substrate
record remains the native producer's responsibility; fixture binding checks actual
exposed contract/empty-directory/workflow/context/isolation properties without
adding a capability accessor. This local validation does not establish native,
actual-agent or human qualification.
Root read the entire432-line original coordination evidence collector and report
specification, verifying9 members and56 input pins. Before execution, root found
its tree reader includes the root-directory entry while the Python runner's
snapshots omit it. The corrected collector must compare normalized snapshot
views while retaining actual root modes for SOURCE-EVIDENCE verification. V1
remains unexecuted; this is a static collector finding, not an application or test
failure. V2 also adds the separate actual derived E01 Rust Plan record.
The small coverage proposal retains all496 clause/136 named requirement rows,
statuses and prior evidence; exactly3 partial rows receive one evidence append.
Root read the independent review and verified5 members at
`/private/tmp/kegari-coordination-substrate-coverage-static-review-v1/REVIEW.md`,
SHA `18d98c7b040e6d1bab0cceff19ad264e27bca8ec008a19fc2fe3b1b2a51c6724`.
Draft2 preserves original Unicode serialization, with identical decoded content
to draft1; SHA `e46cd1bc322aaca8747282b9dd352085cb61434fbfbd4e5699d142fa584a4f2c`.
Root read the full v2 correction and verified10 members/65 input pins, unchanged
v1 and exact patch application. Root's literal diff-string checks used a different
valid blank-line alignment; subsequent in-memory hunk application verified all5
changed files exactly. Review record:
`/private/tmp/kegari-v0-coordination-substrate-collector-root-review-1.json`.
Collector v2 passed its first execution in0.261s, raw SHA
`1f279c3834ebb85795e4fb10fc146e715e1ac31cfe197fea89b1a0f4a50f490f`.
Source/toolchain remained unchanged. Root read the entire report, checked machine
counts/provenance and installed exact reports:
[local coordination evidence](validation/LOCAL-COORDINATION-SUBSTRATE.md) and
[machine evidence](validation/coordination-substrate-checks.json).
Report SHA `5df9c624f230c99549be184dd358a2fe93d173872147da9527d943f14f00e340`;
machine SHA `cfb2bbee368a86e1d1abb8cc21d84389a410026338b8e78a3742477ab4b79a18`.
Exactly3 reviewed evidence entries are appended with all statuses/prior evidence
preserved; ledger SHA `e46cd1bc322aaca8747282b9dd352085cb61434fbfbd4e5699d142fa584a4f2c`.
The report's earlier auxiliary ledger hash is retained as observed, not backfilled.
Installation record: `/private/tmp/kegari-v0-coordination-substrate-evidence-installation-1.json`.
The full collector v2 artifact is `/private/tmp/kegari-coordination-substrate-source1-evidence-draft-v2`,
FILES SHA `079d54f0ecf1eae4da1d344631c3691f588c181ca17009978655e9cae85070e1`.
At precommit readback origin/main also points to46c234f. No remote action was
performed by this builder; that history will not be rewritten. This coordination
preparation increment has a separate behavior/rollback boundary from matcher A.

The next finite-output closure design is held at
`/private/tmp/kegari-e07-output-matcher-slice-b-design-v1`. Root read all DESIGN/API
content and 22 proposed test oracles, and verified eight members and 71 actual
source/schema pins. FILES SHA-256
`cf7959b108d4627af0e8627072eb2c960e359d03fb5c5ac4f627fd346f01c256`.
The design requires original reader/frame membership, actual accepted-response
edges, complete server-attempt dispositions and the original live broker batch;
public DTOs/EOF/ACK cannot substitute. Unknown schema/item mapping stays unavailable.
Independent review identified a missing actual bootstrap metadata-response
settlement path: the public request wrapper otherwise drops every pending edge.
The correction is sealed in design-v2, FILES
`a5b733ca7a6bec1f83fff5900bf653f92993f3f000c79f4009d9c0d726350311`.
Root read the complete correction and independent rereview, verified all13
members/10 original bases and71 pins (69 unchanged,2 exact installed coordination
overlaps). Review: `/private/tmp/kegari-e07-output-matcher-slice-b-v2-independent-review-v1/REVIEW.md`,
SHA `f4cf8958d9b5dec4bfb9fe9a87b8811d071640e3672efa55b04fe24d23d4178c`.
No additional material finding; B-1 is resolved at design level. Root authorized
TEMP-only B implementation against source709; Codex owns composition and Config
its separate adapter/reader/bootstrap layer. All30 proposed test obligations
remain unrun. Typed scorer/C remains separate. Root verification:
`/private/tmp/kegari-e07-output-matcher-slice-b-v2-root-verification-1.json`.
Root also verified both original and revised independent-review manifests.
No public DTO/EOF/ACK/caller boolean may mint a private closure witness.

At the committed 1d48265 checkpoint, the reviewed canary v1/v2 composition, external defaults,
operator documentation and E07 source-data preparer are installed in 20 paths.
The 701 tested inputs are frozen at
`sha256:485e6d0b61c3e1f452b019ec89b23e9c92e72cb887838c141c1648e27be7b010`;
snapshot: `/private/tmp/kegari-v0-canary-source2.json`. All-target check passed
in 19.720s, strict Clippy in 26.822s, retained-storage 4 tests in 44.012s,
evaluation 61 in 2.549s, runtime 33 in 1.751s, broker 8 in 1.806s,
protocol 23 in 0.155s, seven compatibility targets 149 in 70.425s and
formatting in 1.575s. These focused counts overlap and are not a full-suite
result. All 17 new canary tests and four new retained-storage tests passed;
source and toolchain observations stayed unchanged. Full-suite validation passed:
978 tests passed, zero failed, two required native checks ignored across 39 groups
in 789.129s. Raw log SHA-256:
`1dd894a6df79ec8e4797e18532dc4e9cfd9541f1d869f1d6798190217a8511a8`.
Read-only Plan passed in 32.616s, preserving 28 variants/56 slots and disabled
dispatch; raw log SHA-256:
`cc8266400c586b49f3dcb4ea8c3a6cc646448024a2d5c2efd275bd4780e7d77d`.
Session75331 is closed (exit0). The first evidence-collector execution passed
in 0.171s; all 13 command records, exact 39 targets, 21 new and 35 retained
regression memberships, actual 20 Python cases and 349 copied/installed input
files were checked. Inventory passed in 0.046s and diff check in 0.020s.
Reports are installed: [local canary validation](validation/LOCAL-CANARY-DIAGNOSTIC.md)
and [machine evidence](validation/canary-diagnostic-checks.json), SHA-256
`832c08fd1251209b8c5707df9eec77acefa42933718ec78e6d4936f78c9e7dc9`
and `1862663796fa350b47e39aaf0284fe914d2eca58046a766487c7ed09e21655e9`.
Exactly four evidence entries were appended to partial coverage rows, preserving
all statuses and prior evidence; ledger SHA-256
`98a7dfb4604d601323641c3663ee67aa518f4958af2ad87d9fef9ad88cd27f77`.
No native/live/human qualification is claimed. The independent actual-output
audit found no material mismatch; root read it and verified its files:
`/private/tmp/kegari-canary-evidence-output-independent-audit-v1/REVIEW.md`,
SHA-256 `d627bbb962700e6da676f70e8bd51a1f221f2bbbff15f0f363741763c28aeea3`.
Root installed exactly the audited report bytes. All validation processes are
closed. This policy/private-storage/source-preparation increment is kept separate
from f565b42's broker-observation increment because it has a distinct behavior
and rollback boundary; no earlier commit is being consolidated.

Independent collector preflight found missing exact invocation/log bindings,
full-target/unique-case checks, source1/Python identity checks and auxiliary
inventory-tool/ledger pins. These were corrected before its first execution;
an introduced undefined Path name was also fixed before execution. The v4
collector is pinned at `2659a8b16964b796340119fbb73a4dba8c53b243c3c0187dd7ded62c4081e78d`.
Root read the final no-additional-finding preflight review:
`/private/tmp/kegari-canary-evidence-preflight-independent-review-v1/REVIEW.md`,
SHA-256 `cec4501a12db2be8be4e42802e39ef91fd46a9cfa8ee8680bfeed55c031bb3c8`.
The installed optional fields preserve omitted JSON. Rust consumers constructing
Variant, ObservedFacts or TrialReport with struct literals must add
`canary_diagnostic: None` when unused; this source migration will be identified
in the local commit's breaking-change footer.
The focused broker regression retained its owned synthetic uncertainty records
at `/var/folders/s3/gnkb4nw50msf40dxc1jrpsyh0000gn/T/.tmpENdHWq`.
The same full-run case also preserves its uncertainty by design, but the successful
captured test output does not expose its path. No cleanup claim is made for it.
All originals and exact source hashes are retained at
`/private/tmp/kegari-v0-canary-source-preparation-installed-1/INSTALL.json`.

The first all-target check failed (18.292s, exit101) because two existing
PreparedFixture unit-test initializers omitted the new optional canary field.
Root added only `canary: None` in those two initializers, preserving behavior
and assertions. The failing source1 snapshot, raw log and result are retained;
raw SHA-256 `c021786e8a1efdb5cd2ca71db4adeff4be3b30b3fca16ceaf0dcf0c337b57a8a`.
Correction originals/diff are at
`/private/tmp/kegari-v0-canary-source1-default-correction`; the actual failed
log hash in `/private/tmp/kegari-v0-canary-check-1-result.json` is authoritative.
No tests or Clippy ran on source1. Source2 check's raw log SHA-256 is
`0455b149f530497d8b813a341c01c3dcfa3cb26345ecbf20716fcd30cd7edf2f`.
The prior broker test layer is unchanged. The canary v2 independent rereview
found no additional material issue after C1–C4 corrections:
`/private/tmp/kegari-canary-diagnostic-implementation-independent-review-v2/REVIEW.md`,
SHA-256 `f65ac49249ac684619cc68e63d1dcebf25c9768171964483b570969af85c2fde`.
Root read the full correction and review, verified 33 members and all original
bases, and compared the operator documentation with actual policy/DTO behavior.

The E07 source-data tool passed all 20 Python tests without skips on an exact
TEMP copy in 1.739s. All 349 input files remained unchanged and now match the
installed evaluation tree byte-for-byte and mode-for-mode. Session80675 is closed.
Raw log SHA-256:
`f7f2be937f7e4e45acfbe050e614367931ce74848860b061906d71d72d63bbad`.
The separate retained CLI preparation passed in 0.301s with 20 recorded commands;
actual E07 base is `21bc1e88cef10799c0eed6fd8ecf841aba3eb91d`, head
`4a9fd6935fd53fcf16156aed23bb274d78b66e96`. Evidence and output:
`/private/tmp/kegari-e07-source-data-validation-1/retained-e07`.
SOURCE-EVIDENCE SHA-256:
`0f0a9dfca94b48cfff2e2477828f6dae87c91fbfedac719dcfa2ccff20ba0b50`.
Root's initial post-run inspection wrongly compared the compact CLI summary to
the entire evidence document and failed after the CLI's successful exit. That
inspection error is retained in `inspection-failure.json`; the original output
was inspected with the correct five-field projection, without rerunning it.
Runtime schema3/owner-path support remains missing; the output explicitly has
runtime_supported=false, dispatch_authorized=false and canary_materialized=false.
No fixture program or live model was run. The author output/evidence audit
found no material discrepancy; root read its full report:
`/private/tmp/kegari-e07-source-data-author-validation-audit-v1/REVIEW.md`,
SHA-256 `3f5e827bd5d6d21bef0bbe32eda7cd37dda872dd829240a232d2e3897afba7bc`.
It verified the actual local Git objects, modes, parentage, clean checkout,
absence of remotes/alternates and exact allowed 351-file derived suite.
This author audit is distinct from the preceding independent implementation
review. The test result record did not archive the environment map: all 20
non-skipped tests prove both conditional families were enabled, while the exact
copied-suite environment paths are recorded only in root's executed tool call.
The independent installed integration review found no material issue; root read
it and verified its files: `/private/tmp/kegari-canary-installed-source2-independent-review-v1/REVIEW.md`,
SHA-256 `5add59169250fa10fdb6c9891361d3fa5937de26412e6b8208e1794b7678253c`.
It verifies all 701 inputs, all 19 installation entries, the two test-only
source2 defaults, preservation of C1–C4, and all 344 shipped fixture/scorer/
manifest files against the preceding source. Separate TEMP work on schema3/
owned-path runtime support and an output-matcher/scorer design remains in
progress; neither changes this frozen validation source.

The following broker component checkpoint is committed and predates this source.

At that broker checkpoint, the reviewed five-path component test layer passed
all-target compilation (16.462s), strict Clippy (17.345s), all eight broker
component tests (36.012s), 95 integration tests (30.773s) and formatting (1.544s).
The integration targets contain 24 control-storage, 30 provider and 41 worker
tests. All three new regressions passed; all 696 source inputs and toolchain
observations stayed unchanged at
`sha256:f26dfec202ba3840ab7c81dda8c6979ee0abb94aeef11c67d7b0195dc258105c`.
Sessions8702/24399 are closed. Canary code was not yet installed then. Originals
and installation record remain in
`/private/tmp/kegari-v0-broker-components-installed-source1`; the exact evidence
bundle is `/private/tmp/kegari-v0-broker-components-verified-evidence-1`.
Reports: [component validation](validation/LOCAL-BROKER-CALL-FAILURES.md) and
[machine evidence](validation/broker-call-failures-checks.json). Their SHA-256
hashes are `930969c2f9da608590167294f6a29647805846aff21aa73a28306ba66120000a`
and `46eb7765bdd6cdd8694d1998580fb62a91f515902d87f3824525229d3df4f54c`.
No full-suite result is claimed for this test-only increment; the coverage ledger
is unchanged. The uncertain-effect regression intentionally retained its owned
synthetic records at
`/var/folders/s3/gnkb4nw50msf40dxc1jrpsyh0000gn/T/.tmpAZP6yJ`.
The independent installed/evidence audit found no material discrepancy:
`/private/tmp/kegari-broker-components-installed-evidence-audit-v1/REVIEW.md`,
SHA-256 `9b126f4a8194bcb3802e6c3cd885fbd54921be062588065e0fbb813857842061`.
Root read it and corrected the older checkpoint's time-qualified wording.
These directly related broker regressions are consolidated into
`f565b42d29c456189bd0525a0b0a505441879aff` —
`feat(kegari-v0): retain independent broker observations`. It amends only the
unpublished `316c362d86799ae066f48a772d0aa1c4d2256d5a`, preserved beforehand at
`refs/recovery/kegari-v0/316c362-broker-call-tests`. Parent998677a is unchanged.
All eight intended changed files and all 696 tested source inputs match the
resulting commit; every other path matches the recovery commit. The tree was clean
after verification. No other history was consolidated and nothing was pushed.
Verification: `/private/tmp/kegari-v0-broker-components-commit-verification.json`.
The committed broker results below remain evidence for their exact older source.

The broker-observation increment is locally verified: 954 tests passed, zero
failed and two required native checks were ignored across 39 full groups in
785.104s. All seven new tests and all 32 required named regressions passed once
in their actual full targets. The full raw log SHA-256 is
`b7dc6fddbefd8ee736c067fa23064d4b7da1013c54ab3be6c0a0cb8a82ffcf74`.
The dispatch-disabled Plan passed in 32.297s (28 variants, 56 slots, dispatch=false).
Session26323 is closed; no validation process remains running. Source and toolchain
remained unchanged. Native/live/human qualification remains incomplete.

Fifteen source/test paths (two new modules) compose reviewed v1/v2 onto committed transport source3.
Root verified all 158 candidate artifact members, reviewed the five overlap paths
and preserved the existing evaluator loop, transport collector and report prefix.
The six receipt/publication corrections are unchanged. Exact originals and merged
files: `/private/tmp/kegari-independent-broker-on-transport-source3-v1`; installation
record: `/private/tmp/kegari-v0-broker-installed-source1/INSTALL.json`.
The 694 current inputs are frozen at
`sha256:897b56c30c0a32c2ce82724c500e1897a44b09dfe7cbc85486d3b053ac5ea6a5`.
The serial pipeline completed. All-target check passed in
23.988s and strict Clippy in 24.922s, with source/toolchain unchanged. The five
broker component tests passed in 41.504s; Control 103 tests in 12.196s, runtime
33 in 1.759s, evaluation 44 in 1.951s, and protocol observations 23 in 0.165s.
The ten complete integration targets passed 195 tests in 93.884s, including both
new archive tests. Formatting passed in 1.529s. These focused counts overlap and
are not a full-suite total. All seven new functions passed their focused
targets and the final full run. Historical 947-test transport counts are separate.

The executed collector verified all 13 recorded invocations and the full test
memberships, exact source/candidate/history and only two partial ledger appends.
Inventory passed in 0.045s and diff checking in 0.023s. Root read the collector,
verified all 15 held members and 303 pins, and checked its 13 inherited helpers
against the previously reviewed version. A separate collector static audit found
no material issue, SHA-256
`a9a29b71834b351ad22bd7e9b4798e5c1e374f54efdd7230c6c0a3d79ec18eac`.
That auditor authored the broker candidate; this is an audit of the separately
authored collector, not a second independent review of the implementation.
Reports: [local validation](validation/LOCAL-INDEPENDENT-BROKER.md) and
[machine evidence](validation/independent-broker-checks.json). The exact verified
bundle is `/private/tmp/kegari-v0-broker-verified-evidence-1`. Both repository copies
match its COMPLETE manifest. Coverage SHA-256:
`fe0cac41325595a8acba2a035f19b3f95e5312842bac3430e3617b2e8e5b905a`.
Only two existing report/provenance rows received one partial entry each; statuses,
prior evidence and all other rows are unchanged. Ledger originals are retained at
`/private/tmp/kegari-v0-broker-ledger-finalization-1`. Committed locally as
`316c362d86799ae066f48a772d0aa1c4d2256d5a` —
`feat(kegari-v0): retain independent broker observations`. All 20 intended files
and 694 tested inputs match HEAD, with a clean tree after verification. No commits
were consolidated, rewritten or pushed. Verification:
`/private/tmp/kegari-v0-broker-commit-verification.json`.

The final artifact/evidence-scope audit found no material inconsistency:
`/private/tmp/kegari-broker-source1-final-artifact-audit-v1/REVIEW.md`, SHA-256
`d80eb9adcc660b17cc2b216ecc0a4685f74c463554c3ea31f60a5040169a165e`.
It verifies the exact outputs, 13 records, 32 memberships, historical failures and
two-row transform; it is not an additional independent implementation review.

This broker increment retains a separate local commit from transport: original
host calls and Control receipts have different producer paths and failure modes
from protocol streams, and the shared storage allowance and archive consistency
checks have their own component/compatibility evidence. This preserves both
validated boundaries for review without rewriting the earlier checkpoint.

Independent v1 review found B1 (observed success before encoded source-result
bound) and B2 (archive consistency omitted actual manifest comparisons). V2 fixes
both and adds real oversized-result/no-callback and contradictory archive cases.
Its independent rereview found no remaining material issue in the bounded scope:
`/private/tmp/kegari-independent-broker-observation-independent-review-v2/REVIEW.md`,
SHA-256 `e59a8f48802b2b5cd91dc0e078a6d44706811dc9608da49dba42624db90024fd`.
The installed-source integration audit also found no material issue:
`/private/tmp/kegari-independent-broker-installed-source1-integration-review-v1/REVIEW.md`,
SHA-256 `a95c3bb5618caa2aae2cdcd5b1ac191fcd444e03f9069212140fa9369ee4ce53`.
It independently checked the exact 694 inputs, five overlaps, one-Control
callback/lock ordering, docs and eleven-command pipeline.
Original membership, same-write anchors, shared finite capture allowance and exact
Control receipt snapshots remain diagnostic-only. ACK does not prove remote text;
missing/pending data stays unknown; no Complete or acceptance state is introduced.
At that preceding checkpoint, collector-enabled cancellation and actual
finish-write failure had not been exercised; the later component runs above now
cover those bounded cases. Registered native positive evidence remains
unperformed. Operator guidance records the qualification limits.
Canary policy addendum and independent review are complete. A default-omitted
per-variant sidecar can bind actual private materialization without changing the
shipped scorer/suite or existing authority. Named host-input absence stays
separate from unavailable provider discovery and unqualified output coverage.
Addendum SHA-256:
`1b3882c52ef3f50d1908290d2f3400b22f38f97822009235781540fbbd85ea50`;
review SHA-256:
`4ff5653c8c392d4134849646e9023236336a83bebd7c6a3435c9d5eb54de9c86`.
Root verified all 35 source pins and read both. TEMP implementation is assigned,
including original shared-budget capture, append-only actual binding, fixed-parent
path checks, cleanup-owner retention and final readback before work settlement.
No canary code is installed or executed.

Two further broker component regressions are assigned in TEMP from the reviewed
plan `/private/tmp/kegari-broker-cancel-finish-component-plan-v1/PLAN.md` (SHA-256
`cacbca55362618d6338521e160411431b9b6434c2c981bbedd7e49ab461d04ee`).
They will exercise actual call cancellation and a pre-write final journal refusal
with test-seeded diagnostic enrollment and a synthetic transport, retaining the
real host's registration-unavailable result. They must not manufacture native
capabilities or call the pre-write refusal an uncertain post-rename outcome.
The two tests are held only in TEMP at
`/private/tmp/kegari-broker-cancel-finish-components-v1` (FILES SHA-256
`4e3c375ccf41719428872eb6135262728135d873a42e6efd2bd1b6a726c0332e`).
Root read the complete new module, verified all 21 artifact members and confirmed
the exact extraction preserves the complete integration body. Historical drafts
counted 36 tests with a narrow regex; the actual body has 41 including five
parameterized Tokio attributes. No helper test was added or duplicated.
At initial handoff only standalone formatting and byte checks had run. Independent
implementation review found that failure cleanup can accept a nonpending Sent
receipt and discard unresolved-effect storage. A v2 correction is assigned to
require an explicitly settled, unheld receipt and empty retirement obligations;
uncertainty must retain the directory. V1 remains unchanged for review history.
The independent implementation review is
`/private/tmp/kegari-broker-cancel-finish-components-independent-review-v1/REVIEW.md`,
SHA-256 `54f8727639ac301d7f6ffaebe298fda1d9d13e5dc04477d18bd66601205e6f39`.
Its author wrote the plan, not the implementation; root read the full review.
V2 fixes BC-1 and adds a third actual-Control lost-response regression. Its
independent rereview found no remaining material issue in that correction:
`/private/tmp/kegari-broker-cancel-finish-components-independent-review-v2/REVIEW.md`,
SHA-256 `b4043e010c3c8f5c2df12eb76f744723d000ca5f2aa05b74d2ed8e9d73f23507`.
Root read the full delta/review and verified all held manifests before installing
the exact five-path composition. V2 FILES SHA-256:
`a7d366fc790c0a33b852559cc97cd3206e9aeeaabaf551fe0fd47d79c561968a`.
Compilation and focused/compatibility execution now pass as recorded above.
Current full validation remains the
committed 694-input source1. The approved test-only fixture
reuse moves the existing `tests/worker.rs` prelude into
`tests/support/worker_fixture.rs`, included by both existing integration tests and
the new private unit-test submodule. Existing assertions must remain exact.
If necessary for the reused nested support module, a minimal test-only crate
alias in `src/lib.rs` is authorized; no production capability/API is added.
The cancelled-call cleanup may prove the original Control receipt became terminal
and pending cleared; it cannot claim a direct JoinHandle/native join. An unexpected
cleanup timeout must retain the owned fixture rather than delete beneath it.

Canary external literal wiring currently has one known root-owned change after
integration: `tests/evaluation.rs` ObservedFacts needs `canary_diagnostic: None`.
The exact one-line uninstalled overlay is saved at
`/private/tmp/kegari-canary-external-defaults-root-v1` (FILES SHA-256
`f2ef64912f691e18332d2ef8581e07037cdf1a3bf6605027df7b4861bce269f1`).
Owned module defaults stay with the canary author. A separate TEMP design task is
resolving the later E07 native PR source/context and actual forbidden-path operand;
it may not invent E05 OIDs or claim the old ../scorer path reaches the new marker.
That design is now held at
`/private/tmp/kegari-e07-native-source-context-path-plan-v1/PLAN.md`, SHA-256
`4b3307ff233c8a13b09f50e072662f353e7e8ed5cc3275162d442147e083c5d6`.
Root read it and assigned independent review. It proposes E07's own local Git
history, an independent workflow without scratch-dependent check rewriting, and
a narrowly owned context operand before contract assembly. No proposed API,
source object, migrated fixture or E07 result exists from this design.
The independent design review found no material defect, subject to its explicitly
missing private operand producer:
`/private/tmp/kegari-e07-native-source-context-path-independent-review-v1/REVIEW.md`,
SHA-256 `3f9b2b60017fcd3ca37c6e663b761c2a220836ce19f9d6cf2d1823a990cfe99a`.
Root read it and verified all five plan members and 59 source pins. Only the
closed source-data recipe/tool/tests/docs portion is assigned in TEMP now; Rust
schema3/context ownership integration depends on the held canary implementation.
There are no new source objects, generated fixture output or test results yet.

Root also prepared design-based operator documentation in TEMP at
`/private/tmp/kegari-canary-operator-docs-root-v1` (FILES SHA-256
`c597d4ebb6eb8419fe0eb7361ab1dac1d0aacbc7736b1de48318dca75a555ff8`).
It is not installed or evidence of implemented behavior; adoption requires
comparison with the held canary code and its reviewed public schema.

The canary implementation is held at
`/private/tmp/kegari-canary-diagnostic-implementation-v1` (FILES SHA-256
`7914da197af3449da314aa4c0be39962f8d6e1d6ddbac442c66278286be847ae`).
Root read all eight existing-path diffs, both new modules and README/test limits;
independent v1 review is complete:
`/private/tmp/kegari-canary-diagnostic-implementation-independent-review-v1/REVIEW.md`,
SHA-256 `9ffeefc88c5ee58c9a9edfb675a98405e6ffd7b92170a4aa08ae12b084249147`.
Thirteen tests are authored, not compiled or run. Confirmed findings are rejection
of the actual fresh E05 `corrections: []`, a fallible request lock before retaining
the real admitted cleanup owner, and missing original file identity allowing
same-byte replacement. Bounded scan/deadline and form-allocation charging also
need correction, as do two narrow test-evidence claims. V2 is assigned in TEMP,
including a minimal crate-private same-creation protected-file identity helper;
existing storage checks/public behavior must remain unchanged. Root verified all
26 ARTIFACTS members and ten draft/eight base hashes. No canary code is installed.

The separate E07 four-path source-data implementation is held at
`/private/tmp/kegari-e07-source-data-preparation-v1` (FILES SHA-256
`bf848db263868a5ced381581d97a134e5f0ec6d2456b36daf9cb0b093ced2606`).
Root read its full code/test/recipe/doc delta and assigned independent review.
Seven new Python tests are authored, not run; original E05 helpers/tests remain
unchanged. Runtime schema3 and original-owner path support remain missing, and
the proposed tool output says so explicitly. No tool code is installed, no source
objects have been created and no fixture program has been executed.
Its independent static review found no material defect:
`/private/tmp/kegari-e07-source-data-preparation-independent-review-v1/REVIEW.md`,
SHA-256 `42b74b73ef0ac360f6c9d59fe66f5bb3df44b691f103cbada57a45460f12d487`.
Root read that review and verified all 13 held artifact members and actual bases.
Root is now running all original/new Python cases on a separate exact suite copy
with the four reviewed files overlaid in TEMP, both explicit suite inputs set,
and bytecode disabled. Session80675; records/input copy are at
`/private/tmp/kegari-e07-source-data-validation-1`. No repository source is changed.

Canary v2 is held at `/private/tmp/kegari-canary-diagnostic-implementation-v2`
(FILES SHA-256
`6c57af620933def0e9c03a0fc105bcbd9629da62b6d24e38b8fee72240ab3771`),
with a six-path correction and thirteen-path full composition. It reports 21
authored unrun components (17 canary/four storage). Independent rereview and root
inspection are in progress; no canary/storage changes are installed or executed.

Initial resume inspection confirmed the approved SPEC hash, `main`, an empty
index and then only this progress record changed since the broker commit. A read-only `ps`
inventory was denied (`Operation not permitted`); no retry or escalation was
made. Closed owned tool sessions remain the evidence for completed validation;
no global process-inventory claim follows.

The following transport checkpoint is committed and predates the current broker
changes. Its validation remains evidence for that exact source only.

Transport source3's full suite passed: 947 tests, zero failures and two required
native ignores across 39 groups in 800.907s. All 16 new tests, three extended
diagnostics, both previously failing tests and both other affected successor
fixtures passed once in their actual full targets. The 692 tested inputs remain
`sha256:e908c28aada5577ce254bcf2a78e2eb5e5c5c9b8e1950b4d01bccd20ce7e9c65`;
the toolchain also remained unchanged. Full log SHA-256:
`19d9de485c37ce593a61d505209d59750baf48da3bc9c84368944cb4277de409`.
The dispatch-disabled Plan passed in 35.245s with 28 variants, 56 slots and
`dispatch_authorized=false`. Session21501 is closed; no validation process remains
running at this checkpoint. Inventory passed in 0.041s and diff checking in
0.020s. Committed locally as `998677a6c9c49d4545fd523210646fb30010f460` —
`feat(kegari-v0): observe bounded evaluation transport tails`. All 22 intended files
and 692 tested inputs matched HEAD with a clean tree afterward. No commits were
consolidated, rewritten or pushed. Verification:
`/private/tmp/kegari-v0-transport-commit-verification.json`.

Source3 all-target check passed in 17.691s, strict Clippy in 18.110s, both receipt
regressions in 26.167s, exact validation-retry regression in 30.942s and exact chain
regression in 208.590s. Whole control_storage/reconcile targets passed 53 tests in
28.428s; formatting passed in 1.516s. Source1/source2 failures below remain retained.

Source3 includes the reviewed receipt-snapshot and fixture publication-window
corrections: six additional paths, one new module and two component regressions.
Exact originals/composition are at
`/private/tmp/kegari-v0-transport-source3-candidate`. Root and independent reviewers
read both corrections. Receipt review SHA-256:
`e9f49a988b8006d1506cd969dad8a460a60c17642b0e14fa16c35cadcd86a0c9`.
Fixture review SHA-256:
`8f0477e0c16ae4e23a14252ae05612ab08df7a7b14cbd707bfbab10544baefb3`.
No material static finding remains in these bounded changes. Receipt inventory is
captured under Control once and used as immutable data for ordinary/active
reconstruction; strict storage checks are unchanged. The fixture uses its existing
poll barrier only around each synthetic accepted-origin publication sequence,
with a real complete refresh and unchanged admission/mutation checks. Neither
review proves the exact cause of the earlier failed run or native qualification.

The source3 evidence collector completed successfully after root and independent
review. Root verified all 21 held members and 202 external pins. Independent review
found no material issue:
`/private/tmp/kegari-transport-source3-evidence-independent-review-v1/REVIEW.md`,
SHA-256 `28145ef6ed539f5356b49d029b81cdcb918dbd02dff00a57b6d641553eef90c1`.
The executed collector verified all 20 actual records, retained both source2
failures, derived final counts from full3, and verified exactly five append-only
partial coverage entries.
All five rows remain partial; previous evidence, statuses and ordering are unchanged.
Coverage SHA-256:
`fb108febf3e9ae629922e228d854fc738585710440a5494813376fd4daf64fa0`.
Reports: [local validation](validation/LOCAL-DECLARED-TRANSPORT.md) and
[machine evidence](validation/declared-transport-checks.json). Their SHA-256 hashes
are `c7048b70f2437d2e8114324a83d37ca6dd744798965fa343d18cfa6b710643c3`
and `9913659fd4b420cabbe9fae7a1de30c517b359e0045eff31cd8ec07ec6f69833`.
The original exclusive TEMP bundle is
`/private/tmp/kegari-v0-transport-verified-evidence-3`; its COMPLETE manifest was
verified before copying the exact reports. Ledger originals are preserved in
`/private/tmp/kegari-v0-transport-ledger-finalization-3`.
The final artifact audit found no remaining material inconsistency after the
progress record clarified that the executed collector verified the final results:
`/private/tmp/kegari-transport-source3-final-artifact-audit-v1/REVIEW.md`, SHA-256
`85363ad494ea698550e8617f29af1b165723bee87ea880b6e171d16056779d6d`.
A separate source3 integration audit also found no material discrepancy in the
six corrections or nine pipeline records, SHA-256
`8214663a225e55cf0bac4f47bcb2679b25d245213ae9976a27b60eb27b7d01a3`.

This increment retains a separate local commit: the new original-reader
lifecycle and receipt-reconstruction serialization have distinct runtime effects
and regression evidence from the prior observation-record checkpoint. Preserving
that boundary makes the locking correction and transport behavior independently
reviewable; the earlier validated commit will not be rewritten.

Broker v2 composition is being prepared in TEMP and is not installed or tested.
Canary materialization remains a reviewed design; its bounded implementation is
queued after the integrated source and broker correction reviews. The canary design's independent review found no
material defect within its bounded scope (SHA-256
`8939e5e2d3c72c25a3a9b59470cc17ead00121919ef29cdea7ae462e6468e35f`).
The exact policy input, attack placement and E07 source remain prerequisites;
current scorer/fixtures and all native/live/human qualification remain unchanged.

The following transport source1/source2 milestones are historical.

The next transport increment is installed and under validation. Eleven source
and test paths (two new) compose reviewed TEMP v2, v4 and v5, preserving the
committed report-test lint correction. All 67 held artifact members matched;
originals and exact composition are at
`/private/tmp/kegari-v0-transport-install-candidate-1`.
The 691 inputs are frozen at
`sha256:222456b52ef00b6027f84f691a2abfcc47c9ef2638b5bad2a7b519962800a862`.
Source1 all-target check passed in 25.271s. Strict Clippy failed in 18.403s on
two collapsible conditionals; session33820 closed before tests. Source2 changes
only those equivalent conditional expressions, without suppressions or assertion
changes. The exact before/after/diff are preserved at
`/private/tmp/kegari-v0-transport-source1-lint-correction`; standalone rustfmt check
passed. Source2's 691 inputs hash to
`sha256:6053568c0c7ca2a883132f12542bf142aedfc31ab18e561d58fc586b9d132a78`.
Source2 check passed in14.974s, strict Clippy in25.782s, all23 protocol observations
in44.872s, both deadline regressions in0.110s, all117 compatibility tests in65.497s
and formatting in1.515s. Source and toolchain stayed unchanged. Session95959 is
closed with exit101. Full `--no-fail-fast` finished in782.758s:943 passed, two
failed, two required native ignores across 39 groups. Its library completed421 passed and
two failed in377.27s. `exact_validation_retry_uses_native_grant_and_new_checks_without_join_fiction`
failed at continuation_tests.rs351 during source sealing with `unsafe_storage`
(owner-only/ownership/link check). `chained_runtime_retains_merged_contribution_through_deployment_only_predecessor`
failed at chain_fixture.rs394 with `observation_unavailable` (latest reconciliation
did not complete). All other targets completed; Plan did not run. The full log
SHA-256 is `548c1ca0b9b7a9d499add0f47dde092e2a1956d3ae8cba88d021232e3e24a3fc`.
No source or toolchain changes occurred during this run; no Cargo process remains
active. The source2 evidence collector remains held and unexecuted because this
failed run cannot satisfy its passing-report guards.

Independent diagnosis found a reachable receipt-read/atomic-replacement race in
ordinary and retained observation reconstruction: receipt inventories were read
outside Control serialization while effect writers replace those files. An old
descriptor may become unlinked before its strict owner/link check. This matches
the first error, but its exact lost path/metadata was not logged, so causation is
not asserted. A TEMP correction will capture one receipt inventory under Control
and pass it to both read-only reconstruction helpers. Storage checks remain
unchanged. The chain failure's underlying observation error remains unknown;
separate diagnosis is in progress. All failures remain part of the evidence.

The draft reviews found a lost pre-establishment completed-message projection and
a contradictory request-count tail label. V4 corrects both and restores exact
overflow no-write and actual message-payload corruption/readback assertions. V5
corrects only a new test's expected Conflict for a rejected terminal; production
classification is unchanged. All prior artifacts and findings are preserved.
The final narrow independent review found no remaining material issue within its
scope: `/private/tmp/kegari-declared-protocol-transport-v5-independent-review/REVIEW.md`,
SHA-256 `39988117027fea7dc59de7245eebc44356eef8c9883e02cfc180ff9462121d3a`.
Fourteen new component tests remain unrun. Clean read EOF stays diagnostic-only;
all capture remains incomplete. Operator guidance now describes actual bootstrap,
tail bounds and remaining limits. Broker read/effect work remains TEMP-only.

The following declared-observation checkpoint describes committed source, before
the currently unverified transport changes.

The declared observation increment is locally verified: 931 passed, zero failed
and two required native checks ignored across 39 groups in 801.345s. All 14 new
tests and both extended grading-error tests passed once in their full targets.
The 689 tested inputs and toolchain remained unchanged at
`sha256:07ccd31609ff5d4cb534a780f5e6dc3dc5869b8bdecd59f169cb81fdce3ffa4d`.
All-target check, strict Clippy, ten focused tests, 117 compatibility tests,
formatting and the 56-slot dispatch-disabled Plan passed. Inventory passed in
0.047s and diff checking in 0.021s. No validation process remains running;
session34634 is closed. Native, live-provider and human qualification is incomplete.

Four reviewed coverage entries are installed: all four existing rows remain
partial, with previous evidence, statuses and ordering preserved. Coverage SHA-256:
`7e6946ca588a5836d17e694c123d4cc0393850ff1502f01d7efb6b8f81ef1779`.
The independently reviewed collector completed successfully and verified all 12
invocations, including the earlier lint failure. Reports are
[local validation](validation/LOCAL-DECLARED-OBSERVATION.md) and
[machine evidence](validation/declared-observation-checks.json).
The original verified bundle remains at
`/private/tmp/kegari-v0-declared-observation-verified-evidence-2`.
The actual invocation checked that exact output path was absent, its parent
canonical `/private/tmp`, and outside the repository; this is a caller restriction,
not a generic TEMP-path guarantee in the script. The machine archive adds the
later collector review and invocation constraint without changing any observed
result. Review SHA-256:
`d78d7c735d491aa168cd83a18c070b4cef5c6cd4b3eecfb64cd2fd3bc71d86df`.
The increment is committed as
`102984e2239114d6261e2383da589b50510b9371` —
`feat(kegari-v0): retain declared evaluation observations`.
All 21 intended files and 689 tested inputs matched HEAD, with a clean tree after
verification. No commits were consolidated, rewritten or pushed. Verification:
`/private/tmp/kegari-v0-declared-observation-commit-verification.json`.
The final independent artifact audit found no material inconsistency; its SHA-256
is `70ab0db05208fbdcf9299f1aba47688edbf6c07fa510cb6a646c721aef736462`.
These evaluation diagnostics have distinct consumers and qualification limits from
the prior parent effect-history change, so the increment retains its own
validation checkpoint and commit. Post-commit work is limited to this progress
record and review of the uninstalled transport/broker drafts described below.

The reviewed composition installed 16 files, including six new modules. Exact
original/draft hashes and originals are retained at
`/private/tmp/kegari-v0-declared-observation-installed-source1`.
Source1's 689 inputs were
`sha256:e053140cbf917609dee9254fa8ad3c33eb6ddb5fe656a61f2a7d6452251457ee`.
Its all-target check passed in 24.980s; strict Clippy failed in 23.156s on one
needless borrow in a new archive test. Session66513 closed without running tests.
Source2 removes only that borrow; rustfmt made no additional change. Production
behavior and all test assertions are unchanged. Originals and exact one-line diff:
`/private/tmp/kegari-v0-declared-observation-source1-lint-correction`.
Source2 check passed in 0.491s, Clippy in 3.468s, nine protocol tests in 45.294s,
one boundary test in 0.219s, and four complete compatibility targets in 65.469s
(32 protocol, 29 evaluation, 15 report, 41 worker). Formatting passed in 1.533s
and Plan in 35.615s. The full log SHA-256 is
`8f91abe3dc474ed856e0206b650995414752d9baf7d74de2f4ff5ca604cd3e92`.
The exact serial commands and immutable results use the
`/private/tmp/kegari-v0-declared-observation-` prefix. The independent source/test
inventory confirmed all original assertions and the one-line lint correction.

Both source reviews found no material defect within their static scope. Protocol
review SHA-256: `266ec493fb7b165262783814710d8c16dbf7a32c6089bece4e114dbe797dfb2f`.
Ordinary workers never enable raw capture. Declared Independent trials retain
bounded protected records; reports contain references/digests. Capture stays
explicitly incomplete because bootstrap and transport-tail coverage are missing.
No scorer, fixture, tool grant or qualification changes through these diagnostics.
Operator documentation explains both optional observations and their limits.
A separate TEMP draft addresses bootstrap, matched responses and actual reader
EOF while retaining incomplete capture; it is uninstalled and unverified.
The same-owner broker read/effect inventory design passed independent static
review and is being implemented in a separate TEMP draft. Missing receipts stay
unknown, and acknowledgements do not establish remote-content truth. Neither
future draft is part of this tested checkpoint. Descendant containment remains
an actual code gap requiring a permitted ownership mechanism; the existing
`process_tree_unverified` refusal and native prerequisites remain unchanged.

The Control increment is committed as
`1e14788bddababfcd065cdfc9c7ef5dc575044d0` —
`feat(kegari-v0): retain indexed parent effect history`.
All12 files and683 tested inputs matched HEAD; the tree was clean before the
observation installation. No commits were consolidated, rewritten or pushed.
Verification: `/private/tmp/kegari-v0-control-index-commit-verification.json`.
The following Control checkpoint is historical; its commit is recorded above.

Current Control source3 is locally verified:917 passed, zero failed and two
required native ignores across 39 groups in814.456s. All38 new names passed once in
the full --no-fail-fast log. All-target check, strict Clippy,42 focused results,
37 compatibility tests, formatting and the56-slot dispatch-disabled Plan passed.
All683 source inputs remained unchanged at
`sha256:e6ae6dea17e4c78d1d112ef0e9bde39123190e17b3c3c930860141ed86c7185f`.
Session59032 is closed. The independently reviewed v4 finalizer completed:
inventory passed0.033s, diff passed0.014s, and the collector verified all16
invocations and retained both earlier failures. Seven reviewed coverage rows gained
eight partial entries; exactly four not_assessed statuses became partial. No row
became passed. Reports are [local validation](validation/LOCAL-CONTROL-EFFECT-INDEX.md)
and [machine evidence](validation/control-index-checks.json). The original collector
bundle is preserved; the machine report adds five separately hashed later reviews
without changing observed results. The authorized local commit is recorded above.
The Control reservation/reconciliation mechanism and paired-history schema form a
separate coherent change from the prior retirement producer; preserve its own
validation checkpoint in a distinct commit rather than consolidating the two.
Native/live/human qualification is incomplete.

The unexecuted finalizers retained three independent findings: partial writes could
escape rollback; one cleanup error could prevent other cleanup; and a late stdout
error could leave COMPLETE despite rollback. V4 uses flushed atomic ledger
replacement, complete exclusive report publication, independent owned cleanup and
a publication boundary before final checkpoint/stdout. All findings were rereviewed
as resolved before execution. No multi-file crash atomicity is claimed. Finalizer:
`/private/tmp/kegari-v0-control-index-finalize-3-v4.py` (SHA-256
`06de93bb6ed8d7b6aa61966daa478f5ddf518a2b8b98b833047883549231ae17`).

The initial reviewed Control owner-effect index v5 and paired parent history
consumer v1 installation changed eight files (four new), after the verified
parent-retirement commit. Its683 source/test/asset inputs were frozen at
`sha256:81bce073afb21f17c3cb6b9ca6961714bbe89e9629838a7b3b6432fa47f40ea5`.
All exact original and layered draft hashes matched before installation; originals
and manifest are at `/private/tmp/kegari-v0-control-index-installed-source1`.
The serial pipeline `/private/tmp/kegari-v0-control-index-checks-1.py` is starting
all-target check, strict Clippy, the three focused filters, four Control/runtime
compatibility targets, formatting, full `--no-fail-fast` suite and the disabled Plan.
Source1 all-target check passed23.498s; strict Clippy failed16.631s on the
240-byte ParentEffectHistory variant. Session42887 closed without running tests.
Source2 boxes only that payload and adjusts its constructor/comparison uses;
serialized fields and test assertions remain unchanged. Exact original bytes and
five-line correction are at `/private/tmp/kegari-v0-control-index-source1-lint-correction`.
Its683 frozen inputs hash to
`sha256:6bc3de92d1c0477b07896748ee0b98a8bd38a91e367c7e094df039e89a1c94bb`.
Source2 check passed14.330s and strict Clippy24.230s. Its index filter passed34
and failed one in46.427s: the new deletion/substitution fixture restored a receipt
whose callback result was Null; ordinary Option<Value> readback makes that None,
so the unchanged reader correctly refuses its missing terminal observation.
Session17882 closed; no later source2 checks ran. Independent review confirmed
the diagnosis. Source3 changes only that callback to explicit component result
JSON; all assertions and production guards are unchanged. Formatting made no
additional changes. Exact originals/diff are at
`/private/tmp/kegari-v0-control-index-source2-fixture-correction`.
Its683 inputs hash to
`sha256:e6ae6dea17e4c78d1d112ef0e9bde39123190e17b3c3c930860141ed86c7185f`.
Source3 check passed14.354s and strict Clippy15.279s. The index filter passed35
in16.851s, identity passed1 in0.097s and parent passed6 in0.145s. All38 exact new
qualified names passed once across the42 focused results (four are existing tests).
All snapshots/toolchains are unchanged. The four compatibility targets passed37
tests with no failures/ignores/filtering in42.948s; formatting passed1.473s.
The full source3 suite passed917/0failed/2required native ignored across 39 groups
in814.456s. Plan passed32.197s with56 slots and dispatch false. All38 exact new
names passed once in the full log; source/toolchain stayed unchanged. Session59032
is closed. Source1/2 failures remain retained. Native qualification is incomplete.
The boundary/protocol observation work remains separate, uninstalled TEMP work.
Its explicit declared-evaluation opt-in is off for ordinary workers; the private
trial declaration binds suite/fixture/trial/operation identities. Raw bounded
arguments/messages stay in protected evidence; reports contain references/digests.
Static review found bootstrap DenyTools and unread trailing-frame boundaries, so
capture must remain incomplete without actual full channel drain/EOF evidence.
Processed callback counts cannot prove absence. No scorer, fixture or qualification
is changed by that draft; independent review is pending its immutable handoff.

The parent-retirement increment is committed as
`cf6b93474835710be4fba7691c6f43934a7f9a7f` —
`feat(kegari-v0): retain authenticated parent-run retirement history`.
All25 intended files and679 tested inputs matched HEAD, with a clean tree before
the next increment. No commits were consolidated, rewritten or pushed.
Verification: `/private/tmp/kegari-v0-parent-retirement-commit-verification.json`.
A final independent artifact audit confirmed the raw totals, exact source and
partial coverage without material inconsistency. Its review is at
`/private/tmp/kegari-parent-retirement-final-artifact-independent-review-v1/REVIEW.md`
(SHA-256 `dc80aae70a282bef62b16c6974412d555350b31b93292c4dd339dc15c89558b0`). Historical checkpoint follows;
its uninstalled/pending statements describe the state before the new increment.

Current parent-retirement source8 is locally verified: 879 passed, zero failed,
two required native checks ignored across 39 groups (716.047s). All-target check,
strict Clippy, formatting, all 21 delivery tests, inventory, diff and the
56-slot dispatch-disabled Plan passed. All 679 tested inputs remained unchanged.
The final collector verified 38 invocations, preserved every earlier failure,
and installed only eight reviewed partial coverage rows with eleven added entries.
Reports: [local validation](validation/LOCAL-PARENT-RETIREMENT.md) and
[machine evidence](validation/parent-retirement-checks.json).
Session75488 is closed; no validation process remains running. This increment
is committed as recorded above. Native/live/human qualification is incomplete.
The reviewed Control membership index/consumer and authority-boundary observer
remain uninstalled TEMP work; they contribute no evidence to this checkpoint.

The verified native evaluator checkpoint is
`ddc44eaf35d3865640c8ba615c3d0877292c917a` —
`feat(kegari-v0): run explicit owned native evaluation trials`.
All 61 changed files and 672 tested inputs matched the commit; the working tree
was clean afterward. No commits were consolidated, rewritten or pushed.
Verification: `/private/tmp/kegari-v0-native-evaluation-commit-verification.json`.

Parent-retirement source1 is now installed from the independently reviewed
15-file composition, manifest
`59a386b092e23e6b31c593fc77f29e24ae4b1f0d31a1ca932505b917bd24aae0`.
Exact current bases/new paths and copied bytes were verified. Originals are at
`/private/tmp/kegari-v0-parent-retirement-installed-source1`. Three clean merges
preserve all native evaluator changes; two separate reviewed corrections validate
ordered artifact bytes and hash/decode each journal event from the same read.
Source1 all-target compile failed in 14.677s: the carry-history import pointed
one module too high and three new test vectors inferred unsized strings. Source2
corrects the import path and adds explicit Vec<String> types; production behavior
and assertions are unchanged. Original bytes and exact four-line correction are
retained at `/private/tmp/kegari-v0-parent-retirement-source1-compile-correction`.
All-target check then passed in 25.492s. Sessions86718 and16454 are closed.
The 678 current source2 inputs are at
`sha256:82931aec917dc5015df556a790998b1038b7e18af916a832367a9d5a9a190271`.
Source2 strict Clippy failed in 20.451s on a needless clone in a one-element test
slice. Source3 uses slice::from_ref; all-target check passed in 13.891s and strict
Clippy in 18.964s. Three origin regressions passed in 40.411s. The retirement
filter passed ten tests and failed one protected-file inventory test in 12.539s.
That test exposed a production mismatch: both new history modules requested16MiB
reads from PrivateDir, whose existing maximum is8MiB. The rejection occurred even
for small or absent files. Source4 lowers both call-site limits to8MiB, preserving
all storage checks, and formats the new test assertion. Original source3 bytes,
runner and exact diff are at
`/private/tmp/kegari-v0-parent-retirement-source3-bound-correction`.
An independent bounds audit found no further incompatible bound in either module
or its adjacent chunk/journal paths. Sessions47919 and70952 are closed; no full
source3 run occurred. Source4 has678 inputs at
`sha256:6f46bf53dd1a8ded600f8a00b29d952cf63f1b074a8a9b927532a3eef3b867d7`.
Source4 all-target check passed in14.036s, strict Clippy in14.943s, all3 origin
regressions in7.964s and all11 retirement-filter tests in12.451s. The nine new
regressions are included in those filters; five existing related tests also ran.
Formatting passed in1.446s. Full source4 failed after353 library passes and one
failure in450.256s (unchanged source digest). The existing corrected runtime
assembly fixture failed during integrated_worker_assignment: new provenance
capture requires the delta's merge receipt, but its synthetic upstream fixture
hard-coded merge_request818 without producing that receipt. Production validation
must remain strict; a genuine Control/provider-fixture effect correction is being
prepared and independently reviewed. The full failure log and result remain at
`/private/tmp/kegari-v0-parent-retirement-full-4{.log,-result.json}`; log SHA-256
`b015309204b29c3bc5160cf8f2267c79607dc719a4a966d5f08283734ed8ea32`.
Session67031 is closed. Later full-suite groups and Plan did not run. The report
collector correctly refuses qualification; no current full-suite or native
qualification is claimed.
Source5 installs only the two independently reviewed fixture corrections from
`/private/tmp/kegari-parent-retirement-merge-fixture-correction-v1`, manifest
`2eb5064fc0e1c9e58b1dfd078f669b7442ddd68e8d7004d694c2cfd16dd3cf75`.
The real typed merge broker/Control path supplies the actual receipt through the
synthetic provider; upstream review/check/staging qualification stays synthetic.
The existing corrected assignment test additionally deletes that receipt, restores
its exact envelope, and requires refusal plus unchanged Control inventory before
its original positive assertions. No production guard or test assertion was weakened.
Originals, exact diff and coordinator review are retained at
`/private/tmp/kegari-v0-parent-retirement-source4-fixture-correction`.
All678 source5 inputs hash to
`sha256:5e0cbd876b4b666c723b4ea9fdd4b4c0b94a2a5a49312b0f217e7dbc4783fa4b`.
All-target compilation passed15.664s and strict Clippy16.461s. All three successor
scenarios passed250.528s, all three origin tests passed0.343s, all eleven retirement
filter tests passed12.975s, and formatting passed1.542s. The full suite failed after353 library passes/one failure in415.778s on unchanged
source5. The corrected assembly regression passed. The chained successor failed
at release_wire.rs2006 during actual capture_delta: owned child OS identity could
not be established. Its focused run had passed; this is not accepted as a lucky
rerun. Full5 log SHA-256 is
`2ab5384bd433baf02e2bc8129c777744b6e3782746b5c77355504631841af4b4`.
Session69270 is closed. Later integration groups and Plan did not run. Root and
an independent agent are tracing the existing immediate identity/terminal fallback.
The one-shot retained-child scan can lack an exact identity; the existing Git
fallback accepts only immediately observed actual terminal/group closure. A bounded
wait on the same unreaped owned child is under review; exact OS failure cause is
not established by the generic error. No denied native/global probe was retried,
production guard weakened or unmodified lucky rerun started at that diagnosis.
Source6 now integrates the independently reviewed three-file startupv3 correction
from `/private/tmp/kegari-owned-child-startup-exit-v3`, manifest
`f3e417a09a9ad8a766e5bc9773ba25732474cef9e727272fcbcae58b9832598b`.
It waits only for the same unreaped owned Child's actual terminal/group closure,
at most100ms capped by original command/operation deadlines, after identity
unavailability. It retries no ProcessProbe and sends no stdin during uncertainty.
Other capture errors retain their existing immediate fallback. A review finding
was corrected before integration: the command clock begins before startup and is
also enforced at entry/start/after ownership lock/immediately before spawn. Seven
actual-owned-child component tests distinguish missed capture, timeout/cancellation,
no-spawn expiry, post-spawn deadline and withheld input without a Running identity.
Native boot identity and detached containment remain unqualified. Originalsource5,
exactpatch and artifact provenance are retained at
`/private/tmp/kegari-v0-parent-retirement-source5-startup-correction`.
All679 source6 inputs are
`sha256:98fb015e79d1af187df46edc39a67585499ba3b83e03b0995bebb06ef977c9c2`.
Source6 all-target check passed15.781s; strict Clippy failed9.319s on an
unnecessary mut at git_startup_exit_tests.rs133. No later source6 tests ran.
Session90899 is closed. Source7 removes only that mut; original bytes and exact
diff are at `/private/tmp/kegari-v0-parent-retirement-source6-lint-correction`.
Its679 inputs hash to
`sha256:fb7ed9a0872dac4f823c704b4712f05c280a8aa934c659aaacf9e5f785e77d89`.
Source7 all-target check passed14.411s, strict Clippy15.415s, all seven startup
regressions24.881s, and all46 Git/process tests across three complete groups
108.964s with no ignores. Origin passed3 in0.340s; retirement passed11 in12.702s;
formatting passed1.470s. All recorded source/toolchain snapshots are unchanged.
Full source7 failed in491.551s on unchanged source/toolchain. All361 library
tests passed, including all three successor fixtures. Later tests/delivery.rs
passed20 and failed parent_slice_uses_actual_production_and_preserves_dirty_operator_checkout
at1785: new origin capture requires the exact workspace namespace, but the
fixture constructs its protected delivery directory with the generic leaf records.
Root and independent review are comparing that fixture to production construction;
namespace validation remains strict. Full7 log SHA-256
`8e3791f47f0045663904a61d6632fdffc469b271f531bd9f0ca5853495a30326`.
Session42895 is closed; later groups and Plan did not run. No source7 collector,
finalizer, coverage update or final report ran. Existing source7 success templates
are now obsolete and remain contingent/unpublished.
Source8 installs only the reviewed delivery fixture namespace correction from
`/private/tmp/kegari-parent-retirement-workspace-fixture-correction-v1`, manifest
`eff38a2b48fa6289ce4caeb9672c5f8ca13b46256ff314a4d0bcdc6435b86fdf`.
The constructor now uses PrivateDir.child(actual workspace UUID), exactly as
production RuntimeDelivery does. All dirty-index/source/candidate/admission
assertions and production guards are unchanged. Root and independent caller audit
found no other affected integration caller. Original bytes, patch and reviews are
at `/private/tmp/kegari-v0-parent-retirement-source7-fixture-correction`.
Its679 inputs hash to
`sha256:581ef09275d7b370bbfc4dba4382e1e69cca75f3cb112fc5ffd4282193eaecb1`;
independent comparison verified only tests/delivery.rs changed from source7.
Source8 check passed in 1.199s, strict Clippy in 0.828s, all 21 delivery
integration tests in 19.664s, and formatting in 1.422s. The previous namespace
failure now passes with its original assertions. Session75488 completed: the full
suite passed 879 tests with zero failures and two required native ignores across
39 groups in716.047s; the 56-slot dispatch-disabled Plan passed in31.104s.
All sixteen new tests, three successor fixtures and the delivery regression passed
on source8. Unchanged focused source7 passes remain historical evidence.
Final source8 remains frozen; its local report is now published. Native, live
integration and human qualification remain incomplete. The evidence
collector's exact auxiliary key requirement now binds SPEC, contract_inventory.py
and coverage.json. Independent source5 evidence review found no remaining material
issue within its scope; the final source8 success claims now rest on actual full
results and independently reviewed source8 evidence collection.
The source5 finalizer remains unexecuted and obsolete. Source7 collector/render
drafts are independently reviewed at
`/private/tmp/kegari-parent-retirement-evidence-source7-independent-review-v1`;
no additional material finding was confirmed. Root prepared finalizer7, whose
independent review found that a later failure could leave prematurely finalized
coverage. The unexecuted corrected
`/private/tmp/kegari-v0-parent-retirement-finalize-7-v2.py` validates exact commands,
snapshots/logs and rendering before writes, then restores its own ledger and
preserves unpublished outputs if a subsequent step fails. Independent v2 rereview
confirmed the original finding resolved; crash-atomic publication is not claimed.
Source8's separately reviewed guarded finalizer subsequently completed. It
installed the final report, machine evidence and only the eight approved partial
coverage rows; inventory and diff checks passed. All38 invocation records and
all earlier failures remain archived. The producer records actual assignment, preparation and
original joined parent-run provenance; it cannot reconstruct worker/exit/current
authority from historical JSON. Its retained receipt-scan
schema explicitly cannot prove completeness before first capture. A separate
bounded Control index implementation is held at
`/private/tmp/kegari-control-owner-effect-index-v1`; it remains uninstalled.
Independent review found an effect-owner-field removal could bypass membership
validation at async launch/ACK (including an absent-marker cold downgrade), and
a late effect settlement after a retirement-save failure could prevent cleanup
retry. Both findings are recorded in
`/private/tmp/kegari-control-owner-effect-index-independent-review-v1/REVIEW.md`;
v2 corrections are held at `/private/tmp/kegari-control-owner-effect-index-v2`,
manifest `35d713adf0393e624870c6169b5e1582602a9990606180df8299797b9d7e6b0d`.
Root read both complete layers and verified all bases; independent rereview found
the earlier two defects corrected but a further same-host remint seam: deleting a
known owner's receipt/index files could allow its UUID to be readmitted, replacing
retained anchors. A v3 pre-write known-identity guard corrected that remint. Its rereview found an
error path narrowed existing resource holds; v4 unions all prior targets across
four error-only sites and tests actual pending-A/already-active-B exclusion after
exact receipt restoration without ACK. V5 fixes two test usize→u64 arguments.
Latest held composition is `/private/tmp/kegari-control-owner-effect-index-v5`,
manifest `e867c1ac51b8f8e9dd677f8a5a728ea6230365d947d82f3560c8286127fd058d`.
All30 authored tests remain unrun. Root read every layer; independent v5 review
found no remaining material issue in scope. No Control-index layer is installed
or Cargo-tested.
The boundary observer's12-file implementation is separately held at
`/private/tmp/kegari-independent-boundary-observation-v1`, manifest
`026b8e7a77badc46f3743e9e6a4fb3da7d4285e876f68093f97baac8dbcd8bff`;
it is uninstalled; independent review found no additional material defect, and
compilation/execution remain pending. The parent
report/collector independent review is retained at
`/private/tmp/kegari-parent-retirement-evidence-independent-review-v1`. Its two
unsupported lifecycle coverage mappings (§8.3 retention and §11.5 preservation)
remain unchanged from the base ledger. The corrected proposal at
`/private/tmp/kegari-parent-retirement-coverage-correction-v2` (eight rows, eleven
new partial entries) is now installed and verified. Next: commit this verified
parent-retirement increment, then integrate and validate the held Control index
and consumer. A paired Control-index consumer design
was read in full at
`/private/tmp/kegari-parent-retirement-effect-index-consumer-design-v1/PLAN.md`.
The independent design review found no material gap. The six-file implementation
is held at `/private/tmp/kegari-parent-retirement-effect-index-consumer-v1`,
manifest `1771532aad9e71bdd15e43a13cb60411f270e50498138478633dcf40e5f5ebc2`.
Root read the full patch and verified all held/current bases: one serialized batch,
strict indexed membership, explicit legacy/mixed provenance, unchanged unresolved
owner/join/process guards, and preservation of existing schema-1 signed bytes.
Eight authored component/schema tests are unrun. Independent implementation review
found no additional material issue at
`/private/tmp/kegari-parent-retirement-effect-index-consumer-independent-review-v1`.
The source8 collector/render and guarded finalizer also passed separate static
reviews; finalization completed only after full8 and Plan passed with unchanged
source/toolchain identities. No Control index or consumer is installed or
contributes current evidence. The PR-attack observation design at
`/private/tmp/kegari-native-pr-attack-observation-design-v1/DESIGN.md` was read in
full; independent review and a first actual authority-boundary observation slice
are proceeding in TEMP. Scorers, fixtures, grader versions and native qualification
are unchanged.

Native source14 installs the reviewed native evaluator caller, literal PR text,
exact trial subjects, owned failure cleanup, grading-error evidence, first native
E05 context fixture, source-preparation tool and operator guidance. Its 672 build
inputs are unchanged at
`sha256:6f962a238f28a5c05a6fad1fc4f90226d4d638512fee109b782114bb6e2e0f8e`.
All-target check passed in 14.425s, strict Clippy in 14.666s, and all 13 Python
source-tool tests in 0.780s. Those tests include actual disposable local Git data
preparation; they do not qualify native execution. Retained source preparation
also passed in 0.251s and reproduced the base/head below. Output is owned local
fixture data at `/private/tmp/kegari-v0-native-evaluation-source-output-14/prepared`.
The final Rust suite passed **863 tests, zero failures and two required native
ignores across 39 groups** in 835.770s. Formatting, both dispatch-disabled Plans,
contract inventory and diff checks passed. Session72931 is closed; no owned check
process remains. All 35 invocation artifacts, source/tool/log identities and
retained source-preparation bytes/modes were verified. See
[the native evaluator report](validation/LOCAL-NATIVE-EVALUATION.md) and
[machine evidence](validation/native-evaluation-checks.json), SHA-256
`97ea13128dff8d31a766f9546b1617926f33c94ee276dad87d3c9a446356b151`.
The ledger appends partial local evidence to 18 clauses and eight named items;
459 clauses remain not_assessed/37 partial, and 121 named items
not_assessed/15 partial. No requirement is passed. Coverage SHA-256:
`71790c6b2e3f6445b093c0a4d47f3c61fc7f654e03851a32b4e32ed3868a1c5b`.
Actual-agent/provider/native containment and human qualification remain incomplete.
The native evaluator increment was committed separately from prior carry
recovery because they affect distinct runtime paths. Its exact verification is
recorded above; actual-agent/native and human qualification remain incomplete.

Source13 only rustfmt-formatted the exact-TrialKey test assertion; its intentional
source transition from source12 and original bytes remain retained. Source12
strict Clippy and 49 focused evaluation tests passed on their own exact inputs.
Earlier source10 compile errors (two test-only PrivateDir::clone calls) and
source11 lint errors are preserved with the corrections. Root reopens the actual
protected test directory, derives Default, keeps the existing boxed transfer on
error, simplifies equivalent borrowing/conditions and reorders a helper. No lint
suppression, weakened assertion or replacement authority was used.
The exact reviewed owned-return composition is
`/private/tmp/kegari-native-owned-return-on-source9-v2`, manifest
`49016fbd334d030a67d7106b6dd09bce2fbb5c0528ba7173eefd252bbbc07ed0`.
Source-tool v2 fixes bounded directory enumeration and protected output ownership;
independent review found no remaining material defect in those corrections.
Six tool/docs integration paths and originals are retained at
`/private/tmp/kegari-v0-native-evaluation-installed-source14`.
Coverage/report integration is complete. The evidence collector
independently binds exact final commands, log paths, source identity and both
required native-ignore names after review identified filename-only classification.
No claim is derived from a label alone. The next-fixture data proposal is held at
`/private/tmp/kegari-native-next-fixtures-proposal-v1`, manifest
`9ec55d56f3858bfeb1f7da4c99645c4400f582b297588887e2a5eb8b437923f3`.
`independent_eligible` needs coordination and `clean_review` needs product review;
neither is relabeled. PR-attack migration preserves three currently unavailable
host-observation assertions and has no invented base/head OIDs or runnable manifest.
At that earlier native checkpoint, the separate15-file retired-parent producer
composition was uninstalled in TEMP. It is now installed as recorded above, and
still cannot prove receipt-set completeness from retained receipt scans alone.

The 28-file native evaluator source1 is installed from the reviewed caller-v4,
subject-v2 and terminal-report layers, rebased on current main. Exact composition:
`/private/tmp/kegari-native-evaluation-caller-composed-v5`, manifest
`5289c0d7765868493717d4cfaaf4169cde9d14bd2bbdd5a74e7bad82644952c8`.
All bases/new paths and installed bytes were verified; originals are retained at
`/private/tmp/kegari-v0-native-evaluation-installed-source1`. Both clean merges
preserve current cold recovery, exact per-trial subjects and terminal classification.
Initial all-target check passed in 24.809s on 662 unchanged inputs at
`sha256:4e0d2ee0c9a33522d66110a63ed7954e486f5d7ada23b976d3604777f5f30ac2`.
Formatting changed no bytes. Strict Clippy2 failed on a nested if and a
one-element loop; source3 simplifies those without changing their conditions or
responses. Original source and failed invocation remain retained. Session59176
and session18639 are closed. Source3 strict Clippy passed in 24.649s; focused
evaluation tests then reported 36 passed/1 failed because the new report test used
a symlinked macOS temporary path. Source4 canonicalizes only that actual owned
test directory, preserving storage protection and every assertion. Session38918
is closed. Source4 focused library evaluation passed all 37 tests in 13.268s.
Source5 adds two independently reviewed CLI/HTTP boundary regressions; all
136 tests across 7 related API/CLI/app/config/evaluation integration groups
passed in 68.952s. Session30344 and session29780 are closed. Actual
ownership through preparation/unused-trial failure remains a separate in-flight
TEMP correction. No native evaluator is
qualified and compatible native fixtures remain incomplete.

The reviewed ten-file PR-text layer is now installed as source6 from
`/private/tmp/kegari-native-independent-pr-text-on-source5`, manifest
`d85e60668cd2fb24355a78391d6233b86949e0bcab411ba15543c6dfc19787ab`.
All exact current bases and bytes were verified. The two clean merges preserve
actual evaluator transfer gates and the provider lint correction. Metadata and
literal text come from the same bounded wire response; full current/active/source
comparison binds text without changing review/work identity. General status omits
raw text; protected worker claim provenance also retains the gate text. Root read
the complete patch; independent review found no additional material defect
(`/private/tmp/kegari-native-independent-pr-text-independent-review/REVIEW.md`,
SHA-256 `9399d35663e001547dc5a447ca361ca997a0de055a176f1270a7b05ebb6997b7`).
Source6 all-target check passed in 23.234s, strict Clippy in 24.814s and all
98 provider/reconcile/worker tests across three groups in 59.254s. Its focused
library evaluation group passed 38 tests in 42.548s, and the retained
import-history regression passed in 20.331s. Sessions48446,80599,32742
are closed. Exact invocation/log/source identities are retained under
`/private/tmp/kegari-v0-native-evaluation-{check,clippy,text}-6-*`.
Scoring-error loss of already-collected report evidence is confirmed separately
and under correction; no false success or qualification is claimed.

Evaluator source6 had 662 build inputs at
`sha256:7fbe90aefd480e2c012bb7a7906fc32f8f72b4add9ae4332e0bcfc42255d6cdb`.
All-target compile, strict Clippy and focused results retain their exact earlier
source identities. No current full-suite or native qualification is claimed.
The four-file grading-evidence layer is now installed as source7 after full
root review and exact base/hash verification. Independent review found no further
material production defect; its exact report is
`/private/tmp/kegari-native-evaluation-grading-evidence-independent-review/REVIEW.md`
(`c9381deafa10f4ec9e5026c112ec126e065c7f483995b5ab7284c1fc26df51bc`).
Source7 all-target check passed in 21.839s, strict Clippy in 22.825s and 42
library evaluation tests in 35.521s. Integration checks passed 29 evaluation
tests and 12 report tests, with one new report test failing because it selected
comparison row zero instead of its exact trial key. The original failure is
retained. Source8 corrects only this test lookup; all 13 report tests then passed
in 3.338s. Sessions85815 and95686 are closed.
Manifest `ae9c391ebcaa09b724953565cf7301593950ec595e57acee7f104f13abbf440e`;
663 inputs at
`sha256:0e7e5dfac055469156da19c8b8b41de550579c054b2fd2bce5bd5f9fbbcaf5f8`.
Current source8 retains 663 inputs at
`sha256:922a318a5e25c18417618cf6b3de178d611f99c4a9cd29ab58ff205796b8d9f5`.
It retains exact scorer errors and observed effects without substituting a
successful grade or changing execution/cleanup authority. Original inputs are at
`/private/tmp/kegari-v0-native-evaluation-installed-source7`.
The original owned-return TEMP layer was held while root found two compile defects
(signature and missing original preparation reference); separate overlays correct
both. Independent review then found receipt-capture retry and cross-owner drain
liveness gaps. The shared exact admission encoder and independent drain loop
resolve those findings in installed source10. A third compile correction removes
a nonexistent Receipt.created_at reference; the actual Permit.registered_at
remains bound through the immutable payload digest. Original drafts are retained.
The retired-parent producer is also held and explicitly records that retained
receipt scans cannot prove completeness without an owner-emitted receipt index.

Root prepared the first native E05 fixture at
`/private/tmp/kegari-native-independent-fixture-draft-v1`, manifest
`b165ab63a368cc4a0a8a3780af55e079ac62949494d1a586b6df74c858005746`.
Its real owned local Git base is `f27eea6d92b3a6e7cb6973c5e389fedd225c31d5`
(`test(evaluation): seed inclusive boundary review base`), and head is
`2b2b21aca2c287e5f89a35100fb3aa5c3fec1d06`
(`test(evaluation): introduce boundary defect for independent review`).
These are disposable fixture commits, with signed human attribution and no
consolidation or remote access. Exact commands/objects/modes are retained in
SOURCE-EVIDENCE.json. The suite-v4 draft preserves 28 variants/56
trials and an unresolved profile; exact production Plan passed in 32.082s with
dispatch_authorized=false. All fixture/scorer inputs were unchanged. Session67298
is closed. The five fixture files and two-file component test bridge are now
installed as source9 after exact current-base/hash verification; originals are at
`/private/tmp/kegari-v0-native-evaluation-installed-source9`. The bridge reads the
required native scope declaration and exercises actual fixture capture and whole
context matching with explicitly data-only host values. It creates no worker or
source capability. Independent bridge review found no further material defect.
The actual captured-context component passed in 14.444s and all 42
evaluation/report integration tests passed in 13.074s. Session99121 is closed.
Source9 has 664 inputs at
`sha256:e79fbe3d8a4665cf81f622b21d759b3f1ee0a189930f3cdc1dec7a0a06f083be`.
Actual native context/source admission and reusable source-preparation tooling
remain incomplete. The PR-text production path is installed and tested above.
Fixture/source data grants no runtime capability.

The completed-current carry checkpoint is
`2770aa5c43bc24343a2344ce687306928f14fd16` —
`feat(kegari-v0): recover completed carried acceptance from exact history`.
All 23 committed files and 653 tested inputs matched; the tree was clean afterward.
No commits were consolidated, rewritten or pushed. Verification is retained at
`/private/tmp/kegari-v0-completed-current-carry-commit-verification.json`.

Completed-current carry recovery source3 passed **825 tests, zero
failures and 2 required native ignores across 39 groups**
in 843.114s. All-target check, strict Clippy,
formatting, dispatch-disabled Plan, inventory and diff checks passed. Six new
recovery/inventory/import-history component tests also passed separately and are
included in the full total. All 653 inputs stayed unchanged at
`sha256:d0288da1dcd547c8a670373630ebad3542c8ff2aed57b63576e5ddb6d62db46b`.
Session40448 is closed. All 15 invocation artifacts, source/tool/log
identities and the unchanged SPEC were verified. See
[the completed-carry recovery report](validation/LOCAL-COMPLETED-CARRY-RECOVERY.md)
and [machine evidence](validation/completed-current-carry-checks.json).

The real live writer retains exact recovery selections; a canonical protected
reader yields historical provenance, followed by actual scoped current metadata
reobservation and final serialized installation. Additional production integration
proofs retain the original closed owner/import/source and predecessor closure.
No worker authority or joined cache is reconstructed. One initial private-sibling
compile error was corrected by ancestor-only visibility with unchanged behavior;
earlier failures/results are retained. The final inventory formatting failure
was corrected without semantic changes; inventory4 and diff4 passed afterward. Root and independent complete-layer reviews
found no remaining material defect in this scope. Native cold carry, visual
hydration and cold worker reconstruction remain unqualified or incomplete.

The independently reviewed ledger now marks five carry clauses and V0-T17 partial,
linking committed deterministic evidence and explicit native not_run. It contains
465 not_assessed/31 partial clauses and 126 not_assessed/10 partial named items;
no requirement is marked passed. Coverage hash:
`e7ff810b945b9008a405ed835be980b833aad1bf82b433ca0f786419990f016c`.

Next useful work is to verify the installed retired parent-run producer, then
complete the owner-emitted receipt membership and remaining native fixture/observation paths.
The exact 15-file retired composition is
`/private/tmp/kegari-retired-parent-run-on-native-source13-v1`, manifest
`59a386b092e23e6b31c593fc77f29e24ae4b1f0d31a1ca932505b917bd24aae0`.
Root and independent review verified all three clean merges preserve evaluator
ownership, exact identity, completion retries and existing cold-carry behavior.
Its source bases match source14; it is now installed as parent-retirement source1
and awaits compilation. Receipt
completeness still needs an actual dispatch-time effect index; retained scans
alone cannot supply it. Native model/provider/containment execution and human
calibration remain incomplete. Root owns repository changes/checks/commits;
agents work only in separate TEMP layers. Earlier caller-v4 and terminal-report
artifacts remain preserved at their recorded TEMP paths; source14 supersedes
those intermediate uninstalled checkpoints.

The verified carried-acceptance checkpoint is
`7b933f2cb5d11f149846ba81628ef5c5cc6c1501` —
`feat(kegari-v0): reassemble carried acceptance on known baselines`.
All 53 committed files and 648 tested inputs matched; the tree was clean afterward.
No commits were consolidated, rewritten or pushed. Verification is retained at
`/private/tmp/kegari-v0-carried-acceptance-commit-verification.json`.

Carried-acceptance source6 passed **819 tests, zero failures and
2 required native ignores across 39 groups** in
771.633s. All-target check, strict Clippy,
formatting, dispatch-disabled Plan, contract inventory and diff checks passed.
The affected release-origin test also passed separately. All 648
build inputs remained unchanged at
`sha256:741b805e1a0209cb2994bf66190e7a1aca4648978bd5e0f527abbe1ab4e4184b`.
Session36561 is closed. All 22 invocation artifacts, including earlier
compile/lint and regression failures, were checked against exact source/log/tool
identities. See [the carried-acceptance report](validation/LOCAL-CARRIED-ACCEPTANCE.md)
and [machine evidence](validation/carried-acceptance-checks.json).

The 50-file root composition and narrow integration corrections implement known
baseline reassembly, owned finite object acquisition, explicit/same-grant retries,
carried empty-slice no-release and actual active owner/E2E child gate retention.
The granted inventory JSON digest defect was fixed while preserving legacy bytes.
Two stale test assumptions now assert the actual gate/namespace boundary and
unchanged protected effects. All failures and review corrections remain in the
report. Current native carry and release qualification remains incomplete.

Root owns all repository changes, checks and commits. Next useful work is the
independently reviewed completed-current carry recovery draft, then the concrete
native evaluator caller and remaining native fixture migration. The caller's
scorer/deadline, active lifecycle and actual-retirement corrections are composed
into 23 independently reviewed TEMP files at
`/private/tmp/kegari-native-evaluation-caller-composed-v4`, manifest
`231a86032d1c7fb376074b6cd05d5736e89781bef557458b067a98a353fe92be`.
They remain uninstalled/uncompiled. Independent review found repeated
trial identities also need distinct legitimate scheduling authority. Native
preparation failure closure and richer failed-trial observations remain under
review. Production integration history hydration and cold worker reassembly
are separate bounded follow-ups. No serialized history creates native authority.

The evidence ledger checkpoint is `0599cebb67ae3ab80340fa71fb95d7b7314f3d3d` —
`docs(kegari-v0): map verified recovery and trial evidence`. Both committed files
match and all 629 runtime inputs remained unchanged; tree was clean afterward.
No commits were consolidated, rewritten or pushed. Verification is retained at
`/private/tmp/kegari-v0-recovery-evidence-commit-verification.json`.

Evaluation terminal-finalization source1 passed **100 focused tests across six
groups**, all-target check, strict Clippy, formatting, inventory, diff and a
nonexecuting Plan. All 629 inputs stayed unchanged at
`sha256:95f47600a9a67c9b8f7399eb526a4949caa3f4ed08081b788478de3aa9ce554a`.
Sessions88617 and94425 are closed; no root Cargo process remains running. All ten
invocation artifacts and tool identities were verified. See
[the terminal-finalization report](validation/LOCAL-TRIAL-FINALIZATION.md) and
[machine evidence](validation/trial-finalization-checks.json). The four-file
change retains actual execution, checkpoint, hook and owned-exit evidence
independently of rubric outcome. Actual native finalization remains unqualified.
The verified local checkpoint is `b295c0fdc5e56cc402c043c391ed9b2710cc8938` —
`fix(kegari-v0): finalize failed trials from actual lifecycle evidence`. All seven
committed files and 629 tested inputs match, with a clean tree afterward. No
commits were consolidated, rewritten or pushed; this separate increment changes
evaluator lifecycle semantics. Verification:
`/private/tmp/kegari-v0-trial-finalization-commit-verification.json`.

Carry43, same-grant transient6, the test-module ordering correction and carried
no-release8 now compose cleanly into 48 TEMP Rust files at
`/private/tmp/kegari-carry-composed-on-finalization-source1-v5`, manifest
`17e3968c579bea9a531d2620966c96ecd6c1e30bf0ab061e16fa89ebbce3f581`.
The exact three-way merge preserves terminal-finalization runtime behavior.
The later active-grant8 correction is composed into 50 files at
`/private/tmp/kegari-carry-composed-on-finalization-source1-v7`, manifest
`8d4bddf3b7f661ade59b5866b238e3dbd6cf350af0252d01e67c9fed199cbc8b`.
It fixes the ordinary-admission lookup during active carry and release, including
outer publication selection. Independent review then found its exact-parent work
check wrongly rejects the actual E2E child's integrated proof. Codex owns a narrow
follow-up that must authorize the exact retained child; installation remains
pending. Partial v6 preserves a second TEMP base-layout script failure with no
repository source changes. The previous active8 remains immutable.
All copies parse; they are uninstalled/uncompiled pending the active-grant
observer correction and independent review. Partial v4 remains preserved after
a composition script assumed the wrong TEMP base layout; no product source was
changed by that failed script. Cold-current carry planning and the concrete
one-trial caller continue in separate TEMP assignments.

Paired recovery source6 passed **798 tests, zero failures and two required native
ignores across 39 groups** in 809.522s. All-target check, strict Clippy, formatting,
dispatch-disabled Plan, the 496-block/136-named inventory and diff checks passed.
All 629 tested inputs stayed unchanged at
`sha256:ad153d5922671337911480153f8d7d2230847eda3747dae5aae76de2feb7cb84`.
Session11140 is closed; no root Cargo process remains running. The collector
verified all 19 invocation artifacts, including compile/lint failures and older
source results, against their exact source/log/tool hashes. See
[the paired-recovery report](validation/LOCAL-PAIRED-RECOVERY.md) and
[machine evidence](validation/paired-recovery-checks.json).

This increment preserves original paired-repair review and implementation
budgets through actual metadata/model resume, completed-result consumption and
subsequent failed validation. Independent review corrected omitted grant and
receipt ancestry, repeated recursive traversal, recursive Arc destruction and
two direct-only completed-repair checks. Required native positive execution
remains unrun; no serialized reference manufactures a capability. Earlier
source1 compilation and source3 Clippy failures are retained in the report.
The verified local checkpoint is `8bce8ff5e2511412cbd2adfb95a4f5602a3f257a` —
`feat(kegari-v0): preserve paired repair recovery lineage`. All 49 committed
files and 629 tested inputs match; the tree was clean afterward. No commits were
consolidated, rewritten or pushed. Verification is retained at
`/private/tmp/kegari-v0-paired-recovery-commit-verification.json`. The separate
commit preserves the runtime-recovery boundary from the earlier evaluation grader.

The combined carry43 TEMP draft is held at
`/private/tmp/kegari-carry-composed-on-paired-source6-v3`, manifest
`a8ece743d274e104d4165a1c347744c8c59fb9e35366c75d3a63b868826fdf9d`.
Its same-grant transient retry and eight-file carried no-release overlays remain
separate, uninstalled and uncompiled. Carried no-release passed independent
bounded static review; its qualified native current-to-cold positive remains
unproven. The granted-parent observer has one confirmed active-drain regression:
it consults current ordinary admission after harmless workflow reload instead
of actual retained active authority with pinned configuration. Codex is preparing
a narrow correction; new work must retain strict current admission checks.
Config continues the concrete native one-trial caller in TEMP. No evaluator or
carry draft is included in the paired source's validation claim.

Mandatory actual-agent repeats, native/provider and containment qualification,
human calibration, cold/current carry and other recorded implementation gaps
remain incomplete. The coverage ledger now links six recovery/budget clauses to exact executed
component regressions and adds two scoped terminal-finalization references. These
remain partial: 470 of 496 clauses and 127 of 136 named requirements are not
assessed; 26 clauses and nine named requirements are partial. No full pass was
added. Referenced files and passed test names were checked against retained logs;
the inventory check passes. This documentation-only update occurred after both
source-bound validation sets, whose auxiliary hashes remain historical evidence.
The ledger/progress evidence reconciliation is ready as a separate documentation
checkpoint spanning the two verified increments; runtime source is unchanged.
The dated checkpoint narrative below records prior states, including work that
has since advanced; the current summary and linked source-bound reports govern.

Structured-review evaluation source1 passed96 focused tests (29 evaluation,
10 report/archive,39 worker,18 module), all-target check, strict Clippy, formatting,
496-block inventory, diff and dispatch-disabled Plan. Eight invocation artifacts
retain unchanged616 inputs and before/after Rust/Cargo1.99.1 identities:
`sha256:19ea8279293431fe4db5cb7c0cbdb162e2f0321062eb4d65e6a1e01fb52d0bfd`.
Session86937 is closed. The verified checkpoint is
`e55ee191ec027c9c8be8f60bccdf1e4f6dad4773` —
`feat(kegari-v0): grade recorded structured review outcomes`. All45 committed
files and616 tested inputs match, and the tree was clean afterward. No
consolidation, history rewrite or push occurred. Verification is retained at
`/private/tmp/kegari-v0-structured-review-commit-verification.json`. This
evaluation change remains separate from the preceding runtime admission fix
because it changes grader semantics and report provenance. The original precise-
URL false rejection was corrected in v2 and independently reviewed by Config.
No full crate suite or actual-agent trial is claimed for this increment. See
[the structured-review report](validation/LOCAL-STRUCTURED-REVIEW.md).

Paired recovery composition remains TEMP at
`/private/tmp/kegari-paired-recovery-composed-on-e55ee19-v3`:41 source files,
original bases and merge outputs preserved. All616 current source inputs and
composition bases still match clean e55ee19. The two original end-of-file test
conflicts retain every test and existing source4 fixture fix. Config v2
(manifest0e7a8c6abcf7a4d9e1559c0a972f7e03d83883ff08b7d96f184ae8ab20581a12)
and leaf v2 (70577a55932b9edf4a3dbda4b80a45133eb0399db35ffad4f1b04a44f8da1984)
merged without conflict; two newer guard literals gain only optional None fields.
The corrected actual native-grant closure passed independent static review.
The raw Control reader also now checks earlier repair/failed-validation ancestry,
but independent review found its passed-validation references still unchecked.
Control v3 now fixes that through a nonrecursive shared exact-reference reader;
manifest2ba06ed6b20825def53581991da5300b9949b51a72a24a26d8bf8fb4af5265e5
merged three files cleanly and includes a first-new-grant Initial source
regression. Original v1/v2 artifacts remain unchanged. Mandatory opaque maps
already rejected those missing records, so no native authority bypass is claimed.
A separate material traversal defect is confirmed: continued repair queues its
previous plan and calls that same plan recursively through unfinished work with
a fresh visited set, causing exponential repeated work and linear stack depth.
This affects the new grant/bridge collectors; source/joined paths are under audit.
Codex owns a shared bounded traversal correction before installation.
Rustfmt parsed the combined draft; it is not installed or compiled and no
native/data joined success is inferred. Carry's33-file vertical draft and its
11-file object-acquisition overlay passed independent bounded static review with
three confirmed corrections pending: successful preparation result shape,
active carried-release configuration pinning, and same-work reassembly markers
across grant namespaces. Object overlay manifest
bd64dbb9b06700cfbcc39dbc1ebe139338b02ad8cf20c8a3f8de235078165b42 is held at
`/private/tmp/kegari-carried-object-acquisition-overlay`; no Cargo or native
positive is claimed. Root's actual Git2.56 query compatibility fixture is retained
at `/private/tmp/kegari-carry-object-query-review-4wwn484k/results.json`
(sha43781a8373cbc9a3d9c26e940792a9e3141ce309a00c290614adb6371431b4cb):
present object exits0 with exact SHA; missing object exits1 silently; wrong type
exits1 with stderr. This tests CLI behavior only. No remote requests occurred.
Cold/current carry recovery, parent granted reassembly, carried no-release and
qualified integrated/visual/native positive behavior remain incomplete.
A concrete one-trial native evaluation caller design is
retained at `/private/tmp/kegari-native-evaluation-one-trial-design/DESIGN.md`;
it is not implemented and no live trial is authorized or running. Root traced
the failed-trial retirement prerequisite through actual evaluator/lifecycle/broker
and runtime code; findings and four exact inputs are retained under
`/private/tmp/kegari-native-evaluation-finalization-review`. A failed report is
not an actual joined capability, and no such capability was fabricated.

The isolated first-new-grant review-round omission fix passed all-target check,
strict Clippy,24 unique child-continuation/scheduler tests, formatting,496-block
inventory and diff checks. Source1 retains615 unchanged inputs at
`sha256:223772a0e6b513f7ce0aba2ecf456772f98957b8c72cee50417d5f220b00fabb`.
Sessions84681/97996 are closed, with eight verified before/after source/tool/log
artifacts. The new marked1 regression is included in continuation1 and not counted
twice. This focused result is not a new full-suite or native qualification.
The verified guard checkpoint is `51e035e2d8811f5a75518bf2f5599a05e6f5361e` —
`fix(kegari-v0): retain review rounds across retry grants`. All11 committed file
bodies and615 tested inputs match; the tree was clean afterward. No consolidation,
history rewrite or push occurred. Proof:
`/private/tmp/kegari-v0-review-retry-round-guard-commit-verification.json`.
Broader paired native/results remain TEMP.
See[the guard report](validation/LOCAL-REVIEW-RETRY-ROUND-GUARD.md). This separate
fix closes a review-grant admission gap; the prior checkpoint implements changed-
source validation continuation, so keeping their local commits separate preserves
a clear behavior and test boundary. No history rewrite is planned.

The changed-source validation successor passed **766 tests,
zero failures and two required native ignores across 39 groups** in
772.838s. Full4 retained615 unchanged inputs at
`sha256:7582d1afeec97bd6632d8848f6e9ead1e7d2f71ec7f442e49f23d12d2b47d439`.
StrictClippy4, fmt4,496-block inventory, diff4 and the dispatch-disabled plan4
also passed with before/after Rust/Cargo1.99.1 identities. Session99672 is closed;
the collector verified15 retained invocations, including the earlier
compile and lint failures. Required native ignores remain incomplete. This does
not establish genuine passed-check/native joined qualification, live evaluations
or required human calibration; the Goal remains incomplete.
The verified local checkpoint is `4244532bf5393c94e6954f3a255132119d06c1af` —
`feat(kegari-v0): preserve validation budgets after source changes`. All19 committed
file bodies and615 tested inputs match, with a clean tree afterward. No commits
were consolidated, rewritten or pushed. This separate coherent increment
continues validation after source changes;2494a28 binds retries to unchanged
source. Verification is retained at
`/private/tmp/kegari-v0-validation-successor-integration-commit-verification.json`.
Check1 failed on a moved borrowed
fixture template; check2 passed after cloning it. StrictClippy2 found one nested
conditional, corrected without changing its predicates. Admission3 found the
invalid three-attempt fixture cap; source4 fixes only its declared data limit. Full historical failures
and source identities remain in the successor report. Paired/carry/evaluation
layers remain independent TEMP work, unqualified and uninstalled.

The exact-source validation retry increment passed **760 tests,
zero failures and two required native ignores across 39 groups** in
737.369s. Full5 retained612 unchanged build inputs at
`sha256:0c7431dfd4c037d499a2320f3fb440c34a6e7742d6b2400ffe2bd50d2c318402`,
with before/after Rust/Cargo1.99.1 identities. StrictClippy5, formatting,496-block
inventory consistency, diff and dispatch-disabled evaluation plan also passed.
Root session1423 is closed. The collector verified 23 retained invocation
artifacts, including failures. Native ignores remain incomplete requirements;
this deterministic evidence does not qualify live/native operation or complete
the Goal.
The verified local checkpoint is `2494a28f24410f33b64952af5b23da94b99e874c` —
`feat(kegari-v0): retain exact validation retry provenance`. All36 committed
file bodies and612 tested inputs match, with a clean tree afterward. No commits
were consolidated, rewritten or pushed. Verification is retained at
`/private/tmp/kegari-v0-validation-retry-integration-commit-verification.json`.
The actual iterative Drop fix keeps long histories bounded without changing retry
budgets; two65,536-node data-only regressions verify unique and shared tails.
Full4's11 fixture-probe timeouts, its same-source12/12 diagnostic and the source5
owned journaled-probe correction remain retained in the detailed report. No
production timeout or assertion was changed; source5 release5 also passed12/12.
The source successor is now installed above; paired recovery and carry remain TEMP only.

The corrected changed-source successor layer is independently reviewed at
manifest7edb2a93fb1308f7d0ee3e7e11a81c05a02160f58745577028d2025956f9ff8a.
It fixes successful-preparation closure in two readers and adds strict
SourceAdvanced validation lineage for mixed original/granted history. Direct{}
bytes remain unchanged; actual G caps and joined source/receipt closure persist.
Six regressions are authored; current executed results are recorded above.
Codex's exact TEMP source5 merge is now installed, preserving compiled
bounds/destruction/fixture fixes.

Root's paired native layer is TEMP only at
`/private/tmp/kegari-paired-recovery-native-source5-draft` (eight Rust files).
It retains actual registered round/admitted-resume capabilities, routes separate
metadata/source/model stages, and verifies completed bridge identity separately
from fresh clean-source admission. Config independently reviewed the narrow
completed-record/historical enrollment readers with no new concrete finding;
the full layer remains uncompiled and needs review/integration with Config's
opaque paired result/descendant consumers. One TEMP routing edit assertion
stopped before writing; the subsequent scoped edit succeeded. No repository
source was changed by those drafts. Git's carried-acceptance vertical layer also
remains uncompiled TEMP work, with actual baseline-transition integration pending.

Codex's complete eight-file static native review found one scalar/namespace
conflation: a restarted review-grant ordinal must not lower the retained
implementation review counter. Root v2 at
`/private/tmp/kegari-paired-recovery-native-source5-v2` preserves their max,
manifest3124de9ec5846fa722c050b86dc137dc512e16f2b2603c2c80817bce4c1668b9;
Codex verified the narrow fix. Config owns matching actual predecessor-derived
Control accounting and its regression. The original reviewed manifest04f211cc
remains intact. Config's opaque completed-resume and descendant layer must also
carry actual admitted bridge handles through every flat source/budget/pass
closure; native callbacks need the second registered bridge reader. Those
changes remain incomplete and uninstalled, not a successful recovery claim.
Codex separately prepares a versioned structured-review evaluation grader layer;
all actual-agent/provider/native/human evaluation gates remain incomplete.

Root leaf propagation is retained at
`/private/tmp/kegari-paired-resume-validation-leaves-source4`, manifest
68aff68e4f86ab9ffc50de77d212279aa6005f1f4b04f64e5e0f502af9a51836.
Config independently reviewed the four files without a new concrete finding;
these are syntax-only drafts requiring upstream bridge inventories. Root native
v3 added matching actual registered-resume readers; independent review then
found a material new-grant omission: exact-phase scheduling could miss a marked
review predecessor in the old grant namespace. Native v4 adds request/planning
round guards at `/private/tmp/kegari-paired-recovery-native-source4-v4`, manifest
a4307bdbc5ccdf339a1c405ba03a738779809108149ca9968e3cb0fdcafdb851.
Codex owns its actual-origin ReviewRetryWork predicate and pure scheduler
regression. This finding remains unvalidated; v4 is not installed or compiled.
The older native artifact and contrary guard rationale remain historical only.
Root subsequently reviewed the typed completed-resume Control reader and flat
opaque result. A concrete retained-proof gap was confirmed: older non-resumed
source/checkout ownership checks did not survive flattening. Config now owns
preserving a flat actual source inventory plus every joined execution ownership
check, including a new copy of the four leaf collector files; the original68aff
artifact remains intact. Correction, independent review and compilation are
pending. No native or cold recovery success is inferred.

The structured-review evaluation draft is retained at
`/private/tmp/kegari-evaluation-structured-review-draft`, manifest
beb2f39dcf48ac5cef3c26d9f4fafa4de9dea30d5009c177b98550ee2679655a.
It projects actual completed recorded review claims and preserves raw prose for
human assessment; it grants no product/native authority. Root review found a
false-negative for precise GitHub blob-link source references; its correction
and independent integration checks remain pending. No actual evaluation ran.

Source1 check1 failed13.181s on a missing chrono Duration import, parent visibility
and one optional fixture field. Source2 check2 passed20.073s. StrictClippy2 then
failed17.379s on four get/is_none lookups and one test Box replacement; source3
fixes only those lint findings. Every invocation retains unchanged source/tool
snapshots and logs; unrun suffixes after failures have no inferred result.
The installed31-file layer includes actual native retry/paired-failure/passed
successor paths and the shared65,536-record resource bound; configured session
caps and16KiB round limits remain independent. Current data/native-join refusal
fixtures pass without a manufactured success capability.

The destructor fix is installed from
`/private/tmp/kegari-validation-iterative-drop-overlay` (manifestfd5f836f741e1099344beb19c9cf840c196fdd308a6e8049c6d3241cfd2b8f9f).
No retry cap or thread stack was altered. This fixes the direct ValidationRetryWork
previous-chain destruction, not every separate existing review/source graph.

The later changed-source path after a genuine granted validation pass, review
Findings and paired repair remains incomplete. Codex prepares an opaque
ValidationBudgetLineage extension, independently reviewed by Config, so the new
validation retains actual G ancestry/remaining caps and rejects old-source cache
reuse. That layer is TEMP only and must update every transitive joined reader.
Git prepares carried acceptance with actual known-released baseline proofs.
Its further prerequisite is typed same-generation parent reassembly: Complete
currently stops, and plain integrated/review admissions cannot reset budgets.
Generic ArtifactReference data cannot supply either capability. Config now owns
TEMP paired Control metadata/resume admissions; root's native seams/design are in
`/private/tmp/kegari-paired-recovery-control-design`. Original paired producer
still lacks dedicated native consumption and repeated resumed-failure bridge.

The inventory correction is committed as
`4d2a62030012ef81cacc0409cdcc5439c39587dd` —
`fix(kegari-v0): retain all implementation contract blocks`.
Seven exact file bodies and the clean tree were verified before this retry
installation; no commits were consolidated or pushed. Source8's core suite below
is historical evidence for its exact603 inputs, not a pass for current612 inputs.

The child review-round core passed **747 tests, zero failures and
two required native ignores across 39 groups** in 727.152s.
Full8 and its strict Clippy, formatting, inventory, diff and dispatch-disabled
plan checks retained unchanged603 source inputs and observed Rust/Cargo1.99.1
before/after identities. Source digest:
`sha256:5e614d0b3417e80dc611d69f5750e7b3610c8cdfeac0b06104e24a8c8e86f3aa`.
Session69517 is closed. The collector verified 23 retained invocations and
their log/source/tool artifacts, including earlier failures. The two native ignores
remain incomplete requirements; this deterministic pass is not operational
qualification or completion of the Goal.
The verified core checkpoint is `6ef65e5efb7303c97c9b85f1df0536fdff8c21b0` —
`feat(kegari-v0): bind child reviews to bounded round ancestry`. All42 staged
files and603 tested inputs match the commit, with a clean tree afterward.
No history was rewritten. Initial automatic review rejected the local commit
for missing trusted explicit authorization; exact authorization in the
user-designated objective was reverified and normal automatic re-review approved.
No bypass/manual gate or remote mutation occurred. Verification:
`/private/tmp/kegari-v0-review-round-core-commit-verification.json`.
The later retry source is now installed as source5 above; prior core evidence remains exact.

Full6 failed405.337s with241 library passes/3 failures, later groups unrun.
Source7 corrected the obsolete validation-as-review-round assertion; schedule7
passed12/12. Source8 corrects the actual round fixtures' refreshed-phase gate:
after native refresh, use the current Authorized gate and its new phase-scope
digest, preserving original proofs, timestamps, limits and assertions. Earlier
compile/schema/lint/legacy-diagnostic failures remain in the
[round-core report](validation/LOCAL-REVIEW-ROUND-CORE.md). The strict Initial{}
serde correction preserves canonical JSON and rejects unknown fields. Legacy
compatibility helpers remain test-only; no native success capability was added.

Next validation retry/passed-successor work is composed only in TEMP at
`/private/tmp/kegari-validation-combined-core8-v1` (31 files, manifest
`d16f19050f675692275209e099ec0aed73a9237242127263654f862accf483ae`).
Exact original/base/draft hashes were checked in order: native3 f302b548,
paired Direct/Retried9 8acacb87, final clean round9 f68e7241, final pipeline5
5089ae7b and fixtures/defaults10 b329f12f. Known final integration includes the
PassedValidationReference reexport; obsolete TEMP comments also need removal.
This original v1 composition is retained as history; its corrected v3 successor
is installed and source5 is under full validation above. Independent reviews cover
the actual cause/bounds/current-successor/grant seams; genuine passed/native
execution qualification is still absent. Original layers remain preserved.

The contract inventory correction is now installed and verified:496 blocks,
including133 newly enumerated unassessed entries. All363 original clause objects,
136 named requirements and stale entries remain exact. Six repository tests,
ledger consistency and diff checks pass with unchanged603 Rust inputs. No Cargo
repeat or product evidence upgrade was needed. Two independent reviews confirm
all559 in-scope nonstructural lines are indexed once. Historical completeness
claims are corrected explicitly in the [inventory report](validation/LOCAL-CONTRACT-INVENTORY.md).
This evidence-tool increment is committed as4d2a620 above.

Root's next retry composition is `/private/tmp/kegari-validation-combined-core8-v3`,
31 files, manifest5df0414b2f0429e171cfb83e091653365acf13853855d4f00e628c5ade906196.
It adds the reviewed shared65,536-record resource bound, required type reexport,
and obsolete-comment cleanup; it was installed as source1 and corrected through
the source5 validation checkpoint recorded above.
The failed partial v2 TEMP preparation is retained (wrong overlay source path;
no repository write). A separate changed-source validation successor must keep
actual granted G ancestry and remaining caps after passed retry/Findings/repair;
Codex prepares it and Config independently reviews. Carried acceptance and
external production observations remain unfinished engineering. Required native,
actual-agent, provider and human qualification remain separate blockers.

The history evidence below applies to committed source10, not these new inputs.

The owned-target/scoped-history increment passed **721 tests, zero failures
and 2 native ignores across 39 groups** in 727.621s. Full10 ran at
unchanged 593 inputs, `sha256:b65dcbbcc2a32882362d2a4c73a50841e8fd811a96043ef6bd4e273c69a16214`,
with actual before/after Rust/Cargo1.99.1 identities. Source10 strict all-target
Clippy, formatting, contract inventory, diff and dispatch-disabled evaluation
plan also passed. The plan contains56 slots and did not dispatch evaluations.
Session52897 is closed; root now owns serial core checks. Root exclusively owns source windows,
serial Cargo, progress and local commits.

This increment retains ordinary owned-target provenance in immutable no-release
history. Every target keeps its complete artifact map and exact closed original/
resumed receipts. Authenticated failed references deny the affected resources and
exact dependency outcome while preserving unrelated valid history. Malformed,
unauthenticated, unknown-scope and unsafe outer storage remains globally blocked.
First authenticated reads, remembered identities, completion writes and complete
queue/cache installation share Control serialization. Shared failures protect
fresh facades; ordinary and no-release failures remain structurally separate.

Actual regressions cover lost seals before any sync, denied cached fallback and
new-facade remint, scoped refusal with disjoint eligibility, explicit complete-scan
recovery, and a production inventory reader racing a genuine delayed commit.
The race uses fresh runtime caches sharing a registered ControlHost, bounded
operation-local test barriers and joined fixture threads; it is not a full OS
restart. Upstream accepted/check state remains explicitly synthetic in some
fixtures. No native confinement or successful required check capability is forged.

The [owned-target report](validation/LOCAL-OWNED-TARGET-HISTORY.md) and
[machine evidence](validation/owned-target-history-checks.json) retain
36 completed invocations, exact commands, source/tool snapshots and
hashed logs. Focused source9 checks passed before the lint-only source10 edits:
check9 (16.926s), no-release9 (14/14,92.698s), ordinary9 (1/1,185.531s) and chain9
(1/1,208.274s). Full10 exercises those scenarios again. Earlier failed compilation,
assertions, stack aborts and strict linting remain retained, with their fixes and
subsequent results. The prior full2 pass719/0/2 predates scoped quarantine;
full10 is the current complete-suite evidence. Bounded independent reviews of
original identity, scoped quarantine, shared failure/lock ordering and native
receipt ancestry have no unresolved concrete finding in this increment.

The verified local checkpoint is
`6523997487d7952487f9af4d2c8d008c88b1306f` —
`feat(kegari-v0): preserve scoped immutable delivery history`.
It consolidates related unpublished6936aa21. Recovery reference
`refs/codex/recovery/no-release-history-6936aa21` preserves the old commit.
All39 staged file bytes and593 validated inputs were verified after amendment;
the tree was clean before starting the next increment. Exact verification is
`/private/tmp/kegari-v0-owned-target-history-commit-verification.json`.
Read-only remote heads/tags still placed main at44b7578e before consolidation.
Published44b7578e and946a15f remain preserved. No remote mutation was performed.

The operator remains **unattended**. Actual-agent and real-provider qualification,
native containment and required human calibration remain incomplete. Arbitrary
repository commands fail closed before unqualified execution. No live evaluations,
remote writes, installation, deployment, denied native probes or host security
changes were performed. SPEC remains version1.2 with approved SHA
`80b6f703d224ce7800c9e04a23c6356ab72f02133ab9be5ccb129fd6eb10e326`.
This checkpoint does not establish readiness to coordinate the lasting build.

Next: commit the verified inventory correction, then integrate and validate
validation retry and clean-successor lineage only after their concrete producers,
consumers and independent findings are resolved. Further engineering includes
paired transient recovery, external production observations, cold/corrected and
configured-hook recovery, carried acceptance and concrete evaluation composition.
Live/human blockers remain separate from useful authorized engineering.

## Historical preparation notes

This section preserves earlier preparation and validation checkpoints. Its
in-flight wording is superseded by the current summary above and later reports;
it is not authority to run stale commands, amend old commits or replace sources.

Root prepared the 36-file core paired-round slice against source10 in
`/private/tmp/kegari-review-round-core-source10-prepared-v2`, manifest
`2b5c3e317407cb9b5fb60436a28026d8d671a375e54f702a51e790873c138188`.
All original bases and selected draft hashes matched. Only the release reader's
four marker exclusions needed a clean merge onto newer history code. This slice
is now installed but not yet compiled successfully; root will validate it before
layering unfinished validation retry and session recovery work. The first TEMP preparation stopped
on an absent physical fixture base copy; v2 verifies the retained original hashes
against actual files and preserves that failed partial TEMP directory.

Git's separate validation component fixture is held at
`/private/tmp/kegari-validation-retry-fixture-overlay`, manifest
`c85d7c3ba365be095690ac6b702c3eca870320c498f65556324201e1a63792ca`.
It authors actual grant/Control/source/guarded RPC paths, while absent actual
runtime joins and required check executions remain precise refusal barriers.
Bounded static native-adapter review found no additional issue; no compilation
or execution claim follows. Config's clean-successor design is at
`/private/tmp/kegari-validation-clean-successor-design/README.md`; its narrow
opaque producer/child handoff and Codex's matching round/Control bridge are being
prepared separately. All original layers remain held.

Committed source10 passed complete validation. The core paired-review slice is
installed; future validation-retry and paired-session-resume work remain TEMP only. Root owns `/private/tmp/kegari-review-round-runtime-draft` (runtime,
worker adapter and scheduler copies). Codex owns
`/private/tmp/kegari-review-round-draft` (opaque producers, Control and child
pipeline); Git owns `/private/tmp/kegari-review-round-fixture-draft`. These are
uncompiled. A reviewed preparation-link mismatch in the root adapter was corrected:
actual preparation UUID/whole digest is retained, not decoded as a model-run link.
Actual native joined evidence remains mandatory; pending fixtures must assert its
real absence, not seed a success capability. Raw current review limits and each
review grant remain distinct from original implementation attempt/turn caps.
Validation after a paired repair needs phase-correct accounting without consuming
another review round; this seam remains under review.

Current round integration precedence is recorded in
`/private/tmp/kegari-review-round-integration-source9.json` (36 selected files,
not applied). It includes the held same-pair fixture overlay and new
`/private/tmp/kegari-review-round-native-source8-overlay`; the latter rebases only
State.released_inventory_error into both captured runtime base/result. Original
layers remain intact. Root's 13-file
`/private/tmp/kegari-review-round-mechanical-source8-overlay` adds actual legacy
LaunchRequest.review_round=None and explicit worker-marker exclusions; independent
review and standalone rustfmt passed, with no compilation claim. Its release.rs
marker hunk must merge with newer history/race edits rather than replace the file.

Config's `/private/tmp/kegari-validation-retry-pipeline-draft` holds four syntax-
formatted modules (manifest700160158d44c8a1635c7cfaf71565e38d3dea7a1803191e6ef4c71f6225f71f).
It binds failed Validate retries to actual assignment/source/grant/receipt and
completed lineage before genuine local checks; the additive paired result consumer
and native adapter are explicit missing dependencies. Config now owns a separate
TEMP native ValidationRetry layer based on the new source8 native overlay. Codex
owns a separate actual paired-session-resume producer/Control-link draft, followed
by the Direct/Retried paired-validation consumer. Git owns new actual validation-
retry component/protocol fixtures. None is installed, compiled or qualified; no
raw saved references can replace missing opaque source, joined completion or
failed-check evidence. Root owns future integration and all Cargo.

Config's `/private/tmp/kegari-external-observation-draft` has additive config and
protocol interfaces only; the distinct actual serving-artifact/source/read-only
verifier and runtime producer remain unfinished. Its ordinary recovery design is
read-only, with separate actual closed observation finalization. Config is also
preparing a separate opaque validation-retry producer for the concrete failed
Validate-stage liveness gap; no shared runtime/pipeline edits are installed.

Before another local commit, recheck publication state. At the last observation
origin/main remained 44b7578e and local HEAD was 6936aa21. If still unpublished,
the related owned-target increment should consolidate into that local no-release
checkpoint under the applicable commit rule, after creating a local recovery ref
and verifying final file contents. Never rewrite it if publication is observed;
no push or remote mutation is authorized.

## Prior increment validation and ownership

Full suite 23's source-sealing failures used current retained provider projections
at each journaled Git spawn/completion. RuntimeDriver previously awaited a whole
delivery phase under its cycle lock; supervisor heartbeats refreshed process and
intent only, and the driver-step deadline was 30 seconds. A long phase could
expire observation gates or lose its waiting future. The implementation now owns
one retained progression task and one separate bounded read task, counts both in
drain checks, joins before clearance, and keeps fenced cancellation independent.
Real refresh failures invalidate current and retained gates without redating them.
Production freshness/deadline limits are unchanged; no uncertain effect is replayed.

Codex agent's driver changes and tests are saved and held. Git agent's continuation
fixtures are saved and held: actual shared observation ownership surrounds Git
sealing; boxed futures avoid nested stack expansion; actual bulk provider requests
prove refresh. Driver composition covers nonblocking polls, actual data changes,
pause/force-stop, preserved permit ownership, real task join and truthful no-process
retirement. Diagnostic incomplete projections are tolerated only while fresh
authority remains closed. Driver 28 passed 13/13 in 22.204s wall.

Config agent's active-contract correction is saved in runtime_delivery_parent.rs,
runtime_delivery_baseline.rs, delivery.rs and a narrow type re-export. The new
input is minted only from an actual ActivePhase under exact permit/current owner/
fence checks and a parent fetched from the retained observation registry. Its
adjusted observation revision is distinct from the admitted execution revision.
Ordinary parent_contract retains exact configuration equality; all other scope,
definition, child, target, dependency and workflow checks remain shared. Both
baseline derivation and parent progression now use the active path. Codex agent's
independent review found no material issue; the Result<()> callback shape was
corrected before compilation. Root owns integration and commits.

Progression 28 closed 101 after 11.612s (three passes, one stale_result at parent
contract construction). The fix passed progression 29 (4/4, 41.576s wall). The
nonempty A-to-B held-baseline reload regression preserves exact origin, effect,
source/target and receipt identities while feeding the retained proof into real
successor import and assembly. Chain 29 closed 101 after 130.262s at a later B
admission after synthetic origin publication (chain_fixture.rs:523); the latest
reconciliation was unavailable. Config agent investigates the explicit fresh-read
boundary and background observation ordering, without changing production refusal.
That fixture correction passed chain 30 (1/1, 223.875s wall). Inventory 31
and diff 30 pass at the latest source (363 blocks/136 named requirements).
Clippy30 and formatting30 passed at3535f1 source. Full30 subsequently closed101
with204/205 library passes; its sole old retention-fixture lease mismatch is
corrected without changing production checks. Focused retention31 passed1/1.
Full 31 completed with 705 passes, zero failures and two native ignores across
39 groups; its session is closed. Clippy 31, formatting 31 and the dispatch-disabled
plan 31 also passed at the source above. Final independent bounded reviews found
no new material defect in active contracts, completion or driver/resume ownership.
The report collector now verifies retained log and snapshot hashes, complete test
groups and separate inventory checker inputs. It found no mismatched runner
artifacts; legacy records are explicitly unverified and cannot qualify final checks.
Inventory invocation 29 used an incorrect script path and
never ran the check; its failure remains in the command evidence.
All earlier compile, fixture mismatch, stack and partial-observation failures are
retained in the detailed report; they have not been replaced by later passes.

## Baseline and authority

- Started 2026-10-25 on existing `main`, clean worktree and empty index, at `36a3e83` (`docs(kegari-v0): finalize the specification and build prompt`). No project branches/worktrees created.
- Read the complete SPEC 1.2; SHA-256 `80b6f703d224ce7800c9e04a23c6356ab72f02133ab9be5ccb129fd6eb10e326` matches the approved contract. Source baselines remain those recorded in SPEC §1.1. SPEC/GOAL and existing license are preserved.
- Applicable instructions are the user-supplied AGENTS rules; no additional AGENTS.md exists in this repository or its ancestor directories inspected. The goal authorizes local signed-off commits, owned disposable Git test fixtures and implementation inside this Rust repository. Remote repository writes/pushes/deployment and host configuration changes remain prohibited.
- Effective tools: workspace-write sandbox, automatic approval review; repository and temporary paths writable, host configuration not writable. No permission/security change made.
- Installed: Rust/Cargo 1.99.1 (under existing `stable` alias), Git 2.56.0, Codex `0.164.0-alpha.11.1`, macOS sandbox-exec/Keychain/launchctl. `tailscale`, `ffmpeg`, `ffprobe` not found on PATH. `rust-toolchain.toml` now pins 1.99.1; development uses `RUSTUP_TOOLCHAIN=stable` only because this alias was verified to contain that exact version. No toolchain installation is authorized or needed on this host. This is availability evidence only, not compatibility qualification.
- Builder model/speed settings are inherited unchanged. The runtime profiles will remain explicitly configured and separate. Assistance attribution will omit a model identifier unless independently verified.

## Implementation plan and historical foundation checkpoint

1. Establish strict typed identities/configuration, canonical evidence, durable records, and runtime-owned instruction assembly.
2. Implement the pinned app-server and enforced execution boundary; implement Git provenance, candidate isolation and all separate evidence gates.
3. Connect real provider adapters through serialized admission/mutation/recovery; implement scheduler, local/HTTPS CLI control and platform lifecycle.
4. Deliver bounded executable evaluation fixtures/graders and operator preparation/qualification guidance.
5. Run build/fmt/clippy, observable deterministic integration/failure tests; independently review and correct. Commit coherent verified increments on `main`.
6. Prepare exact bounded live plans and human review material. Qualification stays incomplete until separately authorized live checks and human calibration pass.

Historical foundation checkpoint: strict configuration/workflow, embedded instruction assets, canonical digests, private durable storage, serialized intent/admission/mutations, authorization/correction/scheduling primitives, immutable paged snapshots, pinned app-server/isolation adapter, and deterministic Git/evidence/release primitives are implemented and have focused tests. Real Git fixtures preserve the source checkout/index, exclude staged B from A's candidate, and preserve failed candidates. Provider schemas/config parsing are verified locally; actual sandbox enforcement is not.

Root owns repository integration, validation, this record and commits. Current agents work only in TEMP: configuration handles declared protocol observation, Codex independently reviews gaps/observations, and delivery prepares evidence. The earlier foundation work split Control/runtime, release history and worker recovery across these agents; that ownership snapshot is superseded. The native `main.rs`/factory is now wired to the concrete delivery decisions and shared worker runtime. CLI-only executable checks pass; no service was activated. Component and focused fixtures do not establish all complete delivery/recovery scenarios.

The detailed local integration report records the checkpoint's full-suite counts.
Later focused results below identify the tested increment and do not establish a
complete SC/V0-T scenario or real-provider qualification.

## Current remaining engineering

A bounded independent caller audit reconciled the old checklist with current source:
`/private/tmp/kegari-remaining-engineering-reconciliation-v1/REVIEW.md`, SHA-256
`291cdaccc27dc62b086ddd2d140b0fbb2c389056c7dc4154add83439144f6e1b`.
Root read the complete report. It is static evidence for the listed slices, not an
exhaustive SPEC audit or a new native qualification.

| Scope | Current remaining work |
| --- | --- |
| Control membership and parent history | Committed1e14788 after917 full-suite passes and two required native ignores; all38 new names passed. Evidence and narrow partial coverage are finalized. |
| Native evaluation and E07 | Declared authority/protocol observations committed102984e after931 passes; transport extension committed998677a after947 passes; broker read/effect inventory committed316c362 after954 passes; three further broker regressions now locally verified, canary corrections and E07 source-data tool in TEMP. Concrete canary/output rubric and versioned pr_attack migration remain. Only independent_malicious currently has native context;27 contexts and other role/prior-state lanes still need genuine producers. All56 slots remain unqualified. |
| Coordination and continuation observations | Derive assertions from actual scoped publications, receipts and predecessor lineage. Repository-free coordination needs its own substrate; fixture JSON cannot create prior execution authority. |
| Cold recovery and native containment | Historical parent seals do not recreate worker/source/completion capabilities. Implement a distinct supported cold obligation with fresh evidence. Actual descendant containment remains a code gap in addition to the denied native host prerequisites; group/leader exit is insufficient. |
| External targets and visual cold readback | Add explicitly supported serving-artifact/source and authenticated video retrieval producers. Existing deployment-free/owned-target paths remain valid within their local evidence limits. |

The prior no-release/owned-target history, structured review claims, exact paired
round/retry consumers, carried acceptance, released dependency cache and typed
historical Overview/Show now have concrete callers and separate local reports.
Do not reimplement them from the obsolete proposals below. Narrow same-process
provider-thread reuse refusals do not by themselves imply missing required behavior
when a reconciled fresh bounded admission already exists. Live/provider/account,
host compatibility and human calibration remain separate recorded prerequisites.

## Historical engineering slices (superseded checkpoints)

The following original observations/proposals are retained for chronology. Their
present-tense statements describe earlier source; the current reconciliation above
and the top checkpoint take precedence.

The deployment-free no-release history drafts are integrated into the working
source, with source authority/refusal and missing-reference facade corrections
being finalized before root compilation. TEMP draft artifacts remain under
`/private/tmp/kegari-no-release-{history,fixture,inspection,root}-draft` as review
inputs, not current verification evidence. The origin-owner/approval audit keeps
the established observed-publication boundary under SPEC §8.6; requiring later
decision/closure success would reject valid acceptance after a lost response.
No broader transitive-closure claim is made.

The separate unintegrated review-round provenance draft remains under
`/private/tmp/kegari-review-round-draft`. It now includes common completed-round
capability/plan producers and continuation preparation, but atomic Control/session
accounting and scheduling are not wired and no Cargo verification exists.

The next owned-target historical no-release draft is under
`/private/tmp/kegari-owned-target-history-draft`; it carries actual target proof/
challenge/closed receipt ancestry through the baseline and complete accepted
vector. Independent static review found no material defect; an authentic target
fixture and signed legacy roundtrip are still required. External deployment proof
needs a distinct actual read-only observation producer, described under
`/private/tmp/kegari-external-baseline-design`; no owned deployment dispatch is
invented. None of these next-slice source drafts are integrated into this run.

- Native evaluation composition remains an implementation obligation. The
  checked-in binary exposes only Plan/Inspect/Compare; no concrete
  `LiveAuthority` implementation or native suite executor is present. The 28
  fixtures have legacy synthetic contexts rather than declared complete native
  contexts. Production `authorized_context` includes canonical issues, child
  projections, native observations, scoped evidence and work identity, all of
  which must be declared and matched without normalizing away fields. Several
  scorers also expect synthetic state paths (for example coordination children
  and repeated-step counts) absent from `RuntimeCoordinator::observed_state`.
  Required next composition must establish those facts from actual verified
  coordination/continuation receipts, install declared corrections through
  ordinary intake, and use genuine prior-session proofs for recovery variants.
  A fixture JSON or authority-reference string must not mint these capabilities.
  Existing owned preparation, `AdmittedTrial`, `finish_trial` and grading APIs
  supply reusable pieces; their presence is not evidence that a native suite
  can execute. Keep all 56 slots unqualified until this composition and the
  separately authorized live/human checks are complete.
  Independent inspection also found a grading-contract mismatch: shipped
  review instructions require a structured verdict/findings/links claim, while
  `ScopedWorker::observe` looks for an `AgentSummary` under `claim.summary` and
  current review graders expect summary/review-state fields. Migrate the
  versioned graders to the authenticated structured claim, retaining independent
  source/effect/publication checks; a verdict must not become authoritative
  completion. Coordination uses a production substrate without a repository,
  whereas the existing owned-clone fixture binder requires seeded repository
  bytes. These need explicit separate fixture paths. A read-only independent
  review is the proposed first complete native composition case.

- Exact-source failed child review retry now has producer, consumer and component
  tests through repeated transient failures and clean publication. Review
  findings/repair across mixed grant budgets and cold lineage remain to be
  completed; old phase records and caps must stay visible.
  Independent design review confirmed the next correction: new ordinary and
  granted review admissions need an authenticated, versioned round identity
  scoped to their complete review PhaseKey and exact reviewed source. Transient
  retries retain that identity while consuming a session attempt; Findings pairs
  its required repair with the same round. Final-round repairs remain bounded by
  the original implementation attempts, while another review is rejected beyond
  the review ceiling. Existing scalar histories must not be silently reinterpreted
  or have counters reduced. Completed source-advanced reviews must retain their
  exact completion/claim/receipt lineage for recurring Findings and transient
  retry; persisted references alone cannot recreate execution authority.
  Proposed integration seam (not implemented): a separate typed
  `ReviewRoundReference` on preparation and run admissions, atomically persisted
  by the existing Control admission transactions, with a sealed
  `VerifiedReviewRoundPlan` produced from actual completed child/source/review
  assignment capabilities. Initial, transient-retry, paired-repair and
  after-repair causes bind exact operations, whole receipts and source/claim
  digests. `LaunchRequest` carries that opaque plan; preparation and final
  admission recheck current grant, inventory, assignment, contract and capacities.
  Keep `RunReference`'s session budget independent and require preparation/model
  references to agree. New ordinary and granted review producers must both emit
  round provenance, while unmarked legacy chains remain conservative.
- Cold recovery cannot recreate original `WorkerContract`/`RunCompletion` from
  current digests. First persist a bounded signed recovery record at the genuine
  unfinished producer, then reconstruct a separate obligation capability only
  after fresh native authority, real owned Git/source and closed receipt/journal
  checks. The first review-findings path must drop all in-memory proof state in
  its positive restart test. Legacy/missing/local-check-only/uncertain records
  remain blocked; no synthetic confined execution is acceptable.
- Provider resume now has protected submitted-input/history provenance, guarded
  metadata recovery, actual source/family proofs, typed admission and a native
  preparation consumer for the narrow known transient terminal-failure case.
  Automatic same-process runtime selection and metadata/source acquisition are
  wired and locally checked; native qualification is still required. Repeated
  resumed-thread prefix reconciliation, corrected/review
  sessions, configured before/after hooks and active/unresolved turns require
  separate work. Preserve actual joined completion identity; no cold JSON may
  construct it. Shared-directory enrollment is tested, and every member's cleanup
  and retained redirect must remain valid through actual cleanup.
- Chained integration recovery now verifies bounded signed admission/supersede/
  publication lineage, rejects cycles and proves current-main Git ancestry while
  deduplicating original contributions. The actual A/B/C Git/Control/native-wire
  fixture retains A exactly once through a held deployment-only B and corrected
  C after restart; it still blocks at missing genuine integrated evidence.
  Carried acceptance remains a producer/integration gap, not only a historical
  reader gap: all concrete runtime release paths currently pass `carried: None`.
  The pure evidence constructor and negative tests do not supply an actual
  producer or durable native publication. The next slice needs original and
  current verified parent results, the unchanged ordered patch proof, actual
  fresh production validation and a sealed known-released baseline lineage.
  Its signed publication must retain the original human-reviewed token/time and
  both parent/vector references while the plan names the newer machine result.
  History/completion then needs tagged direct-or-carried reference inventories;
  no JSON record may recreate current carry authority.
- Exact original target proof recovery uses immutable authenticated identity
  and complete historical observations separately from current readiness. The
  current increment implements terminal transitive completion projection from
  actually successful, retired descendants and the authenticated chain, preserving
  original failed outcomes and unrelated holds. Focused and prior library checks
  pass; its latest nonempty reload chain is being checked before the final suite.
- Released-parent dependency propagation is a separate production gap:
  `DependencyOutcomes::released` has no callers and accepts a raw attempt, while
  `effective_evidence` merges only fresh delivery snapshots. Independent design
  review recommends a separate cache of sealed release completions. Generalize
  `VerifiedReleaseCompletion` from two-or-more attempts to one-or-more; ordinary
  successful retired releases need the same complete authenticated inventory but
  clear no ancestor holds. Insert dependency credit only from that opaque proof,
  using the queue's exact generation/outcome digest. Install outside existing
  Control/release locks, and revalidate immutable references before consumption.
  Missing/replaced references revoke only that exact credit with diagnostics.
  Keep current runtime/workspace/account binding and actual observation gates.
  SPEC §5.2 requires B pinned to released A1 to remain satisfied after A2 is
  authorized; A1 cannot satisfy a dependency pinned to A2. Historical release
  facts therefore must not expire with current delivery snapshots or require
  production to remain at the historical revision. Native missing/inaccessible
  prerequisites still block. Tests need ordinary completion, C without failed
  ancestor credit, A1/A2 pins, reference deletion/restoration, cold recovery after
  production moves, config removal/remapping, failed observations and unrelated
  credit preservation. This design is not implemented in the current increment.
- Operator history still needs a typed Overview projection for successful
  releases: current release inspection enriches Show details, while the only
  `HistoryRow` producer feeds no-release-required outcomes. Ordinary excluded or
  inaccessible parents are retained and remain resolvable. Authenticated release
  history absent from the current parent projection needs a separate read-only
  canonical resolver; it must never expand scheduling or mutation targets.
  Next-slice design (not implemented): inspection identity is workspace UUID plus
  full `IssueKey`; each attempt retains exact recorded `WorkIdentity`. Return a
  sorted typed vector rather than a JSON map keyed only by issue. Merge it into
  `RuntimeInspection`, never `RuntimeProjection.parents`, retained scheduling
  refs or dependency outcomes. Overview appends/filter/sorts history outside the
  current-parent loop; Show combines exact current/historical identities and
  rejects ambiguity. An authenticated removed workspace UUID may be selected
  only by this read-only resolver. Missing legacy name/title/key/URL stays unknown;
  Open requires an actually observed URL. General workspace and approve/release/
  correct resolution remains current-only. Existing release proof validation
  requires a current repository contract; do not weaken it for removed-workspace
  display. Those records may expose authenticated recorded outcomes plus explicit
  diagnostics, not recreated success/readiness. Only existing terminal completion
  validation for the exact work/generation may suppress that generation's active
  display. `DependencyOutcomes::released` currently has no production caller;
  this separate delivery integration gap cannot be filled with a UI predicate.
  Tests must cover removed workspace/root, cross-workspace ambiguity, later
  corrections, missing actual completion/owner/step refs, missing/unsafe URL,
  unchanged mutation targeting and unchanged receipt/permit counts.
- Same-grant transient failed reviews mint fresh per-attempt capabilities
  preserving assignment/source, original caps, backoff and contiguous history.
  The current Findings-to-repair and later source-advanced review implementation
  rechecks retained grant lineage during advisory, preparation, admission and
  publication; focused checks pass and stale/deleted/forked grants cannot authorize
  repair. Recurring Findings and transient failures after changed-source review
  still need exact paired-round provenance. Validation retry follows separately.


## Requirement-to-evidence checklist

Status key: **not run** is not a pass; **implemented** requires tests/review before **verified**. Detailed deterministic cases will be linked as they are executed. The complete normative contract remains SPEC; this grouped ledger does not waive any clause.

| Contract / retained requirements | Planned evidence | Current status |
| --- | --- | --- |
| §§1–2: product boundary, identities, authority, persistence; REQ-14/31/42/45/46/52/54 | Build, canonical identities, role/broker rejection, durability/ownership | Primitives implemented; 12 storage/control tests pass; process registration implemented; native qualification blocked |
| §§3.1–3.4: strict config, credentials, workflow/reload; REQ-34/35/40/41/50/51/57; H01/H02 | Config/workflow/secret/reload tests, installed asset assembly | 28 focused tests pass including context-composition review fixes; native Keychain implemented but unrun; serialized setup integration in flight |
| §§3.5–4: preparation, provider reads, authorization/scope/review; REQ-01–04/29/37–39/43/44/62 | Paginated provider fixtures, event provenance and scope revocation | Offline provider/reconciliation/decision fixtures pass in the frozen checkpoint; real provider provenance/compatibility qualification remains unrun |
| §5: lifecycle/dependencies/corrections; REQ-07–09/19/22–24/30 | Correction races, retained child sets, dependency generations | Local correction/scope primitives tested; provider reconciliation tested; full workflow loop in flight |
| §6: scheduling/bounds/claims; REQ-12/21/32/35; V0-T14 | Fair account/workspace admission, attempts, exact review-round ancestry and pause races | Core round Control tests40/40 and child continuation9/9 passed; current full-suite run pending; cold/corrected/repeated and retried-validation recovery incomplete |
| §7: isolation/Codex/harness; REQ-31/32/34/42/54; H01/H02 | Pinned-schema protocol fixtures, OS sandbox probes, manifest/discovery | Pinned protocol, active corrections and shared worker/evaluation path tested locally; actual containment canaries blocked and live discovery unverified |
| §8: local/hosted/review/staging/evidence/acceptance; REQ-05/06/10/11/19–21/25–28/43/44/53 | Exact-revision evidence, attestation, Git fixtures, stale tokens/video | Native provider adapters and separate gate progression integrated and locally tested; complete live workflow/media qualification outstanding |
| §9: candidates/carry/queues/release/recovery; REQ-13/15–18/27/28/30 | A/B isolation, patch bytes, partial effects/holds, correction races | Committed owned-target immutable/scoped history passed721/0/2; actual anchor race tested; external production observation and carried acceptance remain incomplete |
| §10: complete CLI/control/overview/TLS; REQ-49–51/55–57 | CLI/API observable tests, peer identities, snapshot pagination | CLI/control/dispatcher/native executable factory wired; deterministic CLI/API/transport checks pass; native service qualification unrun |
| §11: intent/cancellation/recovery/cleanup/logs/handover; REQ-31/33/36/40/41/52/58–62 | Crash boundaries, restart/stop/force, preservation and redaction | Offline control/process/recovery/cleanup fixtures pass in the frozen checkpoint; complete historical and native lifecycle qualification remains incomplete |
| §12.1 SC-01–SC-26, each retained scenario | Observable deterministic + required live profiles | Partial primitive coverage; no complete end-to-end scenario claimed |
| §12.2 V0-T01–V0-T37, each complete adversarial case | Deterministic process/provider/Git/transport failure fixtures | Partial focused coverage; full rows remain unverified |
| §12.4 V0-E01–V0-E08, every variant repeated per profile; H03 | Actual integrated harness trials, isolated graders, all results | Blocked live authority; 56 declared trial slots and deterministic graders delivered; actual trials not run |
| §§12.3–12.4 independent review and human calibration | Independent code/behavior review; human qualitative review | Multiple bounded independent reviews and corrected regressions recorded; full conformance review/human calibration incomplete |
| §§13–15 traceability/licenses | Source ID coverage, no spec changes or license regression | Baseline verified; final audit not run |
| REQ-05/33/40/41/45–51/53 graphical/distribution portions | Dashboard, wrapper, frontend, Homebrew, other OS packaging | Deferred exactly as SPEC §§1/13 |

## Validation and evidence

- **Native recovery/history checkpoint:** complete suite 683 passed, zero failed,
  two native ignores across 38 groups. Library 190/190 in 255.01s includes both
  corrected/chained successor fixtures. Final strict all-target Clippy passed in
  17.76s; deployment 19/19 and terminal target 1/1 focused reruns passed before
  the full run. Formatting, inventory and diff checks passed. Final 564-input
  digest `sha256:c438a22a3ea57152fcf48f92ae9023c1fb753cec9f18c621d875da9dac159c58` was
  unchanged during and after validation. The current report and JSON above
  retain exact commands, log hashes, source versions and remaining limitations.
- **Retained failures in this increment:** initial compilation used field access
  as a method; first full invocation stopped after 187 library passes/3 failures
  because target-proof validation rejected actual active Prepared admissions and
  a fixture raced unstable refresh. Exact live permit checks and stable fixture
  readback corrected these while keeping retired historical gates strict. A later
  deployment run passed 18/19; its second-target fixture now supplies complete
  native facts while retaining its missing-proof assertion. Strict Clippy found
  argument-count/enum-size issues; grouped stage inputs and boxed payloads fixed
  them without suppression. Final full evidence passed on the corrected source.
- **Independent review corrections:** automatic review retry now requires minimal
  typed transient provenance bound to the full terminal content digest; saved
  contract identity is independent of fresh current authority. Actual joined
  metadata task evidence controls delayed reservation release and recovery errors
  are visible. Exact release plan/resume permit scope, reconciled dispatch effect
  identity, interrupted index repair and read-only recovery from absent or valid
  old current pointers are enforced. Corrupt/dangling pointers, ambiguous proofs,
  unknown exits and cold Prepared receipts remain blocked. Bounded peer reviews
  found no remaining material defect in these paths; complete conformance and
  live/human qualification remain outstanding.

- **Active session-resume admission component:** root added `RunResumeReference`
  and serialized same-phase admission binding the exact latest predecessor,
  recovery/assembly/directory digests and consumed review rounds. Original account,
  profile and minimum phase caps, one-second spacing, no duplicate recovery,
  failed terminal/exited/reconciled predecessor and a mandatory fresh proof gate
  are required. These references grant no provider/directory capability. Typed
  marker `session_resume` is excluded from preparation/continuation/release
  provenance. All three actual-Control component regressions passed in 2.33s,
  log `/private/tmp/kegari-v0-session-resume-control.log`; their terminal observation
  is explicitly synthetic and cannot qualify native resume. Independent bounded
  review found no confirmed admission bypass; full provider history and native
  failure classification remain separate opaque proof requirements. That initial
  run emitted an unrelated in-flight unused-method warning. Subsequent runtime
  proof consumption and family enrollment are described below; no new frozen
  all-target checkpoint is claimed.
- **Directory-family cleanup work in progress:** initial protected family creation
  binds actual root/repository device+inode, exact member source/work/workflow and
  complete registration inventory; foreign aliases and incomplete enrollments fail
  closed. All-member receipt/process/hook checks and final registration/family/
  observation predicates now protect cleanup; retained tombstones cover every
  member. Existing 14 tests passed, then three new regressions passed with the full
  17-test target in 23.90s (`/private/tmp/kegari-v0-directory-family-regressions-rerun.log`).
  An initial compile failed because three fixtures called the no-argument
  constructor with `false`; that was corrected without weakening assertions
  (`...-directory-family-regressions.log`). Independent review found unlocked
  process/retention read-modify-write, a legacy final-gate missing registration
  digest, and cleanup's stale initial hook-ledger copy. Registration updates now
  serialize with Control/cleanup and each other. The real protected worker ledger
  is registered by immutable directory identity and reread, including later
  sessions/hooks; native before_remove uses that same ledger. Missing/replaced
  registered ledgers never fall back to the old copy. Five additional integration
  regressions passed with the complete 22-test target
  in 26.61s (`/private/tmp/kegari-v0-housekeeping-family-final-reattachment.log`).
  This includes removed canonical-reference refusal, changed-location denial
  without writes and exact original reattachment. Initial concurrency fixture
  setup used the host Git path and failed strict pin ownership; it now uses the
  actual pinned fixture binary. No host rule changed. The first combined compile
  exposed private sibling helper visibility and a peer fixture closure lifetime; owners corrected these.
  Opaque unchanged-source/family proofs and typed serialized enrollment are now
  implemented. Four actual host/source tests pass in 37.11s and two actual family
  enrollment tests in 34.16s (`/private/tmp/kegari-v0-git-reuse-host-final.log`,
  `...-git-reuse-family-tests-4.log`). They use actual scoped Git and Control, with
  explicitly synthetic terminal metadata and no joined-runtime cache. They cover
  exact/replayed enrollment, ordinary-admission denial, inherited creation, unknown
  ledger refusal and source changes invalidating both replay and enrolled proofs.
  The shared-family cleanup/tombstone positive passed 1/1 in 51.38s
  (`/private/tmp/kegari-v0-git-reuse-family-cleanup.log`): stale tokens fail
  before receipts both while the successor is active and after retirement; a fresh
  plan retains exact source and marks both operation tombstones removed. Strict
  library/tests Clippy passed in 9.32s (`...-git-reuse-family-final-clippy-2.log`). Native scheduler
  selection is not connected; a full native resumed run is not claimed. Root owns
  the family/hook implementation;
  delivery owns source-proof/family component tests and new housekeeping regressions
  during this phase.
- **Exact-source review retry increment:** actual Git/Control/native-wire component
  passed 1/1 in 28.79s, all continuation tests 15/15 in 46.47s and delivery tests
  21/21 in 17.39s. Logs are `/private/tmp/kegari-review-retry-final-test.log`,
  `...-all-cycles.log` and `...-delivery-tests.log`. Independent producer/runtime/
  publication review found no confirmed defect. The initial fixture omitted the
  native grant's required supporting link; it was corrected rather than weakening
  publication. Full native joined-runtime success remains unverified; no fake
  joined-cache entry or confined execution proof was added.
- **Metadata recovery increment:** typed hook outcomes and actual input provenance,
  opaque full-history recovery, finite distinct metadata admission and guarded
  initialize/account/config/thread-read wrapper are implemented. Worker/protocol
  tests passed 68/68 (37+31), Control metadata tests 2/2, plus full history and
  handshake scenarios; current-cap/fencing/owned disposable-process timeout tests
  passed. Logs use `/private/tmp/kegari-v0-recovery-*` and
  `/private/tmp/kegari-v0-resume-metadata-tests.log`. Root's bounded read found no
  confirmed defect. No live Codex/model was launched. Metadata recovery itself grants no model
  permit; the subsequent sealed bridge described below consumes actual retained
  completion, exact instructions, provider proof and directory enrollment.
- **Native resume consumer in progress:** `runtime_resume.rs` rechecks the actual
  retained completion Arc, exact metadata receipt, source-family proof, native
  authority/grants/provider holds, shared capacity, minimum original caps, failure
  backoff and consumed review rounds under the common final admission gate.
  `runtime_workers::prepare_resumed` performs real assembly, enrollment, new-bound
  execution/services, canonical inherited hook ledger and sealed resume options;
  it never reconstructs the original completion from JSON. Independent bounded
  review found no confirmed new authority defect. The first compile had an extra
  `RunServices.control` field; corrected to existing `WorkerServices.control`.
  A mistaken Control filter selected zero tests and is not counted; the corrected
  `control::session_resume::tests` command passed all three in 2.35s, log
  `/private/tmp/kegari-v0-resume-control-three-current.log`. Source/filesystem proof
  and family replay checks were strengthened after review. Automatic selection,
  repeated resumed-session prefix recovery, configured before/after hook support,
  cold recovery and genuine native qualification remain unfinished. Strict
  library/tests Clippy passed in 15.39s (`/private/tmp/kegari-v0-resume-bridge-clippy.log`).
  A root all-target run encountered in-flight fixture HTTP type errors, corrected
  by the fixture owner; no new frozen all-target checkpoint or commit is claimed.
  The private resumed-wire component also passed 1/1, including fresh history
  readback/current prompt/new-session accounting and history/policy refusals
  (`/private/tmp/kegari-v0-resume-wire-bridge-tests.log`). It cannot mint native
  joined or admitted-directory capabilities. A final review found pre-first-turn
  resumed thread usage could be misattributed to the new operation; the owner is
  fixing baseline-only handling with explicit prior/unattributed usage evidence.
  This narrow correction must pass before the source freeze. Integrated review
  also found source bytes could change after session binding but before outbound
  thread/resume or first-turn start: existing protocol gates checked only current
  authority. The owner is retaining the enrolled source verifier through those
  serialized startup gates, with a no-outbound-frame regression, and releasing
  the initial source constraint only after actual first-turn dispatch so later
  legitimate source edits remain possible.
- **Chained integration work:** an actual A/B/C Git/control/native-wire fixture is
  in progress and stops at missing genuine integrated evidence. Independent review
  found unchanged deployment-free repositories omitted from recovery's complete
  accepted baseline inventory (owner fixed), and historical successful deployment
  reads incorrectly coupled to the current target index/current main (pending
  immutable history route). Transitive queue eligibility is restricted to verified
  signed lineage and an ephemeral view; durable ancestor holds are preserved.
  The actual chain completed every assertion in 198.15s, including original A
  contribution exactly once and C integrated-gate refusal, but asynchronous poll
  teardown raced fixture deletion and logged a background panic. That run is
  retained as noisy development evidence, not the final clean result. The owner
  is adding explicit abort/join and rerunning both successor fixtures. An earlier
  large unboxed test future overflowed the normal test-thread stack; the fixture
  now boxes stage futures, with no host/thread-stack setting change. Full chain
  recovery still lacks overwritten historical target proof lookup and terminal
  transitive hold completion projection; neither is claimed complete.
  The next combined successor rerun failed after concurrent Cargo relinking
  replaced the running test executable, so the actual macOS process-path probe
  returned absent; an observation poll also saw `unstable_observation` during
  signed publication. The fixture now retries only that bounded read failure,
  without treating it as a valid projection. Final test runs must use an exclusive
  Cargo window after all sources settle, preserving the strict process probe.
  Log `/private/tmp/kegari-v0-lineage-runtime-final.log` retains both failures.
- **First frozen focused verification:** all 561 code/test/instruction/protocol/
  fixture/Cargo inputs had `sha256:c8f5d7d5b934fec93da762706ab4ce4c9ed90f82c43a2b85a61c549466f9614a`.
  Protocol/recovery tests passed 2/2 in 0.04s and actual source/family/startup-gate
  tests passed 6/6 in 49.99s, sequentially with no competing Cargo relink. Logs
  `/private/tmp/kegari-v0-worker-release-recovery-protocol.log` and
  `...-families.log`; initial source inventory retained as `...-source-v1.json`.
  Before full validation, root found that delayed usage for a historical turn
  could still be charged after a new turn started: the pinned notification has a
  required turn ID, but accounting discarded it. This narrow per-turn attribution
  correction is saved; the first source hash is historical, not final. The sealed
  resume origin now retains the bounded historical turn-ID set and rejects reused
  IDs on start, acknowledgement and completion, preserving policy signals first.
  Unknown-turn usage remains explicitly unattributed even after a new turn starts.
  The updated focused protocol target passed 2/2 in 0.04s
  (`/private/tmp/kegari-v0-worker-release-recovery-protocol-v2.log`).
  The full-suite base 561-input hash was
  `sha256:49b29e829fa6b366f7661222929201d139a6bd211aee55f748fea64cb0099dc4`.
  The exclusive full suite passed **668 tests, zero failures, two native ignores**
  across 38 groups; library tests passed 184/184 in 246.61s, including both
  successor fixtures without teardown errors. Input hashes remained unchanged.
  Log `/private/tmp/kegari-v0-worker-release-recovery-full.log` has SHA-256
  `430eaeee0b66d27b7c5639b961c87369593da6c733e677b5bc6aac20a85a5a81`;
  the source inventory is retained as `...-source-v2.json`. The following two
  read-only review findings were corrected and verified separately below.
  The native evaluator also now rejects a resumed admission before binding because
  its current TrialOperation cannot carry the sealed resume mode. No fabricated
  capability or tautological guard test was added; relevant trial regressions
  passed in both the full base suite and final affected checks.
- **Final read-only review findings (resolved and checked):** while the frozen
  suite ran, Codex found a successful turn acknowledgement could reach a failing
  mutation-receipt write before persisting its actual policy signal. The saved
  fix preserves same-request/thread policy before acknowledgement bookkeeping,
  with a real duplex response and rejecting observation gate regression. Delivery
  also found that the first new-turn cumulative counter may contain unobserved
  prior usage: full history readback has no authenticated final token snapshot.
  The first resumed known-turn counter now establishes an explicitly unqualified
  boundary and retains ambiguous increases; only later known-turn deltas are
  assigned. Both corrections passed 122 affected checks on final source and
  independent rereview. Strict all-target Clippy passed in 18.66s. The complete
  base suite and final two-code-file delta have separate recorded identities.
- **Post-review fixture failure retained:** the first corrected-source protocol
  run passed 1/2 and failed the new acknowledgement-ordering fixture. Its parent
  Tokio test used the default single-thread runtime; production guarded dispatch
  correctly rejected it before transmission because it requires a multithread
  host. The fixture's uncertainty assertion hid that precise prerequisite error.
  The fixture was corrected to the actual supported runtime and reports
  scenario/error context without weakening dispatch. Its final rerun passed 2/2
  in 0.05s, followed by all other affected checks. Log
  `/private/tmp/kegari-v0-worker-release-recovery-protocol-final.log` and source
  `sha256:069b48cc37d2b8e63fa15d8388881b2dbd5eb1a4ed6a392344b84d8b23a094a1`
  are retained as a failed intermediate state; final affected checks then passed.
- **Validation process ownership:** all root-owned focused sessions above have
  completed. A read-only `ps` inventory was denied by the sandbox (`operation not
  permitted`); no escalation or alternate process-enumeration route was attempted.
  Use the tracked tool session completions and agents' explicit process handoffs.
  The full suite, failed and successful focused runs, final affected tests, lint
  and current evaluation-plan build all completed. Final sources remain frozen
  until root commits the reviewed checkpoint. No Cargo process remains owned by
  root or a delegated agent.

- **Earlier retry-checkpoint implementation history:** read-only review found that explicit retry
  skipped every completed phase, including an exhausted genuine repair obligation.
  The fix must retain the actual unfinished proof and bind a new explicit native
  retry grant; completed work cannot become generically retryable. A full runtime
  successor fixture is also in progress. Actual integrated-check production proofs
  remain unavailable behind the previously denied native sandbox/descendant
  prerequisites; the team will not repeat those probes or fabricate confined
  execution. Reachable actual Git/runtime assembly and fail-closed preservation
  remain independently testable.
- **Retry publication/finalization corrections:** independent review found that
  the real selector searched again for an ungranted phase after observing its
  own native grant. The postpublication path now verifies the exact signed current
  comment and unchanged prior-admission map. It does not let a stale cached
  publication-before-ACK error override complete fresh evidence; provider/session
  holds still gate this path. A second actual-control regression exposed two
  finalization races: the runtime can already consume the grant, and its unknown
  effect then blocks a generic metadata acknowledgement. The final predicate
  permits only the exact granted phase's admissions/effects. A narrow local
  completion path verifies exact command/publication lineage, records only fixed
  acknowledgement metadata and leaves successor holds intact. The first new test
  failed on generic overlap; the next failed on mismatched provider versus work
  target identity. Both were corrected; the exact positive/wrong-budget regression
  passes (`/private/tmp/kegari-v0-retry-start-race-native-target.log`). Original
  failures remain in `...-retry-start-race-exact.log` and
  `...-retry-start-race-local-ack.log`. The initial partial-name command with
  `--exact` selected zero tests and is not counted. All 14 preexisting decisions
  had passed earlier. Independent review then found the earlier preparation
  interval also needed exact provenance. A new typed durable preparation reference
  binds worker operation, phase/budget, workspace and account to its canonical
  preparation admission; it grants no authority by itself. The actual runtime
  records it, and finalization recognizes only matching preparation/model work.
  All 15 decisions and 25 evaluation integration tests pass, including the
  preparation/model × exact/wrong-budget race regression
  (`/private/tmp/kegari-v0-retry-evaluation-review-regressions.log`). Root review
  then found that the advisory schedule could become stale before preparation
  admission. The worker agent is adding the exact current retry/continuation
  planning predicate inside serialized preparation admission without lock reentry.
  The exact current attempt, caps, trigger and spacing must still match. Actual
  native grant deletion/edit tests pass and leave no preparation receipt, permit
  or effect (`/private/tmp/kegari-v0-retry-preparation-native-tests.log`, 1 test,
  44.39s). Previously admitted preparation still drains under retained authority.
  The genuine fixture's postconsumption assertion was initially misplaced before
  consumption; it is now after the actual repair admission and passes. Earlier
  focused results did not establish that specific timing assertion.
- **Explicit retry of genuine unfinished work:** producer/consumer now retain
  the actual unfinished proof and bind its original completed implementation,
  failed checks/reviews, original admission map, exact current native grant and
  fresh first admission. Ordinary completed work is still not generically
  retryable. Two actual delivery repair cycles and four selector tests pass;
  source edits/deleted grants and consumed proofs are rejected. Logs:
  `/private/tmp/kegari-v0-explicit-retry-cycle-tests.log` and
  `/private/tmp/kegari-v0-explicit-retry-selector-tests.log`. Mixed phase budgets,
  cross-correction and cold proof recovery remain incomplete. These focused runs
  precede the latest root finalization changes and will be covered again by the
  combined checkpoint.
- **Reachable runtime successor fixture:** one expanded production-path fixture
  passed in 164.38s, plus 11 shared release adapter and 13 correction regressions.
  Actual command publications/admissions, signed correction routing, retained
  predecessor recovery, original/corrected Git candidate assembly and complete
  vectors are verified. Historical parent/check/retained-plan seed inputs are
  explicitly synthetic. Missing genuine integrated execution blocks progress,
  preserves the unknown hold and produces zero PR/merge/production dispatch.
  This does not establish full partial-merge/multi-target supersede, carried
  acceptance, deployment-only execution or chained integrations. Logs:
  `/private/tmp/kegari-v0-runtime-successor-fixture.log`,
  `/private/tmp/kegari-v0-successor-release-regressions.log`, and
  `/private/tmp/kegari-v0-successor-correction-regressions.log`. Focused strict
  Clippy passed. A one-off read-only process-status diagnostic was sandbox-denied;
  it was not retried or escalated. No denied native probe was repeated.
- **Resume prerequisites:** actual instruction assemblies now retain exact
  compatibility provenance; only typed operation/attempt metadata can normalize,
  while rendered workflow and all effective instructions remain exact. Provider
  journals retain ordered complete terminal-turn fingerprints and interrupted
  known turn IDs. Three assembly and 29 protocol tests pass. These are
  prerequisites, not a working resumed provider session: metadata-only recovery,
  reused owned source/cleanup and atomic new-admission composition remain undone.
  Independent review found malformed provider `itemsView` values could normalize
  to full history and a late/duplicate policy notification could lose intervention
  provenance. The adapter now rejects invalid present history views, classifies
  actual same-thread policy intervention before ordinary history errors, and
  durably checkpoints that hold on either request/pump error path. Protocol 31,
  assembly 3, control/storage 24 and release-control 5 tests pass, including legacy
  record serialization and actual retired preparation rejection as release origin.
  The release adapter's 11 tests and strict lib/test Clippy also pass. Logs:
  `/private/tmp/kegari-v0-resume-review-regressions.log`,
  `/private/tmp/kegari-v0-release-preparation-isolation.log`,
  `/private/tmp/kegari-v0-release-typed-admission-regressions.log`, and
  `/private/tmp/kegari-v0-resume-review-clippy.log`.
- **Owned evaluator placement in progress:** source preparation now binds its
  scratch token to one anticipated operation; binding consumes the actual admitted
  worker contract and compares exact seed/clone bytes, modes, full context,
  workflow, declared scope and scorer boundaries. Binding grants no live authority
  and cannot occur after worker start. Library compilation passed; the first
  new focused test compilation stopped on missing imports in a concurrently
  extracted shared delivery fixture, before placement assertions. A subsequent compile hit an in-flight release
  seed helper; both failures are retained in the original/rerun logs. Once those
  sources compiled, all five placement tests passed against actual disposable
  Git repositories (`/private/tmp/kegari-v0-owned-fixture-tests-current.log`).
  The evaluator integration suite passed all 25 tests, including a new private
  grading executable rejection (`/private/tmp/kegari-v0-owned-fixture-integration.log`).
  Source compilation is `/private/tmp/kegari-v0-owned-fixture-check.log`.
  The earlier strict lint attempts stopped on concurrently changing retry/release
  helpers; later agent strict lib/test checks passed. A complete frozen check of
  all targets with the newest root changes remains pending. A later independent
  review found private host instruction discovery and undeclared scratch input
  paths. Actual auth/workflow/supplied/discovered instruction paths now exclude
  grader/report roots, and current declarations require truly empty scratch at
  binding and before trial hooks. All seven placement tests pass, including an
  actual assembled global-guidance canary and scratch contamination regressions
  (`/private/tmp/kegari-v0-owned-fixture-review-fixes.log`). Root then extended
  private instruction checks to reject hardlinked inputs, with an actual private
  rubric/global guidance alias regression (`...-owned-fixture-private-links.log`).
  Independent rereview
  found no remaining material issue in those paths or exact preparation retry
  lineage. These are bounded code/test reviews, not native qualification.

- **Continuation checkpoint admission:** the host now records the run and its
  unfinished-work reference in one serialized admission. It requires the exact
  latest reconciled/exited predecessor, an actual same-work failure admission,
  one-second spacing, unused trigger/material-progress digests, retained minimum
  attempt/turn/review limits and monotone consumed review rounds. Four new
  actual-journal tests pass, including concurrent predecessor consumption,
  callback refusal without an admission, restart retention and unknown-effect
  preservation. The existing control/storage suite also passes (24 tests).
  Logs: `/private/tmp/kegari-v0-continuation-control.log` and
  `/private/tmp/kegari-v0-continuation-control-storage.log`. Initial compilation
  exposed a module-visibility error and two concurrently edited fixture
  constructors; both were corrected before these runs. Producer/consumer
  integration, independent review and combined checks remain in progress.
  Independent worker-agent review found no material defect in that atomic seam.
  A new positive producer fixture subsequently exposed two integration defects:
  completed validator/reviewer phases need a distinct verified changed-source
  trigger after repair, and raw delivery task payloads were replacing the full
  native instruction context. The team is correcting both without resetting
  phase budgets or weakening assembly. Root added the distinct atomic
  `SourceAdvancedReference` path and authored its journal regression; the next
  test runs stopped at concurrently changing release enum matches and the new
  assignment envelope method before assertions. A subsequent coherent run passed
  all five control continuation/follow-up tests, including the final-gate check
  that a newer implementation admission invalidates an older source trigger
  (`/private/tmp/kegari-v0-source-followup-control.log`). Broader pipeline and
  full-checkpoint validation subsequently passed as recorded above.
- **Continuation checkpoint evaluation corrections:** the outer runner now limits the whole
  executor (including preparation/admission) to the lesser of the declared trial
  duration and remaining suite time. Its new cancellation/retained-report
  regression passed with the evaluation suite (23 tests, log
  `/private/tmp/kegari-v0-continuation-evaluation.log`). A declared native context
  binding is now implemented and library compilation passed: versioned sorted
  typed slots come from the actual assembly identities, and all remaining native
  context must match exactly. The declaration is included in the fixture hash
  and private preparation seal. Two matcher tests now pass, and the evaluation
  suite passes all 24 tests including production assembly/file provenance
  (`/private/tmp/kegari-v0-native-context-tests.log` and
  `/private/tmp/kegari-v0-native-context-evaluation.log`). The positive assembly
  test first failed on an unsupported top-level `native_context`; production and
  fixture context now place it inside the documented `evidence` object. A separate
  positive producer fixture found the public generation `authorization` digest
  collided with secret-field rejection; the team is adding a typed public
  provenance projection while retaining that rejection. Full
  suite migration, owned-clone binding, live authority/CLI composition and actual
  host-result projections remain engineering work. This is not live evidence.
  Independent review then identified two additional fidelity defects: the native
  trial path omitted actual phase versus declared variant equality, and fixture
  bytes could change between the initial source hash and later copying/decoding.
  The shared phase preflight and substitution regression are now implemented;
  immutable bounded fixture capture is also implemented. Six capture tests and
  all 24 evaluation integration tests pass on that source, with strict selected
  lint and formatting. Capture verifies one descriptor's bounded bytes/modes,
  directory entry/depth limits and the final reset against the captured inventory.
  Changed-and-restored source, missing native binding, wrong copied bytes/mode
  and extra files have regressions. Logs are
  `/private/tmp/kegari-v0-fixture-capture-tests.log`,
  `/private/tmp/kegari-v0-evaluation-capture-integration.log` and
  `/private/tmp/kegari-v0-fixture-capture-clippy.log`.
- **Current no-release implementation and review:** already-production completion
  now has a whole-parent immutable signed intent, a real finite metadata admission,
  current native parent/acceptance references, complete current production source
  and target observations, broker publication and exact native comment/receipt
  readback. Current proofs enter dependency and release queue caches; history/show
  metadata is separate. An owned task retires only its joined publication while
  preserving unknown provider holds. The shared metadata planner gives known-not-sent
  attempts the original finite cap/backoff and never replays unknown effects. Three
  planner regressions pass. Independent review found stale no-release outcome/link
  retention after acceptance replacement and a provider-target/work-key mismatch;
  both are corrected. The actual positive runtime fixture passes. Narrow explicit
  publication/admission reconciliation is implemented but its authorized-decision
  failure/recovery regression is still pending. Negative native record/baseline
  fixtures and combined verification are in progress. Earlier compilation stopped
  on two new test-fixture argument types; the fixture owner corrected them.
- **Executable/recovery checkpoint candidates:** the concrete delivery decision
  implementation now handles queue, resume, abandon, resolve, supersede and exact
  operation reconciliation. Main/CLI/local CLI tests pass 3/24/10. Eighteen release
  unit scenarios pass, including actual adapter source/PR/merge and old owned-merge
  production recovery without relabeling current main. The historical runtime
  fallback is wired but a complete positive runtime fixture is outstanding.
  Independent review found no new material bypass in bounded successor-intent
  consumption. A separate review found nonempty `RepositoryId` map keys could not
  serialize to JSON; two retained release/preparation maps now use sorted tuple
  arrays with bounds, identity and duplicate/order checks. Their actual signed
  private-directory roundtrip regression passes. The first roundtrip fixture used
  an uncanonicalized temporary path and failed before assertions; the canonical
  owned path passed. Deployment-only successor representation remains future work.
- **Correction/import checkpoint candidates:** affected protocol (28), worker
  (34) and evaluation (22) suites pass; correction-filter (15), corrected claim (2)
  and ACK race (1) tests pass separately. Protocol ACK observation preserves its
  exact original effect during a new correction, and classification waiting uses
  the original deadline with source/effect gates closed. Imported Git sources have
  17 passing parser/acquisition/retention/clone cases, including changed/removed
  receipts, changed binary/cache bytes, STOPPED missing-cache reads with no network
  or creation, full fork identity and exact retained per-delta root selection.
  The queued manual-release identity regression passes. No live provider was used.
  All registered source edits are frozen for a fresh full run; targeted lint
  results do not yet establish a combined checkpoint. The 525 source/fixture/Cargo
  inputs have digest `sha256:d8ecbd165a7320a864934c4eea40451e0ed1d80ebdd78d334d08f3f3ea2146e9`.
  Full tests passed (577/0/2, 36 groups, Cargo exit 0) in
  `/private/tmp/kegari-v0-correction-recovery-full.log`; strict all-target Clippy passed (exit 0) with evidence in
  `/private/tmp/kegari-v0-correction-recovery-clippy.log`. Full formatting, contract
  inventory (363/136) and diff whitespace checks also passed.
  Source hashes are `/private/tmp/kegari-v0-correction-recovery-source.json`.
  The evaluation plan CLI reports 56 declared slots, suite digest
  `sha256:v1:e59fa4a3e35486210374a57511a54033e6a1a334a7a728be78453d83b57647d5`,
  and `dispatch_authorized: false`; no live trial was launched. The first manual
  invocation used an unsupported `--suite` option and returned usage/exit 2; the
  documented positional manifest invocation then passed. The full suite exited
  successfully and the exact source hash matched afterward. All owned check
  processes have finished; no uncertain live side effect was created.

- **Post-checkpoint recovery integration:** new release resumes use a distinct
  actual admission that binds the immutable original plan, exact predecessor,
  acknowledged decision authorization and monotone ordinal. A serialized check
  rejects active/unknown ownership, stale acceptance, paused intent, reused
  commands and forked predecessors. Four control regressions pass. Recording
  local recovery intent can coexist with unknown effects but neither clears
  their holds nor grants a permit. The normal control/storage (24) and decision
  (13) suites pass after this change. An additional pending-correction recovery
  authorization regression passes, as does the release agent's actual stopped
  DecisionManager-to-abandon completion regression. The closed completion path
  rejects an authorization receipt presented as a completed recovery action.
  Operation reconciliation has the same closed metadata completion rule: it
  requires the exact actual Reconciled target and can finish while STOPPED
  without changing a different runtime-wide Sent hold. Its new regression also
  rejects an unknown target and altered result; all four control cases pass.
  The first new unit-test compile
  failed on a moved fixture string; cloning that test value fixed it without
  changing assertions. Step dispatch now carries exact attempt/prerequisite
  ordinals (release agent: 9 adapter and 4 backend checks pass). Correction
  publisher regressions verify exact signed predecessor chains and edited-source
  races (2 pass); live steering integration remains in flight. A disposable
  protocol-v2 probe used real local Git to fetch/verify nine objects and preserve
  dirty source/index/config/refs; it exercised no remote service or credentials.
  Bounded acquisition implementation and actual runtime wiring remain in flight.
- **Post-checkpoint evaluator fidelity review:** the shared admitted-worker path
  validated its actual contract and permissions but did not repeat direct
  assembly's exact declared task-context check. The correction applies the same
  check to both paths, seals prepared fixture input/prior-state/baseline identity,
  and compares the complete supplied variant with the suite declaration before
  live admission. Independent review found nested hint fields could still enter
  the host-added workspace/operation context; complete canonical context equality
  now rejects them. Scope grading now counts nested `.git` paths instead of
  silently excluding them. The targeted evaluation integration suite passes 22
  tests and the two evaluation unit tests pass. Suite version 2 and grader
  `kegari-observable-v2` explicitly version these changed rules; all 28 scorer
  digests are updated, while earlier results remain historical and require
  regrading for comparison. The versioned evaluation/report suites pass 22/9.
  The first unit rerun stopped at two
  in-flight release compilation errors, then passed after the release agent's
  corrections. The normative coverage ledger now records partial evaluation
  implementation/deterministic evidence separately from unrun actual-agent and
  human evidence. This does not solve the
  separately outstanding symbolic-fixture-to-native-context composition and
  authorizes no live trial.
- **Current affected recovery rerun:** control/storage (24), decisions (14) and
  evaluation (22) pass together, including the shared correction-loop code at
  that build. An earlier run stopped before assertions on in-flight correction
  execution-binding visibility and broker methods; the coherent-source rerun
  passed after the owning agents completed those interfaces. These targeted
  results do not replace the frozen 523-test full-crate checkpoint. Independent
  review found no new material bypass in either closed recovery-completion path.
- **Current independent recovery/correction findings:** root review found that
  reconciling the original release admission could select newer resumed steps,
  and resume could retire a healthy between-step plan before proving a held or
  terminal state. The release agent accepted both findings and is adding scope
  and ownership regressions. Correction review found an installed epoch's
  pending-classification error could abort an active worker on the next comment,
  and freshness checks on an already received protocol response could turn it
  into an unresolved acknowledgement. The worker agent is separating bounded
  waiting/observation from permission to send new effects; ordinary effect gates
  must remain closed throughout pending classification. These findings remain
  unresolved until their affected tests pass.
- **Remote object acquisition evidence:** the delivery agent reports 11 passing
  offline acquisition/parser tests, including actual protocol-v2 upload-pack,
  strict pinned index-pack, binary/mode/symlink preservation, missing ancestry,
  expansion/depth bounds, changed refs/authority, cache tampering, fork PR identity
  and two-root squash/delta verification. The correction-aware execution bridge
  and 18 execution integration tests pass. Native staging and parent assembly
  now use admitted acquisition; worker wiring and read-only restart discovery
  remain in flight. No remote server or credential was used by these tests.
- **Frozen progression verification passed:** the full crate run completed with
  523 passed, zero failed and two native qualifications ignored (35 groups,
  Cargo exit 0). Strict all-target Clippy, formatting, inventory and diff checks
  passed. All 497 source/test/fixture inputs retain digest
  `sha256:60831f1db54e9807d560ae3d9a452a8d9a6d59301f409f7e1bb093f6610f8ac1`.
  The [durable check record](validation/progression-checks.json) includes group
  counts and the log digest. Raw local evidence is in
  `/private/tmp/kegari-v0-progression-full.log` and
  `/private/tmp/kegari-v0-progression-clippy.log`; input hashes are in
  `/private/tmp/kegari-v0-progression-source.json`.
  The built evaluation CLI reports all 56 declared slots with dispatch disabled.
  An initial manual invocation used nonexistent `suite.json` and correctly
  returned `io_error`; the actual `evaluations/manifest.json` invocation exited 0.
  No trial was launched. The final release callback rejects changed acceptance
  token/digest/time without an admission receipt. Failed provider-read invalidation
  shares admission serialization. Known terminal phase failures retire only
  proven finished local work, preserve the error/original operation, and grant no
  retry; unknown effects remain held. Release regressions cover receipt failure,
  restart/retirement, corrected-token preparations, global acceptance ordering
  and native evidence replacement. This milestone remains separate from the
  previous control/worker composition checkpoint because it adds independently
  reviewable provider delivery, process ownership and recovery behavior. No
  history rewrite or consolidation is planned.
- **Current full-run failure and review:** the first post-checkpoint full suite
  stopped in deployment after 9/10 cases passed: `verify_existing_target`
  compared a refreshed observation timestamp as if it were source identity.
  The correction normalizes only that timestamp and retains exact provider,
  repository, source/tree, artifact and target identity; deployment (10),
  provider (28) and release-wire (8) checks then passed. A fresh full run passed
  503 tests with two native ignores (35 completed test groups, Cargo exit 0),
  and strict all-target Clippy, formatting and inventory passed for that
  pre-review snapshot. Subsequent corrections require affected checks again.
  Independent review also found impossible zero-turn successful agent archives,
  missing input/permission/evidence-class comparison conditions, and historical
  reconstructed grader-version substitution. Those report fixes pass 8 archive
  and 20 evaluation tests, with independent rereview confirming the changes.
  Delivery review found the designated E2E candidate-preparation
  cycle, parent generation replacement deadlock, pre-admission launch retry
  deadlock and admitted-phase draining gaps. Their focused corrections are
  being implemented; no completed parent/runtime scenario is claimed.
- **Current review regression results:** cleanup's previous final metadata check
  could miss same-inode, same-size content edits or mode changes. Descriptor
  hashing outside the control lock and exact descriptor/path/time verification
  at unlink now preserve modified files; 2 unit tests cover four mutation cases
  and positive unlink, and all 14 housekeeping integration tests plus strict
  selected Clippy pass. The first fixture run failed on `/var` versus `/private`
  path canonicalization before reaching the regression and was corrected.
  Capacity preflight now checks real occupying receipts before preparation while
  retaining final atomic worker admission. An observed admission callback binds
  the actual serialized control generation. Control/storage (24), scheduling
  (7), evaluation (20) and archive (9) tests pass together. Archive grades also
  require every declared predicate exactly once; missing historical timestamps
  and build provenance remain unavailable rather than invented.
  The first scheduling rerun stopped at an in-flight missing parent E2E helper;
  later cleanup builds also stopped at a missing snapshot-release wrapper and
  release-backend borrow/serialization errors. These were compile failures, not
  test results; coherent-source reruns are reported separately. Further review
  found final new-worker admission could reuse drain-only authority after filter
  exclusion, and quota observations could race serialized admission. Both fixes
  pass the six current runtime unit tests, including two new regressions using
  actual ControlHost admission and deterministic ordering channels.
- **Read-only evaluation report tooling:** `kegari-v0-evaluation plan/inspect/compare`
  now provides executable local inspection and version-aware comparison commands.
  The affected evaluation suite (20) and new report/archive/actual-CLI tests (6)
  pass. Missing started results remain interrupted/unknown; altered criteria are
  incomparable; later passes do not resolve earlier failures. Report writes now
  use owner-only descriptor-relative exclusive creation, and replacement/link/
  corruption cases are retained as regressions. Initial compile exposed a private
  directory-revalidation visibility error, corrected with crate-only access.
  This tool does not launch live trials or satisfy human review; concrete live
  fixture/authority composition is still outstanding. Strict lint is in flight.
- **Harness fidelity review correction:** live evaluation previously bypassed the
  guarded provider startup path and projected the original Git commit as a
  filesystem candidate digest. It now requires the actual scoped worker's opaque
  journaled launch capability and request gate. Observations hash actual files,
  retain each check's original candidate, and verify private retained artifact
  bytes. The evaluation suite passed 20 tests; the affected worker suite passed
  32 after adding stale-check and corrupted-archive regressions. The first new
  stale-check regression incorrectly expected a dirty-source rerun to succeed;
  the production exact-revision gate rejected it with `dirty_execution`. The
  regression now explicitly verifies that rejection and continued ineligibility.
  The current shared-launch worker/evaluation/host-process rerun passes 54 tests
  (33/20/1). An outer-suite deadline could previously drop the trial before its
  explicit cancellation block; a new guard fences the admitted operation before
  provider-handle destruction. Its actual-control/drop-order regression passes
  (1), without claiming an actual provider or native containment run.
  Guarded live-run composition still requires affected checks and a concrete
  authorized evaluation runner; no actual trial was launched.
- **Current integration review corrections:** protected worker outcomes now require
  the latest completed protocol turn, exact bound contract/manifest, contiguous
  journal and matching correction watermark. Starting or failing a later turn
  invalidates an older completed claim (4 claim regressions pass). Delivery and
  dependency caches recheck their 30-second evidence age, complete current provider
  projection, native source digest, required-child set, corrections and current
  monotone permission fences on every read. Acceptance attaches to the actual
  sealed snapshot without refreshing its observation age (4 runtime tests pass).
- **Finite authorization preparation:** metadata admissions now atomically retain
  original attempt bounds independently of model sessions. A next publication ID
  requires actual local retirement, a proven never-sent effect, retryable failure
  and backoff; unknown writes retain the original request. The 2 planner tests and
  22 control/storage tests pass, including restart/cap-change cases. The real-wire
  authorization fixture passes all 9 scenarios: exact publication, lost reply,
  STOPPED recovery, missing/edited/duplicate native records, owner loss/revocation,
  and one preflight retry with exactly one mutation. The latest 23-test
  control/storage run also verifies that reconciliation sees an intervening stop
  under the control lock, preserves a rejected unknown receipt, and resolves
  only a fresh proof while STOPPED. The first new regression failed to compile
  because IntentRecord has no PartialEq; it now compares the intended fields.
  Read-only native recovery
  never retransmits a missing result.
- **Delegated latest increments:** correction publication and prefix/final recovery
  pass 10 tests; deployment baseline tests pass 10; authenticated Linear asset
  association/readback is included in 28 provider tests. Latest selected
  configuration/authority/reconciliation suites pass 65. Host-local metadata
  permits and process journals bind actual registered control ownership separately
  from worker leases; selected host/process tests pass, but native containment and
  strict installed-Git prerequisites remain unresolved. These overlapping targeted
  results are not summed into a new full-suite count. The executable factory,
  production phase graph and final integrated checks are still in flight.
- Baseline `git status --porcelain=v2` and `git diff --cached --stat`: empty. `shasum -a 256 crates/kegari-v0/SPEC.md`: approved digest. Entire SPEC read before implementation.
- `rustc --version`, `cargo --version`, `cargo fmt --version`, `cargo clippy --version`, `git --version`, `codex --version`, app-server/schema help inspected. CLI emits a PATH-alias write warning under sandbox; no host changes attempted.
- Current official [Codex app-server documentation](https://learn.chatgpt.com/docs/app-server) retrieved for protocol context. Installed generated schema and actual bounded tests, not current docs alone, will decide supported-profile compatibility.
- `cargo check --locked -p kegari-v0`: passed for registered foundation modules. `cargo test --locked -p kegari-v0 --lib --test authority_scheduler --test control_storage --test config_harness --test codex_protocol --test snapshots --test git_delivery`: 87 passed, zero failed/ignored; all use the explicit environment above. Strict Clippy for those targets passed after refactoring initial lint findings. Formatting passed after sorting module declarations. Upcoming source changes require affected checks again.
- [Local foundation report](validation/LOCAL-FOUNDATIONS.md) records findings, fixes, commands and limits. [Contract coverage inventory](validation/coverage.json) covers 363 normative prose/table blocks and all 136 named REQ/SC/V0-T/V0-E/V0-H identifiers. `tools/contract_inventory.py --check` passes; inventory is not conformance proof.
- Independent review produced and corrected mutation-rejection replay, recovered-work duplication, unresolved-target retry, ambiguous correction, stale turn completion, scope/definition conflation, YAML type coercion, release-plan omissions, extra selected deltas, unverified carry and lost failed-step history. Their regressions remain. Further review found omission of mandatory context in a plain-text workflow and scratch/source overlap in assembly: both are now corrected, with the affected config/harness suite rerun (28 passes), bringing current total to 89.
- Further protocol review found missing list/operation pagination continuations, weak optional-filter typing and unchecked GET bodies. All three were corrected; the 8 API tests include full 501-child projection traversal, invalid filters, duplicate query fields and oversized GET bodies. The 2 secret/storage tests use a clearly identified in-memory secret fixture: native macOS Keychain calls compile but have **not** been invoked or qualified.
- Build provenance now includes a deterministic source/asset/dependency-lock digest in instruction manifests and evaluation reports. Raw private TOML reads/writes retain the descriptor-relative ownership, symlink and durability checks. The affected storage/control and instruction suites were rerun (12 and 28 passes).
- Platform's first native run failed because `kern.bootsessionuuid` is denied by the outer sandbox (`Operation not permitted`, confirmed by the read-only system query). No bypass was attempted. Deterministic registration tests now inject a labeled process observer through the production ownership code: 4 pass. Two native ownership/process-group cases remain explicitly ignored/blocked, not passes. Actual Keychain, service installation and process-supervision qualification have not run.
- The transport increment passes 10 tests, including an actual Unix socket request with OS peer identity and a disposable-CA rustls handshake. Certificate/identity/revocation/reload checks are covered locally. The configuration suite now has 29 passes after requiring stable numeric Tailscale UserProfile IDs. These do not qualify a real Tailscale enrollment or deployed certificate.
- Runtime wiring review found two additional platform gaps: an unresolved pre-handshake startup had no safe recovery path, and the registration object could outlive its control lock. Pending launches now retain their original launcher identity and require complete process/lock inspection before replacement; registration and control share the live lock descriptor. The platform fixture suite now passes 5 cases with 2 native cases still blocked. Three inherited-channel tests pass for bounded framing, cancelled partial reads and stale/contradictory lifecycle state. The actual child supervisor still requires native qualification and the executable host loop is not yet wired.
- **Corrected delivery integration finding:** the original review binding included the future staging squash SHA, making review-before-merge impossible. Review evidence now uses a separate pre-integration digest; staged artifacts retain their full identity. Root reran all 28 actual-Git/evidence fixtures successfully, including a second-squash/same-tree regression, replacement mapping, authenticated partial-production recovery and no-release-required accounting. Complete provider-backed delivery remains in progress.
- **Corrected local-execution integration finding:** a worker check initially held the control mutex while waiting for the process. `begin_effect` now persists a Sent receipt and launches/registers the owned process under the mutex, then releases it for the wait; `complete_effect` rejects evidence after fencing/revocation. The worker regression observes responsive status during a blocked check, refuses cancellation before durable force-stop, then cancels the owned handle and rejects its late success. Root control regression separately verifies the retained receipt and cleared local hold.
- **Corrected actor-binding integration finding:** retaining only operator display ID/roles lost the original Unix/Tailscale principal during asynchronous preflight. Actors now retain the verified transport principal; mutation receipt digests bind it and current mapping is rechecked inside serialized configuration/lifecycle gates. The dispatcher regression rejects a cached actor after principal reassignment.
- The shared dispatcher has 4 passing tests through authenticated API composition: STOPPED inspection creates no receipts/work; start remains reconciling until admission actually opens; a reassigned principal cannot reuse its old role; service-stop reservation fences subsequent start. Native service persistence/installation and actual process supervision are still being implemented, not qualified.
- Root validation: `RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test -p kegari-v0 --test app --locked` (4 pass); affected API/control run (10/15 pass), prior platform/channel/connection run (5 pass +2 blocked /3/2 pass). An initial new test failed to compile because a deliberately panicking launch callback needed an explicit generic type; corrected without changing its assertion. Current strict full lint remains pending while delivery/configuration work is in flight.
- Root checkpoint validation: the frozen Git/evidence/release files pass 28 `git_delivery` tests, selected-file `rustfmt --check`, and `git diff --check`. This increment adds canonical signed-publication framing, pre-merge review identity, explicit replacement contribution inventories, authenticated production-integration recovery, no-release-required proofs and read-only manual-release eligibility. It is kept separate from the earlier nonadjacent core checkpoint because it is a new independently reviewable contract correction with intervening provider/harness commits; no history rewrite or consolidation is needed.
- Dispatcher/recovery corrections now pass 6 application, 17 control/storage and 15 reconciliation tests: background configuration work survives HTTP timeout, active work cannot be prematurely reconciled, snapshot continuations preserve original metadata, and recovery retains required-child identities while re-reading signed provider authority. The initial observation cache remains explicitly pending, and failed refreshes cannot authorize admission.
- The bounded artifact archive reader uses exact `zip` 9.1.0 with the Rust deflate backend, locked with its dependencies. Package metadata and primary crate documentation were checked; archives are bounded and parsed in memory, never extracted into an execution directory.
- **Corrected provider responsiveness and cancellation finding:** provider preflight
  and response waits no longer retain the control mutex. A unique durable
  Prepared reservation precedes asynchronous reads; current gates run before
  reads and before serialized Sent/owned-task dispatch. Actual network tasks
  survive caller aborts, cannot be prematurely reconciled or service-stopped,
  and preserve unknown receipts. The two new offline wire regressions pass as
  part of 23 provider tests. One first run failed only because the new test
  asserted `status_id` instead of the adapter's established `record_id`; the
  assertion was corrected to the real receipt schema, retaining all behavior
  checks. Decision callbacks were also changed to prepare complete observations
  asynchronously and use cache-only final checks; observation publication now
  serializes with dispatch.
- **Canonical ownership and active reload corrections:** canonical issue/PR
  claims now remain stable across generations and filter/profile edits. Durable
  phase identity still binds exact work generations and budgets. Atomic run
  admission counts actual active receipts under the control lock, including
  global, account, workspace and normalized-state limits; the concurrent
  capacity regression passes. Opaque admitted settings snapshots retain original
  workflow/profile settings while re-reading current permissions and provider
  facts. The 21 reconciliation and 17 configuration tests include ordinary reload,
  invalid future workflow, actor/path/access revocation, known reductions in an
  otherwise invalid proposal, persistence/restart, and unknown-effect holds.
  Independent-review drain and final runtime integration checks remain in flight.
- **Dispatcher permission regression:** retained cached transport roles cannot
  survive disabling HTTPS or a known monotone role reduction. Both tests pass
  through the real dispatcher; unchanged inspect authority remains usable.
- **Owned-process finding and retained failure:** leader exit alone did not prove
  process-group descendants exited. The new owned-group helper reserves the
  unreaped leader while observing/terminating remaining members. Darwin quick
  exit returned ESRCH from getpgid and EPERM for a terminal-only group, initially
  breaking ten decision fixtures and the bounded Git process test. The helper
  now captures creation provenance and performs complete read-only native group
  inspection; its three actual local process fixtures and bounded Git unit pass.
  Shared affected suites are being rerun. No security-policy change or sandbox
  bypass was made; escaped sessions and denied/incomplete observations remain
  explicit uncertainty.
- **Material native process-containment finding (open):** the new finite
  escape fixture demonstrates that a descendant can call setsid, close inherited
  streams and continue writing after the original group exits. Existing Seatbelt
  configuration has no verified escape denial. Native group/stream completion
  therefore no longer grants the stronger all-processes-exited/check capability;
  `process_tree_unverified` retains the Sent receipt, permit and owned reference.
  This is an incomplete implementation guarantee, not merely an unrun test.
  Built-in Codex shell is now disabled for every role; declared commands remain
  scoped host tools. A real process-containment/ownership mechanism and supervised
  native qualification remain required. No sandbox bypass or host change was made.
- **Descendant implementation research (2026-10-25):** local macOS 27.0.1 SDK
  and Apple XNU source rule out recursive kqueue NOTE_TRACK/NOTE_CHILD: unsupported
  since 10.5 and rejected with ENOTSUP. The installed SDK documents the macOS 27
  Endpoint Security descendants client, but it requires an Apple-granted signing
  entitlement. No entitlement application, signing change or client activation
  was attempted. This is a possible engineering route, not validated containment;
  exact event-loss, identity, cancellation and exit guarantees still need design
  and adversarial qualification. See [native host notes](docs/native-host.md).
- Runtime authority, actual worker-handle collection, retained admission settings,
  signed pending outcomes and durable attempt planning are being composed in
  `src/runtime.rs`, `src/runtime_schedule.rs` and `src/runtime_workers.rs`. The
  worker helper uses separate preparation and model admissions. Explicit
  disposable test-resource write grants and no-repository read-only child
  contexts are being integrated. Worker completion remains a pending claim.
  Full scheduling/phase progression/continuation/correction/recovery and the
  executable entrypoint remain incomplete. Abrupt-host orphan recovery and exact
  remote Git object acquisition into owned caches are newly identified gaps;
  configured source checkouts are never assumed magically up to date.
- Real-agent evaluations: not run. Real integrations: not run. Human calibration: not run. No provider mutations, Keychain changes or unknown remote outcomes exist for this build.
- Runtime decision composition now delegates actual human commands through
  `DecisionManager`, stores only sealed acceptance/correction/retry proofs, and
  requires a concrete release/recovery backend. Retry selection rejects ambiguous
  phases, incomplete admissions and unresolved effects. Restart recovery checks
  exact current signed native comments against publication receipts and complete
  prior admission digests. Its newest recovery regression is awaiting an affected
  rerun; the initial three selection regressions passed.
- Review found that correction/retry generations could bypass provider intervention
  if only the latest exact-phase receipt were checked. New account/work guards
  scan historical receipts, and the worker checkpoints authoritative session state
  before continuing. Runtime inserts intervention/input holds under the same gate
  as new admissions and recovers bound protected journals. Later successful exits,
  profile edits and new retry budgets cannot clear them. Six scheduling tests and
  nine then-current library tests passed; the latest recovery/cache additions need
  their affected rerun. No supported provider-resolution adapter exists yet.
- Additional review fixes: cached acceptance now compares the exact evidence token
  within a generation; edited/deleted acceptance/retry/correction publications are
  re-read before proof insertion; a shared pre-dispatch correction fence closes
  stale normal and active gates until intake or actual `not_sent` reconciliation.
  Agent-reported latest decision/reconciliation/cleanup run: 13/24/11 passes.
- Process journaling now preserves Starting references, actual spawned process
  identity and an inherited OS guard. Three deterministic journal fixtures pass.
  Group exit still does not prove full descendant exit; durable identity records
  never authorize signalling a PID reconstructed from disk. Worker injection and
  crash recovery integration remain in progress.
- Full Clippy and the latest root focused rerun initially stopped at shared schema
  migration compile errors (`HostedProofExpectation.context_digest`, then the
  parent evidence fixture's `ProductionBaseline.target_artifacts`). These are
  retained integration failures, not test passes. Target-specific deployed artifact
  provenance is being implemented; no single artifact is selected arbitrarily for
  multiple deployment targets. SPEC SHA and `git diff --check` still pass.
- The complete registered crate subsequently passed 411 offline tests with two
  explicit native ignores, plus strict all-target Clippy and the coverage inventory.
  Independent review then found a retry grant could survive a deleted publication
  until status inspection refreshed the grant cache. Planning, selection and the
  final serialized admission now revalidate the exact current native grant and
  reject superseded budgets; cached recovery errors also block retry selection.
  The affected 11 library/13 decision/6 schedule/28 worker tests passed again.
  Unregistered development drafts were moved out of `src`, because the build
  provenance hashes all source-directory files. The pending executable is at
  `/private/tmp/kegari-v0-main.rs.pending`; no coordinator has been activated.
- **Post-checkpoint integration:** the authorization publisher now rechecks the
  actual orchestration owner before admission and immediately before dispatch;
  its provider-wire regression passes all five scenarios, including owner loss
  at final dispatch with zero POSTs. The guarded process/source increment passes
  27 protocol, 6 process-journal, 28 worker and 1 provider-source tests, plus
  selected strict Clippy (agent-reported). Real Git candidate/execution fixtures
  pass 20 delivery and 18 execution tests; 13 decision and 11 housekeeping tests
  also pass after adopting the strict fixture executable identity.
- **Executable prerequisite:** strict pinning correctly rejects the installed
  Homebrew Git path because an ancestor is group-writable. Production policy was
  preserved. Disposable tests copy the exact installed Git bytes into a private
  fixture directory and verify their digest; this is not an installation or a
  qualification of the original path. Actual native boot identity remains denied;
  fixture probes identify their simulated boot UUID explicitly.
- **Runtime recovery review (correction in progress):** joined worker results now
  retain sealed check capabilities before receipt retirement. A failed retirement
  cannot discard the result or skip other completed workers. Empty process
  registries cannot upgrade a joined-but-uncertain process tree to confirmed exit.
  Pending correction proposals now retain a signed historical authority envelope;
  recovery checks actual admission, contract, completed turn and journal output.
  These new paths compile; focused adversarial tests remain in flight.

## Blockers and bounded follow-up dependencies

| Dependency / affected requirements | Observed evidence, permitted work, smallest next action |
| --- | --- |
| Live agent trials and account compatibility; §7, E01–08, §12.3.3 | Not attempted: no explicit accounts, finite live plan or execution authorization. Prepare cases and production runner locally; later approve named account/profile, isolated fixtures, repetition/turn/time bounds and handling of uncertain outcomes. No paid fallback. |
| Disposable Linear/GitHub integration; §§4/8/9/12 | Not attempted: remote repository writes and deployment remain prohibited, no approved disposable targets. Prepare exact account/target/action/evidence/recovery plan; explicit scoped authorization must amend applicable prohibitions before remote test actions. |
| Tailscale/media/background/locked-session/handover; §§7/8.5/10.3/11 | Required utilities not on PATH; no installation attempted. Implement adapters and deterministic fixtures; later identify installed supported paths or operator-provisioned tools/certificates and approve bounded host testing. |
| Actual macOS execution boundary; §7.1, V0-T12, E07 | Installed Codex schema generation and credential-free strict named-permission parsing succeeded. A nested sandbox probe exited 71 (`sandbox_apply: Operation not permitted`) under the existing outer sandbox. No bypass/unsandboxed retry. Continue deterministic boundary tests; actual canaries need a permitted supervised execution environment before qualification. |
| Native execution descendants; §§7.1/11, V0-T12/23/E07 | An ordinary finite owned fixture proves setsid descendants can outlive group/EOF completion. Native result promotion now fails closed with `process_tree_unverified`; implement verifiable containment/descendant ownership before native checks can pass, then qualify in an allowed supervised environment. This is an engineering gap, not permission to use group exit as full proof. |
| Native process ownership; §§6.1/11, V0-T07–09/23 | Read-only macOS boot-identity query returns `Operation not permitted` in this sandbox. Production ownership discovery fails closed; two retained native tests are explicitly blocked. Injected deterministic process fixtures verify registration/intent/lease logic only. A permitted supervised environment is required for actual process birth/boot identity, termination and restart qualification. |
| Trusted native Git executable; §§7/8/9 | The installed Homebrew Git has a group-writable ancestor and fails the unchanged executable-identity policy. Exact-byte private copies are used only inside disposable tests. Production setup needs an operator-provisioned supported executable path satisfying the owner/mode/ancestor checks; no host permissions or installation have been changed. |
| GitHub-hosted runner provenance; §§8.2/9.4 | Job labels do not prove runner type. Signed OIDC verifier now binds fixed issuer/JWKS, audience, exact job/run/attempt/workflow and time/replay guards. The trigger SHA is distinct from the source actually checked out: the reviewed pinned proof workflow must verify the tested checkout and derive the audience accordingly. Adapter fixtures pass; proof workflow preparation/collection and actual hosted route remain unqualified. |
| Human qualitative review/calibration; §12.4.3 | Operator explicitly available for supervised decisions as of the 2026-10-26 preparation message; Goal remains paused. Calibration has not been performed. Prepare representative evidence across every qualitative rubric and all material failures for the real human gate; supervision itself is not a completed review. |

These block only dependent validation, not local implementation. No missing prerequisite is permission to weaken the contract.

## Milestones and recovery notes

- Frozen continuation/successor/evaluation checkpoint: 613 passing tests, zero
  failures, two native ignores, strict lint/fmt/inventory/diff checks passed.
  Actual test and Clippy sessions exited zero; all 534 before/after input hashes
  match. The read-only evaluator reports 56 slots with dispatch disabled.
  Independent review corrections and remaining gaps are recorded in
  `validation/LOCAL-CONTINUATION-SUCCESSOR.md`. No live activity occurred.
- Local checkpoint `90ce6d4631dc605a43ba1aa4033c839832b9cf95` —
  `feat(kegari-v0): connect corrections and delivery recovery` records native
  executable composition, active correction transitions, bounded remote object
  acquisition, explicit release recovery and no-release completion. Frozen
  verification: 577 passed, two native ignored; strict lint/fmt/inventory passed.
  Signed off by the configured human committer with `Assisted-by: Codex`.
  No commits were consolidated or pushed; the worktree and source hash were
  verified immediately after committing.

- Local checkpoint `6b5a304e5898ab4ae87969ae41085c90b06e73d2` —
  `feat(kegari-v0): add guarded delivery progression` records guarded process and
  cleanup ownership, exact child/parent/release adapters, correction publication,
  runtime progression, instruction asset version 2, and evaluation archive tooling.
  Frozen verification: 523 passed, 2 native ignored, strict lint/fmt/inventory
  passed. Signed off by the configured human committer with `Assisted-by: Codex`.
  No commits were consolidated or rewritten; clean tree was verified immediately
  after commit. Later local work above is intentionally uncommitted while in flight.
- Initial inspection complete; clean scaffold contains only `fn main() {}`. No existing runtime to preserve or activate.
- Initial dependencies fetched successfully into `/private/tmp/kegari-v0-cargo-cache`; exact versions are locked. Generated Codex schema probe data is under `target/codex-schema`; selected fixtures are in `protocol/`. No model turn was launched.
- One formatter raced creation of the toolchain pin and attempted rustup sync; the sandbox rejected its temporary-file write before installation. Repeated with the existing verified stable alias successfully. No host config changed.
- Local checkpoint `24c7d4e` — `feat(kegari-v0): add durable control and instruction foundations` records strict config/workflow/assets, authority/correction/scheduling, private storage/control and snapshot primitives plus focused tests and the initial coverage ledger. No commits were consolidated or rewritten. Coordinator alone stages/commits; all source remains task-owned and initial index was empty.
- Local checkpoint `06c8786` — `feat(kegari-v0): add the pinned Codex execution adapter` records the installed-schema profile, bounded stdio adapter, explicit filesystem/tool settings, deterministic fixtures and source attribution. Generated schemas retain applicable Codex Apache license/notice; the installed alpha's exact public source commit is not established. No binary is redistributed. No commits were consolidated or rewritten.
- Local checkpoint `55712e1a6ba1de8e17d9cbf95ee166c7347a5d58` — `feat(kegari-v0): enforce isolated delivery and evidence gates` follows 18 passing focused tests, including the atomic lock-reconciliation regression. No commits were consolidated or rewritten. Later replacement/production accounting changes remain uncommitted pending review correction.
- Local checkpoint `20a932b6727c93dd5af77964efa22db66583b1ca` — `feat(kegari-v0): add scoped provider adapters and hosted proof verification` records 18 passing offline wire/proof/receipt tests and strict Clippy. Provider HTTP implementations and fixed-issuer signed proof verification are committed; executable integration and real provider compatibility remain incomplete. Separate checkpoints preserve the independently reviewable adapter, harness and delivery contracts; no history rewrite or consolidation was performed.
- Local checkpoint `92b5cd42b9acf88884006086cfb11f2d87b1f032` — `feat(kegari-v0): add version-bound agent evaluation fixtures and runner` records 28 variants/56 bounded trial slots, 20 passing deterministic runner/grader tests, full tool/mode provenance review corrections and build-bound instruction manifests. No actual-agent trials or human calibration ran. No history rewrite or consolidation occurred.
- Local checkpoint `ac79bdb7002143fb4324a895de7b4a29cc2096c8` —
  `feat(kegari-v0): track replacements and bind delivery provenance` follows 28
  passing Git/evidence fixtures and selected formatting/diff checks. The initial
  same-change commit `4a689e284c68662ae7754e3f0af2363418b79d0f` was immediately
  amended only to wrap its body at 80 characters. Recovery reference
  `refs/recovery/kegari-v0/4a689e2` was created first; its file tree and final HEAD
  were verified identical. No other commits were consolidated or rewritten.
- Local checkpoint `e9d16ba16b58ec58227178e80630e12e6bd258b5` —
  `feat(kegari-v0): compose scoped control and worker services` records the real
  provider/control/CLI/worker components, 411 passing offline tests and two
  explicitly ignored native qualifications, strict all-target Clippy and the
  subsequent affected retry-publication review correction. No commits were
  consolidated or rewritten. This remains an incomplete application checkpoint.
- Subsequent source/process/deployment/runtime changes remain in flight. All test
  processes are bounded local Cargo tests; no lasting service has been installed
  or started. Git fixtures are owned temporary repositories. No project branch,
  worktree, push, deployment or coordinator activation occurred.

Local checkpoint boundary: this increment remains separate from 946a15f because
it introduces new historical dependency and inspection consumers, whereas the
previous checkpoint also contains independent worker/driver ownership changes.
Keeping the validated consumer increment separate makes both changes reviewable;
no published history is rewritten and no remote action is authorized.


### Held paired recovery provider bridge

Root prepared the one-file metadata provider bridge at
`/private/tmp/kegari-paired-recovery-provider-draft`, manifest
c03351f2e5424784cc4c9462f0fd428d4b5b386b495338cbc834a1d90fbb753a.
It adds a mandatory-callback paired entry point over the unchanged original
metadata protections and binds exact round/recovery provenance into paired scope.
It depends on Config's typed Control layer and a real native candidate/caller;
syntax-only, not installed/compiled. It does not create model/source/ownership
capabilities or solve repeated resumed failures. Root owns future native integration.
