# Managed resource binding — alpha.8

This increment adds bounded SDK/library content identity to managed execution.
It is a prerequisite for the offline Rust/Cargo profile; public command admission
still returns `managed_profile_unqualified`. The final exact-source release suite
passed **1,117 tests, zero failures, three native ignores, across 40 groups**.
All-target clippy, formatting and the 503-clause/136-requirement inventory pass.
No separately gated native test or qualification grant is included.

| Area | Implemented behavior |
| --- | --- |
| Configuration | Optional content digest and explicit resource-relative mappings; omitted content preserves prior encoding |
| Observation | Streamed file hashes through no-follow descriptors; original filesystem identities, directories and closed internal links retained |
| Bounds | Shared policy limits for entries, file bytes, manifest text, mappings and observation duration; depth/path/link traversal bounded |
| Isolation | Original explicit resource read root retained; only enumerated regular files gain library mapping, with no added process execution or model-visible read grant |
| Preparation and admission | Full content observations outside Control; serialized callbacks reconstruct retained bindings without SDK reads |
| Evidence | Changed inputs prevent accepted check evidence and leave hook state unknown; cleanup checks content before its host effect gate |
| Compatibility | Content-bearing managed records use schema 3; old schema 1/2 records preserve their encoding; handover explicitly requires managed schema 3 |

See [configuration](../docs/configuration.md#managed-repository-execution-policy)
for manifest semantics and finite limits. This is a pre/post observation, not an
immutable OS snapshot. A concurrent trusted-host update can invalidate it.
Read grants remain scoped to the configured roots; the workload cannot write
those roots. File-by-file read rules were deliberately avoided because a real
SDK's policy could exceed process argument limits. Mapping grants stay explicit.

Independent review corrected two actual edge cases before the first compile:
ordinary names sorting before the synthetic `.` root were refused, and a decoded
duplicate root could reach an invalid parent slice. The manifest now places its
root first and rejects any additional synthetic root. Regression cases include
punctuation names and malformed retained snapshots. Capture and retained-binding
validation share policy-wide bounds; per-root validation cannot reset the budget.

Call-path review kept full SDK reads out of Control callbacks, including fresh
preparation predicates and cleanup. It also found an existing source-write final
callback that ran a Git revision probe under the mutation gate. That callback now
uses the same authority/ownership/isolation validation as the initial write,
retaining canonical path and expected-content comparison immediately before
replacement. The source-write regression uses unavailable Git and still checks
successful scoped writes, conflicts and forbidden paths. Check admission and
evidence retain their independent Git revision observations.

New tests cover byte changes, new/deleted files, byte-identical replacement,
closed links and cycles, exact mappings, finite bounds, serialization and forged
records, omitted-field compatibility, worker-policy binding and handover refusal.
Production LocalExecutor regressions inject a successful command that changes a
bound library: the actual persisted check becomes uncertain and the hook remains
started/unknown. Those injected command cases establish deterministic enforcement,
not native sandbox behavior.

The initial full run stopped after 19 groups with 858 passes, two new execution
fixture failures and one native ignore. The fixture had put the host Git path
under managed executable validation; its ancestry failed `unsafe_storage`.
Using the fixture's existing protected Git copy corrected the test setup without
weakening executable protection or any uncertainty assertion. Execution then
passed 20 tests and the remaining integration targets passed 257, with two native
ignores. Independent review confirmed the correction.

Initial clippy found that the added policy data enlarged a private delivery-source
enum, plus a range-pattern style issue. Boxing only that enum's in-memory
isolation field and using the equivalent schema range corrected both. No persisted
encoding or ownership check changed. After independent review, clippy and the
full final-source suite passed. The original failure logs and pre-correction
source remain retained; injected panic/uncertainty diagnostics are not silently
removed. [Machine evidence](managed-resource-checks.json) records every attempt.

The exact release executable is:

`target/prereleases/kegari-v0-0.2.0-alpha.8-aarch64-apple-darwin-7c2ca3de037b/kegari-v0`

- SHA-256: `7c2ca3de037b098efdd5c22b979237d3b9ae84d8386b260b45bea35d1aea8fae`
- Build identity: `sha256:bd82ae2e770562550c846b5af5e9e3b343759c4ec80ab488b6fa86c41adc07dc`
- 41,259,376 bytes, mode 0555, release profile, aarch64-apple-darwin.
- Rust/Cargo 1.99.1; 326 build inputs and 724 final test inputs are recorded.

Five staged-executable inspections passed: help, version, setup help, build
metadata and handover metadata. They used a minimal environment, closed stdin
and an empty disposable cwd, with zero exit codes, empty stderr and no cwd writes.
Embedded instruction asset hashes match the manifest. This is a development
artifact, not an installation, activation or whole-binary reproducibility claim.

An independent final evidence audit verified 1,077 referenced hashes, exact
membership of 326 build inputs and 724 final test inputs, and the actual 40-target
test inventory. It reproduced the binary/build identities, reconciled original
and archived logs plus source corrections, and checked metadata, all six
instruction assets and the documentation's scope. No material mismatch was found.

The prior [NQ-02C pass](LOADER-BOOTSTRAP.md) remains exact-source alpha.7 evidence
for three fixed foreground cases. Its approval is consumed; it is not a native
test of this resource-bearing profile. NQ-01B remains separately pending.

Remaining work includes the closed shipped support/case manifest, complete
vendored dependencies, protected Cargo configuration, explicit compiler/linker/
SDK environment bound into launch identity, private writable state, generated
code/doctest paths and separately approved native compatibility cases. Services
still need scoped network support and native qualification. The later 0.3–0.7
milestones and full v0 evidence obligations remain unchanged.
