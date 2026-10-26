# Finite managed checks — alpha.6

This increment implements explicitly declared, finite services owned by one
validation check. The final local release suite passed 1,097 tests with no
failures and three native tests ignored across 40 groups. Milestone 0.2.0 and
operational qualification remain incomplete. The public repository-command prerequisite is
still closed. The independently approved NQ-02A foreground test failed; its
[original failure evidence](NATIVE-MANAGED-COMMAND.md) is preserved separately.

| Area | Implemented behavior | Evidence boundary |
| --- | --- | --- |
| Workflow and policy | Optional ordered services with literal start/readiness argv, finite lifetime, and explicit `check_bundle_v1` opt-in | Declarations do not grant qualification or new filesystem/network authority |
| Control | One original validation effect retains work/resource holds; unique ordered attempts persist before each launch | Serialized data cannot recreate a launch token or bypass another hold |
| Ownership | Aggregate registration precedes spawn; actual member handles and waiters survive caller cancellation | Opaque start, journal, collector or ownership failure retains uncertainty |
| Bounds | Original absolute check deadline and one captured-output allowance span all members | Async bounds cannot preempt a blocked synchronous OS or storage call |
| Services | Finite readiness plus original-leader observations, TERM shutdown, bounded KILL escalation, complete member joins | Unexpected exit or any forced group cleanup prevents successful service completion |
| Artifacts and repair | All service writers stop before source/artifact snapshots; main-check nonzero failures retain existing repair semantics after clean services | Service/readiness/shutdown failures are blockers, not code-repair evidence |
| Recovery | Exact original live owner permits warm pending-prefix observation; cold/terminal readers require exact durable membership | Missing directories, removed/substituted rows, duplicate identities and cold unknown ownership remain unresolved |
| Compatibility | Existing no-service records retain their encoding; schema-3 bundle/process records and schema-2 managed policies are explicit | Replacement metadata must advertise those formats; legacy rows are never promoted |
| Native launch correction | Fixed OS loader mappings, exact executable mappings and opt-in build mappings; native fixture pins Bash directly and persists diagnostics | The failed run did not identify its exact denial; corrected native behavior is not yet established |

Readiness is a bounded configured command plus observations of the original
service leader before and after it. It does not assert continuous service health.
Services must stay in the managed workload and handle shutdown with a clean zero
exit. A policy forbidding detachment does not prevent detached descendants.
Full descendant coverage remains a prerequisite for qualification on untrusted
external material, including public pull requests.

The current service slice grants no TCP, Unix-socket or other network access.
Endpoint services need a scoped network implementation and separate qualification.
The supported Rust/Cargo profile also still needs its complete offline dependency,
compiler, linker, SDK and generated-executable contract and actual native evidence.
A passing shell fixture alone cannot qualify those workloads.

Independent review corrected original-deadline renewal during launch preparation,
forced descendant-group cleanup missing from the service result, schema-1-only
evaluation observation, and cold-only inventory handling during warm maintenance.
Actual pre-dispatch refusals now retain zero-attempt closure; opaque callback
errors retain their original holds. Bundle registry ownership lasts until verified
terminal settlement, including after child exit. The suspected healthy-worker
pruning race through `observe_local_exits` was not established; the correction
enforces the ownership lifecycle across all pruning callers.

Component tests exercise real Control/worker/executor paths with an explicitly
injected command boundary, plus actual owned Child/group/pipe/journal objects
with synthetic identity probes. Neither class is native Seatbelt qualification.
Descriptive schema tests create no live authority or completion capability.

The worker-path tests exposed two separate defects. The new fixture initially
omitted required integrated-check configuration; after correcting it, the real
executor rejected a canonical digest containing a floating-point media limit.
The bundle plan now binds the five integer bounds actually used for checks:
command timeout, captured output, artifact bytes, source bytes and source files.
Service bounds remain in the resolved workflow. Media-only limits belong to the
separate video path. Independent review checked those producer/consumer bindings;
all five worker-path cases then passed.

The first full suite retained 1,091 passes and four fixture failures. The second
retained 1,094 passes and one existing Git-probe failure; each ignored three native
tests across 40 groups. The latter failure occurred in journaled `git --version`
startup, before the missing-object query. Its guard-release probe observed a
holder after group exit and conservatively retained uncertainty. The holder was
not identified in that run. Review found a reachable unrelated-host-fork window:
a concurrent child temporarily inherits the parent's close-on-exec descriptors
until exec completes. A launch/retirement interlock now excludes those host
fork windows without removing the permanent real-holder conflict latch. The
controlled regression pauses an actual unrelated child before exec, verifies
that retirement waits, then verifies retirement succeeds while that unrelated
process is still alive after exec. The existing detached-holder negative remains.
Another regression verifies that waiting for the interlock cannot permit a new
launch after its original deadline. All process starts use the boundary only
through synchronous spawn return; no child wait, Control callback or async await
runs under its mutex. Metadata, browser and service-control helpers retain one
original cutoff and their existing timeout categories. Independent review found
no remaining material finding in this correction.

NQ-02B was separately approved and executed once after the final local suite.
It failed its first Bash case with SIGABRT before script execution; owned-group,
journal and collector closure succeeded, and the other two cases were not run.
The matching OS crash stack locates the abort in dyld/libignition bootstrap.
The strongest next correction is Apple's documented literal-root/fixed-Cryptex-
ancestor read access, missing from the tested profile. The exact denial remains
unproven. See [NQ-02B evidence](NATIVE-MANAGED-RECHECK.md). Its pin predates the
final interlock; neither this failure nor the local suite opens qualification.
NQ-01B is still unexecuted. Both NQ-02A and NQ-02B single-use approvals are consumed.

The exact development executable is:

`target/prereleases/kegari-v0-0.2.0-alpha.6-aarch64-apple-darwin-f273893778f2/kegari-v0`

- SHA-256: `f273893778f2d48f50f737da6e0a1e553a1c067c6f1d07f82a8410b066f88af7`
- Build identity: `sha256:a75a0af27f0150c1a0bcaca1e9e91bb0daa2cc98088c30f1ab48c967895ec98e`
- 41,263,104 bytes, mode 0555, release profile, aarch64-apple-darwin.
- Rust/Cargo 1.99.1; the full suite and build share the final 723-file test/input
  manifest. Instruction assets are separately checked against embedded metadata.

Final checks: release suite 1,097/0/3; focused process/interlock and Git lifecycle
runs each passed 12 tests; all-target clippy with warnings denied, formatting and
contract inventory passed (503 clauses, 136 named requirements, no promotions).
Five staged-binary inspections passed: `--help`, `--version`, `setup --help`,
`__build-info`, `__handover-info`. They ran with closed stdin, a minimal environment
and an empty disposable cwd, with zero exit codes, empty stderr and no cwd writes.
No installation or activation occurred. This is not whole-binary reproducibility
or operational qualification.

[Machine validation history](managed-check-bundles-checks.json) records commands,
actual outcomes, failed attempts, source snapshots, reviews, artifact checksums
and exact native limitations. Raw logs and preserved earlier source are copied
under the artifact's `validation/` directory. Initial compile/fixture/lint failures,
the canonical-float bug and the full2 guard conflict remain in that history.
An independent final artifact/evidence audit matched all 325 build inputs, 723
final test inputs and 931 referenced evidence hashes, recomputed the build
identity and reconciled the native failure. No material mismatch was found.
The archived file named `kegari-alpha6-final-test-source.json` is the historical
722-input full2 snapshot; the artifact's `test-source-inputs.json` is the final
723-input full3 snapshot. Neither historical failure is replaced by the later pass.

Next correct and qualify loader bootstrap, then implement the closed offline
Rust/Cargo contract in [the qualification plan](../docs/managed-qualification-design.md).
Endpoint services also need scoped network support and native service cases.

Application version: `0.2.0-alpha.6`. Instruction assets remain version 3;
handover metadata remains version 3, requiring process schema 3 and managed
execution schema 2. Ordinary process rows remain schema 2; member rows use schema
3. Checks without services retain schemas 1/2; service-bearing checks use schema 3.
Configuration, control API and receipt schemas remain version 1.
