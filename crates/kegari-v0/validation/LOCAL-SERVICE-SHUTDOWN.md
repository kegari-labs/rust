# Service-shutdown recovery correction and alpha.2 artifact

The service-shutdown gate now rejects unresolved recovered local ownership and
latched control-persistence uncertainty before reserving platform shutdown.
Prepared/Sent shutdown replay returns `operation_uncertain` without repeating
the reservation. Retired local work can still stop while its remote-only result
remains unknown; that receipt is preserved. Historical terminal replay remains
idempotent. These corrections apply to the managed-workload contract in SPEC 1.3.

The original implementation failed the new recovered-admission regression: empty
live maps allowed a successful service-stop receipt despite the retained local
ownership hold. The failure log is retained. The final **85 release-profile tests
passed**, with zero failures/ignored tests across six targets. Three new tests
cover recovered admission/suspension, unfinished shutdown replay, and persistence
failure/recovery; an existing remote-uncertainty test now covers cold service stop.
The persistence fixture induces a real pre-write filesystem failure and checks the
error latch; it does not simulate a post-rename/fsync crash.

| Check | Result |
| --- | --- |
| Initial focused regression against old implementation | Expected failure, exit 101 |
| Release build | Passed |
| Release tests: app, config_harness, control_preparation, control_storage, local_cli, main | 85 passed |
| Clippy, all targets, warnings denied | Passed |
| Formatting | Passed |
| Four direct staged-artifact inspections | Passed |
| SPEC 1.3 inventory | 503 clauses, 136 named rows; no promoted coverage |

Commands use the verified Rust/Cargo 1.99.1 `stable` alias and private cache
`/private/tmp/kegari-v0-cargo-cache`, from the Rust workspace root:

```sh
cargo build --locked --release -p kegari-v0 --bin kegari-v0 --message-format=json
cargo test --locked --release -p kegari-v0 --test main --test config_harness --test control_storage --test control_preparation --test app --test local_cli
cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings
cargo fmt --all -- --check
```

Artifact: [kegari-v0 0.2.0-alpha.2](/Users/lgr/Development/com.kegari/com.kegari.rust/target/prereleases/kegari-v0-0.2.0-alpha.2-aarch64-apple-darwin-19fdb21e361c/kegari-v0).
It targets `aarch64-apple-darwin`, contains 40,308,032 bytes, and has executable
SHA-256 `19fdb21e361cb90a4193079ebbafda569e8020724accc13901e19e0e8e0dd633`.
Its source/profile digest is
`sha256:6ea30df674c0dc6efbcad2a664b03e4a9c21f5e026b1ed3319ba3f233f11da9e`.
All 313 build inputs are retained by hash in the adjacent `artifact.json`; the
build digest was independently reproduced. Embedded instruction content remains
version 3, with its runtime catalog bound to application alpha.2. No dependency
or persisted schema version changed. The alpha.1 artifact remains unchanged.

The staged artifact passed `--help`, `--version`, `__build-info` and
`__handover-info` from an empty disposable cwd with a minimal PATH environment,
closed stdin and 15-second command timeouts. No files appeared in that cwd. Its
metadata and all six embedded asset hashes matched source. Cargo test produced
a different executable in the shared output path; entrypoint integration tests
used that separately recorded binary. Both have the same source/profile digest,
but no byte-equivalence or complete toolchain reproducibility claim is made.
The staged binary and all build inputs remained unchanged after the tests.

Independent source review found no material issue. Exact commands, file hashes,
logs, review and limits are in [service-shutdown-checks.json](service-shutdown-checks.json).
No complete-suite rerun or native/agent/provider/human qualification is claimed.
The two required native checks remain unexecuted. Tests use owned disposable
state and existing fixture adapters; no real service, Keychain, installation or
activation is exercised. The artifact is development output removable by Cargo
cleanup, not an immutable installation or permission to activate a coordinator.

Managed-workload supervision is still awaiting integration and qualification;
repository commands retain their preflight refusal. The public handover route,
worker-reconciliation proof, exact old-supervisor removal, partial-handover
reconciliation and qualification-record workflow remain incomplete. Full
descendant coverage and qualification for untrusted external context remain
separate future obligations; this lifecycle fix establishes neither.
