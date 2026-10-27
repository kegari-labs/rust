# Local discovered child-validation increment

Partial implementation and deterministic evidence for SPEC 1.4,
0.2.0 / M02-2–M02-4. Baseline: `9b5a3c1cae3069cac1564b4872ef2d01b529a6e1`
on `main`. Package remains development `0.2.0-alpha.9`; instruction assets are
version 5. No minor milestone, native qualification or actual-agent trial is
complete. This increment adds no toolset-specific interpretation or user-authored
environment/command inventory prerequisite.

## Implemented boundary

Actual child-local workers retain successful ordinary source/context reads only
after their protected tool completion is persisted. `validation_propose` enrolls
bounded obligations against those observations, the complete clean admitted
source, original ownership and actual effective correction input. References are
rechecked against the original journal and current bytes. Plans retain prior IDs,
reasons and predecessor history; revisions cannot silently discard obligations.
Optional repository-selected configured checks remain constraints. Unselected
workflow definitions do not become mandatory.

`validation_run` consumes the current opaque plan and an obligation ID. The
executor observes literal argv/cwd/environment and main/service executables,
retains original authority and deadline, and requires actual command/service
closure before taking the resulting complete source snapshot. It excludes no
source paths. Ordinary stdout-only commands need no report wrapper: the host
creates a fresh transcript after closure. Optional artifacts must be in admitted
scratch. Records use schema 4; descriptive records cannot reconstruct execution
or enrollment capabilities. Legacy check serialization remains unchanged.

Discovered check events share the existing bounded protected chunk journal;
compact tool replies include bounded output previews, full byte counts/digests
and the complete record reference. Original registry holds survive until both
Control acknowledgment and the validation journal acknowledgment settle.
Uncertain execution, failed service prefixes and incomplete persistence remain
sticky. Completed records survive even when no repair capability can be issued.

Both real worker completion bridges retain the current plan, all completed
attempts and uncertainty. An opaque original-worker history seal binds that full
inventory; the protected worker outcome binds the seal digest. Delivery checks
both before choosing a validation path. Pruning a later failure, substituting a
plan or clearing all discovery metadata cannot expose an earlier pass.

Child delivery accepts mechanically complete current-plan results without a
configured check list. A separate read-only review receives full bounded plan
history and attempts, plus host-computed references it can cite without executing
code. Its schema-2 assessment must address every obligation, exact current result
and overall sufficiency. Staging and final child publication require that exact
adequacy assessment alongside the existing hosted/review/delivery gates. Signed
readback enforces the same schema, current results and optional configured checks;
legacy review does not implicitly qualify discovered validation.

## Review and corrections

Independent executor review covered original plans/ownership, full source
identity, cwd derivation, executable observations, deadlines, service ordering,
writer closure, artifacts and uncertain results. Separate worker/core and
delivery/evidence reviews found and resolved:

- Removing a pending plan before its settlement write could lose its unresolved
  ownership. Pending enrollment and plan state now survive failed persistence or
  dispatcher cancellation.
- Complete attempt history cannot be inferred from successful/repair capabilities.
  All actual records and sticky uncertainty now accompany completion.
- Public completion vectors could omit a later failure. The original history seal
  and protected outcome marker now reject pruning and legacy downgrade.
- Early selected-check lookup needed the actual nested delivery-assignment
  context. It now uses that envelope and ignores unrelated top-level shadows.
- Generic bundle cleanup could prune a settlement hold before the validation
  acknowledgment. It now retains the original hold until settlement.
- Reviewers needed actual attempt digests, and executing agents needed bounded
  output previews. Both are host-produced from retained observations.

The implementation authors did not count their own work as independent review.
These static reviews and deterministic fixtures do not substitute for native,
actual-agent or human qualification.

## Retained limits and next work

This is a bounded child-local slice. Parent integrated discovery still uses the
transitional named-check contract. Cross-worker discovered-plan continuation,
repair and cold recovery explicitly refuse where the original history cannot be
preserved; no decoded record becomes a live capability. Those paths remain work,
not completed recovery.

Complete validation evidence is currently carried inline. It has an 8 MiB
evidence bound, with stricter instruction-assembly and publication bounds. New
publication preflight refuses a body above 60 KiB or a private record above
256 KiB before the affected publication; it does not truncate evidence. Compact
signed summaries with retained full-history access remain needed for practical
large repositories/results. Full output paging also remains open.

Generic environment admission, empty/unborn source baselines, complete parent and
continuation migration, wider discovery fixtures and actual-agent adequacy
evaluation remain incomplete. Native execution still refuses the unqualified
managed profile. Existing native approvals are consumed; unattended resumption
authorizes no additional host/provider/live action.

## Validation record

All Cargo commands use installed stable Rust/Cargo 1.99.1,
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`, `--offline --locked`.
Ignored native tests are not enabled. Disposable Git fixtures are owned local
development tests; no provider/model session or native sandbox qualification runs.

Preliminary integration compile logs are retained as
`/private/tmp/kegari-discovery-check1.log` through `check3.log`: temporary parse
errors, a borrowed-enum move, a temporary-lifetime error and missing new fixture
fields were corrected. The first focused engine run passed seven cases
(`focused1.log`). The broader focused run passed 51 and failed one new assertion
that read the wrong internally tagged JSON path (`focused2.log`); the assertion
now checks the actual `state: finished` shape and unchanged output bytes.

Initial source capture and the completed formatter output are both retained as
`/private/tmp/kegari-discovery-source1.json` and
`/private/tmp/kegari-discovery-source1-formatted.json`; these preliminary runs are
not claimed as final-source regression evidence. Source 2 is
`sha256:341a6eb86467e15e8fd0c3e62396e149a6f30d9fd11ade82eb3d0779620b3067`.
Its first strict lint found duplicate loading of the Git fixture helper and a
large journal enum variant. The helper now loads once per test crate, and the
large optional provenance is boxed without changing JSON encoding. No assertion
was removed or warning suppressed. A role test additionally rejects the new
proposal capability for coordination/review roles.

Strict all-target Clippy passed on source 3, followed by the complete release
regression: **1,185 passed, zero failed, three native ignored**, 40 result groups.
All 738 inputs matched after testing:
`sha256:a9a22171ef5f703a0265afd6782627e6632c7698027eda63579e118ea74a4a09`
(`/private/tmp/kegari-discovery-source3.json`). Logs are
`/private/tmp/kegari-discovery-clippy2.log` and
`/private/tmp/kegari-discovery-tests1.log`. The 32 new cases comprise six executor,
three enrollment, fifteen worker and eight delivery/evidence tests. Existing
applicable regressions remain intact.

The final source differs only in `instructions/README.md`, correcting the status
of the newly implemented child-local path. No runtime instruction asset, Rust
source or test changed after the full regression. This documentation is a build
input, so its exact identity is separately recorded rather than claiming another
full run:
`sha256:1a985269882a349933b0ca583611c2d8119ec388959896b31f993b398d1b0c04`
(`/private/tmp/kegari-discovery-source4.json`).

The release-profile development build passed on that final source. Static
`__build-info` and `__handover-info` inspection succeeded, and independently
recomputing all 340 build inputs for `aarch64-apple-darwin` / release matched the
actual executable. All final logical inputs remained unchanged. Formatting,
whitespace and inventory consistency passed; inventory tests passed 11. All
Cargo/test handles are terminal.

- Executable: `target/release/kegari-v0` (development, no new release package).
- SHA-256: `b4195c51319083275357fdd9800014e7799024340b7c9b6940a863e80af2a4dd`.
- Build identity: `sha256:31202830790a08e50130ec998a793f5ca9e6948169cf554a8a963963c443fa70`.
- Metadata: handover 4, managed 5, process 3, workflow-source 1, assets 5.
- Evidence archive: `target/validation/m02-discovered-1a985269882a`, 188 files
  plus manifest SHA-256
  `a09fe9066301ce32c6fef3b507fb3780c9b894f7b6ac1e7f86c072ff4f69b9dc`.

The archive retains failed/successful logs, source manifests and exact changed
inputs for each captured checkpoint, the final source patch, test totals and
build metadata/recomputation. Initial preformat and formatted enrollment source
are both preserved. It contains no manufactured actual-agent/native evidence.

Commands, with the Cargo cache/offline/locked settings above:

- `cargo +stable clippy -p kegari-v0 --all-targets -- -D warnings`
- `cargo +stable test -p kegari-v0 --release --no-fail-fast`
- `cargo +stable build -p kegari-v0 --release --bin kegari-v0`
- `cargo +stable fmt --all -- --check`
- `python3 crates/kegari-v0/tools/contract_inventory.py --check`
- `python3 -m unittest discover -s crates/kegari-v0/tools -p 'test_contract_inventory.py'`

Four §7.6 coverage entries gain partial deterministic evidence. No named behavior,
real integration, native or human qualification is promoted. This increment stays
separate from the previous generic-invocation commit because it adds obligation
and adequacy semantics, rather than nonqualifying execution observations. No
history consolidation, push, installation or activation is performed.
