# Explicit native evaluation caller: local validation

The final source passed **863 Rust tests, zero failures and two ignored
required native probes across 39 groups** in 835.77s. All-target
compilation, strict Clippy, formatting, contract inventory and diff checks passed.
The separate source-preparation tests passed all 13 cases. Production Plan
accepted both the shipped and prepared suites with 56 declared slots and
`dispatch_authorized=false`. These checks establish local component behavior;
actual-agent and native operational qualification remain incomplete.

All 672 build inputs remained unchanged during final checks at
`sha256:6f962a238f28a5c05a6fad1fc4f90226d4d638512fee109b782114bb6e2e0f8e`.
[Machine evidence](native-evaluation-checks.json) retains 35 invocations,
exact source snapshots, command lines, Rust/Cargo identities, log hashes, earlier
failures and the one intentional formatter transition. SPEC 1.2 remains at its
approved SHA-256. [Coverage](coverage.json) appends partial evidence to 18 clauses
and eight named requirements; it declares no requirement passed. Every one of the
34 mapped test names was matched to a passing line in the final full-suite log.

Commands ran in the Rust workspace with `RUSTUP_TOOLCHAIN=stable` and
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`:

| Command | Observed result |
| --- | --- |
| `cargo check --locked -p kegari-v0 --all-targets` | Pass; 14.425s |
| `cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings` | Pass; 14.666s |
| `cargo test --locked -p kegari-v0` | 863 passed; two ignored; 835.77s |
| `cargo fmt --all --check` | Pass |
| `cargo run --locked -p kegari-v0 --bin kegari-v0-evaluation -- plan crates/kegari-v0/evaluations/manifest.json` | 56 slots; no dispatch authority |
| Same Plan command with the prepared `suite/manifest.json` | 56 slots; no dispatch authority |
| `python3 crates/kegari-v0/tools/contract_inventory.py --check` | 496 clauses; 136 named requirements; no implicit passes |
| `git diff --check` | Pass |

The Python component checks used the actual shipped suite explicitly:

```sh
KEGARI_TEST_NATIVE_SUITE=/Users/lgr/Development/com.kegari/com.kegari.rust/crates/kegari-v0/evaluations \
  python3 -B -m unittest discover \
    -s crates/kegari-v0/evaluations/tools -p test_prepare_native_source.py -v
```

The tests and tool invocation ran with bytecode output disabled, preserving all
captured suite bytes. Actual local-Git positives used the invoking human's
configured identity and disposable owned directories. No fixture program, hook,
malicious payload or live model was executed.

## Delivered behavior and evidence boundaries

The explicit local command uses the existing authenticated runtime, Control owner,
account limits and worker preparation. It accepts only declared trial identifiers
and observed configuration/control revisions. It cannot supply a new profile,
lease, permit, arbitrary source path or larger budget. The API route requires a
Unix peer and the `evaluate` role. CLI and route tests exercise real parsing and
routing with injected authenticated connection metadata and an Echo dispatcher;
they do not establish an authenticated native trial over an actual Unix socket.
Reserved synthetic provider transport cannot fall back to external HTTP. Inactive
namespaces expose no PR; genuine source admission is required before activation.
Ordinary startup/workflow paths exclude these explicit evaluation workspaces by
source inspection. The new route tests alone do not prove every no-auto-dispatch
scenario required by V0-T37.

The complete declared case/variant/profile/repetition plan assigns distinct PR
ordinals across all 56 slots. Actual durable command and metadata receipts anchor
the protected plan against removal, replacement and slot replay. No plan JSON or
retained report creates execution authority. The three reservation regressions
prove identity/receipt consistency; the conclusion that slots do not reset
ordinary phase budgets also relies on inspection of the unchanged admission and
budget paths. It is not inferred solely from these three tests.

One monotonic execution deadline spans source inspection, preparation, model work
and final collection. Actual Tokio task-handle tests preserve ownership after a
waiter disappears and join once. Deadline settlement fences once and retains
cleanup. Original preparation/model admission identities and actual returned
completion owners survive receipt-read, persistence and cleanup failures. Later
owners still drain if an earlier owner fails. Actual Control/private-storage
regressions exercise the receipt paths; plain ownership-slot and report tests
remain data components. None of these constructs a model-descendant exit proof.
Native cleanup still requires the actual execution registry, provider/process
receipts and original runtime capabilities; an uncertain owner remains held.

Independent PR metadata and literal title/body come from the same bounded wire
response. Missing fields refuse, null and empty bodies differ, and reobservation
compares complete text without changing review identity or allocating another
budget. Protected worker provenance retains it; general status omits raw text.
Injected transport/projection/context tests establish these boundaries. They do
not qualify real GitHub behavior or resistance by an actual agent to adversarial
text. The E05 fixture attacks root repository guidance; its benign PR body is not
substituted for the separate E07 PR-body attack.

Scorer staging preserves exact declared bytes in private host storage before
fixture preparation. File replacement and linked/substituted inputs refuse. Real
scorer-load and grading failures retain already-collected counters, provenance,
observations and exact errors. Completed execution without a grade becomes blocked;
failed/interrupted execution remains distinct. Absent effects are unknown, while
an actual empty observation is retained as empty. Persistence, inspection,
assessment and comparison cannot relabel an error as passed or erase it after a
later success. These are filesystem/grading/report checks, not native-agent
scorer-isolation canaries or human calibration.

The first E05 fixture uses the full production context shape, `independent_review`,
`native_independent`, nine declared scalar identity slots and literal benign PR
text. Actual fixture capture and complete context matching reject substitutions
and an absent PR subject. Test host values are explicitly data-only. The manifest
retains all 28 variants, two repetitions each, and the unresolved profile.
Other roles, contexts, continuation seeds and legacy state predicates still need
native observation/fixture migration; they were not forced through the first
independent-review caller.

## Reproducible local source data

The delivered [source-preparation tool](../evaluations/docs/prepare-native-source.md)
captures bounded regular-file bytes/modes, rejects links/special/Git metadata
inputs, preserves the declared profiles and creates two signed-off local commits
inside an absent protected owned output. Enumeration reserves the global entry
bound before descending and before sorting; final verification repeats that bound.
Output creation uses retained descriptors and protected ancestry with identity
checks around later path operations. This does not claim resistance to arbitrary
concurrent same-user manipulation.

The retained preparation is
`/private/tmp/kegari-v0-native-evaluation-source-output-14/prepared`:

- Base `f27eea6d92b3a6e7cb6973c5e389fedd225c31d5`:
  `test(evaluation): seed inclusive boundary review base`.
- Head `2b2b21aca2c287e5f89a35100fb3aa5c3fec1d06`:
  `test(evaluation): introduce boundary defect for independent review`.
- Suite `sha256:v1:f6a8c85452a95fce1568d05f0a9a18d2d35c852bd828eefcc25d7aa46c5e5a03`.

Only `src/limit.py` changes from `<=` to `<` between those commits. Both use the
configured human signoff and `Assisted-by: Codex`, with the disclosed fixed fixture
time `2026-10-26T00:00:00+00:00`. This time is not a freshness claim and signoff is
not a cryptographic signature. The copied context's two base fields and derived
fixture digest bind the actual objects. Command records, object/tree IDs, modes,
all 348 input/copied-suite files and generated source data are preserved in
`SOURCE-EVIDENCE.json` and included in machine evidence. Their retained bytes and
modes were independently rechecked after Plan without running Git or fixture code.
The production Plan validated this derived suite; no native source admission was
attempted. The test repositories cleaned up their own data; the retained output
above remains available for inspection.

## Findings, corrections and remaining obligations

Root read the production/test compositions and merge deltas; independent agents
reviewed caller/subject ownership, literal PR text, report retention, context
matching, admission-identity retry, per-owner draining, source preparation and
coverage limits. Receipt-read retry originally lacked a stable expected identity;
it now derives that identity from the original actual permit/reference before the
fallible read. Cleanup originally skipped later owners after an earlier error;
it now visits all retained owners and preserves the first error. Source-tool v1
could allocate an unbounded directory list and did not adequately protect its
output parent; v2 bounds enumeration and protects descriptor-based creation and
ancestry. Independent rereviews found no remaining material defect in these scopes.

Earlier failed invocations remain evidence. Initial Clippy reported a nested `if`
and one-element provider loop. A new report test used a symlink spelling of the
macOS temporary directory; root canonicalized its actual owned directory. Another
new comparison test selected row zero rather than its exact TrialKey; the lookup
was corrected without removing assertions. The owned-return composition first
failed compilation on two test-only nonexistent `PrivateDir::clone` calls; root
reopened the same protected directory. Subsequent lint findings were fixed by
deriving Default, preserving the existing Box on error, simplifying equivalent
borrowing/conditions and moving a helper. No suppressions or relaxed controls were
used. Source13's formatter changed only one exact-key test assertion's layout.
Each historical check retains its actual source identity; the final full suite
covers the installed corrections together. Evidence review also found that an
earlier collector draft classified successful checks by filename label without
matching each command. The corrected collector requires all ten exact final
command lines and log paths, the announced source identity and both named native
ignores. Independent rereview found no further material verification gap; the
unexecuted draft and corrected review are retained.

The two required native probes remain ignored. Repeated actual-agent trials,
real-provider/account compatibility, OS containment, actual model/process exit,
native failure cleanup and mandatory human qualitative review/calibration remain
unqualified. The run remains unattended. No live evaluation, remote mutation,
persistent installation, deployment, account/profile choice or previously denied
native probe was attempted. The caller/operator examples authorize none of those
steps. See [operator prerequisites](../docs/evaluation-run.md) and the retained
blocker plans in [PROGRESS](../PROGRESS.md). The Goal is incomplete.
