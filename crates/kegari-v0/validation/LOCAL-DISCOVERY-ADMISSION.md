# Kegari v0 0.2.0-alpha.10 stabilization

**Completed on 2026-10-28: locally verified and packaged alpha checkpoint.**
Goal mode is cancelled; work proceeds through individual prompts and stops here.
Milestone 0.2.0 remains incomplete. Native repository-command admission remains
blocked; this is not operational qualification or an installation.

## Changes and compatibility

Repository CI, production CI, no-change and deployment declarations may be omitted
during authorized discovery. Malformed supplied policy, explicit null and empty CI
arrays remain invalid. Missing policy blocks dependent delivery; only explicit
`deployment = "none"` establishes deployment-free policy. Existing supplied-policy
serialization is preserved. Identity, scope, ownership and current-authority checks
remain required. Omission grants no production target.

Configuration reports separate discovery admission from delivery blockers. Missing
repository read access or identity still blocks admission. Complete policy receives
the existing workflow/protection checks. Production setup uses the configuration
transaction directly; the narrower initial-preparation helper is tested separately.
Policy removal still fences authority even if an unrelated field invalidates the
proposal. Current outcome-specific policy and original contract binding protect
recovery closure. Hosted proof reuse requires complete current requirements and
trusted route pins.

| Original failure | Bounded correction |
| --- | --- |
| Three discovery fixtures failed before their assertions | Supply valid routing labels matching the fixture profile; keep policies omitted and all setup, persistence, reporting and refusal assertions. |
| Unchanged policy caused a false permission fence | Compare equivalent TOML representations; preserve untyped proposal inspection and genuine removal fencing despite unrelated invalid fields. |
| Evaluation endpoint failed canonical validation | Generate GitHub's trailing `/`, retain Linear `/graphql`, join fixture URLs, and retain strict namespace/credential boundaries and negative cases. |
| Process-free recovery fixture encountered a retained lock | Acquire the existing spawn/retirement boundary before opening its lock and hold it through cleanup. Production locking and assertions are unchanged; no retry or suite-wide serialization. The original lock holder was not identified. |

Application, Cargo lock entry and embedded catalog name `0.2.0-alpha.10`.
Configuration schema stays **1**, handover metadata is **5** with repository-policy
reader schema **1**, and instruction assets remain **7**. Workflow-source schema
is 1, managed-execution schema 5 and process schema 3. Older or missing required
reader declarations still refuse.

## Verification and retained failures

Commands used the installed `stable` alias verified as Rust/Cargo **1.99.1**, the
existing `CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`, and `--offline --locked`
for Cargo build/test/lint dependency resolution. Tests retained normal parallelism.

| Check | Result |
| --- | --- |
| Original complete release regression | **1,254 passed, 6 failed, 3 native ignored**, 41 result groups; retained without relabelling. |
| Initial alpha.10 focused attempt | Compile interrupted (exit 130) when the embedded catalog's stale alpha.9 runtime version was found; corrected before restarting. No test result claimed. |
| Configuration parsing/transactions, discovery, delivery, host-process, CLI and storage-lock integration targets | **107 passed**; includes actual inherited-lock refusal and delivery-policy regressions. |
| Internal host recovery / process spawn / evaluation provider | **3 / 4 / 2 passed**, respectively. |
| Bounded independent stabilization review | **No material findings**. Compared 747 retained inputs: 10 intended changes and 737 identical files; preserved earlier delivery fixes. |
| Formatting and strict all-target Clippy (`-D warnings`) | Passed. |
| Contract inventory and its tests | **519 blocks, 140 named requirements**, no implicit passes; **11 tests passed**. |
| Final complete release regression | **1,260 passed, 0 failed, 3 native ignored**, 41 result groups. |
| Optimized executable and extracted-archive inspection | Passed; arm64 Mach-O, 382 package files verified, five commands successful, asset hashes matched and disposable runtime state unchanged. |

The final broad commands were `cargo +stable test -p kegari-v0 --release --offline
--locked --no-fail-fast`, `cargo +stable clippy -p kegari-v0 --all-targets --offline
--locked -- -D warnings`, and `cargo +stable build -p kegari-v0 --release --offline
--locked --bin kegari-v0`. Formatting used `cargo +stable fmt --all -- --check`;
inventory used `tools/contract_inventory.py --check` and its 11 unittest cases.
No broad rerun followed a successful final result. No additional tests were ignored.

Earlier compiler fixture corrections and reviewed closure/proof fixes are retained
in this patch. The original failed run used source digest
`sha256:1b562e9872e7b86116ec8546d75835de0ee72b62fa9fab11a6c16d2dd6591db4`.
Its raw log, the interrupted attempt, final logs, commands and review record are in
the package's `validation/` directory; original `/private/tmp/kegari-policy-*`
evidence remains intact. The three ignored cases are native managed foreground,
owned-child capture/signalling, and host/birth identity qualification.

## Artifact and source identity

Base commit: `c385cffe4a2d1667b247cc6d7bc56dd8f254e77f` on `main`, plus the reviewed
uncommitted working tree. All **747** final build/test inputs match the frozen
source manifest; documentation is outside the build digest.

- Source: `sha256:9c3bd1506eccc818c50050fa3161817b4a91b230bae4b6294dd2f3b508c242e0`.
- Build: `sha256:e8fabe9a8a22b5547e84e1fa23d147360c02d499f97ae9034f1acaf8d82a5e7c`.
- Executable SHA-256: `c7422735d5b04cfd36ba3c493f0f4021e1f2bc09b4779c2333884558e5be2856`.
- Archive SHA-256: `864a4796e238e0a2dfa7bf87b9d3078a5f5117395c12368ae5b9c89d55967d38`.

[Package directory](../../../target/prereleases/kegari-v0-0.2.0-alpha.10-aarch64-apple-darwin-c7422735d5b0/)
and [archive](../../../target/prereleases/kegari-v0-0.2.0-alpha.10-aarch64-apple-darwin-c7422735d5b0.tar.gz)
contain the executable, licenses/notices, release notes, build/schema/asset metadata,
source-input hashes, validation records and `SHA256SUMS`. The adjacent
[inspection record](../../../target/prereleases/kegari-v0-0.2.0-alpha.10-aarch64-apple-darwin-c7422735d5b0.inspection.json)
and `.sha256` file bind the archive and inspection output.
The adjacent [reviewable patch](../../../target/prereleases/kegari-v0-0.2.0-alpha.10-aarch64-apple-darwin-c7422735d5b0.diff)
contains the complete uncommitted changes against the base, including new tests
and this report; it is separate from the immutable archive.

The extracted executable passed `--help`, `--version`, `setup --help`, `__build-info`
and `__handover-info` with only PATH, disposable HOME/TMPDIR and LANG. All exited
zero with empty stderr. The extraction, working directory, home and temporary tree
were unchanged by those commands and removed afterwards. No setup or activation ran.

## Stopping point

No native/live/actual-agent/human qualification, installation, service activation,
commit, tag or remote operation occurred. Native execution remains unqualified;
unregistered detached descendants remain outside the managed shutdown guarantee.
Crate separation, paging beyond the current review limit, empty/unborn repositories,
execution enablement and later milestones require separate work. No material
stabilization finding remains open. Stop here and await the next planning prompt.
