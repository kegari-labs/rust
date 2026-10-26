# Managed command supervision and alpha.5 artifact

Subsequent supervised NQ-02A execution failed in its first case; see the
[native failure record](NATIVE-MANAGED-COMMAND.md). The alpha.5 checks below
remain historical deterministic/artifact evidence, not native qualification.

Alpha.5 implements managed foreground completion using the original owned group,
durable process-journal retirement and joined output collectors. Checks, hooks
and the owned clone dispatcher reserve cancellation ownership before starting.
The release regression suite passed **1,051 tests, zero failures, three ignored**
across 39 groups. Production repository execution still refuses unqualified
profiles; this increment does not complete milestone 0.2.0.

| Capability | Implemented and checked | Qualification limit |
| --- | --- | --- |
| Foreground completion | Original `OwnedGroupExit`, successful inherited-guard/journal retirement and both joined EOF collectors are required | Owned group coverage; no arbitrary detached-descendant claim |
| Caller cancellation/drop | An immediately started supervisor retains process and collector ownership when `wait` is dropped; cleanup has finite async deadlines | Async deadlines cannot preempt a blocked synchronous OS/storage call |
| Output and journal failure | Output uncertainty is monotone; failed readers and their siblings are joined or aborted; journal failure retains the original token for retry | An uncertain result never becomes a successful check from EOF alone |
| Launch ownership | Bounded slots precede checks, hooks and clone tasks; late cancellation remains attached; known preflight failures precede reservation | Opaque start failures preserve uncertainty; unrelated legacy registry consumers retain their existing limits |
| Stop handling | Every retained cancellation is attempted; the first error and all per-effect failures are preserved | One failure cannot silently skip another owned cancellation |
| Native test preparation | One ignored fixed Seatbelt component test covers output/redaction, fixture read/write boundaries, nonzero exit and timeout | Compiled and pinned as NQ-02A; not executed or qualified |

The initial focused run passed seven tests, failed five and ignored one. Those
failures occurred in the synthetic cold-reader assertions after actual command
outcome/closure checks. The fixture now supplies synthetic `Gone` only after
asserting the actual original exit token and durable retirement. A compiler
warning prompted explicit whole-collector-guard capture and an
abort-before-first-poll regression. The corrected focused run passed 13 tests
with the native test ignored; three registry component tests also passed.

After the full suite, clippy rejected a test lock lifetime around an `await`.
The test already explicitly dropped the guard; the correction makes its scope
unambiguous by copying the boolean before awaiting. The affected exact regression
passed again and all-target clippy passed with warnings denied. No production or
native-fixture code changed after the full suite. The earlier full-suite evidence
is reused, with both source manifests and the one changed test source retained.
Formatting and the 503-clause/136-requirement inventory check passed; no coverage
status was implicitly promoted.

A final independent evidence audit verified the artifact/input/log hashes, raw
test totals, retained source correction and unexecuted native request.
Independent reviews covered the supervisor, preflight, registration/cancellation
races and actual call-site retirement predicates. No further material finding
remained after corrections. Review also led to retaining the native fixture's
root on failure or timeout rather than deleting uncertain ownership evidence.
Ordinary component tests use real child/group/pipe/guard/journal objects with
explicitly synthetic process metadata or cancellation callbacks. They are not
native Seatbelt qualification or authenticated worker admissions.

All Cargo commands used `RUSTUP_TOOLCHAIN=stable` and
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache` with Rust/Cargo 1.99.1:

```sh
cargo test --locked --release -p kegari-v0 --lib isolation:: -- --test-threads=2 --nocapture
cargo test --locked --release -p kegari-v0 --lib worker::execution_registry_tests -- --test-threads=2 --nocapture
cargo test --locked --release -p kegari-v0 --no-fail-fast -- --test-threads=2 --nocapture
cargo test --locked --release -p kegari-v0 --lib isolation::command_tests::retained_exit_and_joined_collectors_survive_original_journal_write_failure -- --exact --test-threads=1 --nocapture
cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings
cargo fmt --all -- --check
python3 crates/kegari-v0/tools/contract_inventory.py --check
cargo build --locked --release -p kegari-v0 --bin kegari-v0 --message-format=json
```

[Machine evidence](managed-command-supervision-checks.json) records commands,
failed and successful logs, source manifests, reviews and limits.

Artifact: [kegari-v0 0.2.0-alpha.5](/Users/lgr/Development/com.kegari/com.kegari.rust/target/prereleases/kegari-v0-0.2.0-alpha.5-aarch64-apple-darwin-b7f0e3ca25de/kegari-v0).
Target `aarch64-apple-darwin`, size 41,015,904 bytes, mode `0555`.
SHA-256: `b7f0e3ca25de43318ac0b1a0c0074482791799c794acc7205e8d35826e505e3d`.
Source/profile build identity:
`sha256:ddc6c4ac4312912ff5f46475a5ad2509a096fba7229fdf74792f7f6cd3d9fc78`.

The adjacent artifact manifest reproduces the build identity from 318 inputs;
separate manifests retain 714 test/evaluation inputs before and after the
one test-only lint correction. The staged executable passed `--help`,
`--version`, `setup --help`, `__build-info` and `__handover-info`, each with a
15-second timeout, closed stdin, minimal environment and empty working directory.
No inspection created cwd files. Full tests used Cargo test artifacts; the staged
standalone binary received only those five inspections. Required license/notice
files accompany it, and prior artifacts are preserved. Instruction assets remain
version 3; handover metadata 3, process journal 2 and managed schema 1 are unchanged.

NQ-01A's previously approved identity result remains narrow historical evidence.
NQ-01B and NQ-02A await separate approval. NQ-02A pins the full-suite test artifact
before the later test-only lock-scope correction; its production and native
fixture source is unchanged. Its exact request, executable hash and one-attempt
wrapper are retained at
[target/native-qualification/nq-02a-a989ca4d2520/request.json](/Users/lgr/Development/com.kegari/com.kegari.rust/target/native-qualification/nq-02a-a989ca4d2520/request.json).
No native check, setup, Keychain operation, service installation/start, provider
mutation, signing, publication or push was performed in this increment.

The next qualification step is a closed shipped supported-profile/case manifest,
matching configuration checks and separately approved actual native evidence.
A new runtime qualification issuer/registry is not required by SPEC and is not a
blocker; see the [qualification design](../docs/managed-qualification-design.md).
Registered services, supported build-toolchain qualification and later milestones
remain incomplete. Full descendant coverage is still required before claiming
qualification for untrusted external context.
