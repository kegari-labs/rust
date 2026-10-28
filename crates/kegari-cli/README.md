# Kegari CLI

This package owns the `kegari-v0` executable and the `cli`, `client` and
`local_cli` library modules. Rust consumers use `kegari_cli` for these modules.
Command arguments, output, local setup, service actions and hidden process
entrypoints retain their existing behaviour.

The temporary dependency is `kegari-cli` → `kegari-v0`. The backend continues to
provide configuration, protocol types, storage, credentials, platform operations
and the runtime. This is not yet a lightweight standalone client. The backend's
build identity covers both packages' application sources; CLI version output and
internal metadata identify that same assembled application.

From the workspace root:

```sh
cargo build --offline --locked --release -p kegari-cli --bin kegari-v0
cargo run --offline --locked -p kegari-cli -- --help
cargo test --offline --locked -p kegari-cli
```

Use `--workspace` for the complete test and lint checks. The evaluation-inspection
binary remains in the backend package. See the [command guide](../kegari-v0/docs/cli.md),
[backend documentation](../kegari-v0/README.md) and [progress](../kegari-v0/PROGRESS.md).
The archived alpha.10 package is unchanged by this source refactor; no new
prerelease, native qualification or service activation is implied.
