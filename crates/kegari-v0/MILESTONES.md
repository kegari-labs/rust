# Kegari v0 version milestones

**Current target: 0.2.0. Active work packages: M02-2–M02-3, integrate parent discovery after verified bounded child validation. Goal: active; unattended.**

Plan revised on 2026-10-27 under [SPEC 1.4](SPEC.md). This is the single
authoritative version/milestone plan referenced by [GOAL.md](GOAL.md) and the
attached goal objective. SPEC controls product requirements; existing permissions
control actions. [PROGRESS.md](PROGRESS.md) holds detailed evidence and history.
Editing this plan does not resume the Goal, start implementation or authorize
native/live operations, installation, publication or deployment.

The user reported **1d 20h 15m 16s** of elapsed goal time at this checkpoint.
Prioritize closing these milestones through coherent implementation and evidence;
elapsed effort, test counts and prerelease numbers are not completion measures.
No minor milestone below is complete. Existing partial implementations must be
reconciled, not rebuilt indiscriminately or presumed compliant.

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
- Preserve generic supervision and truthful outcomes. Remove/rework incompatible
  implementation across the repository, including committed code and the pending
  alpha.9 work; do not hide it behind optional flags. Preserve historical evidence,
  failures and unrelated work without inheriting old passes for changed contracts.
- Required Git, Codex, Linear, GitHub and OS integrations, Kegari's own Rust build,
  and representative test data remain permitted. They cannot become loopholes for
  target-toolset execution logic.

## Version sequence and completion gates

Each row is a delivery checkpoint within v0, not a claim that the full application
is qualified for operational use. Apply the common exit audit below to every row.

| Version | Outcome and completion gates | Current state / dependency |
| --- | --- | --- |
| **0.2.0** | **Generic execution foundation and reviewed executable.** Remove the rejected design; deliver agent-led discovery without command/environment prerequisites, generic operation-bound execution/evidence, and foreground/registered-service lifecycle through completion, cancellation and cleanup. Complete M02-1–M02-6 below, including a bounded native viability subset and exact-source build. | **In progress; immediate priority.** Existing supervision is reusable; pending alpha.9 is superseded development work, not a release to finish as designed. |
| **0.3.0** | **Cold recovery and current authority.** Complete workflow recovery/reassembly, provider-intervention resolution/readmission, and recovery of discovered obligations, invocations and uncertain effects. Reject stale authority/context/results; retain intent, budgets, ownership and unknowns without blind replay. | Queued after 0.2.0; audit existing partial recovery against the corrected records and semantics. |
| **0.4.0** | **External-target and visual evidence.** Complete external-target identity/proof and authenticated visual evidence recovery, bound to exact candidates. Reject stale/substituted evidence and preserve unavailable/uncertain outcomes. | Queued after 0.3.0; depends on recovery and provenance. Actual upload/playback/target compatibility remains separately identified until exercised. |
| **0.5.0** | **Operator handover and installation route.** Complete the authenticated qualification-record workflow, immutable installation/handover procedures, compatibility checks, safe refusal/recovery and operator guidance. Real evidence and human decisions remain required. | Queued after 0.4.0; implementing the route does not authorize installation or qualify an untested replacement. |
| **0.6.0** | **Complete evaluation machinery.** Deliver production-path callers, finite observation, authenticated grading, all required contexts and genuine prior-state preparation for continuation/recovery. Include E01–E09 fixtures/rubrics, scorer isolation and positive/negative grader tests; leave no integration placeholders as substitutes for implemented behavior. | Queued after 0.5.0; complete runner/fixture implementation is distinct from completing actual-agent trials. |
| **0.7.0** | **Full managed-v0 qualification.** Complete all required separately authorized native-host, real-provider and repeated actual-agent validation, human calibration and final independent audit. Resolve material failures, reuse still-valid evidence and rerun affected cases against the final implementation. Audit every SPEC requirement before declaring the comprehensive Goal complete. | Queued after 0.6.0; includes unresolved qualification obligations retained from earlier milestones. |
| **1.0.0 / v1** | A future, explicitly agreed production-readiness decision. | Outside this Goal. Do not rename `kegari-v0`, start the lasting build or infer production readiness from a completed 0.x milestone. |

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
| **M02-1 — Correct the implementation baseline** | Audit the whole repository against SPEC 1.4. Remove/rework target-toolset code and mandatory workflow/environment/command assumptions, including the pending Cargo paths and their configuration/schema/test/doc dependencies. Retain generic safeguards. Reconcile the approved-spec inventory and affected coverage entries without promoting stale evidence. | Reviewable removal/rework inventory; independent architecture review; retained applicable regressions; new/changed requirements explicitly unverified until tested. | **In progress.** Pending Cargo-specific path removed; default workflow and stale-coverage reconciliation implemented. Process-guard ordering/reuse correction, local checks and independent review are recorded in PROGRESS. Catalog removed; automatic-domain migration, two freshness corrections and local verification recorded in [domain evidence](validation/LOCAL-GENERIC-COMMAND-DOMAIN.md). Bounded child discovery/check/adequacy integration is locally verified in [child evidence](validation/LOCAL-DISCOVERED-VALIDATION.md); parent and continuation interfaces remain open. |
| **M02-2 — Complete generic execution** | Provide one generic command path for exact argv/cwd, automatically observed environment, source/input/scratch boundaries, original operation authority, bounded output/deadlines and actual outcomes. Dynamic commands, generated programs and transitive tools use the same enforced boundary. | Focused production-path deterministic tests, including unfamiliar commands and capability-specific refusal. No per-ecosystem runtime branches or catalog prerequisite. | Automatic domain/ordered lookup locally verified; failed full-run fixtures and their successful correction reruns retained. Generic worker argv/cwd invocation, original deadlines, bounded output, actual outcome records and conservative history readback are locally verified; independent findings corrected. Full 1,153-pass regression and final focused checks are source-bound in [command evidence](validation/LOCAL-GENERIC-COMMANDS.md). Native execution remains unqualified. Bounded child-local discovered obligations now have local component evidence. Environment admission and parent integrated discovery remain open. |
| **M02-3 — Deliver discovery and usable onboarding** | Use installed generic guidance when no workflow is selected. Let agents discover commands and validation obligations from existing projects, and derive scoped preparation for empty repositories. Record real absent/unborn baselines, decisions and required checks; preserve explicit-selection errors and actual missing-authority blockers. | Production context/command-flow tests for existing, empty and unfamiliar-tool cases; T01/T33/T38/T39 coverage; review of failure/omission handling and the launch/setup guidance. Actual-agent claims require actual trials, not fixtures. | Bounded child-local read provenance, plan enrollment, no-catalog execution and adequacy gates are locally verified. Parent integrated discovery, empty/unborn baselines, broader discovery fixtures and actual-agent conformance remain open. |
| **M02-4 — Reconcile lifecycle and evidence** | Integrate generic commands/checks/optional hooks/services with cancellation, retirement, source/artifact checks and cleanup. Preserve writer closure, uncertain outcomes, correction races and honest required-check completeness. Remove any blanket closed-toolset refusal only when generic backend prerequisites are established. | Focused lifecycle/failure/race regressions and independent review. EOF, zero exit and a configuration declaration cannot substitute for the required observations. | Generic invocation ownership/closure/history now has local component evidence. Discovered child checks/services, complete attempt inventory, original settlement holds and adequacy gates are locally verified in [child evidence](validation/LOCAL-DISCOVERED-VALIDATION.md). Cross-worker lineage, large evidence transport, full output access and cold process reconciliation remain open; native closure is unqualified. |
| **M02-5 — Establish bounded native viability** | Prepare pinned, finite requests for the changed generic backend: foreground/transitive/generated execution, registered services, unsuccessful exit, timeout/cancellation, denied access and uncertain closure. Execute only the separately approved cases and resolve material findings. | Actual native evidence tied to the tested source/backend. Historical NQ-02C is limited alpha.7 evidence and its approval is consumed. Missing approval/evidence leaves this package and 0.2.0 incomplete. | Not run for the corrected design; approval required. |
| **M02-6 — Deliver 0.2.0** | Close remaining package findings, run appropriate regression/release checks, independently review, build and inspect an exact-source release executable with matching assets, and document launch/setup plus remaining qualification limits. | Exit audit links to source/commit, commands/results, executable digest/build identity, asset/schema versions and review. A runnable client or high test count alone cannot close the milestone. | Pending M02-1–M02-5. |

M02-5 establishes only the exercised backend's native viability. Full repeated
E01–E09 behavior evaluation, all real integrations and required human calibration
close at 0.7.0. Prepare and run useful affected trials earlier when authorized;
retain their exact provenance and reuse them only while valid. Do not defer a
check necessary to substantiate a 0.2.0 claim by calling it a 0.7.0 concern.

## Prereleases and compatible corrections

| Version label | Intended use |
| --- | --- |
| `0.2.0-alpha.8` | Preserve the existing committed/artifact baseline and its historical evidence. It is not SPEC 1.4 qualification. |
| `0.2.0-alpha.9` | Current development checkpoint with locally verified workflow, guard, automatic-domain, generic-invocation and bounded child-validation changes. No newly packaged release artifact. Continue M02-1–M02-4; the old Cargo packaging plan is abandoned. Reconcile any newly discovered packaged artifact before reusing a label. |
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

A permission/resource blocker blocks dependent work only. Continue useful
authorized work within the current milestone; if only later independent work is
available, identify that exception and keep the earlier milestone visibly open.
Do not repeatedly retry unchanged denied actions or quietly substitute a different
milestone for the blocked one. Required qualifications remain required.

## Progress reporting during resumed goal work

- On resumption, reconcile this plan, PROGRESS, actual source/staging, evidence,
  pending processes and approvals. State the current target and next package before
  implementation. Select the first incomplete eligible package; the target at this
  planning checkpoint is **0.2.0 / M02-1**.
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

Suggested compact update: **`0.2.0 · M02-1 — Completed: … Evidence: … Remaining:
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
