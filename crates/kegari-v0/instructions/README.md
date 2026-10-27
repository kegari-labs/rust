# Runtime instruction and repository workflow contract

The installed executable embeds six role/authority assets and a versioned generic
workflow with their checked SHA-256 catalog. `instructions::assemble` is the production assembly entry point;
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
6. An explicitly selected repository WORKFLOW.md supplies additional guidance.
   If no workflow is selected, the installed generic workflow supplies discovery
   guidance without reading a file or inferring a cwd-relative selection. Neither
   source changes roles, accounts, scope or execution permission.

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

`examples/WORKFLOW.md` is an optional copy of the installed generic guidance.
Choose an explicit new destination through `instructions::export_starter` (which
uses create-new and cannot overwrite an existing file) if repository-specific
additions are useful. Omit `workspace.workflow` to use the installed default.
An explicit selected path must resolve to its actual bounded, valid workflow;
missing, unreadable or invalid selections never fall back. No file is guessed
from the working directory.

The default declares no executable catalog, check commands or hooks. It directs
the agent to discover relevant guidance, environment and validation from the
actual permitted task/repository context, including an empty or unborn repository.
This guidance does not establish an execution facility or missing source evidence.
The worker's `command_run` accepts literal `argv` and `cwd` through its admitted
generic boundary. It records actual source, environment and outcome observations;
it does not mint check evidence or authorize broader access. Results include
bounded output previews and protected history references, also available through
`context_read` with key `command_history`. Native qualification remains closed.
Discovered validation obligations, complete output paging and empty/unborn Git
runtime support remain separate implementation work. Empty configured check sets
are accepted as undiscovered inputs; dependent validation/delivery gates still
refuse missing obligations and do not count them as passes.

A file workflow records its actual canonical path and byte digest. The installed
default records an explicit embedded version/digest and has no filesystem path.
Worker binding and retained cleanup compare the original source identity; an
obsolete embedded version or a changed selected file cannot silently replace it.
Existing file-origin records retain their string path and omit the embedded field.
Handover metadata schema 4 explicitly requires `workflow_source_schema = 1` for
these config/instruction/cleanup formats. Missing or unsupported reader support
refuses replacement even if the advertised asset catalog has matching names.
Historical six-asset binaries also fail the existing asset-key compatibility gate;
that incidental mismatch is not the definition of workflow reader support.

Optional configured commands and artifact paths resolve from the isolated
execution/candidate cwd. An explicit relative workflow resource link is available
only for a selected file workflow; an embedded default has no filesystem resource
anchor. Result presence alone cannot establish execution, revision or freshness.

The front matter is a closed mapping: `schema_version: 1`, optional `hooks` and
optional `checks`. Hooks are explicit shell strings (`after_create`, `before_run`,
`after_run`, `before_remove`) plus positive `timeout_ms`. Each named
`checks.local.NAME` or `checks.integrated.NAME` contains `argv` (a string array),
`result_artifact` (a path string), and optional bounded `services` declarations.
These are optional inputs/obligations rather than a required command catalog.
Duplicate keys, implicit non-string argv values, unsupported fields and empty
prompt bodies fail.

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

Repository context may include concise entry points, architecture/conventions,
existing root/nested instructions and target-owned scripts/configuration. The
agent discovers relevant commands and environment from permitted evidence; the
operator does not need to maintain an executable inventory or command manifest.
An empty repository starts from the authorized task, without invented existing
files or revisions. Required inaccessible context remains a blocker.

Account identities, actors, repository/resource scope, roles and release policy
remain host-owned authorization inputs. Actual hosted proof, review, strict-base
integration, deployment and human acceptance gates remain required when their
operations apply. Some current configuration and delivery paths still require
predeclared hosted CI/no-change/deployment identities; removing discovery blockers
there requires a separate implementation preserving those evidence gates.
`config check` is read-only and must distinguish verified, unverified and blocked.
Neither generic instructions nor optional configured checks grant permissions or
qualification, and they cannot make a failed or unrun obligation pass.

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
