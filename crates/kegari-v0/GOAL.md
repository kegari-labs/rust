# Six-hour Codex orchestration goal

Approved on 2026-10-29. This replaces the cancelled historical assignment.
Reading, editing or committing this file does not activate a goal. Start a new
Goal only on the user's explicit activation; do not resume the obsolete objective.
The activated run and absolute timestamps are recorded in PROGRESS.md.

This repository file is the single authoritative goal assignment. The attachment
objective points here; it is not a separately maintained copy. Historical progress
and verification reports retain evidence only, not alternative implementation
instructions. SPEC 1.5 and the current milestone queue govern the redesign.

## Objective

Implement Kegari as an orchestrator of Codex, working unattended for at most
**six hours from activation**. Follow [SPEC 1.5](SPEC.md), the ordered feature
queue in [MILESTONES.md](MILESTONES.md), and the actual state in
[PROGRESS.md](PROGRESS.md).

Codex owns repository reasoning, inspection, editing, command execution, sandbox
enforcement and supported terminal management. Kegari owns workflow authority,
scheduling, role/scope selection, context, account selection, provider writes,
source-bound acceptance and release decisions. Do not recreate Codex's execution
harness or reproduce the removed custom machinery around its event stream.

First connect an ordinary admitted child workflow to role-scoped native Codex
tools and structured command/file results. Delete the superseded Kegari repository
execution harness and its obsolete configuration, instructions, tests and
unused dependencies. Preserve shared machinery still required for Kegari's own
Codex, Git and control processes. After that correction, advance dependency-ready
features through the revised 0.2–0.6 queue. Comprehensive stabilization and
qualification belong to 0.7 and separately authorized follow-up.

## Inputs and architecture boundaries

- Repository: `/Users/lgr/Development/com.kegari/com.kegari.rust`. Work on its current
  branch. Preserve unrelated edits and partial staging.
- `kegari-cli` owns the `kegari-v0` executable; build it with `-p kegari-cli`.
  Read the [workspace overview](../../docs/workspace.md) for current ownership.
  No further crate extraction is a prerequisite or part of this goal.
- Keep the pinned Codex version initially. Use supported permissions and native
  tools, with precise refusal for unavailable capabilities. Never use an
  unsandboxed fallback or silently upgrade Codex.
- Retain narrow workflow/provider tools and deterministic host Git integration.
  Independent PR review cannot execute repository code; a read-only filesystem
  profile alone does not enforce that restriction.
- Preserve operation/thread/turn/item/source attribution and available actual
  results. Do not require bespoke per-command resource fingerprints or journals.
  Model summaries do not establish successful checks or current acceptance.
- Keep the existing control-host runtime driver and lock/lease/heartbeat child
  arrangement, with exclusive ownership and admission fencing. Do not start a
  process-relocation or new IPC project.
- Fence Kegari mutations before interruption and use supported Codex cancellation
  and terminal cleanup. Preserve uncertain effects and workspaces. Do not adopt
  arbitrary processes or blindly replay interrupted commands.
- Remove superseded production paths rather than hiding them behind flags,
  wrappers or fallback modes. Preserve old records as historical data; never
  reinterpret old execution proofs as new Codex evidence. Change persisted or
  instruction identities explicitly when their contracts change.
  Runtime-loaded instruction assets must migrate with their actual tools and
  callers, including their versioned catalog identities; do not leave the binary
  instructing agents to use removed tools or reinterpret old asset evidence.
- No target-toolset-specific adapters, manifest parsers, environment catalogs or
  predefined-command prerequisites. Codex retrieves and reasons about project
  guidance and commands within the authorized scope.

## Feature-first working method

The user's strong requirement against over-testing controls this assignment.
Prioritize working features and validate each increment only enough to proceed.

1. Select the next dependency-ready feature from MILESTONES; do not restart the
   completed audit or extractions. Use bounded parallel work with non-overlapping
   ownership where useful; the coordinating agent integrates the result.
2. Compile affected code and use the smallest relevant existing test or smoke
   exercise. For the initial migration, exercise an ordinary production flow
   through simulated Codex transport and a missing/stale-result refusal.
3. Fix observed defects attributable to the change. Add focused coverage only
   when needed to establish the changed behavior or reproduce a concrete defect.
   Remove architecture-specific obsolete tests with a concise rationale; preserve
   applicable assertions. Do not suppress real failures to claim success.
4. Advance once that focused evidence is sufficient. Do not routinely run full
   workspace suites, exhaustive failure matrices, repeated independent reviews,
   broad conformance audits or new validation/evaluation infrastructure.
5. Record deferred checks briefly. Pending comprehensive testing or a
   permission-dependent demonstration does not block independent feature work,
   including later milestones. This policy governs development of Kegari; it does
   not waive operator-configured checks on target projects.

Use the pinned toolchain/cache and `--offline --locked` for applicable Cargo
commands. Do not silently change dependencies or tool versions to get a pass.
Keep the alpha.10 archive unchanged. Reserve alpha.11 for the next coherent
Codex-backed candidate; do not bump versions for edits or checks.

## Permissions while unattended

Local source implementation and ordinary permitted local development checks are
allowed only after explicit activation. The user is unavailable until explicitly
stating otherwise. Decide routine implementation details without waiting for
optional preferences; record material blockers and continue independent work.

Do not commit, push, change remotes, create branches/worktrees, install or activate
services, package a release, access real Keychain items, invoke installed Codex or
live accounts, perform native qualification, deploy, or perform remote mutations.
Prepare concrete bounded requests for later supervision without beginning those
actions. Preserve all applicable AGENTS and tool restrictions. Do not change
approval policies, security settings or host configuration to keep running.
Unexpected approval denial blocks the affected action; never bypass it or retry
it unchanged. Do not ask interactive questions while unattended.

The six-hour limit is a duration constraint, not a guaranteed billing cutoff or
a requirement to exhaust a quota. Do not purchase credits, consume banked resets
or switch to a paid API fallback. Do not invent a token allowance or change the
selected model/reasoning/speed settings without instruction.

## Clock, checkpoints and stopping

On explicit activation, read the current clock once and record `started_at`,
`deadline_at = started_at + 6 hours` and `handoff_at = deadline_at - 20 minutes`
in PROGRESS. Retain these absolute timestamps across goal turns and compaction;
never restart the six-hour window. Display local times in Europe/Amsterdam and
retain unambiguous UTC timestamps for comparison.

Check the clock at each checkpoint and before long operations. Bound tool waits
and subprocess work to the remaining time. At `handoff_at`, stop starting new
features or broad checks; integrate current work, build the optimized executable
and run the existing state-free help/version/setup-help/build/handover inspections
if time permits. Report failures directly rather than expanding test infrastructure.

Keep concise progress updates: target version/package, implemented behavior,
removed subsystems, focused evidence, remaining work and next action. Update
MILESTONES and PROGRESS after coherent increments or material blockers, not every
tool call. Preserve failed results and distinguish implementation, focused
validation and qualification. No fabricated percentages or completion estimates.

The user explicitly requests that the Goal **pause at the six-hour deadline**,
or earlier when all authorized implementation is finished or no useful authorized
path remains. Checkpoint and use the available goal pause operation; do not mark
unfinished milestones complete. Do not spend remaining time manufacturing tests
or repeatedly auditing the same blocker. If the system interrupts earlier,
preserve the latest checkpoint and retain the original deadline on resumption
unless the user changes it.

Deliver the reviewable diff, executable location if built, implemented features,
deleted subsystems, focused results, unresolved defects, deferred testing and
smallest supervised next steps. Simulated/local integration is not demonstrated
live operation. Native/provider/actual-agent qualification and release readiness
remain distinct; completing this implementation window does not establish them.

## First activation and subsequent continuation

The approved run has been activated with the absolute timestamps in PROGRESS.md.
Do not issue the first-activation command again or reset the clock during
continuation. An interruption retains the original deadline unless the user
explicitly changes it. Goal-service state is authoritative for whether the run
is active or paused; a saved attachment or historical preparation note is not.

The following is the original **first-activation** command, retained for context.
It does not authorize another run:

```text
/goal Read /Users/lgr/Development/com.kegari/com.kegari.rust/crates/kegari-v0/GOAL.md and implement its six-hour Codex-orchestration objective. Start the six-hour clock now. I will be unavailable; follow its feature-first policy, action restrictions and requested pause conditions.
```

Reference guidance: [OpenAI Goals](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex)
and [long-running tasks](https://developers.openai.com/blog/run-long-horizon-tasks-with-codex).
