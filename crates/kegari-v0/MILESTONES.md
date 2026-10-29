# Kegari v0 version milestones

**Revised on 2026-10-29 for SPEC 1.5. The six-hour goal is checkpointed for early pause under the weekly-only spending restriction; fixed timestamps and the built candidate are in PROGRESS.md. No minor milestone is complete.**

**Codex owns repository execution. Kegari orchestrates work, supplies context and
permissions, observes results, and controls workflow progression.** Deleting the
superseded execution harness is required implementation work, not optional cleanup.

This is the single current version allocation and implementation queue referenced
by [GOAL.md](GOAL.md) and the attached goal objective. [SPEC 1.5](SPEC.md) controls
product behavior; the approved feature-first development policy below controls
work sequencing. Existing permissions control actions. [PROGRESS.md](PROGRESS.md)
holds concise current checkpoints and historical evidence. Updating these files
does not activate the goal, start application implementation or authorize native,
live, installation, publication or deployment operations.

All seventeen workspace packages remain `0.2.0-alpha.10`. Reserve
`0.2.0-alpha.11` for the next coherent Codex-backed candidate; do not bump versions
for this preparation pass or individual edits/checks. The archived alpha.10
package remains unchanged. Historical automatic-continuation and commit language
is superseded by the new goal; no commit is authorized.

## Starting checkpoint and evidence

The source checkpoint is `4989e93e10bf84335718aeceb288dbad7e4b0aa7`
(`refactor(execution)!: complete native lifecycle extraction`); the subsequent
`0a631ce05fbf5e98b592a401aaa551bb0f1e70db` commit reconciled milestone documentation.
The table records historical results, not SPEC 1.5 implementation or qualification.

| Checkpoint | Retained evidence |
| --- | --- |
| Archived alpha.10 release | 1,260 passed, zero failed, three native ignores; original source/artifact identity in the [stabilization report](validation/LOCAL-DISCOVERY-ADMISSION.md). |
| Refactored source at `4989e93` | 1,366 passed, zero failed, three native ignores; formatting, strict Clippy, inventory, bounded review, optimized build and five state-free inspections passed. The [final extraction report](validation/EXECUTION-LIFECYCLE-EXTRACTION.md) retains identities and limitations. No new release or native qualification resulted. |

The activated redesign now connects ordinary child/validation sessions to native
Codex tools and actual structured results. The custom repository runner, sandbox
renderer, resource inventory and command-level bookkeeping have been removed.
Backend compilation and focused simulated admission/result checks pass. Native
artifact freshness, supported hooks and paged review inputs are integrated;
empty-source, continuation and operator-status work continues in PROGRESS.md.
This is source implementation, not live/native qualification. Historical custom
execution evidence remains tied to its original contracts.

`kegari-cli` builds `kegari-v0`. Extracted crates and the remaining backend are
indexed in the [workspace overview](../../docs/workspace.md). Process/journal
mechanics remain required for Kegari's own Git, Codex and control
operations; preserve still-needed host mechanisms while deleting replaced
repository execution behavior. Keep the current control-host runtime driver and
orchestration-child lock/lease arrangement, with exclusive runtime ownership and
lease-based admission fencing. No process relocation or further crate extraction
is a prerequisite to this correction.

## Direction and feature-first development policy

- **Agents MUST prioritize working features.** Use the smallest appropriate
  affected-code compile, focused existing test or smoke exercise needed to proceed.
  Fix failures attributable to the change; add a regression only when it supplies
  necessary confidence in that behavior or reproduces a concrete defect.
- **Agents MUST NOT routinely run full workspace suites, exhaustive failure
  matrices, repeated independent reviews or broad conformance audits during this
  implementation goal.** Comprehensive regression, end-to-end coverage, native/live
  qualification and human calibration belong to the later stabilization phase.
  Do not create validation infrastructure or fill spare time with extra tests.
- **Deferred testing MUST NOT block dependency-ready feature implementation.**
  Track implementation, focused validation and qualification separately. Record
  deferred checks briefly, then advance; an approval-dependent demonstration
  blocks that demonstration and its actual dependents only.
- This policy concerns development of Kegari. It does not waive configured target
  project checks, hosted CI, review, human acceptance or release authority that
  Kegari must enforce in its product workflows.
- Codex app-server owns agent reasoning, repository inspection/editing, command
  execution, sandbox enforcement and supported terminal management. Kegari owns
  operation/session admission, roles/scopes, context, account selection, durable
  workflow state, provider writes, source-bound results and acceptance/release.
- Use the pinned Codex version and its supported permissions and structured events.
  Record operation/thread/turn/item/source associations and available execution
  results; do not recreate bespoke environment inventories or mandatory admission
  and journals for every Codex-internal command. Never infer execution from agent
  prose or use unsandboxed APIs as an automatic fallback.
- Preserve role boundaries. Independent PR review cannot run repository code;
  a read-only filesystem profile alone does not establish that restriction.
  Unsupported Codex capabilities must produce specific limitations, not a second
  execution engine or silent upgrade.
- Kegari MUST contain no target-toolset adapters, manifest/lockfile interpreters,
  command generators/rewriters or compiler/cache special cases. Users need no
  predefined command lists, executable catalogs or environment specifications.
  Codex discovers commands from authorized repository context, including empty
  repositories and unfamiliar tools. Required Git/Codex/Linear/GitHub/OS integration
  and Kegari's own Rust build remain permitted.
- Preserve applicable tests, unrelated work and historical failures. Delete tests,
  configuration and instructions enforcing the superseded architecture with a
  concise rationale. Changed persisted/asset contracts need explicit identities;
  old records must not silently become Codex execution evidence.

## Version sequence and status

The sequence allocates product work; it is not a requirement to finish every test
or qualification of one version before implementing the next. Advance in this
order where dependencies permit. Use existing implementations rather than
rebuilding subsystems because they were extracted. An implementation-complete
milestone may still have deferred validation/qualification; report that distinction
and do not publish or claim a qualified release prematurely.

| Version | User-facing outcome and implementation | Implementation | Focused validation | Qualification |
| --- | --- | --- | --- | --- |
| **0.2.0** | **Usable Codex-backed orchestration.** Connect native Codex repository tools to ordinary child workflows; delete the duplicated harness; complete discovery/onboarding, empty/unborn sources, result/review paging, validation continuation, account refresh/readmission, status/logging and blocker publication. Complete M02-1–M02-6 below. | In progress: ordinary Codex-native path and executor deletions implemented with focused simulated evidence; native artifact/hook integration and paging have advanced; M02-3–M02-6 remain in progress. | Historical alpha.10/extraction checks exist; changed paths require only focused checks during development. | Live/native compatibility and full coverage deferred to 0.7; missing capability or authority still blocks affected runtime actions. |
| **0.3.0** | **Reliable interruption and recovery.** Cold workflow reconstruction, compatible Codex session continuation, provider-intervention resolution/readmission, uncertain-result reconciliation and parent reassembly preserving obligations, attempts and adequacy. | Partial: [runtime recovery](src/runtime_recovery.rs) now includes protected cold ordinary-child continuation and bounded fresh metadata readback. Signed cold validation histories, cold completed-parent restoration and changed-baseline reassembly are connected. Interrupted replacement sessions retain their actual assignment and receipt lineage. Unpublished E2E preparation can now be checkpointed, restored and published after fresh integrated validation; cold continuation of an already-carried parent after another baseline change now restores its authenticated historical phase ancestry. Missing legacy records still refuse; broader phase reconstruction remains incomplete. | Existing protocol/recovery coverage is reusable where compatible; check changed integrations only. | Restart, native lifecycle and live-provider qualification deferred to 0.7. |
| **0.4.0** | **Complete delivery workflows.** Complete replacement-delta checks/publication/recovery, initial external-target baseline establishment and revalidation, and visual evidence recording/publication/authenticated retrieval bound to the actual candidate. | Partial: candidate-bound native video selection, private publication and authenticated retrieval are connected. External baseline establishment and unchanged/no-release consumption have focused evidence. Replacement production integration and completed-publication recovery are connected; repeated replacements preserve their full contribution ancestry; mid-flight replacement session recovery and unpublished E2E preparation recovery are connected. Cold publication still revalidates actual source, receipts and integrated evidence; broader live recovery remains unqualified. | Use focused fixtures through the completed production callers. | Actual media upload/playback, external-target compatibility and provider delivery remain separately authorized 0.7 work. |
| **0.5.0** | **Operator installation and handover.** Connect existing installation, compatibility, qualification-record and handover mechanisms into a usable operator route with clear refusal/recovery behavior. | CLI installation plus candidate inspection, reviewed qualification recording and exact-plan/idempotent [handover](src/housekeeping.rs) routes are implemented with focused disposable evidence. Actual installation and qualification are deferred. | Focused local administration fixtures and state-free CLI inspections. | Actual installation/activation/handover requires separate authority and real evidence; deferred to 0.7. |
| **0.6.0** | **Complete evaluation functionality.** Complete missing production contexts, genuine continuation preparation and E09 using existing suites/runners/grading. Do this after core product flows work; no new evaluation framework or broad trial campaign during implementation. | [Production caller](src/runtime_evaluation.rs) now connects fresh implementation, testing, documentation and no-change contexts, plus five enrolled E09 variants. Eight committed child-context flows and empty fixture/source components have focused evidence. Actual unborn observation preserves an absent revision, and typed blockers cannot become completion. Genuine continuation, remaining contexts/repetitions and live qualification remain unfinished; independent review retains its pinned-capability refusal. | Focused caller/fixture checks; never present simulated trials as actual-agent results. | Repeated actual-agent runs and human calibration belong to 0.7. |
| **0.7.0** | **Stabilization and managed-v0 qualification.** Comprehensive regression, end-to-end and failure coverage, authorized native/provider/actual-agent runs, human calibration and final corrections against SPEC 1.5. | Later dedicated phase; reuse still-valid mechanisms and evidence. | Full integration/release checks occur here after intended features work. | Open; cannot be completed by the restricted six-hour implementation goal. |
| **1.0.0 / v1** | **Production readiness.** Separately agreed production criteria and resolution of broader untrusted-context/lifecycle requirements. | Outside this implementation goal. | To be agreed. | No production-readiness claim inferred from any 0.x version. |

Keep the `kegari-v0` application/executable identity; eventual internal backend
package retirement is separate work. Full descendant coverage remains a
prerequisite to qualification for arbitrary untrusted external context, including
public PR material. Codex delegation does not establish it. The reported Endpoint
Security application remains unresolved; revisit the outcome and broader scope
before v1 without mandating that mechanism or making it a current implementation
prerequisite.

## Immediate 0.2.0 implementation packages

This allocation supersedes pre-1.5 M02 package meanings. Historical reports remain
historical; their old package labels cannot reintroduce the discarded harness or
its qualification prerequisites.

| Package | Required production outcome | Minimum useful check | Starting state |
| --- | --- | --- | --- |
| **M02-1 — Connect Codex repository execution** | Wire ordinary child admission/source preparation to a role-scoped Codex session using native repository tools. Collect structured command/file events into existing outcome/required-check handling with actual source and operation identities. Stop advertising competing command/edit tools on that path; retain narrow workflow/provider tools. Replace the custom blanket prerequisite with selected-version/permission/capability checks. | Affected code compiles; one ordinary production flow through simulated Codex transport and one missing/stale-result refusal. No installed Codex invocation. | Initial ordinary admitted child/native-result path implemented; focused simulated transport acceptance/refusal passes. Declared artifact freshness and supported hooks have focused evidence; remaining lifecycle limitations are tracked in PROGRESS. |
| **M02-2 — Delete the superseded harness** | Remove replaced repository command rendering, sandbox generation, environment/resource inventories, adapters and command-level bookkeeping, plus obsolete configuration/instructions/tests/dependencies. No optional flags, compatibility wrappers or fallback runtime. Retain shared mechanisms still needed by host Git/Codex/control operations. Preserve old records without silently reinterpreting them; change actual persisted/asset identities explicitly when needed. | Inspect migrated callers and resulting deletions; compile affected packages and use a focused existing check where needed. Do not build a deletion-audit framework. | Custom command/resource/sandbox/bundle implementations and obsolete dedicated tests removed. Remaining caller/test migration and unused integration cleanup continues. |
| **M02-3 — Finish discovery and onboarding** | Use installed generic guidance and Codex reasoning for existing/unfamiliar projects and empty/unborn repositories. Represent real absent baselines; preserve explicit-selection and missing-authority errors. Provide useful launch/setup guidance without command/environment prerequisites. | Small existing production fixtures for changed onboarding/source states; actual-agent claims remain pending. | Guided connection/workspace forms and absent-baseline [source selection](src/runtime_workers.rs) are integrated. Actual nonempty root commits are retained; initial remote publication policy remains unresolved. |
| **M02-4 — Complete result continuity and paging** | Connect discovered obligations and failed attempts through correction/continuation using Codex results. Page bounded review/output inputs without losing required evidence. Retain source/artifact acceptance, cancellation and uncertainty within the revised Codex lifecycle contract. | Focused continuation/paging checks exercising the changed production path. | Paged protected review inputs, source-advanced lineage, provider-failure retries and adequacy-repair continuation are integrated. Cold validation histories, same-process parent reassembly and identity-bound native output paging are connected. Completed carried parents can restore their historical phase ancestry for a further baseline change; other interrupted phase paths remain incomplete. |
| **M02-5 — Make runtime progress actionable** | Implement independent account quota refresh/readmission, runtime/session logging, operator queued/retrying/attempt/next-action status and actionable scheduling-blocker publication/retry. Reuse existing sinks, records and provider adapters. | Focused existing runtime/control/provider fixtures for the changed behavior. | Independent quota refresh, session/dispatch logs, operator status and deduplicated blocker publication/retry are connected. Recent redacted logs and the actual host status envelope have focused evidence. Typed child blockers now preserve reason/next action without completion or automatic retry. Provider intervention resolution remains limited by authoritative observation support. |
| **M02-6 — Leave a usable candidate** | Integrate completed feature work, update operator guidance and build the optimized executable from `kegari-cli`. Reserve alpha.11 for the coherent Codex-backed candidate without modifying alpha.10. Report unfinished features and qualification distinctly. | Existing state-free help/version/setup-help/build-info/handover-info inspections if time permits. Report build failures; no release archive or comprehensive suite required in this goal. | An optimized checkpoint executable builds and all five empty-environment/state-free inspections pass; current identities are in PROGRESS checkpoint 08. Later source edits require a new build. This remains an unqualified development candidate, with unfinished features reported separately. |

Supported services use Codex terminal/lifecycle interfaces; no promise of arbitrary
process adoption or automatic service survival/restart after app-server failure.
Fence Kegari mutations before interruption, request supported cancellation and
cleanup, and retain unresolved outcomes/workspaces. Recover compatible sessions
and workflow facts without blindly replaying commands. Deterministic host Git
assembly, privileged writes and their admission/recovery remain Kegari-owned.

## Six-hour goal operation and progress

The goal was explicitly activated on 2026-10-29. Follow [GOAL.md](GOAL.md) and
the absolute timestamps in PROGRESS.md; never restart the clock on continuation.

- Record activation time and a deadline six hours later in PROGRESS. Check the
  deadline before long operations. Reserve the final twenty minutes for integration,
  an optimized build and concise handoff; do not start an unbounded final test run.
- At the deadline, checkpoint and pause as explicitly requested. Follow the goal
  lifecycle rules for earlier completion or lack of useful authorized work. Do not
  mark unfinished product milestones complete because the time expired.
- Work on the current branch, using bounded parallel tasks with non-overlapping
  ownership. Do not commit, push, create branches/worktrees, install/activate
  services, package a release, access real Keychain items, invoke installed Codex
  or live accounts, or perform native qualification. Prepare concrete blocked
  requests for later supervision and continue independent authorized work without
  interactive questions while the user is absent.
- Six hours is a duration limit, not a guaranteed billing cutoff. Do not purchase
  credits, consume banked resets or switch to a paid API fallback. No token budget
  or quota-exhaustion target is invented.
- Advance the first incomplete dependency-ready feature. A blocked live/native
  demonstration or deferred broad testing does not require returning to audits or
  restarting completed work. Stop rather than manufacture validation tasks when
  useful authorized work is exhausted.
- After each coherent increment and before handoff, report completed capabilities,
  removed subsystems, focused checks, remaining work and the next action. Update
  implementation/focused-validation/qualification status without copying logs or
  rewriting the whole plan. Preserve meaningful failures and actual limitations.
- Handoff includes the reviewable diff, executable location if built, retained
  restrictions and deferred verification. Simulated/local evidence is not a live
  Codex workflow, real-provider delivery or operational qualification.

Suggested update: **`0.2.0 · M02-<package> — Implemented: … Deleted: … Focused check:
… Deferred: … Next: …`** Omit empty fields. Elapsed effort, test counts and version
bumps are not feature progress. These are conversation updates, not automations or
messages to other services.

## Prereleases and historical artifacts

| Version label | Meaning |
| --- | --- |
| `0.2.0-alpha.8` / `alpha.9` | Historical checkpoints only. Alpha.9 retained useful generic workflow/discovery/archive mechanisms, but its custom execution contract is superseded. No alpha.9 artifact was packaged. |
| `0.2.0-alpha.10` | Archived pre-extraction package, with its original 1,260-pass evidence and source identity. All current packages still carry this version; extraction did not replace the archive. Neither checkpoint establishes SPEC 1.5 readiness. |
| `0.2.0-alpha.11` | Reserved for the next coherent Codex-backed candidate. No bump or packaging during document preparation; no release packaging authorized by the six-hour goal. |
| Later `0.x.0-alpha.N` / optional `rc.N` | Identify coherent candidates, not each edit or test run. Preserve source/build/asset/schema identity and distinguish feature completeness from qualification. |
| `0.2.1`, `0.3.1`, … | Conditional compatible fixes to completed corresponding minor releases. New capability or incompatible persisted formats require the appropriate minor/prerelease treatment, not a patch. |

Do not overwrite archived artifacts, relabel prior evidence or lower a product
acceptance gate to claim a qualified release. The revised development order
explicitly defers comprehensive testing; it does not erase those obligations.

## Historical evidence and current authority

Earlier milestone assignments and goal resumptions are superseded. The current
queue above is the only implementation order; prior source/validation reports
remain evidence for their original checkpoints, not instructions to reconstruct
the custom executor or repeat its qualification campaign.

The original 0.2–0.7 allocation was agreed on 2026-10-26. The technology-agnostic
amendment followed on 2026-10-27, and the former goal was cancelled on 2026-10-28.
The source and prerelease table above retains the resulting checkpoints. On
2026-10-29, SPEC 1.5 established Codex-owned repository execution and required
removal of the duplicated harness. No minor milestone is complete. The new goal
was subsequently activated and is checkpointed for early pause; its current build
and fixed deadline are in PROGRESS.md.
