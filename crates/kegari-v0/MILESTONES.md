# Kegari v0 version milestones

**Workspace extraction checkpoint reconciled on 2026-10-29. Goal mode cancelled; work proceeds through individual prompts. No minor milestone is complete.**

All seventeen workspace packages remain `0.2.0-alpha.10`. The completed
extractions establish the boundaries in the [workspace overview](../../docs/workspace.md);
they preserve application behavior and do not close product or qualification gates.
Production repository commands remain blocked pending generic execution
qualification. The archived alpha.10 package is unchanged; no new prerelease was
packaged during extraction. Historical automatic-continuation instructions do not
authorize further work or commits.

Plan revised on 2026-10-27 under [SPEC 1.4](SPEC.md), with implementation states
reconciled on 2026-10-29 without changing the version sequence. This is the single
authoritative version/milestone plan referenced by [GOAL.md](GOAL.md) and the
attached goal objective. SPEC controls product requirements; existing permissions
control actions. [PROGRESS.md](PROGRESS.md) holds detailed evidence and history.
Editing this plan does not resume the Goal, start implementation or authorize
native/live operations, installation, publication or deployment.

The user reported **1d 20h 15m 16s** of elapsed goal time at the 2026-10-27 planning checkpoint.
Prioritize closing these milestones through coherent implementation and evidence;
elapsed effort, test counts and prerelease numbers are not completion measures.
No minor milestone below is complete. Existing partial implementations must be
reconciled, not rebuilt indiscriminately or presumed compliant.

## Current repository checkpoint

The latest extraction was committed as
`4989e93e10bf84335718aeceb288dbad7e4b0aa7`
(`refactor(execution)!: complete native lifecycle extraction`). This subsequent
commit supersedes the uncommitted status recorded at the implementation handoff
in PROGRESS and the extraction report; their historical verification remains valid.

| Checkpoint | Evidence and identity |
| --- | --- |
| Archived alpha.10 release | 1,260 tests passed, zero failed, three native ignores. The original archive and its source identity remain those recorded in the [stabilization report](validation/LOCAL-DISCOVERY-ADMISSION.md). |
| Current refactored source at `4989e93` | 1,366 tests passed, zero failed, exactly three native ignores; formatting, strict workspace Clippy, contract inventory, bounded independent review, optimized arm64 build and five state-free executable inspections passed. The [final extraction report](validation/EXECUTION-LIFECYCLE-EXTRACTION.md) records the source, build and executable digests. This is a verified source checkpoint, not a newly packaged release or native qualification. |

`kegari-cli` owns the `kegari-v0` executable. Controller/control API, context,
scheduler, Codex protocol, providers and foundation crates own their extracted
subsystems. `kegari-execution` now owns managed environments and native command/
journal mechanics. Git execution, native Codex integration, durable admission,
runtime orchestration, configuration administration, delivery and protected
recovery remain backend-owned. The crate split did not change the process
arrangement; CLI and controller still temporarily depend on `kegari-v0`.

Use the current [developer guide](../../docs/development.md) and
[CLI reference](../../docs/cli.md). Before any separately requested goal resumption,
reconcile the historical GOAL/objective instructions that still place the main
executable in the backend package or retain the old paused documentation hold.
They do not override the cancelled goal state or the current workspace boundaries.

## Direction that applies to every version

- Kegari contains no target-toolset-specific orchestration/harness code, including
  optional adapters, manifest/lockfile parsers, command rewriters, dependency
  validators or compiler/cache/library special cases.
- Users do not have to supply environment specifications, executable inventories,
  exact-digest catalogs or predefined command lists. Agents retrieve/infer commands
  from the repository, applicable guidance, observed environment and authorized
  task. Empty repositories and unfamiliar tools are valid starting points.
- The host enforces generic authority, scope, filesystem/network/credential
  boundaries, lifecycle, bounds and evidence. Automatically recorded observations
  and validation obligations are evidence, not another user-maintained manifest.
- Preserve generic supervision and truthful outcomes. Remove/rework any remaining
  incompatible implementation across the repository, including work inherited
  from the historical alpha.9 checkpoint; do not hide it behind optional flags. Preserve historical evidence,
  failures and unrelated work without inheriting old passes for changed contracts.
- Required Git, Codex, Linear, GitHub and OS integrations, Kegari's own Rust build,
  and representative test data remain permitted. They cannot become loopholes for
  target-toolset execution logic.

## Version sequence and completion gates

Each row is a delivery checkpoint within v0, not a claim that the full application
is qualified for operational use. Apply the common exit audit below to every row.
Queued milestones already have partial implementations; reconcile and complete
them with their current owners rather than rebuilding them because they moved.

| Version | Outcome and completion gates | Current state / dependency |
| --- | --- | --- |
| **0.2.0** | **Generic execution foundation and reviewed executable.** Remove the rejected design; deliver agent-led discovery without command/environment prerequisites, generic operation-bound execution/evidence, and foreground/registered-service lifecycle through completion, cancellation and cleanup. Complete M02-1–M02-6 below, including a bounded native viability subset and exact-source build. | **In progress; immediate priority.** Extracted execution infrastructure and local integration evidence are available. Production repository commands remain blocked; empty/unborn sources, discovered repair lineage, paging and native viability remain open. The refactor is not a milestone exit. |
| **0.3.0** | **Cold recovery and current authority.** Complete workflow recovery/reassembly, provider-intervention resolution/readmission, and recovery of discovered obligations, invocations and uncertain effects. Reject stale authority/context/results; retain intent, budgets, ownership and unknowns without blind replay. | Queued after 0.2.0; partial implementation exists. Protocol recovery belongs to `kegari-codex`, context compatibility to `kegari-context`, and journal observations to `kegari-execution`; protected recovery remains backend-owned. [Runtime recovery](src/runtime_recovery.rs) is still same-process and limited to child implementation. Cold reconstruction, other phase integration and intervention readmission remain open. |
| **0.4.0** | **External-target and visual evidence.** Complete external-target identity/proof and authenticated visual evidence recovery, bound to exact candidates. Reject stale/substituted evidence and preserve unavailable/uncertain outcomes. | Queued after 0.3.0; backend deployment proofs and media/evidence validation exist, using extracted provider and signing primitives. External-target coverage and visual publication/retrieval integration remain incomplete; [parent recovery](src/delivery_parent.rs) still refuses visual evidence without authenticated retrieval. Actual upload/playback/target compatibility remains separately identified until exercised. |
| **0.5.0** | **Operator handover and installation route.** Complete the authenticated qualification-record workflow, immutable installation/handover procedures, compatibility checks, safe refusal/recovery and operator guidance. Real evidence and human decisions remain required. | Queued after 0.4.0; CLI installation and backend [qualification/handover primitives](src/housekeeping.rs) exist. The qualification/handover operator workflow still lacks production callers. Implementing that route does not authorize installation or qualify an untested replacement; actual handover requires real qualification evidence. |
| **0.6.0** | **Complete evaluation machinery.** Deliver production-path callers, finite observation, authenticated grading, all required contexts and genuine prior-state preparation for continuation/recovery. Include E01–E09 fixtures/rubrics, scorer isolation and positive/negative grader tests; leave no integration placeholders as substitutes for implemented behavior. | Queued after 0.5.0; backend suites, runners, reports and grading exist, using extracted context and Codex observation interfaces. The [production caller](src/runtime_evaluation.rs) remains limited to fresh independent-review trials; broader contexts and genuine continuation/recovery preparation are open. Complete machinery remains distinct from actual-agent trial completion. |
| **0.7.0** | **Full managed-v0 qualification.** Complete all required separately authorized native-host, real-provider and repeated actual-agent validation, human calibration and final independent audit. Resolve material failures, reuse still-valid evidence and rerun affected cases against the final implementation. Audit every SPEC requirement before declaring the comprehensive Goal complete. | Queued after 0.6.0; deterministic regression and extraction-review evidence exists. Native/provider/actual-agent qualification, human calibration and final product audit remain open, including obligations retained from earlier milestones. |
| **1.0.0 / v1** | A future, explicitly agreed production-readiness decision. | Outside this plan's implementation scope. Retain the `kegari-v0` application/executable identity; this does not prohibit the separately agreed eventual retirement of the internal backend package. Do not start the lasting build or infer production readiness from a completed 0.x milestone. |

Full descendant coverage remains required before qualification for arbitrary
untrusted external context, including public PR material. The current managed
scope accepts unregistered detached descendants outside its shutdown guarantee;
known or suspected uncertainty still blocks affected completion/cleanup/handover.
The user reported an Endpoint Security entitlement application on 2026-10-26;
approval is not established. Revisit its outcome and the broader containment
decision before v1. No particular platform mechanism is mandated, and the
application is not a prerequisite to the current managed-v0 milestones.

## Immediate 0.2.0 work packages

Work in this order when dependencies require it; parallelize independent tasks
within the current package when useful. Root owns integration and progress records.
Update states and evidence links here as work is completed, without copying logs.

| Package | Required outcome | Exit evidence | Current state |
| --- | --- | --- | --- |
| **M02-1 — Correct the implementation baseline** | Audit the whole repository against SPEC 1.4. Remove/rework target-toolset code and mandatory workflow/environment/command assumptions, including the previously rejected Cargo paths and their configuration/schema/test/doc dependencies. Retain generic safeguards. Reconcile the approved-spec inventory and affected coverage entries without promoting stale evidence. | Reviewable removal/rework inventory; independent architecture review; retained applicable regressions; new/changed requirements explicitly unverified until tested. | **Partially complete; baseline exit audit remains open.** Cargo-specific path removed; default workflow and stale-coverage reconciliation implemented. Process-guard ordering/reuse correction, local checks and independent review are recorded in PROGRESS. Catalog removed; automatic-domain migration, two freshness corrections and local verification recorded in [domain evidence](validation/LOCAL-GENERIC-COMMAND-DOMAIN.md). Bounded child discovery/check/adequacy integration is locally verified in [child evidence](validation/LOCAL-DISCOVERED-VALIDATION.md); bounded parent discovery/adequacy is now locally verified in [integrated evidence](validation/LOCAL-INTEGRATED-DISCOVERY.md); compact publication with complete authenticated history/artifacts is locally verified in [archive evidence](validation/LOCAL-VALIDATION-ARCHIVES.md); generic environment/resource inspection is locally verified ([evidence](validation/LOCAL-RESOURCE-INSPECTION.md)). Context, config and execution now own the corresponding extracted mechanisms. Reconcile the existing removal/rework inventory against the current workspace; extraction reviews do not close the whole-repository SPEC audit. Empty/unborn support and discovered repair lineage remain the concrete gaps tracked in M02-3/M02-4. |
| **M02-2 — Complete generic execution** | Provide one generic command path for exact argv/cwd, automatically observed environment, source/input/scratch boundaries, original operation authority, bounded output/deadlines and actual outcomes. Dynamic commands, generated programs and transitive tools use the same enforced boundary. | Focused production-path deterministic tests, including unfamiliar commands and capability-specific refusal. No per-ecosystem runtime branches or catalog prerequisite. | Automatic domain/ordered lookup locally verified; failed full-run fixtures and their successful correction reruns retained. Generic worker argv/cwd invocation, original deadlines, bounded output, actual outcome records and conservative history readback are locally verified; independent findings corrected. The historical 1,153-pass regression and final focused checks are source-bound in [command evidence](validation/LOCAL-GENERIC-COMMANDS.md). Bounded child-local discovered obligations now have local component evidence. Parent integrated discovery and prelaunch resource-freshness correction are locally verified in [integrated evidence](validation/LOCAL-INTEGRATED-DISCOVERY.md). Existing-boundary environment/resource inspection is locally verified without probes or new grants ([inspection evidence](validation/LOCAL-RESOURCE-INSPECTION.md)); additional host resources still require legitimate authority. **Production repository commands remain blocked:** the [execution prerequisite](../kegari-execution/src/lib.rs) returns `process_tree_unverified` or `managed_profile_unqualified`, with no success branch. Environment and native command mechanisms now belong to `kegari-execution`; backend admission is retained. Define and establish the generic qualification conditions before replacing this blanket refusal; current extraction evidence does not enable it. |
| **M02-3 — Deliver discovery and usable onboarding** | Use installed generic guidance when no workflow is selected. Let agents discover commands and validation obligations from existing projects, and derive scoped preparation for empty repositories. Record real absent/unborn baselines, decisions and required checks; preserve explicit-selection errors and actual missing-authority blockers. | Production context/command-flow tests for existing, empty and unfamiliar-tool cases; T01/T33/T38/T39 coverage; review of failure/omission handling and the launch/setup guidance. Actual-agent claims require actual trials, not fixtures. | Bounded child-local read provenance, plan enrollment, no-catalog execution and adequacy gates are locally verified. A genuine parent assignment with no configured commands now reaches worker discovery/execution in local fixtures, with independent adequacy gates ([evidence](validation/LOCAL-INTEGRATED-DISCOVERY.md)). Bounded process-free resource/environment discovery is locally verified ([inspection evidence](validation/LOCAL-RESOURCE-INSPECTION.md)). Alpha.10 locally verifies discovery admission with unresolved downstream delivery policy, preserving dependent gates ([stabilization evidence](validation/LOCAL-DISCOVERY-ADMISSION.md)). Default guidance/configuration now belong to context/config crates; source preparation remains backend-owned. [Source selection](src/runtime_workers.rs) still requires an exact commit/tree [revision](../kegari-git/src/lib.rs). Empty/unborn baselines, broader discovery fixtures and actual-agent conformance remain open. Current [CLI setup guidance](../../docs/cli.md) is available but does not implement the missing source states. |
| **M02-4 — Reconcile lifecycle and evidence** | Integrate generic commands/checks/optional hooks/services with cancellation, retirement, source/artifact checks and cleanup. Preserve writer closure, uncertain outcomes, correction races and honest required-check completeness. Remove any blanket closed-toolset refusal only when generic backend prerequisites are established. | Focused lifecycle/failure/race regressions and independent review. EOF, zero exit and a configuration declaration cannot substitute for the required observations. | Generic invocation ownership/closure/history now has local component evidence. Discovered child checks/services, complete attempt inventory, original settlement holds and adequacy gates are locally verified in [child evidence](validation/LOCAL-DISCOVERED-VALIDATION.md). Integrated adequacy ownership, terminal-failure retirement and original-resource prelaunch checks are locally verified ([evidence](validation/LOCAL-INTEGRATED-DISCOVERY.md)). Compact schema-3 publication, complete history/all-attempt artifact archives, sealed original-byte handoff and bounded actual reviewer input are locally verified ([archive evidence](validation/LOCAL-VALIDATION-ARCHIVES.md)). Native command/journal mechanisms now reside in `kegari-execution`, with [backend admission](src/process_records.rs) retained and [extraction verification](validation/EXECUTION-LIFECYCLE-EXTRACTION.md) complete. Cross-worker discovered-validation repair still refuses with `validation_lineage_pending` ([continuation](src/delivery_continuation.rs)); [review input](src/delivery_discovered_validation.rs) still lacks paging beyond its 1 MiB bound. These gaps and cold process reconciliation remain open; native closure is unqualified. |
| **M02-5 — Establish bounded native viability** | Prepare pinned, finite requests for the changed generic backend: foreground/transitive/generated execution, registered services, unsuccessful exit, timeout/cancellation, denied access and uncertain closure. Execute only the separately approved cases and resolve material findings. | Actual native evidence tied to the tested source/backend. Historical NQ-02C is limited alpha.7 evidence and its approval is consumed. Missing approval/evidence leaves this package and 0.2.0 incomplete. | **Open; no new native qualification during extraction.** The moved [ignored foreground fixture](../kegari-execution/src/command_native_tests.rs) covers output/access, exit 7 and timeout; it neither exercises authenticated admission nor issues reusable qualification. It does not cover the complete generated/transitive-command, service, cancellation and uncertain-closure matrix. Prepare bounded source-pinned requests; separate approval remains required. |
| **M02-6 — Deliver 0.2.0** | Close remaining package findings, run appropriate regression/release checks, independently review, build and inspect an exact-source release executable with matching assets, and document launch/setup plus remaining qualification limits. | Exit audit links to source/commit, commands/results, executable digest/build identity, asset/schema versions and review. A runnable client or high test count alone cannot close the milestone. | **Pending M02-1–M02-5.** The current source has a reviewed optimized executable and successful metadata/state-free inspections ([report](validation/EXECUTION-LIFECYCLE-EXTRACTION.md)). Build mechanics are verified; product exit criteria are not. No new release was packaged from the refactored source. |

M02-4 retains the minimum safe cold process reconciliation needed to establish
ownership, closure or uncertainty before 0.2.0 completion/cleanup claims. Version
0.3.0 covers full cold workflow reconstruction and resumption across phases. This
distinction does not move or waive any existing 0.2.0 criterion.

M02-5 establishes only the exercised backend's native viability. Full repeated
E01–E09 behavior evaluation, all real integrations and required human calibration
close at 0.7.0. Prepare and run useful affected trials earlier when authorized;
retain their exact provenance and reuse them only while valid. Do not defer a
check necessary to substantiate a 0.2.0 claim by calling it a 0.7.0 concern.

## Prereleases and compatible corrections

| Version label | Intended use |
| --- | --- |
| `0.2.0-alpha.8` | Preserve the existing committed/artifact baseline and its historical evidence. It is not SPEC 1.4 qualification. |
| `0.2.0-alpha.9` | Historical development checkpoint with locally verified workflow, guard, automatic-domain, generic-invocation, child/parent discovery, adequacy, authenticated archives and bounded environment/resource inspection. Instruction assets are version 7. No alpha.9 release artifact was packaged; the old Cargo packaging plan is abandoned. Its useful work is retained in the alpha.10 stabilization candidate. |
| `0.2.0-alpha.10` | **Historical packaged checkpoint, before crate extraction.** Six regression failures corrected; final release suite 1,260 passed, 0 failed, 3 native ignored. Strict lint/format/inventory checks, bounded independent review and extracted-package inspection passed. Native execution stays blocked; no milestone completion or operational qualification is claimed. The original stabilization pass stopped before crate separation; later separately authorized extractions are recorded above and did not replace this archive. |
| `0.2.0-alpha.N` / optional `0.2.0-rc.N` | Use a new identifier for each newly distributed or retained release artifact when source changes. A release candidate may identify a feature-complete candidate awaiting final exit audit. Do not bump for every edit, test run or finding, or create an artificial prerelease train. |
| `0.2.0` through `0.7.0` | Close the corresponding minor milestone only after its actual exit audit. Use prereleases while criteria remain unmet. |
| `0.2.1`, `0.3.1`, … | Reserved for demonstrated compatible corrections to the corresponding completed minor release. Later patches increment normally. New capability or incompatible persisted formats belong to a subsequent minor milestone/prerelease, not a patch. These are conditional correction slots, not mandatory extra milestones. |

Keep application, instruction, protocol, configuration and evidence identities
explicit and synchronized where required. Do not overwrite historical artifacts
or relabel earlier evidence. Current planning changes do not alter package versions.

## Exit audit and focus rules

Before declaring a milestone complete, record its implemented outcome, relevant
local/native/agent/human evidence separately, independent review and resolution of
material findings, exact source/artifact identity, and every remaining obligation
assigned to a later milestone. An unmet criterion of the current milestone cannot
be silently reassigned merely to publish its version. Later milestones do not
waive SPEC requirements or authorize the lasting coordinator.

Choose the smallest coherent work package that closes the current exit criteria.
Every proposed task must name the criterion it advances or the concrete defect
that blocks it. Avoid speculative frameworks, new ecosystem support, unrelated
polish and repeated bookkeeping/retesting without changed inputs or new findings.
Once relevant checks pass, close that increment and advance its package state.

The recommended next capability plan is bounded generic execution viability and
enablement across M02-2/M02-4/M02-5, after reconciling M02-1 from existing evidence.
Specify the evidence needed to replace the blanket execution refusal and retain
that refusal until its conditions are established. Empty/unborn source support
and authenticated review/output paging are concrete independent increments where
authorized. Further crate extraction is not a prerequisite imposed by this plan.
This recommendation authorizes no implementation or native operation.

A permission/resource blocker blocks dependent work only. Continue useful
authorized work within the current milestone; if only later independent work is
available, identify that exception and keep the earlier milestone visibly open.
Do not repeatedly retry unchanged denied actions or quietly substitute a different
milestone for the blocked one. Required qualifications remain required.

## Progress reporting during resumed goal work

Goal mode remains cancelled. The instructions below apply only after a separate
explicit resumption; regular execution prompts remain the current working method.

- On resumption, reconcile this plan, PROGRESS, actual source/staging, evidence,
  pending processes and approvals. State the current target and next package before
  implementation. Select the first incomplete eligible package. The version target
  remains **0.2.0**: reconcile M02-1 from retained evidence, then plan the next
  concrete capability increment above without reopening completed extractions.
- Keep the user informed during active work with concise meaningful updates. At
  every goal-turn handoff, identify the target version/package, what changed since
  the previous checkpoint, evidence established, remaining exit criteria, the next
  concrete action, and any exact approval/resource blocker. Do not report unchanged
  activity as progress or invent percentages, deadlines or completion estimates.
- Update this file's package/milestone states and PROGRESS after a coherent
  increment, material failure/correction, blocker change, exit audit and before
  handoff. Keep detailed commands/logs in PROGRESS or linked validation records;
  do not rewrite the full plan after every tool call.
- On a milestone transition, explicitly announce the completed version, artifact
  and evidence, retained limitations, and next version. If no milestone criterion
  advanced, say why and identify the action that can unblock it.

Suggested compact update: **`0.2.0 · M02-<package> — Completed: … Evidence: … Remaining:
… Next: … Approval needed: …`** Omit empty fields. These are instructions for
updates within this Goal conversation, not a request to create scheduled jobs or
send messages elsewhere.

## Plan history

- **2026-10-26:** Agreed minor sequence: 0.2 managed execution/build; 0.3 recovery;
  0.4 external/visual evidence; 0.5 handover; 0.6 evaluation machinery;
  0.7 full managed-v0 qualification.
- **2026-10-27:** SPEC 1.4 prohibited target-toolset execution code and prerequisite
  environment/command specifications. This plan retains the minor sequence,
  corrects 0.2.0 first, establishes its work packages and exit evidence, and adds
  milestone progress reporting. The Goal remains paused pending explicit resume.

- **2026-10-27, resumption:** User resumed the Goal unattended. M02-1 is active;
  preserve all native/live approval boundaries and report dependent blockers.

- **2026-10-28, cancellation and stabilization:** User cancelled goal mode and selected regular planning/execution prompts. Alpha.10 is a clean checkpoint before a separately planned crate split; native execution remains blocked.

- **2026-10-28, CLI extraction:** The first approved extraction is locally verified:
  `kegari-cli` owns the existing executable and CLI/client modules, temporarily
  depending on `kegari-v0`. The full workspace suite passed 1,260 tests with three
  native ignores; independent review and build/identity checks passed. Workspace
  version stays alpha.10, its archived package is unchanged, and no new prerelease
  or minor milestone is claimed. See [PROGRESS.md](PROGRESS.md).

- **2026-10-29, post-extraction reconciliation:** Audited the seventeen-crate
  workspace at `4989e93e10bf84335718aeceb288dbad7e4b0aa7`. The individual
  extractions through complete execution infrastructure ownership are committed;
  the latest source verification is 1,366 passed, zero failed and three native
  ignores, with review, lint/format/inventory and executable inspections passed.
  Recorded current owners and remaining capability gates, distinguished this
  source checkpoint from the unchanged 1,260-pass alpha.10 archive, and clarified
  process reconciliation versus full workflow recovery. The 0.2–0.7 sequence,
  exit criteria, application identity and approval restrictions remain intact.
  No minor milestone is complete, no new release is packaged and goal mode remains
  cancelled. See the [final extraction report](validation/EXECUTION-LIFECYCLE-EXTRACTION.md).
