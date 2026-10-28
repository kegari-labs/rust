# Secrets extraction verification

Status: verified; no unresolved material extraction finding.

Baseline: `315dfce450fb8f88914c4a584017a61eb174c0a4` on `main`.
Version remains `0.2.0-alpha.10`. No release is packaged.

## Boundary and compatibility

`kegari-secrets` owns `Secret`, `SecretStore`, `Keychain`, `resolve_reference`,
`AttestationKey` and `Signed<T>`, exported at its root. Generic signing is in a
private module; native helpers, key fields and signing-byte construction remain
private. CLI/backend depend directly on the crate. Its workspace dependencies
are types/digest; it has no backend/provider/configuration/storage/process or
networking dependency, including development dependencies.

Backend `connections::provider_credential` replaces `Secret::credential` with
the same UTF-8 check and provider constructor. Four callers retain their original
argument/error ordering. Credential verification, import authorization,
publication framing and domain evidence policy remain backend-owned. No
compatibility reexports remain. All 239 external lockfile entries are unchanged.

The native Keychain and generic signing implementations match the baseline.
Secret limits, reference precedence, redaction, cleanup, Keychain namespaces and
create/update behavior, native errors and unsupported-platform refusal remain
unchanged. HMAC signing bytes, signature encoding, record schema/digests and serde
behavior remain unchanged. Persisted/API schemas and instruction assets retain
their existing versions.

## Review and checks

One bounded independent review found no material issues in the ownership,
native/private interfaces, signing compatibility, conversion ordering, caller
migrations, fixture visibility, dependencies or build hashing. The review also
confirmed that 56 other changed caller bodies only changed imports/paths.

The initial workspace all-target compile passed with two redundant-import
warnings in migrated fixtures. Removed those imports before focused tests.
Original output is retained at
`/private/tmp/kegari-secrets-extraction-check.log`.

Standalone release checks: eight passed, zero failed, zero ignored. The original
reference/redaction and unserializable-payload tests moved with their owners.
Five compatibility tests cover the golden vector, tampering/error order,
serialization, constructor limits and injected reference resolution. A private
naming test verifies runtime/connection namespaces without native access.
The provider-conversion redaction assertion moved into backend coverage.
Protected-storage and canonical-publication tests remain in the backend.

The synthetic golden vector was captured before extraction from the previously
verified backend build
`sha256:64815b98f0fa6d0ca6c42e28c0f4d87ed61443e1fe00ec270d9124243673fd99`.
The fixture records exact serialized bytes, signature and digest; the new crate
matches it. The capture is retained at
`/private/tmp/kegari-secrets-baseline-vector.json`; the repository fixture is
`crates/kegari-secrets/tests/fixtures/attestation-v1.json`.

All 179 focused checks passed: eight standalone secrets tests, 126 backend
configuration/connection/API/housekeeping/Git-delivery/reconciliation checks,
one provider-conversion unit test and 44 CLI checks. Zero failed or ignored.
Commands/results are retained at
`/private/tmp/kegari-secrets-extraction-focused-checks.json` and
`/private/tmp/kegari-secrets-extraction-focused-results.json`.

The first full release suite completed with 1,284 passes, two failures and the
three existing native ignores. Both failures were existing Git startup fixture
assumptions: a one-nanosecond budget need not expire before the clock's next tick,
and a concurrent fork can temporarily inherit a socket writer until exec closes
its descriptor. The failures were investigated before any rerun.

Two test-only corrections retain all existing assertions. The deadline fixture
constructs valid limits, then explicitly exhausts its private command budget and
also checks the exact command-timeout error. The stream fixture holds the
existing spawn/retirement boundary from socket creation through EOF and cleanup.
Production Git code, accepted limits and suite concurrency are unchanged. No
test was disabled and no retry, timing allowance or new hook was introduced.

The original run, command results and exact source snapshot are retained at
`/private/tmp/kegari-secrets-extraction-first-tests.log`,
`/private/tmp/kegari-secrets-extraction-first-checks.json` and
`/private/tmp/kegari-secrets-extraction-first-source.json`. All 13 corrected Git
startup checks passed with normal concurrency. The original reviewer checked
the two fixture corrections and found no material issue or boundary reentrancy.
That focused result is retained at
`/private/tmp/kegari-secrets-extraction-focused-git-startup.log`.
The final broad verification passed: **1,286 passed, zero failed, exactly three
native tests ignored**, across 67 result groups. Formatting, strict workspace
all-target Clippy, contract inventory (519 blocks, 140 named requirements) and
its 11 Python tests passed on the corrected candidate. Normal concurrency was
preserved. Two broad runs were made: the retained failed run and this final run
after the justified fixture change.

Checks used Rust/Cargo 1.99.1 through the verified `+stable` toolchain and
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`, with `--offline --locked` after
the offline lockfile update. Commands and logs are indexed in
`/private/tmp/kegari-secrets-extraction-final-checks.json`; complete suite counts
are at `/private/tmp/kegari-secrets-extraction-test-results.json`.

## Executable and source identity

Built `target/release/kegari-v0` from `kegari-cli`: an optimized Mach-O arm64
executable. Minimal-environment `--help`, `--version`, `setup --help`,
`__build-info` and `__handover-info` inspections all succeeded with consistent
application identity and no runtime-state creation. Persisted/API schemas,
instruction asset digests and asset version 7 are unchanged.

Independent reconstruction matched the application hash over 369 build inputs,
including the new crate's manifest and both source files. Cargo rebuild tracking
covers the new manifest/source tree. All 779 source/test/build inputs matched the
final reviewed snapshot.

| Identity | SHA-256 |
|---|---|
| Final source/test/build snapshot | `eb9cc8ce17c88aebace7cce0340b9256b6345ae2a60c648a9cee2094d9c13966` |
| Application build | `00033144c8d5115fbfbd0d5517bc3acf6d94e7f0437145b57b660514e4d6dc1a` |
| Optimized executable | `ad7c35647bb5b7f04259d2b1ae327ec6345e9b02ad9ae4caacf9220498746bef` |

Inspection evidence is at
`/private/tmp/kegari-secrets-extraction-inspection.json`, with final source inputs
at `/private/tmp/kegari-secrets-extraction-source.json` and review evidence at
`/private/tmp/kegari-secrets-extraction-review.json`. The existing alpha.10 archive
retains SHA-256
`864a4796e238e0a2dfa7bf87b9d3078a5f5117395c12368ae5b9c89d55967d38`.
This is a local verification build; no release was packaged.

## Retained limitations

Tests use synthetic key material and injected stores/environment lookup. No real
Keychain item is read, written, initialized, unlocked or deleted. All three native
qualification ignores remain unchanged. This refactor does not establish native
or operational qualification or change managed-workload limitations.

Goal mode remains cancelled. The existing alpha.10 archive is preserved. No
installation, service activation, release packaging, commit or remote operation
is included.
