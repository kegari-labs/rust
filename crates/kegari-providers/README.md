# Kegari providers

This crate owns GitHub/Linear HTTPS adapters, credentials, provider records,
selection/discovery, bounded reads and fixed typed writes. Import public clients
and records directly from `kegari_providers`. It depends on `kegari-config`,
`kegari-types` and `kegari-digest`, with no backend dependency.

Typed writes are trusted Rust integration APIs, not authority grants. Kegari's
backend retains `providers::ProviderBroker` and the unchanged `ProviderMutation`
command representation. Its durable reservation, preflight, final authorization,
owned dispatch and receipt completion sequence governs application writes.
The crate exposes no raw authenticated URL or GraphQL execution interface;
credentials, HTTP core and upload grants remain private.

The backend also retains hosted-proof verification and replay storage, receipt
ownership, association checks and sealed `VerifiedLinearAsset` construction.
Provider asset reads return bytes, not admitted evidence. The opaque
`LinearAssetUrl` uses the exact native Linear URL rules. GitHub object sessions
retain fixed origins and finite read bounds without revealing credentials.

## Rust migration

- Adapter imports move from `kegari_v0::providers` to `kegari_providers`.
- Broker, command, hosted-proof and verified-asset types keep their backend paths.
- Use `kegari_v0::providers::linear_issue_key(&client, issue)` instead of
  `client.issue_key(issue)`.
- Use `kegari_v0::providers::download_uploaded_asset(&client, &host, &reference)`
  instead of the former client method.
- Shared hashes come directly from `kegari_digest`.

The `validation` module contains only the pure parsing rules needed by retained
backend checks. Provider SHA syntax remains 40 ASCII hexadecimal characters of
either case; this differs intentionally from backend Git's lowercase 40/64 OIDs.
PR text remains limited to 1 MiB, with its bound checked against the backend
instruction context. These public helpers neither perform I/O nor grant authority.

## Verification and limits

Use the pinned toolchain from the workspace root:

Workspace-wide test/lint commands below are later stabilization references.
The current [goal](../kegari-v0/GOAL.md) requires only affected compilation and
minimal focused validation while features are being implemented.

```sh
cargo test --offline --locked -p kegari-providers
cargo test --offline --locked --workspace --release
cargo clippy --offline --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Tests use injected wire fixtures. They do not qualify live providers or native
execution. All six packages retain version `0.2.0-alpha.10`; API/configuration
schemas, provider revision pins and the `kegari-v0` executable are unchanged.
The archived alpha.10 remains untouched. See [progress](../kegari-v0/PROGRESS.md)
for evidence and retained limitations.
