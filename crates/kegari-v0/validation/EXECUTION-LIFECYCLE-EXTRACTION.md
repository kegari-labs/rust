# Execution lifecycle extraction verification

Status: verified; no unresolved material extraction finding.

Baseline: `017f913e1c6ea7570c2bba69fc89b2f8ffaa5be9` on `main`. All seventeen
packages retain `0.2.0-alpha.10`; all 239 external lock entries are unchanged.

## Ownership and compatibility

`kegari-execution` now owns native command construction, output collection,
actual child/group ownership, cancellation, journal mechanics and joined cleanup.
Backend `execution_host` retains worker/invocation/bundle adapters;
`process_records::ProcessJournal` retains worker-only and sealed host admission.
The lower journal store establishes checked resource ownership, not application
authorization. Its concrete guard cannot be reconstructed from a retained PID or
record. Shared leases now belong to `kegari_types::control::Lease`.

The one-shot journal factory runs at the existing post-render journal point and
is consumed before spawn. No callback is retained in supervision or cleanup.
Only original bundle-service options select graceful shutdown; supplied guards
retain foreground behavior and exact binding checks. Existing deadline/output
modes, the shared interlock, retained exit evidence, inherited-holder uncertainty
and joined-output requirements are preserved. Admission, receipts, reservations,
application evidence and protected recovery retain their current backend owners.

All twenty pre-relocation serialized/digest fixtures match after extraction,
including process schemas 1–3, host/lease identities and containing permit/receipt
values. Existing null/default/unknown-field behavior remains intact, including
Serde's extra-field acceptance for the internally tagged `Starting` unit variant.
The fixed fixture's SHA-256 is
`b072665d4f9f2b800c20f5c0a9c5080843f0dce7f573bf9499eba545fc0f305b`.

The thirteen journal and thirteen command-component tests retain their names and
assertions, including the journal harness's exact self-executing test filter.
The native qualification test moved with its private implementation and remains
ignored with its original reason and macOS guard. Backend admission and actual
control-deadline coverage remain backend-owned.

## Review and verification

One bounded independent review confirmed exact journal lifecycle and supervision
algorithms after import/name substitutions, unchanged admission checks, retained
assertions, one-way dependencies and build coverage. A validation gap in the
factory-only test was resolved with a private-path regression verifying rendering
before the factory and its error before any native spawn. No production defect
or unresolved material finding remains.

Verification used pinned Rust/Cargo 1.99.1 through the verified `+stable` alias,
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache` and `--offline --locked` after
the offline lock update. Normal concurrency and exactly three native ignores remain.

| Check | Result |
| --- | --- |
| Pre-relocation compatibility capture | Passed; twenty fixed records |
| Workspace all-target compile | Passed after one missing moved-test import correction |
| Focused mechanism/backend/integration tests | 343 passed, zero failed; one existing native ignore |
| Formatting and strict workspace all-target Clippy | Passed |
| Contract inventory and its eleven tests | Passed; 519 blocks and 140 named requirements |
| Complete workspace release suite | 1,366 passed, zero failed, exactly three native ignores |
| Optimized arm64 executable and five minimal-environment inspections | Passed; consistent metadata and no runtime state created |

The initial all-target compile failed because the moved ignored native fixture
needed an explicit `command_launch_identity` import. It was corrected without
changing assertions; the original diagnostic is retained at
`/private/tmp/kegari-execution-lifecycle-initial-check.log`. No test assertion
failed. The complete release suite ran once on the reviewed final candidate;
no source changes or further broad reruns followed.

Results/log indexes:
`/private/tmp/kegari-execution-final-compatibility.json`,
`/private/tmp/kegari-execution-lifecycle-focused-results.json`,
`/private/tmp/kegari-execution-lifecycle-final-results.json`,
`/private/tmp/kegari-execution-lifecycle-test-results.json` and
`/private/tmp/kegari-execution-lifecycle-review.json`.
The additional factory regression passed at
`/private/tmp/kegari-execution-lifecycle-factory-regression.log`.

## Candidate and stopping point

Independent hash reconstruction verified 400 application inputs,
including moved command/journal modules and shared lease data. All
837 reviewed source/test/build inputs match the inspected candidate.
The executable is `target/release/kegari-v0`, arm64 Mach-O. Handover schema 5,
managed execution schema 5, configuration/API schema 1, process schema 3,
asset version 7 and pinned Git/Codex versions remain unchanged.

| Identity | SHA-256 |
| --- | --- |
| Reviewed source/test/build inputs | `f28c56e8991a5122257d3c3346a31ab0f3bb36be27e05e792a057edd649f31bc` |
| Application build | `bcdb6d6548017a73c1741ac0603cf7d117e9ec4e3b181d4cb448d46a137f9353` |
| Optimized executable | `682412b96b9b8505b7686bc1f4172f0e87dca9c85eb11106fbfd889b5f67ccd7` |

Inspection evidence: `/private/tmp/kegari-execution-lifecycle-inspection.json`.
The existing alpha.10 archive remains unchanged at SHA-256
`864a4796e238e0a2dfa7bf87b9d3078a5f5117395c12368ae5b9c89d55967d38`.
Concurrent README/workspace-index documentation edits were preserved.

The diff is uncommitted. This completes execution infrastructure extraction;
application orchestration and protected recovery remain backend-owned. Public
command qualification refusals and detached-descendant limitations remain in
force. No live providers/accounts, installed Codex, real Keychain, native
qualification, service activation, goal continuation, packaging, commits or
remote operations were used.
