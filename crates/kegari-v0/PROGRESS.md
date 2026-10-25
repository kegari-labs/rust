# Kegari v0 build progress

Local implementation is in progress. The foundation suite now has **89 passing deterministic tests** and strict targeted Clippy; material review findings have regressions. This is not a complete application or operational readiness claim: `main.rs` is still the initial empty entry point, and provider/control/orchestration wiring remains to build. The operator is **unattended**; live agent evaluations, remote mutations, installation, deployment and required human calibration have not been authorized or run. Independent local engineering continues.

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

Root owns shared files, control/persistence, integration, this record and commits. Configuration agent is implementing real Linear/GitHub adapters; Codex agent is implementing bounded evaluation fixtures/runner. Git agent is completing review corrections and will implement replacement/already-integrated delta accounting next. Their in-flight changes are excluded from checkpoint claims. Next: correct the discovered plain-workflow context omission, commit tested foundations, integrate providers and narrow tools, implement CLI/Axum TLS/Unix control and verified process supervision, then the complete reconciliation loop and all remaining acceptance cases.

## Requirement-to-evidence checklist

Status key: **not run** is not a pass; **implemented** requires tests/review before **verified**. Detailed deterministic cases will be linked as they are executed. The complete normative contract remains SPEC; this grouped ledger does not waive any clause.

| Contract / retained requirements | Planned evidence | Current status |
| --- | --- | --- |
| §§1–2: product boundary, identities, authority, persistence; REQ-14/31/42/45/46/52/54 | Build, canonical identities, role/broker rejection, durability/ownership | Primitives implemented; 12 storage/control tests pass; full process ownership not implemented |
| §§3.1–3.4: strict config, credentials, workflow/reload; REQ-34/35/40/41/50/51/57; H01/H02 | Config/workflow/secret/reload tests, installed asset assembly | 28 focused tests pass including context-composition review fixes; Keychain/setup not implemented |
| §§3.5–4: preparation, provider reads, authorization/scope/review; REQ-01–04/29/37–39/43/44/62 | Paginated provider fixtures, event provenance and scope revocation | Not run |
| §5: lifecycle/dependencies/corrections; REQ-07–09/19/22–24/30 | Correction races, retained child sets, dependency generations | Local correction/scope primitives tested; provider-backed loop not implemented |
| §6: scheduling/bounds/claims; REQ-12/21/32/35 | Fair account/workspace admission, attempts and pause races | Authority/scheduler 8 focused tests pass; process scheduling not integrated |
| §7: isolation/Codex/harness; REQ-31/32/34/42/54; H01/H02 | Pinned-schema protocol fixtures, OS sandbox probes, manifest/discovery | 21 deterministic protocol/isolation tests pass; actual canaries blocked; live discovery unverified |
| §8: local/hosted/review/staging/evidence/acceptance; REQ-05/06/10/11/19–21/25–28/43/44/53 | Exact-revision evidence, attestation, Git fixtures, stale tokens/video | Gate primitives tested in 17 Git/delivery cases; providers/media/workflow not integrated |
| §9: candidates/carry/queues/release/recovery; REQ-13/15–18/27/28/30 | A/B isolation, patch bytes, partial effects/holds, correction races | Initial mechanics tested; replacement and already-produced partial delta accounting still incomplete |
| §10: complete CLI/control/overview/TLS; REQ-49–51/55–57 | CLI/API observable tests, peer identities, snapshot pagination | 3 complete snapshot/chunk tests pass; CLI/transports not implemented |
| §11: intent/cancellation/recovery/cleanup/logs/handover; REQ-31/33/36/40/41/52/58–62 | Crash boundaries, restart/stop/force, preservation and redaction | Not run |
| §12.1 SC-01–SC-26, each retained scenario | Observable deterministic + required live profiles | Partial primitive coverage; no complete end-to-end scenario claimed |
| §12.2 V0-T01–V0-T37, each complete adversarial case | Deterministic process/provider/Git/transport failure fixtures | Partial focused coverage; full rows remain unverified |
| §12.4 V0-E01–V0-E08, every variant repeated per profile; H03 | Actual integrated harness trials, isolated graders, all results | Blocked live authority; fixtures/graders still to implement |
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
- Real-agent evaluations: not run. Real integrations: not run. Human calibration: not run. No provider mutations or unknown remote outcomes exist for this build.

## Blockers and bounded follow-up dependencies

| Dependency / affected requirements | Observed evidence, permitted work, smallest next action |
| --- | --- |
| Live agent trials and account compatibility; §7, E01–08, §12.3.3 | Not attempted: no explicit accounts, finite live plan or execution authorization. Prepare cases and production runner locally; later approve named account/profile, isolated fixtures, repetition/turn/time bounds and handling of uncertain outcomes. No paid fallback. |
| Disposable Linear/GitHub integration; §§4/8/9/12 | Not attempted: remote repository writes and deployment remain prohibited, no approved disposable targets. Prepare exact account/target/action/evidence/recovery plan; explicit scoped authorization must amend applicable prohibitions before remote test actions. |
| Tailscale/media/background/locked-session/handover; §§7/8.5/10.3/11 | Required utilities not on PATH; no installation attempted. Implement adapters and deterministic fixtures; later identify installed supported paths or operator-provisioned tools/certificates and approve bounded host testing. |
| Actual macOS execution boundary; §7.1, V0-T12, E07 | Installed Codex schema generation and credential-free strict named-permission parsing succeeded. A nested sandbox probe exited 71 (`sandbox_apply: Operation not permitted`) under the existing outer sandbox. No bypass/unsandboxed retry. Continue deterministic boundary tests; actual canaries need a permitted supervised execution environment before qualification. |
| GitHub-hosted runner provenance; §§8.2/9.4 | Job labels do not prove runner type. Official GitHub OIDC docs and public issuer discovery expose `runner_environment`, `check_run_id`, repository/run/attempt/SHA and workflow claims; planned verifier binds signature, issuer, challenge, exact job/revision/workflow and time/replay guards. Proof workflow preparation and verifier still to implement; actual route remains unqualified. |
| Human qualitative review/calibration; §12.4.3 | Operator explicitly unavailable; not attempted. Prepare representative evidence across every qualitative rubric and all material failures, then request review only when supervision is explicit. |

These block only dependent validation, not local implementation. No missing prerequisite is permission to weaken the contract.

## Milestones and recovery notes

- Initial inspection complete; clean scaffold contains only `fn main() {}`. No existing runtime to preserve or activate.
- Initial dependencies fetched successfully into `/private/tmp/kegari-v0-cargo-cache`; exact versions are locked. Generated Codex schema probe data is under `target/codex-schema`; selected fixtures are in `protocol/`. No model turn was launched.
- One formatter raced creation of the toolchain pin and attempted rustup sync; the sandbox rejected its temporary-file write before installation. Repeated with the existing verified stable alias successfully. No host config changed.
- Local checkpoint `24c7d4e` — `feat(kegari-v0): add durable control and instruction foundations` records strict config/workflow/assets, authority/correction/scheduling, private storage/control and snapshot primitives plus focused tests and the initial coverage ledger. No commits were consolidated or rewritten. Coordinator alone stages/commits; all source remains task-owned and initial index was empty.
- Codex adapter/schema and Git/delivery checkpoints are next. Providers/evaluation work remains intentionally in flight and excluded from foundation validation. Git test fixtures are owned temporary repositories; no project branch/worktree, push, deployment or coordinator activation occurred.
