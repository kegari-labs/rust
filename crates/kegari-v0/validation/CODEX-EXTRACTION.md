# Codex protocol and transport extraction verification

Status: verified; no unresolved material extraction finding.

Baseline: `85616a1565b28e6f922dce1ac5f50a1946ace058` on `main`.
All thirteen packages retain `0.2.0-alpha.10`; no release is packaged.

## Ownership and compatibility

`kegari-codex` owns independent protocol data, decoding, argument encoding,
stream transport and pinned schema assets. The backend depends directly on it;
CLI and controller need no direct dependency. Its only workspace dependency is
`kegari-types`; neither production nor test dependencies point back to the
backend or to configuration, process, storage or provider adapters.

The backend retains `RpcClient`, `SessionState`, request/tool gates, deadlines,
uncertain mutations, permission derivation, observations, native process ownership
and recovery. Original request authorization, cleanup and error ordering are
preserved. EOF remains a stream observation, not process-exit or work evidence.
The transport is the sole owner of framing state, wire-byte accounting and the
valid-output timestamp, including across cancellation. No task, timeout, retry
or process policy is added to the transport.

Consumers import moved types directly without compatibility reexports.
`PermissionConfiguration::for_isolation` becomes backend
`codex::permission_configuration`; `RpcClient::app_server_argv` becomes
`kegari_codex::app_server_argv`. `RpcClient.last_valid_output` becomes a forwarding
accessor. Only the approved event constructor and usage-baseline observation
method gain visibility. The connection probe retains its distinct invalid-object
error after protocol collection and termination; argument bytes and digest
inputs are unchanged.

All 38 relocated protocol files match their original bytes, including manifests,
provenance, licenses and notices. All 239 external lockfile entries are unchanged.
Supported Codex version/profile, persisted/API schemas and instruction assets
remain unchanged. The single application hash includes the new manifest, source
tree and relocated protocol assets with corresponding Cargo rebuild tracking.

## Verification

Four pure tests move out of the mixed backend harness; the decoder portion of
its malformed-protocol test moves separately while its session assertion stays.
All other RPC/session, schema-integration, permission, observation and owned
process assertions remain backend-owned. Seven deterministic stream tests and
four compact contract fixtures cover the new public boundary. Existing tail
queue, EOF, cancellation and residual-byte assertions use read-only transport
accessors; private poison assertions remain with the decoder implementation.

The first workspace all-target compile exposed inherited imports omitted during
migration (`MAX_FRAME_BYTES`, `PriorUnattributedUsage`); direct owning-crate
imports corrected them. Its original output is preserved at
`/private/tmp/kegari-codex-extraction-check.log`. The second compile passed;
its two unused imports were removed before focused testing and review. Its
output remains at `/private/tmp/kegari-codex-extraction-check-2.log`.

Verification uses verified Rust/Cargo 1.99.1 through `+stable`,
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`, and `--offline --locked` following
the offline lockfile update. Normal test concurrency and exactly three native
ignores are preserved.

One bounded independent review found no material issues. It confirmed literal
preservation of moved implementations, assertions from all 32 original protocol tests,
backend authorization/cleanup ordering and the dependency/build-identity boundary.
The 794-input reviewed snapshot is
`a7fc3578c3cf9c612bbecef87888abc28e60c6b0bfd14bae2b5d70195aa16d22`;
review metadata is at `/private/tmp/kegari-codex-extraction-review.json`.

All **168 focused release tests passed**: 16 in the new crate, 26 backend Codex
unit tests and 126 integration tests covering Codex, workers, configuration,
discovery, evaluation and instruction/resume behavior. Commands and outputs are
indexed at `/private/tmp/kegari-codex-extraction-focused-results.json`; totals
are at `/private/tmp/kegari-codex-extraction-focused-counts.json`.

Final verification passed:

- Formatting and strict workspace all-target Clippy.
- Contract inventory: 519 blocks, 140 named requirements and 11 passing tests.
- Complete workspace release suite: **1,302 passed, zero failed, exactly three
  native ignores**, across 78 result groups.
- Optimized application build from `kegari-cli`.

The complete suite ran once on the final reviewed candidate; no test failed and
no additional broad run was needed. Commands and full logs are indexed at
`/private/tmp/kegari-codex-extraction-final-results.json`; test totals and exact
native ignore names are at `/private/tmp/kegari-codex-extraction-test-results.json`.

## Executable and source identity

Built `target/release/kegari-v0`, an optimized arm64 Mach-O executable. All five
minimal-environment inspections (`--help`, `--version`, `setup --help`,
`__build-info`, `__handover-info`) passed with consistent version/build identity
and no runtime-state creation. Schemas and instruction assets are unchanged,
including handover schema 5, configuration/API schema 1 and instruction assets 7.

Independent reconstruction matched the application hash over 379 build
inputs, including the new manifest, sources and all protocol assets. Cargo
rebuild tracking includes each new input tree. All 794 reviewed source/test/build
inputs match the inspected candidate.

| Identity | SHA-256 |
|---|---|
| Reviewed source/test/build snapshot | `a7fc3578c3cf9c612bbecef87888abc28e60c6b0bfd14bae2b5d70195aa16d22` |
| Application build | `8839ea580cb51b3a5e99ef65f39ca6461fdff83a17b0b9aae9ba459ca031a65c` |
| Optimized executable | `4859bcc82b72d80c614c5793a0250a0867c8fd46deaf8b3201be5d7c497713b4` |

Evidence is retained at `/private/tmp/kegari-codex-extraction-source.json` and
`/private/tmp/kegari-codex-extraction-inspection.json`. The archived alpha.10
retains SHA-256
`864a4796e238e0a2dfa7bf87b9d3078a5f5117395c12368ae5b9c89d55967d38`.

## Retained limitations and stopping point

This is the first protocol/stream extraction; a complete Codex client and session
extraction requires a separate plan. Application behavior and qualification
remain subject to the existing milestone and native/live restrictions.

Tests use synthetic streams, schema data and disposable fixtures. Installed
Codex, live accounts and real Keychain items were not accessed. No service
activation or native qualification was performed. Goal mode remains cancelled. No release, commit, tag, installation
or remote operation was performed; the archived alpha.10 is preserved.
