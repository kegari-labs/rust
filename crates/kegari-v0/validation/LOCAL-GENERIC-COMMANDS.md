# Local generic command invocation increment

Partial implementation/evidence for SPEC 1.4, 0.2.0 / M02-2–M02-3. This does not
complete a work package, release, native qualification or actual-agent trial.
Baseline: `aba782808a17693bd3c3bb8a29c7020ec7c21e3c`, current branch `main`.
Package remains `0.2.0-alpha.9`; instruction assets remain version 4. New command
preparation, completion, observation, chunk/reference and effect records use
explicit schema 1. Existing managed schema 5 and native qualification refusal stay.

## Implemented behavior

The actual scoped worker exposes `command_run` to admitted executable roles with
literal argv and cwd. No named check, target-toolset parser, manifest, executable
catalog or supplied environment is required. Cwd derivation preserves the original
isolation anchor, grants and domain; aliases cannot escape it. The production
renderer observes selected and canonical executables, launcher, cwd, fixed
environment, backend policy and original/derived boundary identities.

The executor binds preparation, source and executable versions, invocation,
original admission and acknowledged effective correction. Preparation/dispatch
consume the original monotonic command deadline. Native dispatch carries that
cutoff through renderer, journal and spawn interlock; it shares one output budget
across both streams and bounds the UTF-8/redacted result. Post-exit evidence
collection has a separate finite allowance and cannot start another command.
Filesystem/Git operations use cooperative deadline checks, not syscall preemption.

Generic commands may observe dirty authorized source, change permitted files and
return unsuccessful or timed-out outcomes. Full source snapshots use no worker
output exclusions. Source snapshots represent file bytes, link text and modes;
they do not represent empty-directory inventory. Git metadata and original HEAD
remain pinned. Actual observations never become validation success, a failed-check
repair capability, or permission to broaden the scope.

The worker uses actual Control begin/complete gates and original registered
cancellation ownership. Registry entries survive process exit until protected
completion history and the current-authority acknowledgment are persisted. A
known pre-dispatch refusal consumes the actual preparation into `NotStarted`;
dispatch uncertainty cannot be cleared by absence of a Started event. Nonzero
exit is a completed command fact. Missing closure, stale inputs, persistence
failures or rejected current authority preserve unknown/unsettled state.

Command entries retain their complete typed bytes in ordered protected chunks.
Each entry is capped at 32 MiB; all worker events share the existing 64 MiB recovery
allowance, including hex/envelope overhead. The whole entry is reserved before its
first write; partial writes poison further appends and retain the prefix. The
reader verifies original bindings, exact chunks/digests/order, lifecycle and final
acknowledged receipt. Incomplete or uncertain histories block automatic resume.
Decoded history does not restore an old permit, process handle or live capability.

Tool replies contain compact references and bounded previews; `command_history`
provides bounded recent descriptors with omissions explicit. Full-output paging,
discovered validation obligations, independent adequacy review, empty/unborn
baselines, broader environment discovery/admission and cold per-invocation process
reconciliation remain open. Named legacy validation remains a separate transitional
path; this increment does not claim to complete its SPEC 1.4 replacement.

## Review and corrections

Independent review identified and corrected the mismatch between per-stream
native capture and the invocation's aggregate output allowance, including UTF-8
and redaction expansion. It also caught passing the original rather than effective
binding for an acknowledged correction. Existing adapter, engine and worker
regressions were preserved; new tests exercise both cases. The existing domain
unit test was mechanically adapted to the new observation/projection helper;
its alias/canonical identity assertions are unchanged.

Independent review of the reader found that correction history also needs to
reject changes to original authorization, definition and scope generation fields.
That descriptive consistency correction and exact original deadline equality are
implemented with a rewritten-prefix regression; live correction admission already
enforces those unchanged authority fields. No
serialized history is treated as authority.

The executor/worker and source-capture slices received bounded independent
reviews; the isolation author reviewed their own adapter work. Root reviewed the
integrated boundaries. None of these reviews substitutes for native, actual-agent
or human qualification.

## Validation status

All commands run locally with installed stable Rust/Cargo 1.99.1,
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache` and `--offline --locked`.
No ignored native test is enabled and no provider, installation or live operation
is performed. All historical failures and approval limits remain retained.

- First all-target check failed on the renamed private observation helper in one
  existing domain unit test: `/private/tmp/kegari-command-check1.log`.
- The mechanical test adaptation was followed by a passing all-target check:
  `/private/tmp/kegari-command-check2.log`.
- First full release regression finished with 1,148 passes, four failures and
  three retained native ignores. All four failures were in the new history reader:
  the common evidence digest validator accepts schema 1 but the actual managed
  policy has schema 5. The reader now validates that explicit schema separately.
  Retained full-run log:
  `/private/tmp/kegari-command-tests1.log`, source snapshot
  `/private/tmp/kegari-command-source1.json`, logical source digest
  `sha256:8bd1fb3abaa130466418e0bb672393779ce1d2166f6846cdf72175f594585e55`.

The corrected reader and one added history regression use source digest
`sha256:2339ab5060cbcb8fafba32a0ee2a97e7a2815798bfe974c10e7c4a9ac4a7ba28`
(`/private/tmp/kegari-command-source2.json`). All 12 focused worker tests passed
in `/private/tmp/kegari-command-focused2.log`. The complete release rerun then
passed **1,153 tests, zero failures and three native ignores**, 40 result groups:
`/private/tmp/kegari-command-tests2.log`.

Final lint identified duplicate fixture module loading plus two remainder-check
style warnings (`/private/tmp/kegari-command-clippy1.log`). The command tests now
share the existing component fixture once, alongside the bundle tests; the journal
writer remains confined to the private worker module. Both remainder checks use
the equivalent standard `is_multiple_of` method. No warning was suppressed and no
assertion removed. Independent reverse-delta review reproduced all four source2
hashes exactly and confirmed unchanged bodies/assertions for all 12 command tests.

Final source digest is
`sha256:fb91d04590649f6f9ce668c0a864e64c0c64e268f5942b2ef14bfb7ce82bcf34`
(`/private/tmp/kegari-command-source3.json`). All-target clippy with `-D warnings`
passed (`/private/tmp/kegari-command-clippy2.log`). Final-source worker tests
passed 34 cases; execution, instruction-resume and worker integration tests passed
26, three and 42 cases respectively. The exact-source release-profile development
build, formatting and final inventory check passed. The full-suite result above belongs to
source2; it is not described as a new full run on source3.

Inventory tests passed (11). The first consistency check detected only JSON
rendering differences after ledger editing; the inventory tool restored its
canonical Unicode rendering and the repeated check passed. Three §7.6 entries
have partial deterministic evidence; no full or named qualification is promoted.


## Final source and executable

Final-source commands (same Cargo cache/offline/locked settings above):

- `cargo +stable test -p kegari-v0 --release --lib worker::`
- `cargo +stable test -p kegari-v0 --release --test execution --test worker --test instruction_resume --no-fail-fast`
- `cargo +stable clippy -p kegari-v0 --all-targets -- -D warnings`
- `cargo +stable build -p kegari-v0 --release --bin kegari-v0`
- `cargo +stable fmt --all -- --check`
- `python3 crates/kegari-v0/tools/contract_inventory.py --check`

The final logical source has 730 inputs. Recomputing all 332 build inputs for
`aarch64-apple-darwin` / release agrees with the development executable's static
`__build-info` output:

- Executable: `target/release/kegari-v0`.
- Executable SHA-256: `168c9f0a55171a5d6b1285cf797cc676f9e7a842b8b8dbb88f4f1a06a5982fab`.
- Build identity: `sha256:50ec7a399074b414487bf36e8f4f462ecce82296caf969400511ebc1768543ab`.
- Static handover metadata: handover 4, managed 5, process 3, workflow-source 1,
  instruction assets 4. Package remains `0.2.0-alpha.9`.

The evidence archive `target/validation/m02-command-fb91d0459064` contains 29
files plus its checksum manifest, whose SHA-256 is
`976d9d79b511477bfe0571aac4cee913a58813b3305dfd75c1cca3ff1603b87c`.
It retains failed/successful logs, all source snapshots, exact pre-fix and pre-lint
source files, the final source patch, outcomes and static build/handover metadata.
The original source1/source2 files were reconstructed and verified byte-for-byte
against their frozen manifests; no history or failed results were erased.

This is a development build, not a new packaged release, installation or qualified
runtime. No native/live/actual-agent/human gate was performed. Single-use native
approvals remain consumed; full descendant coverage remains required before
qualification for arbitrary public/untrusted external context.
