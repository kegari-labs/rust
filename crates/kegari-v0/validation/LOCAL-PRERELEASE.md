# Kegari v0 0.2.0-alpha.1 prerelease evidence

The optimized macOS arm64 executable was built successfully. The same source
passed **34 focused release-profile tests**: 31 configuration/instruction tests
and 3 entrypoint tests, with zero failures or ignored tests in those targets.
This development artifact does not complete the 0.2.0 milestone or operational
qualification. Repository commands still refuse `process_tree_unverified`.

| Identity | Value |
| --- | --- |
| Application | `kegari-v0 0.2.0-alpha.1` |
| Target/profile | `aarch64-apple-darwin` / `release` |
| Source/profile digest | `sha256:8b8a461d6e87460abe4b4166e889fa1f77055f2f7ca0bc9ee360e54561b1ad1f` |
| Executable SHA-256 | `7c702b2da058157013ededbb812dc88e94296e11bb0bcc9511695e6263a2b7f9` |
| Executable size | 40,236,832 bytes |
| Instruction asset version | `3` (content unchanged) |
| Compiler / Cargo | `1.99.1` / `1.99.1` |

Artifact: [kegari-v0](/Users/lgr/Development/com.kegari/com.kegari.rust/target/prereleases/kegari-v0-0.2.0-alpha.1-aarch64-apple-darwin-7c702b2da058/kegari-v0).
The versioned directory contains license/notices, exact inspection output,
build/test logs and `artifact.json` with hashes of all 313 build inputs. It is
under ignored development output, not an immutable installation. Its executable
mode is 0555; that alone does not protect its parent path or authorize operation.
Cargo cleanup can remove it. Rebuild from committed sources using the commands below.

Source base: `cc0de47cc279eeee3531dddf1ad067dcb8947265`, plus the package,
lockfile package row and embedded catalog version changes accompanying this
report. No runtime algorithm, dependency version, instruction content or persisted
schema changed. The reported source/profile digest was independently reproduced
from exact input bytes. Actual compiler/Cargo versions and explicit environment
overrides are recorded; this is not a bitwise reproducibility or complete
linker-provenance claim.

Commands ran from the Rust workspace root with `RUSTUP_TOOLCHAIN=stable`
(verified as pinned 1.99.1) and
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`:

```sh
cargo build --locked --release -p kegari-v0 --bin kegari-v0 --message-format=json
cargo test --locked --release -p kegari-v0 --test main --test config_harness
cargo fmt --all -- --check
python3 crates/kegari-v0/tools/contract_inventory.py --check
```

All exited 0. Inventory remains 496 contract blocks and 136 named requirements;
no coverage status was promoted. The staged executable independently passed
`--help`, `--version`, `__build-info` and `__handover-info` from a fresh disposable
working directory with only a minimal PATH environment, stdin closed and a
15-second timeout per command. Each exited 0 with empty stderr. No files appeared
in that directory. All six reported asset hashes matched source and the embedded
catalog; version/build metadata agreed. The staged artifact and all source inputs
remained unchanged after inspection and the test run.

The first evidence collection stopped before writing its report: Cargo's test
invocation replaced the shared `target/release/kegari-v0` with different bytes.
The separately staged build artifact was unchanged. Both report the same source
and profile digest, but the entrypoint integration tests exercised the test-build
binary; the four direct inspections exercised the staged artifact. The corrected
record distinguishes both executable hashes rather than asserting equivalence.

Independent source reviews found no material version/asset-coupling issue. Asset
version 3 is retained; application versioning does not bump configuration,
protocol or evidence schemas. The final evidence review and exact logs are in
the [machine report](prerelease-checks.json).

The historical 1,006-test full-suite result remains evidence for the preceding
0.1.0 checkpoint; it was not rerun or relabelled for this artifact. The two
required native tests remain unexecuted in this increment. No agent/model,
provider mutation, entitlement request, signing, installation, coordinator
activation or remote publication occurred. Containment, native/provider
compatibility and human qualification remain incomplete. The earlier prepared
native-test binary is not the prerelease artifact; its identity must be refreshed
before any approved qualification request for this version.
