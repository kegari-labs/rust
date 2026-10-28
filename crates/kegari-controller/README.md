# kegari-controller

Control-service composition and supervision for Kegari. This initial extraction
retains a temporary dependency on `kegari-v0`; durable admission, registration,
configuration administration, execution and delivery remain backend-owned.

| Module | Responsibility |
| --- | --- |
| `app` | Explicit local bootstrap, authenticated dispatch and response bookkeeping. |
| `host` | Control-host composition, prerequisite handling, listeners and stale-socket recovery. |
| `supervisor` | Bounded driver supervision, lifecycle state and joined shutdown. |
| `orchestration` | Owned child, inherited framing, lease handshake and heartbeat/poll channel. |

The application executable remains `kegari-v0` in `kegari-cli`. Its setup command
calls `kegari_controller::app::bootstrap`; hidden control-host and orchestration
entry points call `host::run_native` and `orchestration::run_child`. The concrete
runtime factory remains `kegari_v0::runtime_native::NativeRuntimeFactory`.

Crate extraction does not change process placement: the control-host process
constructs and runs the runtime driver. The orchestration child owns its runtime
lock and exchanges bounded lease, heartbeat and poll frames; it does not run the
worker/delivery engine independently.

Shared runtime interfaces now live at `kegari_v0::runtime_host`, including the
configuration/workflow/driver traits, runtime dependencies and factory contract.
Durable command identity and current-operator helpers live at
`kegari_v0::control_command`. Operator identity and transport/snapshot operations
are imported directly from `kegari_control_api`. The old backend module paths
are removed without compatibility reexports.

Controller depends on backend/control-api and the existing config, digest,
platform, secrets, storage and types crates. Backend and control-api have no
controller dependency, including for tests. There is no second application build
identity: metadata and orchestration handshakes use the backend's shared identity.

From the workspace root, with the pinned toolchain and dependency cache:

```sh
cargo test --offline --locked -p kegari-controller --release
cargo build --offline --locked -p kegari-cli --bin kegari-v0 --release
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
```

This remains `0.2.0-alpha.10`, with unchanged schemas, assets and executable
arguments. Fixture verification does not establish native/live qualification.
No service activation or Keychain operation is needed to test this boundary.
