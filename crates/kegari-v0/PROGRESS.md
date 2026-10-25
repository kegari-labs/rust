# Kegari v0 build progress

Local implementation and integration remain in progress. The complete offline
crate suite passed **411 tests, with 2 explicitly blocked native tests ignored**.
Strict all-target Clippy passed. A subsequent review correction for stale retry
grants passed its affected library/decision/scheduling/worker rerun. See
[the local integration report](validation/LOCAL-INTEGRATION.md). These are
component results, not full application conformance.
The runtime decision facade and protected provider-session recovery are being
connected. The complete scheduler, phase progression and executable entry remain
in flight. Native descendant containment remains an implementation blocker and
fails closed. The operator remains **unattended**. Live agents, remote mutations,
installation, deployment and required human calibration have not been authorized
or run. Independent local engineering continues.

## Baseline and authority

- Started 2026-10-25 on existing `main`, clean worktree and empty index, at `36a3e83` (`docs(kegari-v0): finalize the specification and build prompt`). No project branches/worktrees created.
- Read the complete SPEC 1.2; SHA-256 `80b6f703d224ce7800c9e04a23c6356ab72f02133ab9be5ccb129fd6eb10e326` matches the approved contract. Source baselines remain those recorded in SPEC §1.1. SPEC/GOAL and existing license are preserved.
- Applicable instructions are the user-supplied AGENTS rules; no additional AGENTS.md exists in this repository or its ancestor directories inspected. The goal authorizes local signed-off commits, owned disposable Git test fixtures and implementation inside this Rust repository. Remote repository writes/pushes/deployment and host configuration changes remain prohibited.
- Effective tools: workspace-write sandbox, automatic approval review; repository and temporary paths writable, host configuration not writable. No permission/security change made.
- Installed: Rust/Cargo 1.99.1 (under existing `stable` alias), Git 2.56.0, Codex `0.164.0-alpha.11.1`, macOS sandbox-exec/Keychain/launchctl. `tailscale`, `ffmpeg`, `ffprobe` not found on PATH. `rust-toolchain.toml` now pins 1.99.1; development uses `RUSTUP_TOOLCHAIN=stable` only because this alias was verified to contain that exact version. No toolchain installation is authorized or needed on this host. This is availability evidence only, not compatibility qualification.
- Builder model/speed settings are inherited unchanged. The runtime profiles will remain explicitly configured and separate. Assistance attribution will omit a model identifier unless independently verified.

## Implementation plan and current checkpoint

1. Establish strict typed identities/configuration, canonical evidence, durable records, and runtime-owned instruction assembly.
2. Implement the pinned app-server and enforced execution boundary; implement Git provenance, candidate isolation and all separate evidence gates.
3. Connect real provider adapters through serialized admission/mutation/recovery; implement scheduler, local/HTTPS CLI control and platform lifecycle.
4. Deliver bounded executable evaluation fixtures/graders and operator preparation/qualification guidance.
5. Run build/fmt/clippy, observable deterministic integration/failure tests; independently review and correct. Commit coherent verified increments on `main`.
6. Prepare exact bounded live plans and human review material. Qualification stays incomplete until separately authorized live checks and human calibration pass.

Current checkpoint: strict configuration/workflow, embedded instruction assets, canonical digests, private durable storage, serialized intent/admission/mutations, authorization/correction/scheduling primitives, immutable paged snapshots, pinned app-server/isolation adapter, and deterministic Git/evidence/release primitives are implemented and have focused tests. Real Git fixtures preserve the source checkout/index, exclude staged B from A's candidate, and preserve failed candidates. Provider schemas/config parsing are verified locally; actual sandbox enforcement is not.

Root owns shared files, control/persistence, application integration, this record and commits. Six completed local checkpoints are recorded below. Current in-flight ownership: root owns the dispatcher/runtime engine and shared control integration; the configuration agent owns active reload/permission fences and decision integration; the Codex agent owns the native host, worker construction and executable/service wiring; the delivery agent owns exact-revision child/parent/release progression. `main.rs` remains unwired at this checkpoint. Component APIs are not a substitute for the complete workflow; configuration/supervisor/factory and full delivery integration remain required.

Latest focused component results: 30 configuration/harness, 13 configuration-transaction, 21 provider, 15 reconciliation, 11 transport, 15 worker, 14 local execution, 15 delivery-composition, 24 CLI, 8 local-CLI, 6 supervisor, 22 protocol and 20 deterministic evaluation tests. These were run by their owning agents and are subject to affected reruns as integration changes continue. Root directly reran the dispatcher/control/API/platform/channel/connection results in the summary. No aggregate full-suite or complete SC/V0-T scenario pass is claimed.

## Requirement-to-evidence checklist

Status key: **not run** is not a pass; **implemented** requires tests/review before **verified**. Detailed deterministic cases will be linked as they are executed. The complete normative contract remains SPEC; this grouped ledger does not waive any clause.

| Contract / retained requirements | Planned evidence | Current status |
| --- | --- | --- |
| §§1–2: product boundary, identities, authority, persistence; REQ-14/31/42/45/46/52/54 | Build, canonical identities, role/broker rejection, durability/ownership | Primitives implemented; 12 storage/control tests pass; process registration implemented; native qualification blocked |
| §§3.1–3.4: strict config, credentials, workflow/reload; REQ-34/35/40/41/50/51/57; H01/H02 | Config/workflow/secret/reload tests, installed asset assembly | 28 focused tests pass including context-composition review fixes; native Keychain implemented but unrun; serialized setup integration in flight |
| §§3.5–4: preparation, provider reads, authorization/scope/review; REQ-01–04/29/37–39/43/44/62 | Paginated provider fixtures, event provenance and scope revocation | Not run |
| §5: lifecycle/dependencies/corrections; REQ-07–09/19/22–24/30 | Correction races, retained child sets, dependency generations | Local correction/scope primitives tested; provider reconciliation tested; full workflow loop in flight |
| §6: scheduling/bounds/claims; REQ-12/21/32/35 | Fair account/workspace admission, attempts and pause races | Authority/scheduler 8 focused tests pass; process scheduling not integrated |
| §7: isolation/Codex/harness; REQ-31/32/34/42/54; H01/H02 | Pinned-schema protocol fixtures, OS sandbox probes, manifest/discovery | 21 deterministic protocol/isolation tests pass; actual canaries blocked; live discovery unverified |
| §8: local/hosted/review/staging/evidence/acceptance; REQ-05/06/10/11/19–21/25–28/43/44/53 | Exact-revision evidence, attestation, Git fixtures, stale tokens/video | Gate primitives tested in 18 Git/delivery cases; providers/media/workflow not integrated |
| §9: candidates/carry/queues/release/recovery; REQ-13/15–18/27/28/30 | A/B isolation, patch bytes, partial effects/holds, correction races | Initial mechanics tested; replacement and already-produced partial delta primitives tested; full release progression in flight |
| §10: complete CLI/control/overview/TLS; REQ-49–51/55–57 | CLI/API observable tests, peer identities, snapshot pagination | 3 complete snapshot/chunk tests pass; CLI/transports and dispatcher component tests pass; executable factory not complete |
| §11: intent/cancellation/recovery/cleanup/logs/handover; REQ-31/33/36/40/41/52/58–62 | Crash boundaries, restart/stop/force, preservation and redaction | Not run |
| §12.1 SC-01–SC-26, each retained scenario | Observable deterministic + required live profiles | Partial primitive coverage; no complete end-to-end scenario claimed |
| §12.2 V0-T01–V0-T37, each complete adversarial case | Deterministic process/provider/Git/transport failure fixtures | Partial focused coverage; full rows remain unverified |
| §12.4 V0-E01–V0-E08, every variant repeated per profile; H03 | Actual integrated harness trials, isolated graders, all results | Blocked live authority; 56 declared trial slots and deterministic graders delivered; actual trials not run |
| §§12.3–12.4 independent review and human calibration | Independent code/behavior review; human qualitative review | Not run; human gate blocked while unattended |
| §§13–15 traceability/licenses | Source ID coverage, no spec changes or license regression | Baseline verified; final audit not run |
| REQ-05/33/40/41/45–51/53 graphical/distribution portions | Dashboard, wrapper, frontend, Homebrew, other OS packaging | Deferred exactly as SPEC §§1/13 |

## Validation and evidence

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

## Blockers and bounded follow-up dependencies

| Dependency / affected requirements | Observed evidence, permitted work, smallest next action |
| --- | --- |
| Live agent trials and account compatibility; §7, E01–08, §12.3.3 | Not attempted: no explicit accounts, finite live plan or execution authorization. Prepare cases and production runner locally; later approve named account/profile, isolated fixtures, repetition/turn/time bounds and handling of uncertain outcomes. No paid fallback. |
| Disposable Linear/GitHub integration; §§4/8/9/12 | Not attempted: remote repository writes and deployment remain prohibited, no approved disposable targets. Prepare exact account/target/action/evidence/recovery plan; explicit scoped authorization must amend applicable prohibitions before remote test actions. |
| Tailscale/media/background/locked-session/handover; §§7/8.5/10.3/11 | Required utilities not on PATH; no installation attempted. Implement adapters and deterministic fixtures; later identify installed supported paths or operator-provisioned tools/certificates and approve bounded host testing. |
| Actual macOS execution boundary; §7.1, V0-T12, E07 | Installed Codex schema generation and credential-free strict named-permission parsing succeeded. A nested sandbox probe exited 71 (`sandbox_apply: Operation not permitted`) under the existing outer sandbox. No bypass/unsandboxed retry. Continue deterministic boundary tests; actual canaries need a permitted supervised execution environment before qualification. |
| Native execution descendants; §§7.1/11, V0-T12/23/E07 | An ordinary finite owned fixture proves setsid descendants can outlive group/EOF completion. Native result promotion now fails closed with `process_tree_unverified`; implement verifiable containment/descendant ownership before native checks can pass, then qualify in an allowed supervised environment. This is an engineering gap, not permission to use group exit as full proof. |
| Native process ownership; §§6.1/11, V0-T07–09/23 | Read-only macOS boot-identity query returns `Operation not permitted` in this sandbox. Production ownership discovery fails closed; two retained native tests are explicitly blocked. Injected deterministic process fixtures verify registration/intent/lease logic only. A permitted supervised environment is required for actual process birth/boot identity, termination and restart qualification. |
| GitHub-hosted runner provenance; §§8.2/9.4 | Job labels do not prove runner type. Signed OIDC verifier now binds fixed issuer/JWKS, audience, exact job/run/attempt/workflow and time/replay guards. The trigger SHA is distinct from the source actually checked out: the reviewed pinned proof workflow must verify the tested checkout and derive the audience accordingly. Adapter fixtures pass; proof workflow preparation/collection and actual hosted route remain unqualified. |
| Human qualitative review/calibration; §12.4.3 | Operator explicitly unavailable; not attempted. Prepare representative evidence across every qualitative rubric and all material failures, then request review only when supervision is explicit. |

These block only dependent validation, not local implementation. No missing prerequisite is permission to weaken the contract.

## Milestones and recovery notes

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
- Provider/evaluation/API/platform increments remain intentionally in flight and excluded from the committed foundation claim. All test processes are bounded local Cargo tests; no lasting service has been installed/started. Git test fixtures are owned temporary repositories. No project branch/worktree, push, deployment or coordinator activation occurred.
