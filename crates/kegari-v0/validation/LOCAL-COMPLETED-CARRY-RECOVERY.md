# Completed-current carried acceptance recovery

The final source passed **825 tests, zero failures and 2
ignored native checks across 39 groups** in
843.114s. All-target check, strict Clippy,
formatting, inventory, diff and a dispatch-disabled evaluation Plan also passed.
The required native probes remain incomplete. These results establish local
deterministic behavior, not a qualified native cold-carry workflow.

All 653 build inputs stayed unchanged at
`sha256:d0288da1dcd547c8a670373630ebad3542c8ff2aed57b63576e5ddb6d62db46b`.
[Machine evidence](completed-current-carry-checks.json) retains 15
invocations with complete source lists, command lines, tool identities and log
hashes, including the initial compile and inventory-format failures. Rust/Cargo 1.99.1 identities and
all source bytes were checked before and after each invocation. SPEC 1.2 remains
unchanged. The five carry clauses and V0-T17 map to partial evidence from the
[previous carried-acceptance checkpoint](LOCAL-CARRIED-ACCEPTANCE.md), with native
integration explicitly not run; the ledger does not declare those requirements passed.

Commands ran in the Rust workspace with `RUSTUP_TOOLCHAIN=stable` and
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`:

| Command | Result |
| --- | --- |
| `cargo check --locked -p kegari-v0 --all-targets` | Pass, 17.007s |
| `cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings` | Pass, 18.249s |
| `cargo test --locked -p kegari-v0` | 825 passed; 2 ignored; 843.114s |
| `cargo fmt --check -p kegari-v0` | Pass |
| `cargo run --locked -p kegari-v0 --bin kegari-v0-evaluation -- plan crates/kegari-v0/evaluations/manifest.json` | 56 trials planned; dispatch unauthorized |
| `python3 crates/kegari-v0/tools/contract_inventory.py --check` | 496 blocks; 136 named requirements; no implicit passes |
| `git diff --check` | Pass |

Six new component checks also passed separately on this exact source: three
recovery-selection/storage tests, two integration-inventory tests and one actual
owned import-history test. They are included in the full suite, not added to its
count. The earlier three recovery tests/check/Clippy passed on source2 before the
additional integration-history layer; that evidence retains its own source identity.

The live carry writer now retains default-omitted recovery data naming the exact
whole final integrated/review receipts, configuration and full production-owner
selection. The protected current workspace/token reader authenticates the original
human acceptance, original/current parent subjects, carry publication, complete
phase/grant inventory and every selected whole receipt. Legacy signed bytes remain
unchanged when optional data is absent; legacy absence cannot select a new current
carry or fabricate original worker results.

Recovery opens an actual scoped read-only metadata owner and independently
reobserves current parent/vector/child/integrated/production and native publication
facts. The pending result cannot yield a recovered current carry until the actual
read owner returns its acknowledged receipt with joined handles and the exact
successful subject digest. Current configuration/gate and the full retained closure
are checked again inside the final serialized Control callback before insertion.
Failures retire the real read with verified=false where possible; unresolved local
ownership remains held. No reconstructed assignment, RunCompletion, check capability
or joined runtime-cache entry is created.

For already integrated contributions, the same actual known-predecessor/scoped
Git reader derives genuine production integration objects. The complete signed
candidate inventory must match exactly by original release and canonical repository;
missing, extra, duplicate or substituted records refuse recovery. Retained history
keeps the real predecessor and original imported source, protected marker/intent/
object inventory, exact successful import and closed original owner receipts. That
history is revalidated after awaited reads, at read retirement and on later use.
It supplies immutable provenance only; fresh current or actual pinned active gates
remain separate requirements. There is no fallback import, new model admission,
budget renewal or replacement human acceptance.

The import-history regression performs the existing owned disposable Git/protocol
import through real Control admission/effect/join, using injected offline provider
and boot observations. It first rejects an open owner, then uses the actual closed
owner and rejects missing/changed protected source/intent/inventory/receipt bytes
and changed quarantine contents. It checks unchanged Control/process inventories
and wire counters after restoration, including protected-only validation inside a
real serialized callback. This qualifies that component, not native containment or
a completed carry. Other new tests are explicitly data/storage predicates and
cannot manufacture opaque execution or carry capabilities.

Root reviewed the complete production/test patches and actual producer boundaries.
A second independent review found no material defect in exact closure retention,
inventory matching, per-use validation or lock ordering. Source1 check failed in
13.182s because the existing parent grant observer was private to a sibling module.
Root expanded only that method's visibility to the common runtime_delivery ancestor;
its body is unchanged and the correction was independently reviewed. Source2 check,
strict Clippy and all three initial tests passed. Source3 adds the independently
reviewed integration-history layer and passes the checks above. Original files and
failed invocation evidence remain retained; no assertion or authority boundary was
weakened to obtain these results.

The first final inventory check rejected Unicode escape spelling in the coverage
ledger. Restoring the original UTF-8 formatting changed no parsed JSON data.
Inventory4 and diff4 passed afterward on the unchanged source3; both ledger byte
versions and the failed invocation remain retained in the machine evidence.

Native positive qualification still requires an actual original acceptance, genuine
completed qualified reassembly/integrated/review/production validation, actual live
carry publication and a full current cold reobservation. Visual artifact hydration,
cold predecessor-worker reconstruction, unfinished publication recovery, legacy
selection inference and a changed saved configuration remain incomplete/refused.
The two required native probes remain ignored. The separate evaluator draft,
repeated actual-agent behavior evaluations, real provider qualification and human
calibration remain incomplete. No live trial, remote mutation, installation,
deployment or previously denied native probe was attempted. The Goal is incomplete.
