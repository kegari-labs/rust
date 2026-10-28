# Kegari secrets

Secret values, runtime-scoped Keychain storage and generic signed-record primitives.

Import the interfaces directly from `kegari_secrets`:

| Interface | Responsibility |
| --- | --- |
| `Secret` | Bounded secret bytes, redacted formatting and existing memory cleanup. |
| `SecretStore` | Read/save contract used by native storage and in-memory fixtures. |
| `Keychain` | Native runtime-scoped storage and attestation-key loading/initialization. |
| `resolve_reference` | Explicit Keychain or whole environment-variable references. |
| `AttestationKey`, `Signed<T>` | Generic signing, verification, signed-envelope serialization and digest calculation. |

CLI and backend depend directly on this crate. Its workspace dependencies are
`kegari-types` and `kegari-digest`; it uses the existing pinned serde, HMAC,
SHA-256, hex, ring and UUID libraries. UUID defaults are disabled and only `std`
is enabled. Tests use serde_json. There is no backend, provider, configuration,
storage, process or networking dependency, including in tests.

## Ownership and compatibility

Native helpers and key material remain private. Generic signing lives in a
private `attestation` module; only `AttestationKey` and `Signed<T>` are exported
at the crate root. The signing-byte construction remains private. Secret values
and keys gain no new serialization or debug interfaces.

Keychain uses the existing `com.kegari.v0/<runtime UUID>` service, `connection/`
provider-secret account prefix and `attestation-v1` key account. Existing-key reuse,
create-only attestation initialization, provider-secret updates, native error
mapping and non-macOS refusal are preserved. There is no rotation, migration or
plaintext fallback. A generic attestation key requires at least 32 bytes; loading
the runtime's Keychain attestation key still requires exactly 32 bytes.

Signing preserves HMAC-SHA256, canonical envelope bytes, signature encoding,
schema 1, record digests and serde behavior. Signature verification establishes
record authenticity. Publication framing, evidence interpretation, delivery gates
and authorization remain backend responsibilities. Canonical hashing remains in
`kegari-digest`.

## Rust consumer migration

| Previous interface | Current owner |
| --- | --- |
| `kegari_v0::secrets::{Secret, SecretStore, Keychain, resolve_reference}` | `kegari_secrets::{Secret, SecretStore, Keychain, resolve_reference}` |
| `kegari_v0::evidence::{AttestationKey, Signed}` | `kegari_secrets::{AttestationKey, Signed}` |
| `Secret::credential()` | Backend-only `connections::provider_credential(&Secret)` helper. |

The backend does not reexport the moved interfaces. Provider `Credential` remains
in `kegari-providers`; the backend helper preserves UTF-8 validation, existing
errors and conversion ordering. Configuration transactions and authorized
credential imports remain in their existing backend modules. The CLI's temporary
backend dependency remains for other integration APIs.

## Verification and version

Run standalone checks from the workspace root with the pinned toolchain/cache:

```sh
cargo test --offline --locked --release -p kegari-secrets
```

Use workspace-wide tests and Clippy for backend and CLI integration. Compatibility
coverage uses synthetic key material, fixed signed-record vectors and an
in-memory secret store. Tests must not read, write, initialize, unlock or delete
real Keychain items. The three existing native qualification ignores remain
unchanged; this extraction adds no native qualification.

Version remains `0.2.0-alpha.10`; signed-record and other persisted/API schemas,
instruction assets and executable behavior are unchanged. The shared application
build identity includes this crate's manifest and sources. No release is packaged,
and the archived alpha.10 remains untouched. See
[current progress](../kegari-v0/PROGRESS.md) and the
[verification report](../kegari-v0/validation/SECRETS-EXTRACTION.md).
