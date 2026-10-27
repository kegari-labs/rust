# Local generic command-domain increment

This is partial deterministic evidence for SPEC 1.4 / 0.2.0 M02-1–M02-2.
It removes the mandatory tool catalog and verifies the replacement domain's
local contracts. It does not complete generic discovery, native viability,
actual-agent evaluation, public-PR qualification or either work package.

The baseline is `97a9d027915e28d95ec8f54c54c89e3d92143759` on `main`.
SPEC SHA-256 is
`b01064b5238554d2def840cabf6a9e5d555535e35ce4877838a55ade2dd0a6e7`.
The package remains `0.2.0-alpha.9`; no release package or installation is made.

## Implemented contract

- Configuration no longer accepts `managed_execution`, executable/digest catalogs,
  mapping inventories or generated-program opt-ins. Empty and null obsolete
  fields also refuse. Generic account, repository, role and resource authority
  remains independently enforced.
- Preparation binds the installed backend policy, then constructs managed schema
  5 from actual owned repository/scratch paths and already-admitted read resources.
  Fixed OS backend resources are included without target-toolset recognition.
  Ordered lookup preserves selected alias spelling; canonical targets must remain
  inside the domain. Explicit deeper paths and arbitrary/generated names use the
  same boundary. Ambient PATH entries do not grant host access.
- Resource schema 2 automatically observes bounded regular-file bytes, original
  filesystem identities and link text through no-follow reads. Full content
  capture/revalidation stays outside Control. Retained structural checks share
  finite budgets. Link targets are neither traversed nor claimed as captured.
- Read grants wholly inside owned or admitted writable resources retain mutable
  semantics. A broad read parent containing a narrower mutable subtree refuses
  before capture, rather than omitting read-only sibling freshness checks.
- Worker contracts require a domain when their actual bound permission allows
  repository execution. A role's maximum permission does not force a narrower
  contract to expose commands. Current authority, exact tools, source/write scope,
  original command/service ownership and correction checks remain controlling.
- Handover advertises and requires managed schema 5. Legacy catalogs/records are
  refused without migration. The backend policy also binds BUILD_ID; matching
  schema alone does not permit cross-build reuse or cleanup of retained domains.

The Codex configuration probe's obsolete executable list was removed from its
Coordination isolation. That resolves its previous pre-probe contradiction;
the existing Codex binary/version verification and read-only probe permissions
remain separate. This source correction was not exercised against a live provider.

## Review and regression corrections

Independent source review found that a broad read grant containing a narrower
writable resource could hide changes in immutable siblings. The correction uses
full coverage rather than any overlap and refuses unsupported mixed partitions.
Root then found the analogous parent-of-repository/scratch case; it received the
same correction. Both regressions first establish an otherwise-valid ordinary
isolation plan, then prove refusal of the unsupported partition and successful
freshness checks after scopes are disjoint. Independent rereview found no further
material issue in those corrections.

Other bounded review covered no-follow capture, original inode/version checks,
resource budgets, actual worker permission binding, configuration reductions and
recovery refusal. Review of the runtime author's own changes is recorded as
self-review; independent review of the core, resources and constructor correction
is separate. No review substitutes for native or human qualification.

Old tests requiring user catalogs, expected hashes, per-file executable mappings,
generated-program opt-ins or closed link dependency trees were replaced because
SPEC 1.4 removed those requirements. Replacement tests cover unfamiliar names,
ordered lookup/aliases, exact-file grants, canonical escape, roles, forbidden
roots, legacy rejection, automatically observed input changes and budgets.
Applicable ownership, correction, cleanup, stale-result and evidence assertions
remain. Historical commits, failed logs and prior qualification artifacts remain.

## Executed local checks

Commands ran from the Rust repository with the installed stable alias
(Rust/Cargo 1.99.1), `CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`,
`--offline --locked`, and no ignored-test opt-in.

| Check | Observed result |
| --- | --- |
| `cargo +stable check --offline --locked -p kegari-v0 --all-targets` | Passed; one unused native-test import warning was removed before the release suite. |
| `cargo +stable test --release --offline --locked -p kegari-v0 --no-fail-fast` | 1,104 passed, 21 failed, three native ignores, 40 result groups. All 20 execution failures stopped in shared fixture setup at noncanonical temporary paths; one housekeeping fixture still indexed the removed catalog. |
| `cargo +stable test --release --offline --locked -p kegari-v0 --test execution --test housekeeping --no-fail-fast` | After the two fixture corrections: all 44 passed, zero failures or ignores. No production/build input changed after the full run. |
| `cargo +stable clippy --offline --locked -p kegari-v0 --all-targets -- -D warnings` | Passed on final source. |
| `cargo +stable fmt --all -- --check` | Passed on final source. |
| `target/release/kegari-v0 __handover-info` | Reports metadata 4, managed 5, process 3, workflow-source 1 and instruction assets 4. No host setup or service start. |

The execution fixture now derives paths from its canonical temporary root. The
housekeeping concurrency fixture retains its original pinned Git path directly
instead of obtaining it from obsolete serialized isolation data. Their actual
effect, concurrency and evidence assertions are unchanged. This is a full run
with retained failures followed by focused correction validation, not a claimed
single all-green full-suite run at the final test-source snapshot.

Source manifests (725 source/test/fixture files each):

- Full run: `/private/tmp/kegari-domain-tests1-source.json`, logical source digest
  `sha256:3a3f50e083b1e6a6785e14e2bbaf9d2906c771ff80473b761ac4ece957758979`.
- Final focused run: `/private/tmp/kegari-domain-final-source.json`, logical digest
  `sha256:8db67ada0c4952fd2277c5553b0c5270e8e3fc41a5459b86ac06c47a3d4bf53c`.

Only `tests/execution.rs` and `tests/housekeeping.rs` differ between those
manifests. Logs use `/private/tmp/kegari-domain-`: `check1.log`, `tests1.log`
and `focused1.log`; final lint/format logs are `clippy1.log` and
`format-final.log`. Their failed outcomes remain visible.

The 13-file archive is `target/validation/m02-2-8db67ada0c49`; its checksum
manifest SHA-256 is
`2e2c71a883e4e8a3dfe3ed04a44432d2744c96a76b831134a57c08d5fc9999fd`.
It includes snapshots, logs, final source patch, metadata and outcomes. Independent
correspondence review reconstructed both pre-fix fixtures exactly and verified all
327 build inputs unchanged. Recomputing `build.rs` identity from current source
matches the inspected development executable:

- Executable SHA-256:
  `18e2dbd547406f2c013b9c6b87221b4166f95ba8bc6809ac9158718b67a13eec`.
- Build identity:
  `sha256:b5d6f124698346ff07b4cd6a2f0bc8bc53dec82326568177522afc92631fc511`.
- Inventory consistency check passed after adding two partial §7.6 evidence rows;
  no named requirement or complete clause is promoted to a full pass.

## Remaining exit criteria

Workers still expose named workflow checks. Add a generic bounded argv/cwd
producer and actual invocation facts, then revision-bound discovered obligations
and independent sufficiency review through the delivery gates. A command that
legitimately changes authorized source is not automatically failed validation;
command completion and validation satisfaction need distinct records. Do not
let arbitrary requested artifacts exclude source files from candidate snapshots.

The host still lacks a general environment observation/admission producer.
Requiring operators to configure every installed tool as a named test resource
would recreate the rejected prerequisite. Discovery must distinguish an observed
host path from actual authority, preserve alias/canonical identity and keep
credentials/private home data out of the worker. System-program-only support
does not close M02-2. Empty/unborn baselines, discovered-obligation recovery,
registered-service integration and native viability also remain incomplete.

No native qualification, setup, Keychain access, service installation/activation,
provider mutation, live agent trial or human review was performed. Existing
single-use native approvals remain consumed. The public generic execution
prerequisite remains closed, and full descendant coverage remains required for
arbitrary untrusted/public PR context qualification.
