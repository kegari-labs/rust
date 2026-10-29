# Kegari Codex client

`kegari-codex` owns the Codex protocol client, session/history data, protocol
recovery, callback-driven observations, JSON-RPC stream transport, argument
encoding and pinned `codex-0.164.0-alpha.11.1` assets. The package remains at
`0.2.0-alpha.10`; protocol/profile pins, wire formats and session digests are
unchanged.

Import its interfaces directly from `kegari_codex`. The implementation modules
are private; backend compatibility reexports are not provided. `RpcClient`
owns request sequencing, deadlines, request-gate calls, uncertain outcomes and
session protocol work. `SessionScope` supplies host revalidation and scope data;
implementing it does not establish execution authority.

[SPEC 1.5](../kegari-v0/SPEC.md) uses Codex app-server as the repository execution
layer, with native repository tools, supported sandbox/permissions and terminal
lifecycle. Kegari supplies workflow authority and consumes actual structured
results. The ordinary admitted child path now uses native Codex tools and the
custom repository executor has been removed. Remaining integration work and
focused evidence are recorded in [PROGRESS.md](../kegari-v0/PROGRESS.md); this
source migration does not establish live compatibility or qualification.

Native ownership of Kegari's app-server remains in `kegari_v0::codex_host`.
`NativeCodexClient` retains the actual child, cancellation and diagnostic task,
and lends its protocol client through `client_mut()`. `from_host_transport`
accepts ordinary streams and startup context, performs no I/O and adopts no
process. Protected session journals, receipt verification and authorized session
resume remain backend responsibilities; they do not imply per-command control
of Codex-internal execution.

`ResumeCandidate` checks retained protocol data and exposes read-only lineage.
It is not a protected journal or an admission permit. Backend
`worker_run::RetainedResumeSession` wraps this candidate after protected loading;
backend verified resume checks authority before invoking `resume_protocol`.
`ProtocolCapture` reports callback-driven observations. `ProtocolTransportEnd`
remains opaque: transport EOF does not establish process exit, successful work
or recovery authority.

`RpcTransport` owns supplied `AsyncRead`/`AsyncWrite` streams and preserves
framing, byte limits, cancellation accounting and queued-message order. It
creates no tasks or processes and supplies no deadline or authorization policy.
Its private state is exposed only through read-only diagnostics; valid-output
timing remains owned by the transport, while semantic-event timing belongs to
the client.

Interface migrations:

- Import `RpcClient`, `SessionState`, `RequestGate`, `ToolBroker`, history, resume
  and observation interfaces from `kegari_codex`.
- Use `kegari_codex::app_server_argv` for protocol arguments and backend
  `kegari_v0::codex_host::permission_configuration` for current permission derivation,
  which must migrate to the supported Codex-owned execution boundary.
- Native launch, executable verification and connection probes are free
  functions in `kegari_v0::codex_host`.
- Replace `RpcClient.last_valid_output` with `last_valid_output()` and
  `RpcClient.last_semantic_event` with `last_semantic_event()`.
- Shared run data now belongs to `kegari_types::run::{RunReference, PhaseKey}`;
  backend receipt parsing remains in `control::run_reference_from_receipt`.

Production workspace dependencies are `kegari-digest` and `kegari-types`; there
is no production backend, configuration, process, platform, storage, networking
or provider dependency. Tests may use protected disposable storage fixtures.
Backend build hashing covers this manifest, source tree and pinned assets; the
application retains one backend-owned build identity.

Use focused checks for changed protocol behavior under the [development policy](../../docs/development.md#feature-first-development-policy).
The broader commands below are a later stabilization reference:

```sh
cargo test --offline --locked -p kegari-codex --release
cargo test --offline --locked --workspace --release
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Tests use synthetic protocol frames, in-memory streams, checked-in schemas and
disposable fixtures. They do not invoke installed Codex, live accounts, Keychain
or native probes. No operational qualification, schema upgrade or new release
is implied.
