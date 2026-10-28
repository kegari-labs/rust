# Kegari Rust workspace

| Package | Responsibility |
| --- | --- |
| [`kegari-types`](crates/kegari-types/README.md) | Shared command, response, error and snapshot data types. |
| [`kegari-cli`](crates/kegari-cli/README.md) | Public CLI, control client and the `kegari-v0` executable. |
| [`kegari-v0`](crates/kegari-v0/README.md) | Remaining backend library and the `kegari-v0-evaluation` inspection executable. |

Both CLI and backend depend directly on `kegari-types`. The CLI also temporarily
depends on the backend; the backend does not depend on the CLI. Shared types are
imported from `kegari_types`, without backend compatibility reexports. Validation,
authorization, exit-code mapping and snapshot operations remain in the backend.
All packages use workspace version `0.2.0-alpha.10`; the executable name, process
roles and serialized schemas remain unchanged. This extraction creates no new
prerelease or qualification claim.

From the workspace root, using the pinned toolchain:

```sh
cargo build --offline --locked -p kegari-cli --bin kegari-v0
cargo run --offline --locked -p kegari-cli -- --help
cargo test --offline --locked --workspace
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

See [current progress](crates/kegari-v0/PROGRESS.md) and
[version milestones](crates/kegari-v0/MILESTONES.md). Goal mode remains cancelled;
native execution and operational qualification remain incomplete.
