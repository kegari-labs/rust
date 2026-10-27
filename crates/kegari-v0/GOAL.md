/goal Implement and independently validate the complete Kegari v0 application and its delivered agent harness against /Users/lgr/Development/com.kegari/com.kegari.rust/crates/kegari-v0/SPEC.md. After explicit goal resumption under the revised milestone plan, continue through implementation, testing, review and corrections until the specification's definition of done is supported by actual evidence. Preserve its scope, authority, isolation, acceptance, release and recovery guarantees. If a required permission, resource or decision prevents completion, finish all independent authorized work and report the precise blocker without marking the goal complete.

Technology-agnostic execution correction — approved 2026-10-27

Keep the Goal paused. The current authorized step is maintaining the milestone
plan and its links/progress instructions. The revised plan is now recorded in
[MILESTONES.md](/Users/lgr/Development/com.kegari/com.kegari.rust/crates/kegari-v0/MILESTONES.md).
Do not start implementation, delete code, package a build or run product/native/live
qualification during this documentation step. A separate explicit request may
authorize a documentation commit; it does not resume implementation. Explicit user
resumption is still required; editing or committing these files does not resume
the Goal. This hold takes precedence over all automatic-continuation language below.

Implement SPEC 1.4's technology-agnostic contract. Kegari's orchestration and
harness code must not contain target-language, framework, build-system,
package-manager or toolchain-specific adapters, parsers, validators, command
rewriters or special cases. This includes manifest/lockfile handling and
optional plugins/helpers. Do not retain Cargo-specific execution support as an
optional feature. Required Git, Codex, Linear, GitHub and OS integrations and
Kegari's own Rust build remain in scope; representative toolset fixtures are
validation data, not a supported-ecosystem catalog. Authorized target-project
code/configuration written by child agents is not prohibited by this rule.

Do not require users to predefine commands, tool inventories, dependency or
environment manifests, exact-digest catalogs or Kegari environment packages.
The agent must inspect available repository instructions, documentation,
configuration and scripts with generic tools, understand the assigned technology
and component boundaries, and discover/infer suitable commands and preparation
from that context and the authorized task. Empty or unfamiliar repositories are
valid starting points. Record absent/unborn baselines honestly and distinguish
routine discoverable details from missing material requirements or authority.
An optional selected workflow may add guidance; otherwise use the installed
version-bound generic workflow without requiring another onboarding file.

The host must enforce generic authority, role, filesystem/network/credential
boundaries, original process/service ownership, deadlines and evidence freshness.
Automatically record actual command/cwd/environment observations and the basis
of validation obligations; these are operation evidence, not user-authored
specifications. Discovery cannot widen permissions, reset budgets, hide earlier
failures or drop required checks. Independent review must assess validation
adequacy; host gates enforce actual outcomes and current bindings. Keep public
PR/untrusted-context qualification blocked by the retained full-descendant
coverage requirement. No additional native, host, provider or deployment action
is authorized by this correction.

When implementation is explicitly resumed, follow MILESTONES.md and first
reconcile the entire repository against this contract, including committed work
and the uncommitted alpha.9 increment. Remove or rework all incompatible code,
interfaces, configuration/schema requirements, tests and documentation; do not
preserve them behind disabled flags or optional adapters. Preserve reusable
generic supervision/enforcement/evidence behavior, unrelated work, historical
commits, failed results and prior validation artifacts. Tests that enforce the
superseded philosophy must be replaced or removed with an explicit rationale;
retain applicable failure regressions and never erase failures to obtain a pass.
Old evidence must remain bound to its actual source/specification and cannot be
promoted to SPEC 1.4 conformance.

Validate generic discovery with materially different existing projects, an
unfamiliar custom tool and an empty repository, without adding toolset-specific
Kegari code or supplying predefined Kegari commands/environments to the agent.
Include misleading/stale guidance, missing necessary tests, permissions and
capability refusal, correction/recovery races, actual invocation evidence and
independent review. Test fixtures may declare finite bounds and scorer-only
expectations; those do not become production environment specifications.

Versioned completion plan — MILESTONES.md

After the hold above is lifted, continue the existing Kegari v0 assignment from its current repository state.
Retain the package and executable name kegari-v0. Version the application
within 0.x.y; reserve 1.0.0 for a future, explicitly agreed production-readiness
decision. Do not begin the lasting kegari implementation as part of this goal.

The completion scope follows SPEC 1.4, including the managed-workload limitation. MILESTONES.md is the single current version allocation, package order and exit-criteria record; it supersedes earlier milestone descriptions in this prompt and historical progress entries.
It does not broaden action permissions or remove unrelated completion gates.

Managed-workload decision — approved 2026-10-26

For v0, qualify the generic execution backend for foreground managed workloads
and explicitly registered managed background services; do not qualify a closed
catalog of target toolsets. Accept that unregistered detached descendants can escape
supervision and fall outside the shutdown guarantee. Document this limitation;
a policy forbidding detachment does not itself prevent it. Filesystem, network,
credential, role and control-authority isolation remain required.

Full descendant coverage remains desired future work and a prerequisite to
claiming qualification for processing untrusted external context material,
including public pull requests. Completion of the managed-workload v0 profile
must not be represented as that broader qualification. Preserve the controlled
adversarial evaluation obligations, including V0-E07; passing those fixtures
alone cannot establish readiness for arbitrary untrusted external material.

This explicit scope decision supersedes the earlier escaped-descendant closure
requirement for the current managed-workload milestone. It does not authorize
an Endpoint Security entitlement application or any host/live action.

First reconcile the actual branch, working tree, staging, PROGRESS.md,
validation evidence, outstanding processes and interrupted subagent drafts.
Preserve valid work and failures. Treat incomplete TEMP drafts as unverified
until inspected against current source. Do not resume the previous evaluation
work merely because it was the last activity before the pause.

Read [MILESTONES.md](/Users/lgr/Development/com.kegari/com.kegari.rust/crates/kegari-v0/MILESTONES.md)
on resumption and before selecting a milestone or declaring it complete. Focus
first on **0.2.0 / M02-1: correct the implementation baseline**, then close the
remaining 0.2.0 packages. Do not finish the superseded Cargo-specific alpha.9
packaging plan. Reconcile actual progress before selecting the next incomplete
package; do not restart completed packages merely because this prompt names the
initial checkpoint. SPEC remains the product contract and existing permissions
remain controlling. Milestone completion is not overall Goal completion.

Milestone focus and user updates

During resumed work, keep the user informed of the current target version and
package, concrete progress since the previous checkpoint, evidence established,
remaining exit criteria, next action and any exact approval/resource blocker.
Give concise meaningful updates during active work and this milestone summary at
every goal-turn handoff. Announce each completed milestone with its artifact,
evidence, retained limitations and next version. Do not report elapsed effort,
test counts or version bumps as completion, or invent percentages or deadlines.

Update milestone/package status in MILESTONES.md and the detailed evidence in
PROGRESS.md after a coherent increment, material finding/correction, blocker
change, exit audit and before handoff. Keep logging proportional; do not repeat
unchanged reports or re-run passing checks without a material reason. Each new
task must advance an exit criterion or resolve a concrete blocking defect.
Continue independent authorized work when approval blocks a dependent step,
keeping that milestone visibly incomplete; explain any work on a later milestone.
Do not silently expand scope, add frameworks or shift an unmet exit criterion to
another version merely to mark a milestone complete.

Use prerelease versions while a milestone is incomplete. Use 0.x.1 and later
patches only for actual compatible corrections; do not place new capability
or incompatible persisted-format changes in a patch release. Keep application,
instruction, protocol, configuration and evidence schema identities explicit.

Under MILESTONES.md after explicit resumption, retain and complete generic launch-time registration, owned
process/group and managed-service lifecycle, bounded cancellation, identity-safe
signalling, failure detection and conservative restart semantics. Qualify the
backend and observe actual command/service behavior without a toolset catalog; do not declare success from EOF or
self-report when their registered work remains active or uncertain. Preserve
the detached-process regression as evidence of the accepted limitation rather
than demanding its containment before enabling the managed profile. A managed
group's exit is not proof that arbitrary detached descendants have terminated.

Retain refusal for unmet prerequisites of the supported managed profile until
its implementation and qualification justify removing it. Document specific
missing prerequisites and continue independent authorized work. Do not repeatedly
retry unchanged denied probes or assume a broader containment guarantee.

For each milestone, implement a coherent increment, run relevant checks,
obtain independent review, resolve findings and deliver an exact-source build
and concise evidence summary. Use full regression validation at meaningful
integration/release boundaries. Reuse still-valid evidence; rerun checks when
changes or findings invalidate it. Keep progress bookkeeping proportional.

Distinguish implemented, locally verified, natively verified, behaviorally
qualified and human-reviewed states. Preserve failures, interrupted outcomes
and uncertainty. A passing build or completed minor milestone is not completion
of the comprehensive goal.

After explicit resumption, continue through MILESTONES.md without requiring routine “continue” prompts, retaining the milestone updates and action restrictions above.
Existing approval, unattended-operation, commit, remote-operation, installation
and deployment restrictions remain effective. Prepare concrete bounded plans
for actions needing additional authority; do not infer it from resumption.

Complete this goal only when the full v0 specification and its evidence
obligations are satisfied. Production 1.0.0 and activation of the lasting build
remain outside this assignment.

This is one comprehensive implementation-and-correction assignment. “One-shot” means I should not have to keep asking you to continue; it does not mean accepting the first generated implementation or completing everything in one model response. Begin local implementation only after the documentation hold above has been lifted, then reconcile the repository and follow the agreed plan. Do not stop at a proposal, scaffolding, a compiling binary or a happy-path demonstration.

Authoritative inputs and starting point

- Working repository: /Users/lgr/Development/com.kegari/com.kegari.rust
- Version/milestone plan: /Users/lgr/Development/com.kegari/com.kegari.rust/crates/kegari-v0/MILESTONES.md. Read it with PROGRESS.md; preserve its exit criteria and report progress as directed above.
- Implement the existing Cargo workspace member and executable kegari-v0 at crates/kegari-v0.
- Read the entire SPEC.md and all applicable AGENTS.md instructions before implementation. The approved specification is version 1.4 (SHA-256: `b01064b5238554d2def840cabf6a9e5d555535e35ce4877838a55ade2dd0a6e7`); its source baselines are recorded in SPEC.md. Inspect the actual branch, working tree, staging and current specification. Preserve later or unrelated work; do not reset or silently substitute an older baseline. Report any material divergence from the approved contract.
- SPEC.md is the self-contained product contract. Its current Kegari rules take precedence over conflicting Symphony defaults and older discovery direction. Operational instructions and permissions still govern actual actions. Do not weaken the specification or rewrite its requirements to match an easier implementation.
- The sibling com.kegari.symphony and com.kegari.specification repositories are read-only references. Use them only to resolve a concrete question; do not restart discovery, require the entire corpus as runtime context, or update their pinned baselines. Prefer targeted official documentation for technical compatibility questions and record what you actually verify.

Implementation scope and boundaries

Implement all retained v0 behavior, including capabilities that are disabled by default. Deliver the Rust application, local and remote CLI/control service, background-operation support, provider integrations, runtime-enforced parent/subissue delivery workflow, reusable versioned instruction content, an installed generic workflow and optional starter WORKFLOW.md, agent-led repository discovery/preparation and operator guidance, deterministic tests, executable behavior evaluations, and validation evidence.

The workflow must remain useful for building the lasting implementation: parent authorization with child-only implementation; scoped accounts/repositories/tools; actionable corrections and continuation; separate local-test, hosted-CI, Codex-review, staging, integrated-evidence, human-acceptance and release gates; isolated approved-slice releases; and durable operator intent with conservative recovery. The detailed specification controls each behavior; this summary does not replace it.

You may create or modify the v0 source, tests, instruction assets, evaluation fixtures, documentation and necessary Cargo/dependency/build configuration within this Rust repository. Use reproducible dependency/toolchain choices. Work on the current project branch and preserve unrelated changes and partial staging. Local commits are explicitly authorized as described below; do not ask again for each ordinary commit. Do not create a project branch/worktree, push, publish or deploy without separate explicit authorization. Disposable local Git repositories, branches and worktrees are permitted solely inside clearly owned test fixtures needed to exercise the specified Git behavior; do not use the operator's checkout or existing work as disposable fixtures.

Keep the lasting kegari package, graphical dashboard, native wrapper, frontend hosting/assets and other deferred product features out of this build. Do not create a separate harness repository or an application/orchestration database. Preserve the existing MIT license and copyright, the adapted specification's Apache terms, and any applicable obligations for material actually reused. Do not alter repository security rules to make a test pass.

This builder conversation is intended to use GPT-6 Astra, Ultra reasoning and Standard speed, selected in the client. Do not silently switch that configuration or enable paid fallback. These builder settings are not Kegari runtime defaults: runtime profiles and Linear execution labels retain their specified ownership.

Working method

1. Inspect the scaffold and instructions, then establish a compact implementation plan and requirement-to-evidence checklist in PROGRESS.md as described below. Cover all normative v0 requirements, the retained source REQ/SC identifiers, V0-T01–39, V0-E01–09 and V0-H01–04. Keep deferred clauses identified as deferred.
2. Check the effective permissions, required local tools, dependency access and validation prerequisites early; record missing prerequisites without treating them as permission to change the host configuration. Identify high-risk compatibility and enforcement assumptions, especially the supported Codex app-server profile, sandbox/tool boundaries, same-account review evidence, HTTPS identity, persistent intent and uncertain external actions. Investigate them without broadening authority. A blocked integration must not prevent independent implementation and deterministic testing.
3. Implement coherent increments and validate them as you go. Prefer straightforward, maintainable Rust and clear ownership of state and side effects. Temporary development scaffolding must not remain as a substitute for required behavior. Do not count mock adapters, TODOs or unsupported-operation placeholders as completed production integrations.
4. Use available subagents for bounded independent implementation or review where useful. Assign non-overlapping ownership, integrate their work, and inspect their evidence. Keep the selected builder configuration where supported. Do not create separate user-facing conversations, project branches or worktrees as a substitute for this assignment.
5. Run the relevant build, formatting, lint, unit, integration and failure-injection checks. Derive acceptance from externally observable behavior and resulting artifacts, not merely internal calls or the implementation's own success messages. Correct defects and rerun affected checks. Do not delete meaningful tests, weaken assertions or suppress findings to obtain green results.
6. Independently review the integrated application and harness against the specification, including authorization, correction races, stale evidence, release isolation, restart/stop intent, duplicate prevention and preservation of unfinished work. Resolve material findings and validate the fixes. Preserve useful failure cases as regressions.

Local commits and Git hygiene

Commit coherent, logically related changes as the build progresses, after their relevant checks, and commit the remaining verified work at final handoff. Choose commit boundaries for clarity, reviewability and a useful history; there is no target commit size or count. Do not combine unrelated changes merely to reduce the count or split a coherent change arbitrarily. Include related tests, instructions, documentation and PROGRESS.md updates where they belong. A useful validated change may be committed while unrelated live qualification remains blocked; describe that limitation honestly rather than implying full readiness.

The coordinating agent owns commits after integrating delegated work. Before committing, inspect the diff and index and stage only the intended task changes; preserve unrelated edits and partial staging. Follow applicable AGENTS.md commit rules, including Conventional Commits, --signoff with the configured human committer, and accurate Assisted-by attribution using a verified model identifier or omitting the identifier if unknown. Do not change Git identity or configuration to get a commit through. Prefer amending or consolidating clearly related recent unpublished commits when appropriate, with a local recovery reference before any history rewrite and verification afterward. Do not rewrite published or unrelated history. Record resulting hashes, subjects and any consolidation in the progress record or final handoff; do not create another commit solely to record the previous commit's hash. This authorization covers local commits only, not pushes, publication, deployment or coordinator activation.

Durable progress and resumption record

At the start of implementation, create and maintain /Users/lgr/Development/com.kegari/com.kegari.rust/crates/kegari-v0/PROGRESS.md alongside SPEC.md and GOAL.md. If it already exists, read and reconcile it with the actual repository and available evidence; preserve relevant history and unrelated content. This is the build assignment's progress record, not a Kegari runtime state store or a replacement for the specification or Goal lifecycle controls. The coordinating agent owns updates; subagents report their results to it rather than concurrently editing this file.

Keep a concise current summary at the top identifying Goal state, target version/package, evidence, next action and blockers, linked to MILESTONES.md, followed by:

- The specification/repository baseline, current implementation checkpoint, next useful actions, and a compact requirement-to-evidence checklist. Distinguish implemented, verified, failed, blocked, not run and deferred work; implementation alone is not verification.
- Validation commands and results, affected files/code state and relevant tool/profile/instruction versions, with links to detailed reports and artifacts. Separate deterministic tests, actual-agent evaluations, real integration validation and required human review. Mark stale evidence when subsequent changes invalidate it. Retain failures and unresolved review findings rather than replacing them with a later success claim.
- Each blocker's affected requirements, attempted action, observed reason, available safer alternatives, and the exact permission, resource or decision needed. Record pending approvals and uncertain outcomes explicitly; do not describe an unexecuted action as failed or an unknown outcome as safe to repeat.
- Material engineering decisions and a short milestone history. Identify in-flight work, owned test processes/fixtures, references for any authorized remote operations, and any cleanup or reconciliation needed before resuming. Exclude credentials, secrets and unnecessary private data; reference redacted evidence instead of copying raw transcripts or logs.

Update the record after initial inspection, meaningful implementation/validation milestones, material failures and blocker changes, and before a normal handoff or an action likely to wait for approval. Checkpoint before a planned context transition when feasible. On continuation or recovery, refresh it from actual state before relying on it. Keep bookkeeping proportional to the work; avoid rewriting it after every tool call or duplicating the specification. Abrupt interruption may leave a stale checkpoint, so the file is a recovery aid, not proof that a recorded action completed or authority to change the task's permissions.

Harness and evaluation obligations

Deliver and test the actual reusable instructions, not just a loader or test-only prompts. Exercise the production loading, context assembly, instruction-discovery, tool-permission and supported-provider paths. Keep runtime-owned instructions version-bound to the installed binary while preserving repository workflow reload semantics. Respect existing project instructions; do not automatically rewrite target AGENTS.md or create a competing configuration layer.

Follow SPEC §§12.3–12.4 for the three distinct evidence classes: deterministic enforcement tests, actual-agent behavior evaluations, and real integration validation. Provide versioned fixtures and explicit assertions/rubrics, protect scorer-only data, repeat each required fixture/variant for its evaluated profile within declared finite bounds, and record every result, including failures, interruptions and skips. Include the required no-change, blocking, correction, review, evidence-honesty, isolation and recovery cases. Retain instruction/model/profile/fixture provenance and redacted activity evidence. Do not equate tool-call presence, a plausible summary or a lucky rerun with correctness. Required human review/calibration remains a real human gate.

Live execution and blockers

Start in unattended mode and treat me as unavailable until I explicitly say that I am supervising or available for human-in-the-loop work. Record that mode and any explicit change in PROGRESS.md; do not infer supervision from elapsed time, a generic follow-up or Goal resumption alone. My absence is not a stopping condition or a reason to defer otherwise authorized engineering, testing, review or corrections. Continue all useful authorized work within the Goal's actual limits. A blocked test or approval should block only the work that depends on it where execution can continue; do not stop engineering merely because one validation step needs my involvement. The intended client setting is Approve for me, with sandboxing retained, rather than Full Access. Inspect and respect the effective permissions; this prompt does not change them or guarantee uninterrupted execution. Do not change approval policies, sandbox boundaries, security rules or host configuration to keep the run moving.

This assignment authorizes local implementation and ordinary permitted local development/testing. It does not grant blanket authority for live agent evaluations, real Linear/GitHub mutations, production fixtures, host security/configuration changes, persistent installation, remote infrastructure or deployment. Do not launch the lasting build or create its Linear backlog.

Before a live validation step needs additional authority or resources, prepare a concrete bounded plan identifying the accounts, disposable targets, actions, limits, expected evidence and cleanup/recovery implications. While unattended, record the plan and missing approval/input for supervised follow-up rather than starting the human-dependent step. Once supervision is explicit, request only what is still missing. Existing prohibitions remain effective unless explicitly changed by an authorized instruction; a repository setting or product requirement cannot waive them. Do not invent a token budget, spending allowance, unavailable credentials or provider compatibility.

Decide routine engineering details yourself and continue authorized work without waiting for optional preferences. While unattended, do not voluntarily open interactive questions or begin an operation known to require my manual approval, authentication, decision or review. Record the precise dependency and a focused question or proposed action in PROGRESS.md, prepare everything that can safely be prepared, and proceed with independent authorized work. Do not repeatedly poll for my return. My absence or silence is not an answer or approval. After I explicitly enable supervision, necessary questions may be raised; Kegari's runtime prohibition on interactive clarification does not apply to that supervised build discussion.

Eligible automatic approval review may still run unattended through the normal mechanism for actions within the authorized task scope; unattended mode does not mean avoiding all approval machinery or granting new action authority. Follow mandatory tool and platform approval instructions. If an unexpected manual approval gate appears, do not bypass it or assume approval; record it and continue independent work only if execution can proceed. If automatic review denies an action, record its stated reason and follow the denial instructions; pursue only a materially safer permitted alternative, never an indirect route to the same prohibited outcome. Do not repeatedly retry an unchanged denied action. An approval prompt or interruption may prevent further execution despite this prompt. When no useful authorized path remains, preserve the work and report the outstanding requirements, attempts/evidence, exact blocker and smallest next action for supervised follow-up. Follow the available Goal lifecycle rules; only the user or system controls pause/budget decisions, and blocked or budget-limited work is not complete.

When I say I am available for monitored, human-in-the-loop work, continue the same Goal conversation, or resume it if interrupted, from the existing work rather than restarting implementation. Work with me to resolve the recorded blockers and finish any remaining implementation, testing, validation and human review; do not assume only testing remains. Re-read PROGRESS.md, applicable instructions and the current repository state; verify the effective permissions and check pending processes and uncertain side effects before retrying anything. Preserve valid prior explicit authorization without asking for it again. Reconcile stale evidence and rerun affected checks where necessary. My statement that I am now present enables discussion of outstanding decisions; it does not itself authorize previously restricted actions or waive validation and human-review gates. Automatic tool approval and subagent review cannot satisfy the human evaluation review/calibration required by SPEC §12.4.3; prepare its evidence for my review and keep qualification incomplete until that gate is met.

Completion and handoff

Audit completion against SPEC §12 and the actual final repository. Mark the Goal complete only when all required deliverables and validation obligations are satisfied and material findings resolved. Required skipped, unavailable or blocked checks remain incomplete. Distinguish a finished local implementation from operational qualification; do not claim readiness to coordinate the lasting build while mandatory live, human or compatibility checks remain unresolved.

Give compact progress updates with the current checkpoint, evidence established, remaining work and any actionable blocker. Update PROGRESS.md for the final handoff and link it alongside the delivered capabilities and locations, exact build/run/test/evaluation commands, requirement coverage and validation results, independent-review findings and resolutions, operator setup/qualification steps, and any remaining limitations. Report the local commits and any remaining uncommitted work, with its reason. Leave the history and changes reviewable on the current branch. Do not push or activate the coordinator unless separately requested.
