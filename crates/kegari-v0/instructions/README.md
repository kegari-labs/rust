# Runtime instruction and repository workflow contract

The installed executable embeds the six versioned role assets and their checked
SHA-256 catalog. `instructions::assemble` is the production assembly entry point;
it validates the catalog, role/tools/permissions and exact profile, discovers
allowed guidance, renders the selected workflow, and returns the prompt plus an
instruction manifest. The runtime never reads these generic assets from the build
checkout. Changes require a newly built, independently qualified binary and the
explicit coordinator handover procedure in SPEC §11.7.

Layers, in assembly order:

1. `authority.md` supplies immutable authority, scope, noninteractive and evidence
   rules. It grants no capability; the broker and sandbox enforce the boundary.
2. `child.md`, `coordination.md`, `review.md` or `validation.md` supplies the admitted
   role. Implementation, testing and documentation share child guidance;
   independent PR review uses review guidance with no source writes/code execution.
3. `continuation.md` supplies correction, bounded continuation and recovery rules.
4. The operation boundary binds identities, generations, profile, cwd, permissions
   and tools. Caller-supplied context must match the admitted snapshot.
5. Accessible Codex-home and repository `AGENTS.override.md`/`AGENTS.md`, scoped
   instruction chains and explicitly required context are supplied with path,
   scope and byte digests. Missing, inaccessible, oversized or secret-bearing
   required input blocks assembly. No target guidance is rewritten.
6. The explicitly configured repository WORKFLOW.md supplies target-specific
   context and validation instructions; it cannot change roles, accounts or scope.

Supplied files and provider-confirmed discovery are separate manifest fields.
Deterministic discovery tests establish what Kegari supplies, not what an actual
provider loaded internally. The pinned profile must independently validate its
configured home, instruction limits, cwd/discovery and review operation. Provider
context that was not observed remains `unavailable`. The implementation does not
inspect Codex internal storage. Scoped guidance for later context reads belongs
with the actual turn/revision in `IncrementalInput`; a new generation or
incompatible effective guidance requires reconciliation and fresh admission.

For native `review/start`, the adapter must first create a thread with the
assembled review guidance and exact revision context, then use the supported
review operation. Its result is a proposal, not a formal self-approval or a host
attestation. The host publishes the authenticated status after gate verification.
A successful assembly call alone does not establish native-review compatibility.

## Selecting and adapting the starter

`examples/WORKFLOW.md` is a valid example that renders through production code.
Choose an explicit new destination through `instructions::export_starter` (which
uses create-new and cannot overwrite an existing file), then configure its
absolute path. There is no cwd or starter fallback. Editing/copying the starter
does not dispatch work, grant access or select a runtime profile.

The shipped commands `./scripts/validate-local` and `./scripts/validate-integrated`
are example target-owned commands. Supply/adapt them, their named local/integrated
suites and current-run JSON result artifacts to the actual repository. Their argv,
shell references and artifact paths resolve from the isolated execution/candidate
cwd. Only an explicitly identified workflow resource link resolves beside the
workflow. The loader never searches the workflow directory for a missing command
or artifact. Result presence alone is insufficient: the host must verify actual
execution, operation, revision and freshness before accepting it.

The front matter is a closed mapping: `schema_version: 1`, optional `hooks` and
optional `checks`. Hooks are explicit shell strings (`after_create`, `before_run`,
`after_run`, `before_remove`) plus positive `timeout_ms`. Each named
`checks.local.NAME` or `checks.integrated.NAME` contains exactly `argv` (a string
array) and `result_artifact` (a path string). Duplicate keys, implicit non-string
argv values, unsupported fields and empty prompt bodies fail.

Templates support `{{ path }}`, `if`/`elsif`/`else`, `for`/`else`, `assign`, literal
array-index/map-key paths, comparisons and logical conditions. Supported filters
are `json`, `escape`, `default`, `join`, `size`, `upcase`, `downcase` and `strip`.
Unknown variables never disappear, including before `default`. Optional fields
must exist as null/empty values. Use `json` for maps/arrays; implicit stringification
is rejected. Templates are bounded to 64 nesting levels, 10,000 total loop
iterations and 8 MiB output; complete operation input has its own stricter bound.

A reference input is provided by `tests/config_harness.rs::AssemblyFixture`: all
thirteen documented context keys exist; `workspace.id` and `operation.id` match
the admitted UUIDs; `profile` is the exact validated profile; `phase` matches the
operation. The fixture model and paths are synthetic and establish no real account
availability. Run `cargo test -p kegari-v0 --locked --test config_harness` to exercise
starter rendering, actual assembly, strict parsing and conflicting cwd fixtures.

## Target preparation before dispatch

The operator must provide a concise repository entry point linking necessary
architecture/conventions; existing root/nested instructions; complete build,
test and lint commands; tools/environment prerequisites; allowed read repositories
and explicit test data/resources; hosted CI workflow/check identities; required
reviews and strict current-base squash protections; an exact-SHA no-change CI
route; and the configured deployment step DAG or explicit `deployment = "none"`.
Account IDs, actor allowlists, resource scopes, child routing labels and manual vs
automatic release policy belong to host configuration/Linear, never front matter.
`config check` must verify these prerequisites read-only and distinguish verified,
unverified and blocked. Generic instructions and example values supply none of
those permissions or qualification evidence.

Prepare one parent and scoped children in Linear before authorization. Include
outcome, relevant context, constraints, dependencies, acceptance criteria and
validation expectations, plus exactly one fenced `kegari` JSON metadata block:

```kegari
{"schema_version":1,"kind":"parent","required_children":["<child-UUID>"],"e2e_child":"<child-UUID>","visual":false,"production_targets":["<configured-target-ID>"]}
```

```kegari
{"schema_version":1,"kind":"child","responsibility":"implementation","write_repository":"<stable-GitHub-repository-ID>","write_paths":["src/","tests/"],"read_repositories":[],"test_resources":[]}
```

Testing/documentation children use their actual responsibility. A genuinely
read-only child uses `write_repository: null` and `write_paths: []`; private
scratch does not grant source writes. Use explicit relative files/directory
prefixes, or `.` deliberately for the assigned repository; never shell globs.
Include the E2E child and required documentation responsibility. Inaccessible
required children/context stay blocked rather than disappearing from completeness.

Every guidance/assembly/tool/profile change needs affected deterministic and
actual-agent evaluation coverage before qualification. Maintain the digest catalog
when deliberately changing generic assets, preserve earlier failed reports, and
obtain the required human rubric review; a new prompt or green unit test is not
operational qualification.
