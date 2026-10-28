# Kegari Rust workspace

| Package | Responsibility |
| --- | --- |
| [`kegari-cli`](crates/kegari-cli/README.md) | Public CLI, control client and the `kegari-v0` executable. |
| [`kegari-v0`](crates/kegari-v0/README.md) | Remaining backend library and the `kegari-v0-evaluation` inspection executable. |

The CLI temporarily depends on the backend; the backend does not depend on the
CLI. Both packages use the workspace version. This extraction preserves the
existing executable, process roles and configuration/protocol schemas.

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
