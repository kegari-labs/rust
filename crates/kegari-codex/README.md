# Kegari Codex protocol and transport

`kegari-codex` owns independent Codex protocol data, JSON-RPC decoding, stream
transport, argument encoding and the pinned `codex-0.164.0-alpha.11.1` assets.
The package remains at `0.2.0-alpha.10`. Protocol/profile pins and wire formats
are unchanged.

Import its interfaces directly from `kegari_codex`. Backend compatibility
reexports are not provided. The backend `codex` module still owns `RpcClient`,
`SessionState`, tool/request authorization, deadlines, uncertain mutations,
history/recovery, protocol observation, native executable probes and process
ownership/cleanup. This is the first extraction toward complete adapter
ownership; the crate does not yet provide a complete application client.

`RpcTransport` accepts caller-owned `AsyncRead`/`AsyncWrite` streams and preserves
framing, byte limits, cancellation accounting and queued-message order. Its
`CleanEof` result is a stream observation, not successful-work or owned-process
exit evidence. It creates no tasks or processes and supplies no permissions,
deadlines, retries or recovery policy. Stream and decoder fields stay private;
read-only diagnostics expose pending/wire bytes, queue/EOF state and the sole
valid-output timestamp.

Interface migrations:

- `RpcClient::app_server_argv` becomes `kegari_codex::app_server_argv`.
- `PermissionConfiguration::for_isolation` becomes backend
  `kegari_v0::codex::permission_configuration`; host permission derivation is not
  part of the protocol data type.
- `RpcClient.last_valid_output` becomes a forwarding `last_valid_output()`
  accessor. Its transport owns that timestamp so cancellation cannot discard an
  already-observed update.
- `SessionEvent::new` and `UsageLedger::observe_resume_baseline` are public pure
  data/observation operations. Neither creates execution authority.

The only workspace dependency is `kegari-types`. The crate has no backend,
configuration, process, storage, networking or provider dependency, including in
tests. Backend build hashing covers this manifest, source tree and pinned
protocol assets; the application retains one backend-owned build identity.

From the workspace root with the pinned toolchain and dependency cache:

```sh
cargo test --offline --locked -p kegari-codex --release
cargo test --offline --locked --workspace --release
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Tests use synthetic protocol frames, in-memory streams and checked-in schemas.
They do not invoke installed Codex, live accounts, Keychain or native probes.
No operational qualification, schema upgrade or new release is implied.
