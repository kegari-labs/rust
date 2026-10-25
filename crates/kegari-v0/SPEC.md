# Kegari v0 specification

Version: 1.2 — approved scope, harness deliverables and evaluation requirements retained; workflow-path and provider-intervention clarifications approved on 2026-10-25.

This is the implementation contract for the temporary `kegari-v0` Rust executable. It includes the complete headless parent/subissue delivery workflow needed to coordinate the lasting implementation after independent validation. It is not an instruction to begin that build or permission to perform remote operations.

This document adapts the OpenAI Symphony Service Specification. It is changed material, not an unmodified Symphony specification or a claim of Symphony conformance. This adapted document is distributed under Apache-2.0; the upstream NOTICE and complete license are reproduced in §15. Original Rust code remains subject to the repository's existing MIT `LICENSE.md`, copyright Luke Gareth Ribchester. This document does not change that license or license subsequently reused code automatically.

## 1. Purpose, authority and conformance [V0-01]

### 1.1 Baselines and precedence

The following local sources were read completely during the initial reconciliation. All three repositories were clean at the revisions below; no fetch, update or alternative baseline was used. Subsequent approved amendments retain these baselines; their requirements and clarifications are identified in §13.

| Source | Revision / location |
| --- | --- |
| Generic OpenAI Symphony specification, README, LICENSE, NOTICE | `be10a1b79df723d6d7612b5651c8522704dafb2e`; sibling repository `com.kegari.symphony` |
| Kegari discovery collection | `47bdc7c33672bc44e736f96386e4e4461f2a2023`; all nine Markdown files in `com.kegari.specification` |
| Deliberate Rust scaffold inspected read-only | `7c2303b49fcf8474939fe7caa04cba557e9450d9`; `com.kegari.rust` |

The discovery files are `README.md`, `workflow.md`, `workspaces-and-execution.md`, `git-and-releases.md`, `dashboard-and-controls.md`, `technical-direction.md`, `acceptance-scenarios.md`, `decisions-and-open-questions.md` and `repository-map.md`. The exhaustive audit and deferred-work ledger belong to the specification reconciliation conversation, not this build contract. No Elixir implementation evidence was required or used to establish requirements.

Product precedence is: the operator's confirmed decisions in this reconciliation, confirmed applicable discovery requirements, then compatible generic Symphony behavior. Recommendations and examples do not override those decisions. This document resolves applicable mechanisms explicitly; an implementation agent MUST use its actual rules rather than importing contradictory upstream algorithms.

The key words MUST, MUST NOT, REQUIRED, SHALL, SHALL NOT, SHOULD, SHOULD NOT, RECOMMENDED, NOT RECOMMENDED, MAY and OPTIONAL are interpreted under RFC 2119 and RFC 8174 only when uppercase. A SHOULD deviation requires a documented concrete reason and preserved safety/authorization outcomes. Requirement strength is distinct from source confirmation status and build order. All MUST requirements in this document are required for v0 readiness, including capabilities that default to disabled.

Applicable operational instructions, repository policies, tool permissions and provider permissions govern actual actions. This specification MUST NOT be treated as permission to create branches, commit, push, change remote settings, deploy or bypass a restriction. The intended future operating environment MUST explicitly permit the actions it enables. A configured capability cannot override a prohibition; unavailable authority is a visible blocker.

### 1.2 Deliverable and exclusions

Both implementations live in the same independent Rust repository, `com.kegari.rust`. The existing Cargo workspace contains `crates/kegari-v0`; its package and executable are `kegari-v0`. The lasting executable will be `kegari`. V0 MUST NOT require its scaffolding. The Symphony checkout remains a reference, not an implementation destination. Earlier separate-bootstrap-repository requirements are superseded.

V0 MUST provide a Rust orchestration core, local and remote CLI control, background operation on macOS after login, and all retained workflow gates. Platform integration MUST be separable from workflow logic. No native wrapper, menu-bar/Dock behavior, graphical dashboard, bundled frontend, frontend hosting, Sanity, Homebrew tap, signing/notarization or completed-product distribution is required. No Qwik, Protobuf, Terraform, macOS-wrapper or harness repository is a prerequisite. Other execution tools, remote worker pools, other-OS distribution and automatic production incident monitoring/rollback are excluded.

The implementation deliverable MUST include the usable runtime instruction content, starter workflow, target preparation guidance and executable evaluation suite defined in §§3.6,7.5,12.4. A prompt loader alone is not the instruction deliverable. These materials belong alongside v0 in `com.kegari.rust`; no separate harness repository or new orchestration framework is required.

Axum is the HTTP framework for v0's HTTPS control service. Its CLI protocol is versioned JSON. This does not select or replace the planned dashboard's Google Protobuf-over-HTTP/HTTPS contract; that cross-repository stack remains outside v0. V0 MUST NOT ship Symphony's optional HTML dashboard, assume compatibility with its optional JSON routes, or add browser sessions/frontend assets to satisfy remote CLI control.

Personal hardware specifications, dates, credit balances and planning-model preferences are context, not defaults or capacity guarantees. No paid API fallback or spending allowance is authorized. Other users MUST use the same documented configuration/onboarding path; no hidden operator-specific behavior is permitted.

### 1.3 Bootstrap sequence

The later comprehensive goal-mode build assignment includes implementation, tests, independent review and corrections; initial generation is not acceptance. V0 MUST pass §12 before coordinating the lasting implementation. That later work MUST use authorized Linear parents and scoped children, including tests and required documentation. Neither bootstrap status nor a one-repository slice permits parent implementation.

V0 MUST run from a versioned installed binary outside mutable checkouts and Cargo build outputs. Required runtime-owned instruction assets MUST be embedded or installed as an immutable, version-bound companion to that binary, without dependence on a development checkout. Building or releasing `kegari`, or changing `crates/kegari-v0`, MUST NOT replace or restart the live coordinator or its runtime-owned assets. Upgrade and handover require the explicit procedure in §11.7. Configured repository workflow changes retain §3.4 reload semantics. Generating this specification does not create Linear issues or start either build.

## 2. Domain, identities and state ownership [V0-02]

### 2.1 Terms

| Term | Meaning |
| --- | --- |
| Kegari workspace | Configuration binding one Linear workspace to account connections, selection rules, allowed GitHub repositories, workflow policy and release settings. Multiple such workspaces share one installation. |
| Execution directory | An isolated, persistent filesystem directory for one child or coordination/review task, with explicitly bounded access. Symphony's per-issue “workspace” means this, not a Kegari workspace. |
| Cargo workspace | The Rust build workspace containing `crates/kegari-v0`; unrelated to work selection or execution claims. |
| Parent | One end-to-end outcome, its required children, dependencies, integrated evidence and human decision. Never an implementation target. |
| Child / subissue | A defined implementation, documentation or testing responsibility with a self-contained specification and write boundary. |
| Execution claim | A local exclusive reservation for an operation. Releasing a claim only frees scheduling ownership; it does not publish software. |
| Software release | Delivery of one accepted parent generation across its declared production targets. |
| Generation | An immutable work-definition/correction snapshot. A later correction on the same parent creates a new generation, including after an earlier generation was released. |
| Operation | One bounded worker session, validation job, integration, evidence publication or parent release attempt, with an operation ID and recoverable outcome. A release attempt may contain ordered provider steps. |
| Control host | The persistent local service handling authenticated CLI requests, durable operator intent and the single orchestration process. It does not execute issue work itself. |
| Orchestration process | Scheduler and workflow engine, with its owned workers. Stop/force-stop terminate this process while the control host remains available. |
| Agent harness | The effective instructions, context assembly, tools, execution environment, provider agent loop and feedback mechanisms surrounding model execution. Codex supplies its supported agent loop; Kegari owns its workflow and enforced boundaries. |
| Agent behavior evaluation | A controlled execution of the delivered harness against a prepared case with predefined observable expectations. It qualifies implementation behavior; it is not a production parent, human acceptance or release authorization. |

### 2.2 Canonical keys and records

Keys MUST use provider identities, not names alone:

- Linear work: canonical API endpoint, Linear workspace UUID and issue UUID. Human keys such as `WEB-101` are display/resolution aids.
- GitHub repository: canonical hostname and stable repository ID. PR identity additionally includes its number; workflow identity includes run ID and attempt.
- Connection allowance: verified provider account identity. Two saved connections to the same account do not create two quotas.
- Runtime: stable UUID, execution host identity, owner generation and canonical local state root. Clients pin the expected runtime UUID and TLS hostname.
- Session: provider thread ID, turn ID and local operation ID; session display ID is `<thread-id>-<turn-id>`.

Renaming an issue, repository, workspace or connection MUST NOT create a second claim. Human identifiers resolving to multiple objects MUST fail as ambiguous. A canonical issue admitted through two Kegari workspace configurations is an ownership conflict, not two jobs. Shared repositories are permitted for distinct work, with centrally coordinated integration and production locks; shared credentials do not grant cross-workspace context access.

The implementation MUST represent at least: configuration revision; authorization event; authorized child-set/scope snapshot; parent and child definition digests; correction revisions and routing; execution profile and instruction manifest snapshots; run outcome; local test results; PR/head/base/check/review/integration evidence; ordered child deltas; isolated candidate; parent evidence token; human acceptance; release plan and step outcomes; blocker; desired operator intent; operation receipt.

Digests MUST be SHA-256 over explicitly versioned, canonical UTF-8 JSON (sorted object keys, ordered arrays where meaningful, no insignificant whitespace), or over exact artifact bytes as specified. References MUST include the digest algorithm and schema version. A display title or Git patch-ID is not an evidence identity.

### 2.3 Sources of truth and permitted persistence

Linear MUST remain authoritative for work definitions, child membership, authorization labels, routing labels, product corrections and human decisions. GitHub MUST remain authoritative for Git content, PRs, review feedback, checks and delivery facts. Kegari's in-memory state is a reconciled projection, not a substitute for either source.

Kegari MUST NOT create, embed, operate or query an application/orchestration database, including SQLite or a provider's internal database used as a substitute. Allowed local persistence is configuration, operator intent, ownership metadata, bounded logs, preserved execution files/evidence artifacts, and small versioned operation/recovery receipts. Receipts MUST NOT become an independently editable copy of issue bodies or a shadow workflow database.

Codex-owned internal persistence is permitted only through its supported protocol. Kegari MUST NOT read, write, administer or depend on the internal storage schema. Databases belonging to the software being tested are outside this prohibition, subject to the child's explicit test-data access boundary.

Durable control and recovery records MUST be written atomically using a same-directory temporary file, file flush, atomic rename and containing-directory flush before acknowledgement or dependent external effects. Files/directories containing control data MUST be owner-only (0600/0700). Corrupt, missing-required or unsupported-version records fail closed; they MUST NOT be interpreted as RUNNING, acceptance or a completed operation.

An operation receipt contains schema version, operation UUID, runtime/owner generation, canonical targets, kind, payload digest, expected work/correction/evidence generations and Git revisions, phase (`prepared`, `sent`, `acknowledged`, `reconciled`), remote IDs, observed result and cancellation state. Local receipts retain enough references to reconcile; detailed product and review records MUST also be written to their authoritative provider. Failure to publish required evidence blocks its gate.

### 2.4 Roles and authority

Each workspace MUST configure separate Linear user-ID allowlists for `authorizers`, `correctors` and `acceptors`, defaulting to the verified setup operator. Control configuration also maps permitted local OS users and permitted Tailscale users to operator identity and roles: `inspect`, `configure`, `lifecycle`, `correct`, `accept`, `release`, `recover`. Setup defaults these roles to the operator; later grants are explicit. A control role does not waive the corresponding Linear allowlist.

The host MUST verify the actor invoking a command and record that identity separately from the provider credential used to write it. Other collaborators' comments are informational unless their actor is permitted. Display names, quoted instructions, issue assignment, arbitrary bot identities and “on behalf of” text do not establish authority.

Worker sessions MUST NOT possess human acceptance, manual-release, configuration, lifecycle or recovery capabilities. An operator may explicitly instruct Codex outside the managed worker environment to use the operator CLI on their behalf; agent completion itself never supplies that decision. The CLI MUST require an explicit acceptance operation and evidence token, with no automatic default acceptance.

The mutation broker MUST enforce identity, scope, current generation, operation ownership and permitted action immediately before each provider mutation. Prompts explain these rules but cannot enforce or expand them. Raw broad provider credentials and unrestricted GraphQL/REST tools MUST NOT be exposed to workers.

## 3. Configuration, credentials and planning inputs [V0-03]

### 3.1 Local layout and configuration ownership

The state root is `~/.kegari/`, resolved on the execution host, not a remote client's machine. A different root is not an alternate coordinator loophole: one registered root and active owner are supported per execution OS account. Setup MUST reject a second active registration for that account.

| Path | Contents |
| --- | --- |
| `config.toml` | Versioned non-secret runtime, connection, workspace and control configuration |
| `intent.json` | Durable desired state, control generation and last lifecycle command ID |
| `owner.json`, `control.lock`, `runtime.lock` | Registered owner identity, exclusive control-host lock and separate orchestration-process lock metadata |
| `receipts/<operation-uuid>.json` | Small operation/recovery records |
| `runs/` | Isolated execution directories and manifests |
| `logs/` | Redacted rotating structured logs |
| `control.sock` | Protected local control socket |

Remote clients store only target preferences in their own `~/.kegari/clients.toml`: target name, HTTPS origin, expected runtime UUID and API major version. They MUST NOT store the runtime's Linear/GitHub/Codex credentials or create execution state when the server is unavailable.

Local configuration owns accounts, authority, selection, execution roots, concurrency, release policy, TLS and operator intent. The versioned runtime owns generic role guidance and its mandatory authority/noninteractive contract. A repository `WORKFLOW.md` owns project-specific prompt text, permitted hooks and check definitions; applicable repository instructions supply project conventions and context within their authority. Linear child labels own child tool/model/reasoning/speed selections. Competing settings at another layer MUST be rejected rather than silently taking precedence; instruction content cannot grant permissions or supply a second routing/profile configuration.

### 3.2 Typed fields and defaults

Configuration MUST use TOML, `schema_version = 1`, strict types and rejection of unknown fields. IDs below are stable provider IDs or configured aliases resolved unambiguously to them. Empty restricted selectors are invalid; `all` is explicit. Paths MUST expand leading `~`, resolve to absolute paths on the host, and reject NULs/traversal outside permitted roots. Only a whole string `$NAME` may reference an environment variable in documented path or secret-reference fields; unset or empty values are errors. There is no general shell interpolation.

| Object / field | Contract and default |
| --- | --- |
| `runtime.id`, `runtime.host_id` | Required UUID and verified host registration; generated/confirmed by setup, never inferred from client input |
| `runtime.execution_root` | Default `~/.kegari/runs`; canonical, owner-only, outside mutable source checkouts |
| `runtime.max_agents` | Required positive integer selected during setup; no implicit ten-worker or spending default |
| `runtime.poll_ms` | Positive integer; default 30000 |
| `runtime.max_attempts_per_phase` | Positive integer; default 3, semantics in §6.4 |
| `runtime.max_review_rounds` | Positive integer; default 3 |
| `runtime.max_turns` | Positive integer; default 20 per worker session |
| `runtime.retry_cap_ms` | Positive integer; default 300000 |
| `runtime.request_timeout_ms` | Positive integer; default 5000 for synchronous provider/app-server requests; long jobs return operation IDs |
| `runtime.stream_silence_ms` | Positive integer; default 3600000; reset by valid app-server output, not a total run-duration cap |
| `runtime.stall_ms` | Positive integer; default 300000; semantic event inactivity threshold |
| `runtime.min_free_bytes` | Nonnegative integer; default 1073741824; low space blocks new writes/work, never triggers destructive cleanup |
| `connections.<name>` | Provider kind, canonical endpoint, verified account ID, credential reference; Codex also isolated auth-home path and executable argv |
| `connections.<name>.max_agents` | Required positive account concurrency limit for Codex; shared actual accounts use the strictest configured cap |
| `workspaces.<name>` | Stable local UUID; Linear workspace ID; Linear, GitHub and Codex connection references; required workflow absolute path |
| `.linear.teams`, `.projects`, `.users` | Each `all` or nonempty ID list; OR within category, AND across categories; user matches parent creator OR assignee |
| `.linear.authorization_label_id` | Required stable label ID; no routing label grants authorization |
| `.linear.states` | Required per-team mappings for prepared, running, blocked, child-done, parent-accepted and cancelled; validate against provider IDs |
| `.github.organizations`, `.repositories` | Union of selected organizations and explicit repository IDs; at least one selection |
| `.github.pr_authors` | Required nonempty account-ID list when independent review is enabled; matches actual PR author only |
| `.independent_review.enabled` | Boolean, default false; capability still required |
| `.actors.authorizers`, `.correctors`, `.acceptors` | Nonempty Linear user-ID lists, default verified setup operator |
| `.profiles.coordination`, `.profiles.independent_review` | Explicit validated tool/model/reasoning/speed objects; independent profile required when enabled; no cost/model fallback |
| `.routing_labels` | Stable label-ID mappings for the four profile dimensions and supported Codex values; exactly one selected label per dimension per child |
| `.max_agents` | Required positive workspace limit; `.max_agents_by_state` optional positive limits keyed by normalized state; default empty |
| `.release.mode` | Required `automatic` or `manual`; setup requires a choice, no implicit production authorization |
| `.release.spacing_seconds` | Required nonnegative integer; zero allows back-to-back delivery |
| `.repositories.<id>` | Canonical checkout/source, `main` and `staging` refs, validation workflow IDs/check contexts, deployment target IDs, exact-revision workflow/input contract and deployment step dependencies |
| `control.https` | Enabled only after configured Tailscale address(es), port, expected DNS name, certificate/key paths and permitted-user mapping validate; no public/LAN/wildcard bind |
| `control.api_version` | `1`; major mismatch rejected |

`codex.argv` defaults to `["codex", "app-server", "--listen", "stdio://"]`, resolved to a verified installed executable. It is an argv array, not shell source. The app-server compatibility profile is pinned during implementation (§7.2). Approval/sandbox settings MUST be explicit in that profile and enforce §7.1; there is no unrestricted auto-approval default.

Every repository contract MUST identify the complete required local and GitHub-hosted CI suites by stable check/workflow identity, the required Codex-review context `kegari/codex-review`, integrated validation commands and deployment contract. For a no-change outcome it MUST provide an exact-SHA GitHub-hosted validation route without an empty PR. A deployment-free repository explicitly declares `deployment = "none"`; otherwise missing delivery configuration blocks release. Environments and target IDs MUST be stable enough to detect overlap across workspaces.

### 3.3 Credentials and connections

V0 MUST support Linear personal API credentials, GitHub user credentials capable of acting as the selected user, and supported Codex ChatGPT-account login. Setup MUST verify and display the authenticated account identity, available permission scopes and connection reuse without exposing secrets. An integration identity MUST NOT silently substitute for the selected GitHub user when opening PRs.

Kegari-managed persistent secrets and the host attestation key MUST use the execution user's macOS Keychain. No silent plaintext fallback is permitted. Noninteractive secret input uses stdin or an explicit environment reference, never a command argument or printed configuration. Provider-required browser/device login may remain interactive during setup; noninteractive setup reports `authentication_required` with the supported continuation instructions instead of bypassing it. Codex auth MUST use its supported credential handling in a separately configured auth home per account connection; actual multi-account isolation and locked-session availability require integration validation.

Workers receive only their bound Codex session access and narrow brokered provider tools. Declared credential environment variables and aliases MUST be removed from child environments. Repository hooks, logs, status, error bodies and model context MUST NOT expose secrets. Provider traffic authenticated with those credentials is permitted only within configured account/scope and actual operating policy.

Token expiry or revoked access blocks affected operations; Kegari MUST NOT switch accounts automatically. Reconnection to the same verified identity reconciles before resuming. Replacing an account mapping requires affected work to be paused/stopped and outstanding actions reconciled; a new identity cannot silently adopt old pending mutations. Permissions revoked while draining remain revoked.

### 3.4 Repository workflow contract and reload

Each workspace selects an explicit `WORKFLOW.md`; there is no cwd-based fallback. The file is Markdown with optional YAML front matter delimited by `---`; front matter MUST be a mapping. Supported front-matter keys are `schema_version` (1), `hooks` and `checks`. `hooks` supports `after_create`, `before_run`, `after_run`, `before_remove`, and positive `timeout_ms` (default 60000). `checks` contains named local/integrated commands and expected result-artifact paths. Commands are argv arrays, except hook strings which are explicit shell scripts executed under §7.1 boundaries. Unknown keys, duplicate YAML keys, secret/account/selection/profile overrides and empty prompt bodies are errors.

Commands and hooks MUST run with the validated isolated execution/candidate directory as cwd. Relative executable paths, command arguments and shell file references follow normal command/shell semantics from that cwd; the runtime MUST NOT rewrite argv elements or shell text relative to the workflow file. Relative result-artifact paths resolve against that same cwd. An explicitly declared absolute artifact path is permitted only within the operation's allowed execution or private scratch/output roots under §7.1. Record the cwd, artifact roots and resolved artifact paths in the operation snapshot; validate containment and current-run/revision provenance before accepting results. Missing scripts or artifacts MUST NOT fall back to the workflow directory or the operator's checkout. Only references explicitly identified as workflow resources, such as a linked context document, resolve relative to the workflow file's directory; this does not grant access or introduce new front-matter keys.

Templates use strict Liquid-compatible interpolation and control flow: `{{ path }}`, `if`/`elsif`/`else`, `for`, and `assign`; filters are `json`, `escape`, `default`, `join`, `size`, `upcase`, `downcase`, `strip`. Unknown variables/filters, invalid syntax or incompatible values MUST fail rendering, not disappear silently. Context objects include normalized `issue`, `parent`, `children`, `workspace`, `scope`, `dependencies`, `corrections`, `evidence`, `attempt`, `phase`, `profile` and `operation`. Arrays/maps remain structured; secrets are absent. Optional fields exist as null or empty collections. The runtime MUST prepend the immutable authority/scope and noninteractive-execution contract; repository text cannot override it.

Workflow/config changes MUST be detected by content digest at each poll and immediately before admission. Valid changes apply to future operations. Each running operation retains one internally consistent configuration/profile/instruction snapshot, except immediate permission revocation and correction steering. Invalid reloads leave inspection available, report precise errors and block new affected work; they do not crash the host or silently use invalid values. Last-good session settings may finish only while their authority remains valid. Security/permission reductions take effect even if another part of a proposed update is invalid; the system MUST NOT use parse failure to preserve a known revoked permission. Reloading instructions alone is not a work trigger, acceptance decision or request to run evaluations.

### 3.5 Self-contained Linear preparation

Planning occurs with the human in Codex before authorization. Issues MUST state the requested outcome, relevant context/references, constraints, dependencies, acceptance criteria and validation expectations, following the applicable OpenAI prompting guidance cited in §14. Linear API documentation governs API use, not the writing style of execution specifications. V0 consumes this prepared work; it is not a general backlog generator.

For deterministic scope validation, each parent/child description MUST contain one fenced `kegari` JSON metadata block with `schema_version: 1`; all other prose remains the actual work specification. Setup/documentation MUST provide these forms:

```kegari
{"schema_version":1,"kind":"parent","required_children":["<Linear-UUID>"],"e2e_child":"<Linear-UUID>","visual":true,"production_targets":["<configured-target-id>"]}
```

```kegari
{"schema_version":1,"kind":"child","responsibility":"implementation","write_repository":"<GitHub-repository-id>","write_paths":["src/","tests/"],"read_repositories":[],"test_resources":[]}
```

Child responsibility is `implementation`, `testing` or `documentation`. `write_repository` is one configured repository or null for genuinely read-only validation; `write_paths` is a nonempty explicit allowlist when writing, empty otherwise. Paths are repository-relative directory prefixes or exact files, not shell globs; `.` explicitly means the entire assigned repository. Read repositories and test resources are explicit configured IDs; they do not confer writes. An E2E child can coordinate authorized read/test access across repositories but cannot implement outside its single assigned write repository. Extra implementation requires separately scoped children. Required documentation MUST be represented by assigned child work when the outcome needs it.

The parent's required set is the union of its declared child IDs, all children returned by complete Linear hierarchy enumeration, and previously authorized children not removed through authorized scope reconciliation. This prevents inaccessible/deleted/reparented work from vanishing from completeness. Unresolvable IDs remain blockers. Zero children, no identifiable E2E child, contradictory metadata, undefined acceptance criteria or inaccessible necessary context means `incompletely_prepared`. The runtime MUST record the blocker without interactive clarification.

### 3.6 Delivered instructions and target preparation [V0-H01]

The build MUST deliver reviewable, version-controlled instruction sources sufficient for every supported agent activity: scoped child implementation, testing and documentation; read-only parent coordination and correction routing; code review, including the implemented opt-in independent PR review; and permitted validation/evidence preparation. The content MUST explain required inputs, allowed actions, completion/blocking outcomes and the relevant verification obligations. Correction, continuation and recovered-session guidance MUST be included. Integration, release assembly, authorization, gate decisions and recovery controls remain deterministic host responsibilities; this requirement does not create additional agents for those functions.

One shared workflow with role/phase composition, separate templates, embedded instruction sources, supported provider review instructions or a documented combination MAY satisfy this contract. A separate Markdown file or skill for every role is not required. The implementation MUST document which content comes from each layer and how the supported provider review operation receives applicable guidance. Generic assets MUST be available to the installed runtime under §1.3; missing or mismatched required assets MUST block affected execution as `invalid_config`, without a fallback prompt.

The deliverable MUST include a valid starter `WORKFLOW.md` that loads and renders through the production path using documented reference inputs, plus instructions for selecting and adapting it. Target-specific commands, resources, repository policies and account/scope settings MUST be identified explicitly; example values cannot establish real access or readiness. The configured workflow path remains mandatory. Setup or documentation MAY support an explicit copy/export into an operator-selected new file, but MUST NOT silently select the starter, overwrite existing guidance or mutate target repository settings. Existing `AGENTS.md`, `SKILL.md` and workflow files remain under their owners' control.

Target preparation documentation MUST cover the repository/context entry point, relevant architecture and conventions, existing scoped instructions, build/test/lint commands, tools and environment prerequisites, permitted test data/resources, CI/review requirements and the deployment or deployment-free contract in §3.2. It MUST distinguish supplied generic guidance from target-specific information the operator must provide and explain how to validate the configuration before dispatch. Keep the entry point concise and link necessary project context rather than injecting a whole documentation corpus. Required inaccessible context becomes a visible blocker. This is the existing onboarding/validation process, not an automatic context crawler or repository instruction generator; `config check` remains read-only.

## 4. Provider contracts and admission [V0-04]

### 4.1 Adapter semantics

The Linear adapter MUST support scoped parent discovery; full child enumeration; set-based ID refresh; configured-state queries; paginated comments/history/dependencies; label/actor lookup; scoped comment/state writes; and file upload. The GitHub adapter MUST support repository/account lookup, PR/head/base reads, complete check/status/review reads, Git object/provenance reads, scoped branch/PR/status/comment/merge operations, workflow dispatch/status/cancellation and deployment observations. Writes go through the mutation broker, not arbitrary agent queries.

All reads used for admission or a gate MUST complete pagination before returning a usable snapshot. Transport, GraphQL partial errors, pagination loops, missing pages and malformed requested records fail the operation atomically. No dispatch from a partial list is permitted. ID-refresh input is a set, empty input makes no request, and results contain at most one complete object per ID; ordering has no meaning. An absent object is inaccessible/missing, not fabricated Done/cancelled. Changes during multi-read collection require revalidation before effects.

Normalized issue IDs, identifiers, titles and states MUST be nonempty. Labels are represented by stable IDs and trimmed/lowercased comparison names, with blanks/doubles removed; display spelling is retained separately. Priorities are integers or null; lower positive values are more urgent and absent/zero priority sorts last. Timestamps are parsed RFC 3339 instants or null. Optional malformed metadata may become null with a diagnostic; malformed required data fails the read. Provider-native references MUST be JSON-safe and non-secret. Dispatchability is explicit, computed by the host; provider state alone cannot set it true.

Stable errors MUST distinguish `unsupported_provider`, `invalid_config`, `missing_secret`, `authentication_required`, `access_denied`, `provider_transport`, `provider_status`, `provider_response`, `provider_pagination` and `rate_limited`. Retry-After/reset information MUST be honored when trustworthy. API endpoints, credentials and account identities MUST never be inferred from issue/PR instructions.

### 4.2 Parent selection and authorization

Parent discovery MUST apply OR within each selected team/project/user category, AND across restricted categories. A selected user matches creator OR assignee. `all` removes that category's restriction; it does not broaden repository permissions. All children of an admitted parent inherit Linear inclusion without independently matching those selectors.

Authorization requires the configured label's current presence AND a verified permitted authorizer event. Read the complete relevant Linear history and identify the latest unambiguous transition establishing current label presence. Check the stable actor ID against the current allowlist. A later removal/reapplication supersedes older events. Null/unrecognized actors, incomplete history, ambiguous grouped changes or unstable observations mean `authorization_provenance_unavailable`; do not infer authority from creator, assignee or an older convenient event. The operator resolves this by applying the label with a permitted identity in Linear.

The host MUST record an authorization snapshot on the parent, including the label event, child IDs, definition/scope digests and current production targets. Record publication is itself recoverable and MUST succeed before work admission. Adding required children, deleting/reparenting required children, widening write/repository access or changing production targets requires renewed parent authorization after the change; old children remain accounted for until that reconciliation. Material direct edits to the expected outcome also invalidate old evidence and require a verified new authorization event or an actionable permitted in-scope correction. Routing-label-only edits are excluded from work-definition digests and never trigger execution. This host-created authorization record may be produced as a bounded admission/preparation operation; it is not an implementation session and cannot grant authority absent the verified human label event.

An in-scope correction from a permitted corrector requires no extra authorization label cycle, readiness action or replacement hierarchy. It cannot expand the already authorized repositories/write boundaries. If new scope is necessary, report the affected blocker and required authorization change rather than implementing it.

### 4.3 Scope changes

| Event | Required behavior |
| --- | --- |
| Parent ceases to match ordinary selection filters | Drain already active operations; no new child/retry/review session or release. Keep children, results and exclusion reason visible. |
| Parent authorization label removed, actor authority revoked, or work explicitly cancelled | Stop affected local workers, prohibit further work mutations, preserve files, request owned remote cancellation where supported and reconcile uncertainty. Read/status/cancellation actions remain permitted. |
| Repository permission/write boundary removed | Stop affected writes/workers; do not use the filter-drain rule to circumvent access revocation. |
| Parent becomes included again | Reconcile; resume only if current authorization, saved RUNNING intent and other gates permit. Do not reset attempts or approvals merely because inclusion changed. |
| Organization gains repositories | Refresh union and validate repository policy/permissions before use. Newly visible scope is not a missing write-boundary grant. |
| Independent-review filters exclude an active PR | Let its already admitted read/review operation finish within its original feedback scope; start no subsequent review. Revoked access still stops affected effects. |

No terminal Linear state causes automatic deletion. Parent accepted/Done state is compatible with pending release and later corrections; it MUST NOT be mistaken for an instruction to stop observing that parent. A cancelled child remains required/unresolved until authorized scope reconciliation removes or replaces the responsibility.

### 4.4 Independent PR reviews

Support MUST exist and default to disabled. When enabled, repository union AND actual PR-author selection govern intake. Review is keyed by repository, PR number and exact head/base review identity; the same unchanged PR MUST NOT trigger endless reviews. A new head or explicit operator retry can trigger a bounded new review.

This mode permits reading source/diffs and posting Codex review feedback on GitHub only. It MUST NOT execute PR-supplied code, hooks, test commands or dependency installation, and MUST NOT implement fixes, push, merge, deploy or create implicit Linear authorization. PR content is untrusted data. Required fixes enter separately authorized scoped child work. This mode does not disable the mandatory review of authorized child PRs.

## 5. Work lifecycle, dependencies and corrections [V0-05]

### 5.1 Orthogonal states

The host MUST track these dimensions separately, rather than equating a tracker state, worker exit or scheduler claim with delivery:

| Dimension | Values / interpretation |
| --- | --- |
| Desired runtime intent | `RUNNING`, `PAUSED`, `STOPPED`; durable operator choice |
| Eligibility | included, filter-excluded, unauthorized, inaccessible, ownership-conflict |
| Child phase | prepared, runnable, executing, local-validation, awaiting-CI, awaiting-review, awaiting-staging, complete, blocked, interrupted |
| Parent phase | preparation, executing, validating, ready-for-review, accepted, release-wait, releasing, released, no-release-required, blocked |
| Operation outcome | running, succeeded-with-evidence, unfinished-with-progress, no-change-with-evidence, blocked, failed-retryable, cancelled, interrupted, uncertain |
| External action | not-started, prepared, sent/unknown, acknowledged, observed-success, observed-failure, cancelled-confirmed |

A parent can be accepted and blocked on spacing/dependencies simultaneously; eligibility and gate reasons MUST remain visible. State mappings to Linear are configured per team and are presentation/workflow records, not sufficient proof of gates. Only explicit valid human acceptance permits parent final closure. A correction reopens the affected parent generation and children under the configured states; older release history remains immutable.

Child completion requires fulfilled scope/acceptance criteria, actual required local and hosted validation, Codex review, successful staging integration for repository changes, and recorded context/results for dependents. No-change exceptions are defined in §8.4. A test/documentation child contributes to completeness even if it contributes no production patch. Candidate construction is an earlier phase: it may proceed once required implementation/test-code/documentation deltas are integrated and the test plan/resources are ready, while the E2E child's execution/evidence is still pending. Candidate validation then completes that responsibility. Parent readiness still requires every required child to be complete, the E2E child to have validated the isolated integrated candidate, and current complete evidence. Zero children never passes this predicate.

### 5.2 Dependency model

Linear native blocking relations are the authoritative declared dependency graph. Child dependencies govern execution/integration; parent dependencies govern production release. The host MUST resolve IDs and report cycles, missing dependencies and inaccessible prerequisites as blockers. A prerequisite child's Done label is insufficient without its completion evidence. A required parent dependency is satisfied only by observed successful release (or validated already-in-production outcome) of its required current outcome. A newly corrected prerequisite is not assumed satisfied by an older release if the dependent requires the corrected behavior.

The parent specification MUST identify any requirement for a particular historical prerequisite outcome; absent that declaration, the prerequisite generation current when the dependent is authorized is the required outcome. Record that dependency snapshot. If B was authorized against released A1, a later correction A2 does not automatically repin B to A2. A change to B's dependency requirement, or evidence that B cannot work without A2, blocks affected readiness until its dependency requirement is reconciled through authorized scope/correction handling. Source edits MUST NOT silently substitute another prerequisite generation.

Git ancestry and isolated candidate validation MUST cross-check declared dependencies. If a selected delta requires unreleased work from another parent, report the missing dependency; do not copy that other parent's changes into the release. No agent may invent an ordering that bypasses a declared dependency. Independent runnable children/parents may continue while another dependency is blocked.

### 5.3 Correction intake and routing

Poll comments independently of the parent's issue `updatedAt`; that timestamp is not proof comments are unchanged. A clear native Linear correction by a permitted corrector is actionable without a separate readiness label. Track comment ID, current body digest, author ID, `editedAt`/`updatedAt` and observed revision. Classify permitted comments as corrective, informational or ambiguous change-related direction. Informational comments (including acknowledgments/status questions) do not create a correction generation or invalidate acceptance. Unclassified or ambiguous change-relevant input suspends affected release/readiness gates until resolved; classification uses an admitted coordination session when needed and cannot bypass pause/stop. Confirmed informational input restores the prior gate state without renewed acceptance. Edits/follow-ups to corrective intent alter the correction generation. Rely on Linear's native author-edit policy for native comments; the original author field MUST NOT be described as independently proving every possible privileged editor. Unknown/bot/delegated provenance fails closed when it could affect authority.

Coordinator-authored routing/evidence comment IDs MUST be recorded and excluded from human-correction detection so those posts do not recurse. Explicit CLI corrections are a distinct authenticated operator action and MUST enter correction processing despite being written through the host. They record both the authenticated operator and provider writer in a host-authenticated marker; plain text claiming such a marker is insufficient.

Correction processing MUST:

1. Fetch and reconcile the current parent/child definitions, authority and all relevant comment revisions.
2. Once corrective intent is established (immediately for explicit CLI corrections), mark superseded readiness/evidence/acceptance ineligible. Preserve earlier records as history. Pending classification cannot authorize release.
3. Use a read-only coordination session with the configured profile to interpret the correction, identify affected existing children and formulate actionable instructions. If material intent remains ambiguous, publish a blocker; do not ask an interactive question or guess.
4. Record corrective instructions on each affected child, with source comment/revision links and the work generation. Deduplicate writes using operation receipts and returned comment IDs.
5. If an affected child session is active, deliver incremental steering using the provider's current turn ID. If no turn is active or a completion raced the steer, queue the necessary next run for that generation. Do not treat a failed steer as feedback consumed.
6. Reuse unaffected child evidence only where its scope, dependencies and revisions remain valid. Repeat affected checks/review/integration and parent integrated validation; publish new evidence and require new acceptance.

Workers report the correction watermark they processed. Before accepting their outcome, the host MUST reread current correction revisions. An outcome behind that watermark cannot establish completion or release eligibility. This check is required before and after completion publication and before each irreversible release step. The scheduler MUST serialize outcome/correction handling so a concurrent event is either delivered to the active session or remains pending for the next one.

Polling can observe only provider-visible revisions; the system cannot guarantee that an unseen concurrent remote edit prevents an already in-flight action. Once a correction is observed, no remaining stale action may start. Record any completed or uncertain effects honestly and apply §9.6. An identical resulting code patch does not preserve acceptance when the expected behavior/correction generation changed.

### 5.4 Completion and new work triggers

Legitimate triggers are initial verified parent authorization, pending scoped correction, a newly satisfied dependency, a scheduled retry within its budget, or an explicit operator retry after blocker resolution. Profile edits alone, repeated polling, a tracker Done state, ordinary restart and successful worker exit are not new-work triggers.

An agent's outcome is a proposal supported by references. The host MUST verify the actual checks, PR states and generation before advancing phases. It MUST NOT accept prose such as “all tests passed” without the corresponding results. `unfinished-with-progress` can continue only within the finite phase budget and with a recorded material progress marker (new relevant code/result, resolved finding or completed required step). Repeated identical outcomes or speculative refactoring cause `no_progress` blocking.

## 6. Scheduling and bounded execution [V0-06]

### 6.1 Ownership, concurrency and ordering

Only the orchestration process holding the exclusive canonical runtime OS lock may schedule or request autonomous execution/integration/release mutations. The control host separately holds `control.lock`, serializes all broker mutations, and may perform authenticated operator decision/configuration writes and recovery observations while orchestration is stopped. Such writes do not admit a worker or release. All autonomous broker requests require the current orchestration lease/generation; stale workers cannot retain mutation authority after their owner stops. Claims MUST be reserved atomically before spawning; a child/coordination/PR-review work identity has at most one live session or pending retry claim. Claims are not software releases and are not proof of completion.

Every new operation, including a worker spawn, preparation/CI operation, integration or release attempt, MUST obtain a control-host admission permit containing its operation ID, owner/control generation and immutable scope. The host validates and registers that permit under the same serialized gate used for lifecycle changes. The scheduler selects work; the host enforces admission. Pause/stop closes this gate before persistence and acknowledgement. Only operations already registered may drain; no separate-process race may launch an unregistered operation after acknowledgement. If intent persistence fails, admission stays closed and the command reports failure. Revocation/force-stop also fences existing permits as required; ordinary pause does not revoke their drain rights.

Admission requires available capacity under global, verified-account, workspace and applicable normalized-state limits. A running session counts once against all applicable limits. A pending timer does not occupy an agent slot but retains its logical claim until revalidation. Integration and release locks are separate from agent slots. Decreasing capacity does not kill authorized sessions already running; it prevents new admissions until usage is within bounds.

Eligible workspaces MUST receive round-robin admission opportunities, skipping those without ready work. Within a workspace, order by known lower positive Linear priority, then creation time (oldest first), then canonical issue ID; unknown timestamps sort after known ones. Dependencies always take precedence. Independent PR review uses the remaining workspace/account capacity and MUST NOT starve authorized child work. Stable ordering is required for reproducibility, not a promise that one workspace owns a separate account allowance.

Quota snapshots MUST be associated with the actual Codex account. Shared-account allowance is not multiplied. Exhaustion pauses new sessions for that account until a documented reset/refresh establishes availability; other accounts continue. Unknown allowance is displayed as unknown, not zero or unlimited; provider denials still constrain execution. Capacity and quota waits do not count as failed work. There is no automatic paid fallback, account switch or invented credit budget.

### 6.2 Poll/reconcile algorithm

On startup, the control host loads validated config/intent, acquires owner supervision and reconciles old process ownership. It starts at most one orchestration process under §11. The orchestration process acquires its lock, loads receipts, reconstructs authoritative provider facts and preserves unresolved effects before considering dispatch. It MUST NOT perform upstream-style terminal-directory cleanup.

Each serialized scheduler tick MUST:

1. Refresh configuration, permission reductions, operator intent and connection health.
2. Reconcile running/uncertain operations, comment revisions, authorization and active parent inclusion before new admissions.
3. Refresh candidate parents and complete child/dependency sets atomically per workspace. Keep previously owned work visible even when excluded/missing.
4. Recompute gates from current facts; publish necessary blockers/results through recoverable writes.
5. If RUNNING, admit eligible operations using fair ordering and all capacity/claim/authority constraints. Otherwise admit no new execution or release.
6. Schedule the next tick, respecting polling/rate limits; coalesce refresh requests so they cannot create duplicate concurrent ticks.

A failed discovery read prevents new admissions from that workspace's incomplete snapshot. Active work may finish under its last verified authorization only while there is no observed revocation; it MUST NOT start new external mutations requiring unavailable fresh gate data. A failed terminal-state query is not permission to delete anything. Read/status failures do not invent completion.

Immediately before dispatch: refresh the work identity, current parent authorization, correction watermark, child scope, dependency results, profile, available capacity and path/sandbox readiness. Reserve claim and durable run receipt, create/reuse execution directory, run preparation hook, render prompt, then start the provider. On startup failure release the running slot, retain recovery context, and classify retry/blocking honestly.

### 6.3 Active operation boundary

An active agent operation is one provider session of at most `max_turns`; subsequent sessions/reviews/retries are new admissions. An already launched validation/CI job can finish during drain. A release is one bounded, recorded parent release attempt with its ordered steps; it may finish during pause/graceful stop unless authority, correction, failure or force-stop requires stopping remaining actions. This does not authorize indefinite agent continuations inside a release.

Pause/filter exclusion permits the currently admitted session's bounded turns and already admitted operation to finish, but never a new session, review run, retry or production release. Following completion, a next phase waits. Observation and recording its result are part of finishing the active operation, not permission to launch further work. Explicit human review/correction commands remain available through the control host.

### 6.4 Attempts, review rounds and failure taxonomy

Each phase/work-generation has at most `max_attempts_per_phase` admitted worker sessions, including the initial session (default 3, not three additional retries). Each session has at most 20 turns by default. Continuations after a session cap consume another attempt; success exit does not create an unlimited one-second loop. Review/fix processing has at most three review rounds by default: a review followed by required scoped fixes consumes one round; the final clean review may pass within the remaining round. Reaching a bound with unresolved work produces a visible blocker, not a passed gate.

Budgets MUST survive restart, filter/profile edits and duplicate notifications through run receipts/provider run records. A materially new permitted correction creates a new budget for affected phases; unchanged work does not. `retry` explicitly grants one new bounded phase budget after reconciling/resolving the blocker, records actor/reason in Linear or the PR, and never authorizes wider scope or replay of unknown effects.

| Condition | Action |
| --- | --- |
| Retryable process/transport failure with known no ambiguous mutation | Exponential retry at `min(10000 * 2^(failure_index-1), retry_cap_ms)`; first delay 10 seconds; honor longer provider Retry-After; remain within phase attempt budget |
| Worker completes with verified unfinished progress | Reconcile first; earliest next session after 1000 ms, only when a legitimate remaining step and budget/gates permit |
| Completed/no-change/blocked/waiting outcome | No unconditional continuation; wait for actual trigger or dependency |
| Authentication, unsupported profile, invalid template/config, scope conflict, missing intent or permission | Persistent blocker; no automatic repeated agent attempts |
| Provider-required verification, policy/safety stop or manual intervention | Persistent blocker under §7.3; preserve it across timeout, process exit and restart until its required resolution is established; no automatic retry or replacement session |
| Capacity/quota/dependency/spacing wait | Observe and resume when conditions allow; no failure-budget consumption |
| Unknown external mutation outcome | Reconcile; block affected effects until established; never blind replay |
| Deliberate pause/stop/revocation | Preserve intent and work; no automatic retry that undoes it |
| No material progress or exhausted bounds | Block with concise attempt/finding history for the next planning summary |

Synchronous read failures use at most three attempts per poll with the same capped backoff, then await the next poll; they do not spawn agents or exhaust an unrelated worker budget. A retry timer MUST rerun admission checks, never dispatch from stale eligibility. If blocked by pause/exclusion it remains pending without consuming an attempt. If work is complete/cancelled, release its execution claim; preserve any unresolved operation receipts.

## 7. Execution isolation and Codex integration [V0-07]

### 7.1 Filesystem, tools and hooks

Execution directories MUST be created under the canonical execution root using a readable sanitized key plus a digest of the canonical work identity. Sanitization permits ASCII letters, digits, `.`, `_`, `-`; other characters become `_`. The appended identity digest prevents collisions. Child paths additionally identify repository and work generation. User-controlled titles MUST NOT become absolute paths or escape the root. Resolve/check symlinks and actual ancestry immediately before every create/use/remove operation; prefix string comparison is insufficient.

Use isolated clones or worktrees with their own working/index state; never execute in the operator's checkout or discard its dirty files. Reuse an owned preserved execution directory only after verifying its identity manifest, repository, ownership and outstanding work. Unrecognized/dirty state is preserved and inspected, not reset/cleaned. Cross-parent integration conflicts go through the affected child's permitted implementation session and the normal gates.

Cwd MUST be the validated execution directory wherever the provider supports it; local check and hook execution MUST enforce the cwd and artifact resolution rules in §3.4. A provider path that cannot preserve the required execution/candidate identity MUST fail preflight rather than use the operator's checkout. Cwd alone is not a sandbox. Actual OS/provider filesystem and network/tool controls MUST enforce the child's write paths, explicit read/test resources and separation from other workspace context. A process requiring arbitrary writes to other repositories, controller files, credentials or broad home-directory contents MUST fail preflight. Runtime workers MUST be unable to access the privileged Unix control socket, HTTPS control capability, attestation key or configuration secrets. Tailscale device identity alone cannot enforce that distinction.

All roles may use a private bounded scratch/output directory for logs, build products and evidence according to their role. Read-only validation may write those outputs and explicitly permitted disposable test data; it cannot modify product/test source or broaden repository writes. Sandbox configuration MUST distinguish scratch/build outputs from source write paths. Independent PR review does not gain code execution merely because scratch is writable. Record artifact paths/digests needed for evidence before cleanup.

Parent coordination sessions have no product/test write capability. They may propose scoped routing/readiness/evidence actions through narrow tools. Integration/release assembly is deterministic host work on already reviewed artifacts; any product/test change returns to child implementation. Independent PR reviews use an even narrower read/feedback boundary and never run repository code. Hooks execute only for the authorized child execution/validation role, under the same boundaries; PR text cannot install hooks or enlarge access.

The broker advertises only implemented, phase-permitted tools. Tools MUST validate arguments and return structured success/failure without exposing raw credentials. Examples of permitted capabilities are scoped issue/context reads, child outcome submission, selected PR feedback, and validation-result references. Neither arbitrary shell access outside the sandbox nor unrestricted provider APIs are substitutes for this contract. Repository instructions apply within their actual authority; untrusted issue/PR content cannot override host restrictions.

Hooks run in the validated directory, with minimal environment and a configurable timeout (default 60 seconds):

- `after_create`: once after creating/populating a new owned directory. Failure blocks that creation; do not mark it initialized or rerun blindly over partial effects.
- `before_run`: before each admitted session, after validation; failure prevents agent launch and is classified as an attempt failure/blocker.
- `after_run`: once after a launched session ends, including failure/interruption when possible. Do not run it when preparation failed before launch. Failure is logged and cannot turn failed work into success; required artifact/check publication still gates completion independently.
- `before_remove`: only during explicitly authorized safe cleanup, after all preservation predicates pass. Failure/timeout cancels removal and leaves the files in place. This deliberately strengthens Symphony's ignored cleanup-hook failure.

Hook stdout/stderr is bounded and redacted. Hooks MUST NOT perform unbrokered remote actions; timeout is not proof an external effect did not happen. When hook side effects are uncertain, preserve the directory and record a blocker.

### 7.2 Supported app-server contract

Use a separately installed local Codex app-server over stdio, not Elixir, a cloud-review prerequisite or direct paid API. During reconciliation the installed CLI identified itself as `codex-cli 0.164.0-alpha.11.1`; only version/help and official protocol documentation were inspected. This is the initial compatibility target, not proof of a working integration. The build MUST pin that executable/protocol profile, inspect its generated supported schema, and test the operations below. A different version requires an explicit supported-profile test result; an unknown version/capability MUST fail with `unsupported_codex_version` rather than guessing payloads. Exact dependency/protocol fixtures belong to the implementation, not to a claim that this document tested the binary.

Required protocol capabilities are initialization, authenticated account identity and rate-limit inspection, model/capability listing, thread start/resume, turn start/steer/interrupt, review execution and structured event/tool responses. Initialization is `initialize` followed by `initialized`; the adapter MUST honor the installed version's actual schema, IDs and experimental-feature requirements. Start a new bound thread for a new task; resume only a known compatible thread from the same work/account/scope after recovery. Never attach an unrelated session merely because credentials are shared.

The adapter MUST send the explicit model, reasoning and speed from the validated profile using supported fields, plus cwd and enforced permission settings. Validate the tool/model/reasoning/speed combination against installed schema and actual account/model availability. Exactly one current child label per dimension is required; missing, duplicate, conflicting or unsupported values block that child. `fast` is used only when actually supported/entitled; no automatic downgrade or expensive fallback. A running session's settings remain fixed after label edits.

At session start, assemble the delivered guidance and actual work context for the admitted role under §7.5. A child receives its full specification, parent intent, authorized scope, relevant dependencies/context, current corrections and actual prior results. Parent coordination receives the authorized parent/child set and relevant evidence/corrections without acquiring child implementation authority; independent PR review receives the selected revision/diff and permitted review context without requiring a fictional implementing child. Include applicable OpenAI prompting principles: clear task/outcome, necessary context, constraints and concrete verification. Do not indiscriminately load other workspaces' private material. Strict template failure occurs before model execution. Later turns reuse the live thread with incremental progress/correction instructions, not repeated copies of the initial prompt. A recovered session is supplied with the actual reconciliation findings rather than assuming its remembered state is current.

For live steering, use the current `expectedTurnId` where supported. If the provider reports no active turn/stale turn, reconcile and leave the correction pending for a new admitted turn/session. Unsupported steering is a compatibility blocker for this required v0 behavior, not permission to drop feedback.

### 7.3 Stream, input and timeout handling

The client MUST correctly parse fragmented and coalesced newline-delimited JSON-RPC messages, correlate response IDs and handle interleaved notifications/tool requests. Protocol stdout and diagnostic stderr MUST remain separate. Limit one decoded message and an unterminated frame to 10 MiB; exceeding either yields a structured protocol error and preserves work. Unknown notifications may be logged safely; malformed required messages cannot be ignored as successful completion.

Emit normalized events for session start/startup failure, turn completion/failure/cancellation, input-required, provider intervention, supported tool result, unsupported tool request, notification, protocol error and process exit. Capture thread/turn IDs, timestamp, provider process ownership, last semantic event and usage. A normal process exit without the required terminal outcome is interrupted/uncertain, not child success; an already observed provider intervention retains its blocking classification.

Requests for interactive user input MUST produce `input_required` blocking and a Linear planning-summary entry, with no interactive question sent to the human. Approval requests are handled only when the specific operation was already authorized and is enforceable within the broker/sandbox policy. Otherwise deny and block; never auto-approve unrestricted operations to keep a run moving. Unsupported tools MUST receive a structured failure response so the session cannot hang waiting for an answer.

The supported-profile adapter MUST distinguish provider signals requiring verification, policy/safety review or other manual intervention from transient transport failures and transient safety buffering. Verify mappings against the supported schema and behavior; current documentation alone does not establish the pinned version's event names. Signals known to require intervention MUST NOT be discarded as generic unknown notifications. Record the authoritative provider source, affected operation/account scope, reason and required resolution in bounded recovery/status records; issue/PR text or a worker's unsupported assertion cannot create or clear this provider-level classification. Preserve the blocker through later watchdog expiry, process exit, restart, profile edits and corrections. Timers, a new thread/model/account or an operator `retry` command MUST NOT bypass it. Clearing requires evidence that the provider's required resolution has occurred through a supported, permitted process; then ordinary admission, attempt-budget and operator-intent rules apply. A provider-ended task without supported resumption MUST NOT be revived through a replacement-session workaround. Reconcile any uncertain effects under §11.3, keep the blocker visible without interactive runtime questions, and allow unrelated authorized work to continue. Transient buffering alone creates no persistent intervention blocker, while ordinary watchdog and outcome handling still apply.

`request_timeout_ms` bounds startup/synchronous responses. `stream_silence_ms` is the maximum silence during an active turn and resets on valid protocol output; it is not a wall-clock task limit. `stall_ms` measures absence of meaningful progress/session events; repeated transport noise does not count. Exceeding a watchdog interrupts that operation, records the reason and follows bounded recovery. Normal watchdog failure handling remains active during drain, but graceful stop itself MUST NOT silently invoke force-stop or cancel otherwise healthy remote work.

### 7.4 Metrics and evidence honesty

Use documented cumulative token counters when available, adding only positive deltas once per provider thread/account with per-turn observations. Preserve the last observed cumulative baseline when resuming the same thread in a new session; historical totals MUST NOT be counted again. If that baseline is unavailable, establish a new observation baseline and report earlier usage unknown rather than assume zero. Handle a new thread's counter reset; do not mistake arbitrary `usage` objects for cumulative totals. Add ended-session elapsed time once, then include live durations when constructing a snapshot. Deduplicate repeated notifications. Unknown usage is unknown, not zero. Rate-limit state is per actual account with observation time; a global last-seen account snapshot is invalid for multiple accounts.

Model messages, summaries and go/no-go opinions are explanatory evidence. They MUST NOT independently change gate truth, human approval, scope, provider outcomes or release state.

### 7.5 Instruction composition and provenance [V0-H02]

The production assembly path MUST combine the immutable host authority/scope contract, applicable delivered role/phase guidance, configured workflow, prepared work context and supported repository instructions. It MUST preserve their ownership in §§2.4,3.1,3.4; neither formatting nor repetition grants authority. Applicable project instructions and referenced context MUST be accessible from the isolated execution environment through the supported Codex discovery/context mechanisms. The adapter MUST account for its configured Codex home, actual repository root/cwd, scoped/nested instructions and provider context limits. It MUST NOT assume instructions in the operator's original checkout were inherited by a clone/worktree or silently omit required guidance. Provider-managed discovery must be validated for the supported profile; Kegari MUST NOT inspect Codex's internal database to reconstruct it.

Each agent operation/session MUST retain an instruction manifest referencing the runtime/build and generic-asset versions/digests, configured workflow digest, relevant repository revisions and instruction/context file digests, assembly schema version, role/phase, work/correction snapshot, execution profile, Codex protocol/version, cwd identity and exposed tool/permission settings. Deterministic host operations retain references to relevant source manifests where needed for provenance, with agent-only fields explicitly not applicable; they do not fabricate agent sessions. Record Kegari-supplied inputs and provider-confirmed discovery separately; unavailable provider-internal context MUST be marked unavailable, not claimed as inspected. Later correction/continuation inputs and relevant context reads observed through supported tools MUST be associated with their actual turn and revision. The manifest and permitted input references are evidence artifacts, not full prompt copies in operation receipts or a new database.

Assembly tests MUST exercise the delivered assets and the same code path used for actual sessions, including native provider review where selected. They MUST verify applicable root/nested guidance in new and reused execution directories, correct role/profile/correction inputs, and exclusion of secrets and unrelated workspace content. Required context that is missing, inaccessible, oversized or incompatible with the supported provider MUST cause a precise blocker rather than a silently weakened request. Snapshot and resume compatibility MUST include effective instructions; an incompatible resumed thread requires reconciliation and a fresh admitted session, not mixing incompatible snapshots. Ordinary changes affect future operations under §3.4 and MUST NOT alone restart work, reset budgets or invalidate product acceptance.

## 8. GitHub gates, staging and parent acceptance [V0-08]

### 8.1 Branches and scoped changes

Participating repositories MUST use `main` as production and `staging` as integration. Each child development branch MUST use `<child-identifier>-<full-parent-title-suffix>`. The suffix is computed once per authorized parent-title revision and reused identically across repositories. Preserve every title word, order, Unicode letters and capitalization. Replace whitespace, control bytes, slash, backslash and Git-forbidden `~ ^ : ? * [` characters with hyphens; replace `@{` with `@-`, runs of two or more dots with a hyphen, and a terminal `.lock` separator with `-lock`; collapse consecutive hyphens and strip leading/trailing dots or hyphens. Validate with Git's ref-name rules. Do not abbreviate, paraphrase, transliterate away words or silently truncate. Empty/unrepresentable names or provider/filesystem length limits block with the naming reason.

The host MUST record the title revision/suffix and branch-to-child association. Existing branches retain their historical names when the parent is renamed; new branches use the new single recorded suffix consistently. Rework may reuse its owned branch when safe; it MUST NOT force-overwrite unrelated published work. If the canonical branch already belongs to another operation, block rather than add an unexplained title suffix. Recovery uses canonical IDs and receipts, not branch-name parsing as authority.

A child's initial checkout is an exact recorded staging revision, with required dependency results available. It may read staged context only within its scope; inherited staged code is not part of its own delta or permission to release another parent. All product/test conflict resolution happens in an authorized child session. The controller may mechanically apply already reviewed patches but MUST NOT repair code to make a merge succeed.

Before opening a PR, required local tests MUST actually run against the candidate tree with captured command, environment identity, revision, result and artifact references. A failure or unrun required check blocks progression. PRs are opened automatically under the selected verified GitHub user in an operating environment permitting it. PR creation MUST NOT require the operator to create them manually. The host MUST attach the child/parent IDs, work generation and operation correlation without leaking private cross-workspace context.

### 8.2 Independent staging gates

The following gates are separate and MUST all hold for changed child work:

1. Required local tests passed for the candidate.
2. The full configured CI suite ran on GitHub-hosted runners against the relevant candidate/merge result and passed.
3. Codex review passed for the current reviewed change and requirement generation.
4. Current repository protections and any additional required approvals are satisfied.
5. The intended squash integration actually succeeded into staging, and its resulting commit/tree/delta match the validated change.

Agent-selected test subsets, missing workflows, checks attached only to an obsolete head, or GitHub's broad “mergeable” status are insufficient. `skipped`, `neutral`, cancelled and missing required tests MUST NOT be represented as tests that ran and passed. Each configured suite specifies its required jobs/check identities; legitimately inapplicable checks need a preconfigured documented predicate, not an agent's ad hoc waiver. Self-hosted runners do not satisfy the confirmed GitHub-hosted requirement.

Gate records MUST include repository/PR, expected head, tested base or test-merge revision/tree, required check identities and actual run attempts, outcome, timestamps and work generation. Head/base movement invalidates affected results and requires fresh validation of the actual candidate. Do not cancel a later human rerun merely because its workflow run ID was once owned; ownership includes the run attempt and origin.

Codex review MUST assess the actual requested outcome, relevant correctness/security concerns and configured checks. Findings and resolutions belong on the GitHub PR, with revision references. Product direction and scope changes remain in Linear. A clean review may conclude no change is needed. Speculative improvements or unrelated refactoring MUST NOT prolong completion; concrete findings still block until resolved or shown inapplicable by recorded review evidence.

### 8.3 Review status and integration contract

The controller MUST publish the `kegari/codex-review` commit status for the exact reviewed PR head and an authenticated attestation on the PR. The attestation contains schema/version, runtime/owner identity, operation/review session, repository/PR, head/base/delta digest, requirement/correction generation, verdict, unresolved findings and supporting links. Authenticate canonical bytes with an HMAC-SHA-256 key held only in the host Keychain. Runtime verification requires both the genuine attestation and matching provider records; a same-named status alone is insufficient. Key identity must be recorded and retained across supported upgrades; loss/rotation without a verifiable old key blocks adoption of old attestations until reviewed again.

GitHub user credentials can publish commit statuses, but a PR author cannot provide their own formal approving review. V0 MUST NOT depend on that self-approval. Setup MUST check that required status contexts, permitted squash merging, current-base validation and branch protections can support the workflow. Repository writers can otherwise forge a status context; managed workers therefore lack raw status-writing credentials, and the controller verifies its own attestation. This is not a claim that v0 prevents an independent repository administrator from changing protections or merging outside Kegari.

Existing required approving/code-owner reviews remain gates. If they cannot be satisfied by the configured identities without routine human action, report the repository as incompatible with unattended staging rather than silently weakening its protections. V0 MUST NOT change repository settings, bypass required reviews or use admin merge bypass. The supported configuration uses required checks with strict current-base protection (or an equivalently verified provider merge condition) and squash merges. Missing enforcement is a preflight blocker.

Serialize integration per repository across workspaces. Immediately before merge, reread authority, corrections, PR head/base, required gates and protections; pass the expected head SHA to the merge operation. If the base moved, validate again; do not trust a check for an earlier base. GitHub's merge acknowledgement is not final proof: observe the actual merge commit and ancestry. An accepted/enqueued request is pending. If the actual squash commit differs from the reviewed candidate or its parent base is unexpected, quarantine the result as a provenance blocker and do not mark Done; record any already produced staging effect.

The recorded child delta MUST be captured from the validated change and linked to the observed staging squash. Its identity contains repository, child and generation, validated base/head/tree, staging parent/commit/tree, and exact patch bytes/digest. Generate canonical patch bytes with a pinned Git version using binary/full-index diff, no external diff/text conversion, no rename inference and stable path order. Preserve paths, file modes, symlink targets and binary content. The observed staging-parent-to-squash diff MUST equal the validated delta. A later arbitrary staging tip is never a source for reconstructing what the child contributed.

The host MUST retain retrievable Git objects and the delta manifest through required recovery/evidence retention. Successful merge without recorded/verifiable provenance is not child completion. Changed code after review goes through the local/hosted/review gates again. After verified integration and required result publication, the host may mark the child complete in Linear.

### 8.4 Evidenced no-change completion

When requested behavior already exists, a child MAY complete without a new commit, branch, empty PR or staging merge. It MUST still establish its scope and acceptance criteria against exact repository revisions, run applicable local tests and the complete configured GitHub-hosted validation route for those revisions, and receive Codex review of the claimed no-change outcome. Record why change-only PR/integration gates are inapplicable; do not label nonexistent gates “passed.”

The review/result record is written to the child Linear issue when there is no PR, with the same authenticated provenance and links to GitHub validation. Independent PR review remains separate. A no-change child cannot silently rely on another parent's unreleased staged feature: that dependency must be declared and satisfied in the isolated parent candidate. Required test/documentation outcomes still count toward parent completeness. If the entire accepted outcome is already in production, record `no-release-required` after parent integrated validation and human acceptance, without manufacturing deployment activity.

### 8.5 Isolated integrated evidence

Parent evidence MUST refer to the actual isolated candidate assembled by §9.1, not a shared staging environment containing additional parents. The E2E child defines and runs integrated checks against the exact repository tree vector, declared test resources and candidate deployment/artifact identities. It also validates relevant integration after staging. If isolation exposes a missing dependency or failure, readiness remains blocked until scoped child work resolves it.

The evidence manifest MUST include:

- Parent ID, authorization/definition/correction generation and complete required-child inventory, with each child's outcome and any justified no-change case.
- Executive summary, touched repositories/components, fulfilled acceptance criteria, remaining caveats and explicit go/no-go assessment.
- Actual local, hosted CI, Codex review, staging and integrated-validation results, commands/check identities, tested revisions, run attempts and artifact links; skipped/unavailable checks named honestly.
- Production baseline, ordered selected delta digests, resulting candidate commit/tree vector, dependencies and production targets.
- Visual/nonvisual classification; applicable video asset identity/digest and its candidate/validation binding.
- Evidence schema/digest, publication operation, creation time and supersession links.

The execution host MUST automatically upload visual E2E video to Linear and associate it with the parent. Use Linear's documented upload flow and returned upload headers, then publish the resulting authenticated asset reference. A successful file upload without parent association is incomplete. The video MUST demonstrate the changed end-to-end flow, with safe test data, against the same candidate represented by the other evidence. Unrelated demonstrations and obsolete recordings do not satisfy the gate. Nonvisual changes require meaningful summaries and actual validation evidence, not artificial videos/example outputs.

V0 MUST produce a standard MP4 recording (H.264 video, optional AAC audio) and validate that its recording pipeline and Linear upload/playback work in the live integration profile. Observe actual provider size/type constraints; if the result is rejected or unusable, block evidence publication with the reason. Do not truncate away the required flow, silently use an unapproved public host, or claim an unviewable upload is ready. Retain the local recording until verified remote association/access and required recovery no longer need it. Linear is the review surface; the CLI opens it on the reviewing device.

### 8.6 Human acceptance and stale results

The host publishes a current parent evidence token derived from the canonical manifest and generation. `approve` MUST supply that exact token, authenticate a permitted acceptor and revalidate completeness, current corrections, required scope and evidence. It MUST write a signed acceptance record to Linear with the human actor, original reviewed token, time and generation, then perform the configured parent closure. A comment saying “approved,” a tracker Done state, an agent go/no-go or a successful run is not an acceptance record.

Acceptance and Linear closure are separate recoverable writes; a lost response cannot create a second approval or imply that closure succeeded. An acceptance record becomes usable only after observed publication and a final generation check. `open`/inspection has no acceptance side effect. Missing, stale or superseded tokens return `stale_result` and the current review reference; do not silently approve a newer result.

Acceptance authorizes the parent result, not immediate production. Manual mode still requires an explicit eligible single-parent release command. Automatic mode may admit release only through all normal gates, including RUNNING intent and filters. Acceptance while paused/stopped remains a decision recorded in Linear but cannot start execution or release. New corrections invalidate release use of that acceptance, including when the patch bytes happen to be unchanged.

## 9. Single-slice release algorithm [V0-09]

### 9.1 Candidate construction

The implementation MUST use this supported algorithm; promoting the staging branch wholesale is forbidden.

1. Resolve the current parent generation, every required child responsibility, declared dependencies and recorded deltas. Require implementation/test-code/documentation deltas to be validated and staged, and test plan/resources to be ready; E2E execution and final evidence may still be pending. Include testing/documentation/no-change responsibilities in eventual completeness even when their patch list is empty. Missing, duplicated, cyclic or ambiguous provenance blocks assembly.
2. For each affected repository, capture current production `main` SHA/tree and separately the target's deployed revision/artifact. Verify their known relationship. Unexplained divergence blocks ordinary assembly. A completely reconciled partial release may use the explicit successor-recovery baseline in §9.6, which records both states without pretending the pending deployment succeeded.
3. Select only this parent's not-yet-integrated production deltas, including corrective deltas and explicitly reviewed replacements under §9.1.1. Track production integration and deployment per repository/target step; do not reapply a delta already observed in `main` merely because another step left the whole release partial. Earlier observed released changes are part of the baseline. Order deltas by explicit child dependencies and, within a repository, recorded integration ancestry. If that does not establish a unique necessary order, block rather than guess. Record the ordered delta list/digests before applying anything.
4. Create a fresh isolated candidate from the captured `main`. Apply the recorded patch bytes unchanged, in order, using Git's binary patch application with whitespace preservation, no whitespace fixing/ignoring, no three-way conflict resolution and no manual edits. Exact-context application with line offsets is permitted; fuzz, dropped hunks, changed paths/modes/binaries and added patch content are not. Binary changes require their exact supported preimage. Any failed application returns to scoped child work or a declared dependency; the release coordinator cannot repair it.
5. Verify the resulting worktree/index contains exactly the selected operations and no ambient/untracked changes. Commit the candidate with recorded provenance, then run the required isolated integrated validation and produce §8.5 evidence. No commit/PR is created when the whole selected outcome is already satisfied in production and §8.4 proves it.
6. Record candidate baselines, tree/commit vector, ordered patch artifact hashes, dependency snapshot, validation/evidence token and planned target steps. This candidate is the human-review subject.

For rework after an earlier release, retain the original parent and history but select only the new correction generation's unreleased deltas. A permanent per-parent `released` bit MUST NOT suppress later legitimate corrections. If an older delta cannot be retrieved or attributed, block; do not recover it by copying a combined staging tree.

### 9.1.1 Explicit replacement deltas

Appending a corrective staging patch cannot repair an earlier patch that fails before it can be applied. In that case a scoped child MUST produce an explicit replacement delta against the isolated production baseline, within its existing write boundary. Record the replaced delta IDs, the original contributions accounted for, the replacement base/head/patch digest and why the replacement is necessary. Other children's contributions cannot be silently dropped or rewritten; if replacement spans their write boundaries, route the corresponding work through each affected child and record a complete mapping.

Replacement creation is implementation work, never a controller/parent edit. It MUST pass required local tests, full exact-revision GitHub-hosted validation and Codex review. Post replacement findings/provenance on the existing related child PR and record the mapping in Linear; use an owned validation branch/commit and configured exact-SHA CI route where a new PR would be empty. Any actual staging code change still requires a normal child PR and successful staging integration. If staging already has the required resulting behavior, the child MUST prove that no additional staging change is needed under §8.4 rather than create an empty merge. Both staged result and isolated replacement result need explicit evidence.

Only a verified replacement mapping may mark old artifacts superseded. Retain them as history; account for all required child outcomes and validate the newly assembled parent result. A changed selected patch list invalidates prior parent acceptance, even if intended behavior is similar. This is the normal scoped correction path and does not expand repository/write authority or waive review/staging gates.

### 9.2 Acceptance carry across production changes

The operator explicitly permits acceptance to carry for an unchanged slice patch after fresh integrated validation. This exception MUST be narrow and observable:

- The parent definition, correction generation, required-child scope/satisfaction, selected implementation deltas, intended behavior and authority are unchanged. Fresh test/evidence records necessarily change and are not required to be byte-identical.
- The ordered selected delta artifacts, paths, modes and contents are byte-identical to the accepted manifest. Git patch-ID equality alone is insufficient.
- The new baseline contains only known released prerequisites/other already released work; no extra unreleased parent is selected.
- The same unmodified patches apply cleanly under §9.1, without conflict-resolution or product/test edits.
- Fresh integrated validation, actual candidate evidence (including visual video when applicable), and all production tests pass for the new result.

Record a `carried_acceptance` link from the original human-reviewed token to the new candidate/evidence token with the unchanged-patch proof and new validation results. Preserve who accepted which original result. Do not imply the human personally reviewed the new recording/result. The host can establish this lineage only under the stated exception; every changed intent, added delta, conflict fix or necessary code/test change requires scoped work, renewed evidence and fresh human acceptance.

Before a release command the CLI displays the original acceptance and current validated candidate/lineage. The release token names the current candidate, not a stale token silently remapped to it. Automatic release may use valid carried acceptance, but all remaining gates still apply.

### 9.3 Eligibility, queues and spacing

Preparation and irreversible delivery have separate admission predicates. Authorized, included work under RUNNING intent may admit bounded candidate assembly, isolated validation, production-PR creation and full production CI before final human acceptance/manual release. These preparatory operations MUST NOT merge to production or deploy. They are ordinary new operations subject to pause, capacity, scope and recovery gates, not a release-authorization bypass. This permits E2E completion and, after a baseline change, the fresh tests needed to establish carry lineage before final delivery admission.

A parent is ready for release only when: current authorized included scope is complete; dependencies are satisfied; current evidence has explicit acceptance or valid carry lineage; required production tests and derived review evidence are current; operator intent is RUNNING; no affected uncertainty/hold exists (except its explicitly assigned successor-recovery hold under §9.6); repository policy and credentials are usable; and spacing permits. Actual release admission additionally needs the manual command when in manual mode. That command supplies authorization for exactly one current eligible candidate and cannot batch other parents.

Release resource keys are canonical repository IDs plus configured production environment/target IDs. An attempt acquires all its keys atomically in stable sorted order. Attempts with overlapping keys serialize across all Kegari workspaces. Disjoint target sets have independent queues and may release concurrently. Parent dependencies additionally constrain admission even when physical targets are disjoint.

Within eligible work, order by dependency readiness, acceptance time (oldest first) and canonical parent ID. An ineligible item does not force unrelated eligible targets to wait. A partial/uncertain hold is different: no overlapping or dependent release may pass it. This is independent queues with explicit shared-resource serialization, not installation-wide serialization or active-active coordinators.

Spacing is measured from the observed successful completion of the previous overlapping parent release to the next release start. The applicable delay is the maximum configured spacing of the previous and next workspace/target policy; zero permits back-to-back release. Failed/partial/unknown attempts create a hold, not a successful-completion timestamp. No-release-required records do not publish production changes or consume a release interval. Use recorded UTC completion instants and conservative clock-skew handling; a backward clock change cannot shorten the wait.

### 9.4 Production PRs and delivery steps

For each repository with a nonempty selected change, automatically open an owned production PR from the isolated candidate to `main` under the configured GitHub user. It MUST rerun the full configured production CI suite on GitHub-hosted runners against the actual proposed release result. Staging CI does not substitute. Code that is unchanged from its valid reviewed delta MUST NOT receive a routine second Codex review solely because it is being released; required repository protections still apply. Any new product/test edits return to scoped child work and its normal review gates.

These PR/check operations belong to preparation (§9.3), so they can establish final eligibility without a circular requirement for an already admitted release. For the new production candidate head, the controller MUST publish a derived `kegari/codex-review` status and authenticated attestation. It references every applicable original/replacement child review, exact selected patch digest, generation and assembly proof, and explicitly states that it carries forward existing reviews rather than reporting a new Codex review. Only unchanged, completely accounted-for reviewed deltas qualify. An invalid/missing source review, additional code or changed scope prevents the derived pass. This satisfies the new head's review context without duplicate routine review or a gate waiver.

Before starting irreversible steps, the host MUST build and record a finite release plan: repositories, expected base/head/candidate trees, PRs, CI results, selected delta hashes, deployment workflow IDs/revisions/inputs, exact artifact/source bindings, target IDs and a dependency DAG of merge/deploy/verification steps. Operator repository configuration supplies this delivery contract. Cycles, missing ordering or unspecified production behavior block release; an agent cannot improvise infrastructure commands.

Every production PR uses squash merge, expected-head validation and current-base protections as in §8.3. If any production base changes, stop before mutation, reassemble and validate; apply §9.2 or require renewed acceptance as appropriate. An expected head SHA does not protect the base by itself, so server-side up-to-date required checks are a prerequisite. After merge, observe the actual commit/tree and verify it equals the tested candidate. A mismatch is a partial/uncertain release failure, never success.

Deployment steps MUST invoke only configured, permitted GitHub workflows with exact source/artifact revision bindings and verified ownership. A workflow accepting merely a moving branch name without a verifiable exact-result contract is incompatible. Remote queued/running jobs remain pending. Record workflow run IDs and attempts, deployment IDs, artifact/source digests and actual environment results. A repository configured with no deployment completes its delivery at verified production integration; otherwise production merge alone is not delivery success.

Cross-repository releases are not atomic transactions. Execute the declared step DAG, observing each prerequisite before dependent steps. Whole-parent success requires every required repository merge/deployment/verification step to succeed. Store per-step effects so recovery can continue or report partial delivery without repeating completed steps.

### 9.5 Freshness and correction races

Immediately before each irreversible step, revalidate current authority, child/correction/evidence generations, acceptance/carry lineage, exact Git revisions, resource locks and release holds. If facts cannot be refreshed, do not start the step. Once a valid release attempt is active, pause/graceful stop alone allows its bounded plan to drain; a new correction, revocation, failure or force-stop prevents remaining stale steps.

There is no atomic transaction spanning Linear comments, GitHub merge and a deployment provider. Do not claim that a comment not yet observable to Kegari can cancel a request already accepted remotely. On observing a correction, preserve all observed effects, request cancellation of owned in-flight jobs where permitted, mark unknown outcomes, and hold affected/dependent delivery until reconciled. New code/test corrections still run only through children. A later operator decision cannot make stale evidence retroactively valid.

### 9.6 Partial delivery, holds and reconciliation

Any failed or uncertain release step holds its affected repositories/environments and dependent releases. Unrelated disjoint applications continue. Publish a parent release record with per-step succeeded/failed/pending/unknown outcomes, exact revisions, cancellation observations, remaining work and operator action needed. Do not mark the parent released because one repository succeeded.

There is no automatic rollback, production incident diagnosis or production-monitoring policy. An operator can pause, force-stop, investigate and arrange separately authorized recovery. `operation reconcile` may reread and classify provider facts; it cannot fabricate success or waive checks. `release recover` supports these explicit operator-directed transitions:

- `resume`: continue only verified uncompleted steps of the same still-current accepted result, after fresh gates. Already integrated/deployed steps are not repeated.
- `abandon`: retire known unstarted steps, retain completed/failed facts and the hold. It cannot cancel an unknown in-flight action by assertion.
- `resolve`: clear a known-failure hold only after all remote actions/cancellations have known terminal outcomes, the actual main/deployed baseline vector is verified and coherent, and the operator records the disposition/reason. Preserve the failed/abandoned history and unsatisfied dependent requirements; clearing a hold does not mark the parent released. An incoherent or unknown baseline cannot be resolved by this command.
- `supersede`: assign the predecessor hold to a specified corrected generation/current candidate of the same parent. Require all old remote outcomes to be known, fence/retire stale unstarted steps, record exact per-target partial integration/deployment state and the successor plan, and require the corrected result's scoped child gates, fresh integrated/production validation and new human acceptance. The successor alone may proceed under that assigned hold; overlapping/dependent releases remain held until successful recovery or explicit resolution. It may use the reconciled main/deployed divergence as a declared recovery baseline, never as proof of successful deployment. It MUST NOT deploy a known superseded artifact merely to make the old plan appear complete.

Preparation of a corrected successor while a known hold exists is permitted under normal execution authority once all potentially conflicting remote actions are reconciled. It creates no production effects. This allows fixes/tests/evidence/acceptance before the explicit `supersede` delivery action. Unknown outcomes still block affected preparation where its baseline or safety depends on them. A successor cannot contain unrelated/unapproved work or gain new scope through recovery.

Unknown outcomes cannot be cleared by a “mark successful” override. If external operator repairs changed code, they need traceable authorized child/review/evidence treatment before dependent delivery resumes. A completely reconciled partial release's already delivered individual deltas remain recorded even when its whole-parent outcome is failed; successor assembly accounts for those exact effects rather than replaying them.

When a remote cancellation fails or cannot be confirmed, retain the hold. An acknowledged cancellation request is not a cancelled job. A job may complete while cancellation is requested; record the observed completion without claiming rollback. A no-longer-owned human rerun is not a cancellation target.

## 10. CLI and control transport [V0-10]

### 10.1 Command grammar and responses

The executable MUST support `kegari-v0 [--target NAME] [--json] COMMAND`. No command means `overview`. An omitted target addresses the registered local runtime; an explicit target selects exactly one client configuration. No unreachable-target fallback is permitted. `--workspace ID` narrows inspection/workspace configuration only; lifecycle operations always affect the entire selected runtime and MUST reject that flag.

First-run `setup` is a local-only bootstrap path when no registered control host/configuration exists. Acquire the exclusive `control.lock` before creating validated initial configuration, Keychain entries and STOPPED intent; do not spawn orchestration. A partially completed setup is reported/reconciled, not inferred complete. Once a control host exists, all subsequent runtime configuration uses its serialized dispatcher. Remote setup may configure an already installed/reachable target but MUST NOT bootstrap a local substitute or an uninstalled remote machine. `service install` and `service start` are explicit local operator actions.

Parent references are `<workspace-id-or-unique-name>/<Linear-UUID-or-key>`. An unqualified reference is allowed only when unique among the targeted runtime's configured workspaces. Ambiguous input returns candidates without mutation. Names/keys are resolved to canonical IDs before authorization and receipt creation.

Human output MUST show target/runtime, affected workspace/parent, actual state, operation/request ID where relevant, result/reason and next action. `--json` emits one UTF-8 JSON response envelope on stdout; diagnostics go to stderr. Envelope fields are `api_version`, `request_id`, `runtime_id`, `config_revision`, `observed_at`, `ok`, `operation_id`, `state`, `data`, `error`. Absent optional values are null. `error` contains stable `code`, safe `message`, `retryable`, `details` and `next_action`. Secrets MUST be omitted, not partially revealed.

Every mutation accepts `--request-id UUID`; without it the client creates one once and retains/returns it across transport retries. The server binds that ID to authenticated actor, operation kind and immutable payload digest. An identical replay returns the existing operation/result; changed payload/actor returns `request_conflict`. Local receipt idempotency does not imply a provider's API is idempotent. If the outcome is unknown, return the ID and uncertainty; do not generate another ID and repeat the mutation.

Lifecycle mutations additionally accept `--expected-control REV`, mapped to `expected_control_generation` in the API. When omitted, the client first obtains the current generation and submits it, so a concurrent newer lifecycle decision causes a stale-generation conflict rather than being overwritten silently. Durable acknowledgements return the resulting generation.

| Command | Required inputs and behavior |
| --- | --- |
| `overview [--workspace ID]` | Complete four-section view (§10.2), no dispatch, approval or release side effect |
| `status` | Control-host/orchestration versions and actual/desired states, prerequisites, freshness, draining/cancellation/reconciliation and unresolved effects; works while orchestration is stopped |
| `show PARENT` | Full parent/child gate states, evidence manifest/token, acceptance lineage, dependencies, release steps and blockers |
| `open PARENT` | Obtain Linear URL/current evidence token; open browser on the reviewing client, never the execution host for a remote command. Browser failure does not alter workflow. `--url-only` prints/returns the URL without launching a browser. |
| `approve PARENT --result TOKEN` | Explicit human acceptance under §8.6; missing/stale token or incomplete gates fails without approving another result |
| `correct PARENT (--message TEXT \| --message-file PATH \| --stdin)` | Exactly one nonempty UTF-8 input, at most 1 MiB; create one parent correction in Linear, return comment/operation IDs, invalidate superseded approval, and queue permitted routing. No new readiness action. File is read on the CLI device. |
| `release PARENT --result TOKEN` | Manual mode: authorize/admit exactly the current accepted, eligible candidate. In automatic mode return the existing eligible release operation or its normal queued state; never duplicate it. Ineligible requests return reasons and do not create a latent release authorization for changed work. |
| `setup [--from PATH]` | Guided setup or complete noninteractive TOML input; same validation. Initial intent STOPPED. Does not dispatch agents. Authenticate connections through supported provider flows. |
| `connection list` | Names/IDs, kinds, verified identities and redacted health/reuse |
| `connection add NAME --kind KIND [--from PATH] [--secret-stdin]` | Guided missing fields or supplied typed connection TOML. In noninteractive mode missing fields fail. Secret input has a distinct stdin path; cannot share stdin with config. |
| `connection reconnect NAME` | Reauthenticate same account; changed verified identity requires explicit configuration replacement/reconciliation, not silent substitution |
| `workspace list` | Workspace IDs/names, connection identities, scope, mode and health |
| `workspace add NAME --from PATH` | Typed workspace TOML; guided equivalent when `--from` omitted in an interactive terminal |
| `workspace configure ID --from PATH --expected-config REV` | Atomically replace that workspace's validated settings; preserve unrelated workspace configuration; stale revision fails |
| `config show [--workspace ID]` | Effective non-secret values, provenance and revision |
| `config check [--workspace ID]` | Read-only validation of config, accounts, profiles, policies, workflows, identity, CI/review/release and isolation prerequisites; report verified/unverified/blocked separately; no agents/test execution/provider mutations |
| `pause` / `resume` | Durable state changes under §11.1; no per-workspace lifecycle meaning |
| `stop [--wait] [--timeout SECONDS]` | Durable STOPPED, drain, then exit orchestration. Waiting timeout reports still draining and never forces. |
| `stop --force [--wait] [--timeout SECONDS]` | Durable STOPPED, terminate owned local work, bounded remote cancellation, preserved files and honest outcomes |
| `start [--wait] [--timeout SECONDS]` | Explicit start/reconciliation; already running idempotent; paused remains paused and reports `requires_resume` |
| `operation show UUID` | Current receipt and observed provider outcomes, including partial/unknown states |
| `operation reconcile UUID` | Authenticated recover role; refresh/classify provider facts, no blind mutation replay or fabricated success |
| `retry PARENT/CHILD --reason TEXT` | One explicit new bounded phase budget after blocker reconciliation; preserve scope/approval gates; log reason/actor on authoritative work record |
| `release recover PARENT --operation UUID --action resume\|abandon\|resolve\|supersede --reason TEXT [--result TOKEN]` | Explicit §9.6 recovery; `supersede` requires the current corrected candidate token/new acceptance. `resolve` verifies known coherent production facts before clearing a hold; no action fabricates success or waives unknown outcomes. |
| `cleanup --operation UUID` | Show eligible paths, retained evidence and deletion predicates; no removal |
| `cleanup --operation UUID --apply --plan TOKEN` | Apply exactly the unchanged inspected cleanup plan under §11.5; no deletion when unsafe/stale |
| `target add NAME --url HTTPS-ORIGIN --runtime UUID` / `target list` | Client-only target preferences; certificate/identity verification required before control, no auto-pinning after mismatch |
| `service install --binary ABSOLUTE-PATH` | Explicit local operator setup of the per-user launchd control service for that immutable installed binary; preserve desired intent; no package/tap deployment |
| `service start` | Explicit local idempotent start of the registered control host/LaunchAgent, preserving desired orchestration intent; no kill/restart of an existing host and no duplicate scheduler |
| `service stop` | Explicit local control-host shutdown only after orchestration is stopped and no active owned local work remains; remote control becomes unavailable |

`--noninteractive` MUST be available for setup/configuration/authentication commands. It never prompts; missing information returns a validation/authentication-required result. Guided and noninteractive routes MUST use the same typed configuration transaction. Routine read commands MUST NOT launch agents or a second scheduler. Human decision writes remain available through the control host when orchestration is stopped; autonomous correction interpretation waits for RUNNING admission.

Exit codes: `0` successful read or durably accepted mutation (pending versus completed stated explicitly); `2` invalid command/input; `3` invalid config/unmet prerequisite; `4` unavailable target/provider or transport timeout; `5` authentication/authorization denied; `6` stale token/config, ownership/identity/request conflict or incompatible version; `7` observed failed/partial/unknown operation. A mutation timeout always includes its request/operation identity where known and uncertainty, even when exit 4 is used. `--wait` defaults to 30 seconds; timeout ends client waiting, not the operation.

### 10.2 Overview and next action

Always show these sections, including empty sections, in this exact order:

1. **Ready for release** — accepted current candidates whose release admission gates are satisfied, including manual ones awaiting the explicit release command. An active release moves to Running.
2. **Ready for review** — complete current parent result/evidence awaiting explicit human acceptance.
3. **Blocked** — blockers and waits requiring a gate to change, including accepted slices waiting for dependency, spacing, paused/stopped intent, exclusion, permission, quota, uncertain delivery or fresh validation. Label waits accurately; do not imply failure when merely waiting.
4. **Running** — active execution/validation/integration/release and eligible queued child work waiting only for an available execution slot, with actual queued/active phase shown.

Rows MUST include workspace/parent identity and Linear link, relevant child progress, current phase, freshness, blocker/wait reason and acceptance/release distinction. Required children cannot disappear from counts. Successfully released/no-release-required history is summarized separately with links and available through `show`; it does not clutter active categories or erase historical outcomes. Independent PR-review jobs and runtime health have clearly labeled supplementary rows without pretending to be parent slices.

Highlight the next useful operator action: first an eligible manual release, then a ready parent review, then the actionable root blocker holding the next otherwise releasable parent. Use dependency chain analysis where available; otherwise say the root cause is unknown rather than choose an unrelated running task. Ties use acceptance/readiness age, Linear priority and canonical ID. Automatic release processing is shown as automatic/queued rather than falsely requiring a manual action. Workspace filtering preserves all four sections and ordering. Displaying status never changes workflow.

### 10.3 HTTPS and local API

The control host MUST use Axum with direct TLS and bind only explicitly configured, currently verified Tailscale IPv4/IPv6 addresses. It MUST NOT listen on wildcard, LAN or public addresses or fall back to plaintext. The operator provisions a valid certificate/key for the full expected tailnet DNS hostname. Validate key/certificate match, hostname coverage, validity, file ownership/permissions and TLS chain configuration. Clients MUST perform normal certificate-chain/hostname verification and pin expected runtime UUID/API version. No insecure mode, automatic redirect, runtime repinning or alternate-target fallback is permitted.

Remote actor identity MUST derive solely from the accepted TCP peer preserved through TLS and verified by the installed supported Tailscale CLI using `whois --json --proto=tcp <peer-ip:port>`. IPv6 formatting must preserve the peer/port correctly. Require a direct recognized tailnet user identity mapped to permitted operator roles; tagged-device identities do not establish human acceptance authority. Check identity/permission on every privileged request; unavailable verification fails closed. Do not authenticate from `Forwarded`, `X-Forwarded-For`, `Tailscale-User-*`, payload usernames or caller-supplied peer addresses. Proxies, Tailscale Serve and ambiguous source identity are outside this direct-TLS profile.

Setup MUST document user-managed Tailscale enrollment/access policy, certificate issuance and renewal. Certificate issuance can publish the machine/tailnet DNS name in Certificate Transparency. Kegari does not configure tailnet policy or issue/renew certificates automatically. Watch certificate/key replacement and atomically reload a validated matching pair. Keep the old pair only while still valid; report malformed replacement/expiry. With no valid pair, disable remote listening while retaining local control and existing authorized orchestration. Inspect actual expiry; do not hard-code certificate lifetime.

Local control uses the owner-protected Unix socket and verified OS peer identity, mapped to the same operator roles. Managed workers MUST be denied this socket and privileged remote endpoint by the enforced execution boundary. If the installed sandbox/host policy cannot enforce that boundary, unattended execution preflight fails; identity of a shared device is not proof that a human made a decision.

Expose these authenticated API routes over both transports, sharing one dispatcher:

| Route | Contract |
| --- | --- |
| `GET /v1/status` | Runtime/control health and desired/actual states |
| `GET /v1/overview?workspace=ID` | Read-only overview snapshot |
| `GET /v1/parents/WORKSPACE/ISSUE` | Canonically resolved parent detail/evidence |
| `GET /v1/operations/UUID` | Operation observation |
| `POST /v1/commands` | `{api_version:1, runtime_id, request_id, command, args, expected_config_revision?, expected_control_generation?}`; closed command enum corresponding to server-side CLI actions above |

No arbitrary shell-command, URL-proxy or provider-query endpoint is permitted. Client-only browser opening, target preferences and local service installation/shutdown are not remote shell commands. Read command names that use the POST envelope MUST remain read-only and cannot create work receipts/dispatch as a side effect.

Require `X-Kegari-Protocol: 1`, expected Host/authority and expected runtime identity on every request; POST requires `application/json`. Reject browser `Origin` headers (including `null`) and preflight requests; provide no cookies, browser sessions, permissive CORS, HTML/static assets or WebSocket dependency. The API is for CLI clients. Maximum request/response body is 8 MiB, maximum headers 32 KiB, and request/identity lookup deadline 10 seconds. Long operations return an operation ID within that deadline; they are observed separately. Media uploads bypass this API and use the execution host's Linear adapter. HTTP errors use the same safe envelope: 400 input, 401 authentication, 403 authority, 404 unknown resource, 409 stale/conflict, 413 size, 429 capacity/rate limit, 503 unavailable. Do not use a request timeout as permission to replay an unknown mutation.

Collection/detail responses MUST paginate instead of silently truncating. The initial read creates an in-memory immutable projection with `snapshot_id`, `observed_at`, `expires_at` (five minutes), and an opaque `next_cursor`; subsequent requests use `snapshot_id`/`cursor` query parameters. A cursor is bound to authenticated actor, target, filters and projection digest. Pages contain at most 100 entries and stay below the byte limit; nested child/evidence/run collections use typed entries with canonical parent references, so no required child is omitted to fit a page. Large textual fields use ordered UTF-8 chunks with byte offsets and a complete-content digest. The CLI retrieves/verifies every page before emitting its single complete output envelope or reports an incomplete-snapshot error. Expired snapshots return 409 `snapshot_expired`; restart the read, never splice revisions together. Local CLI uses this same contract. Pagination is transient memory, not a database or authoritative workflow store.

## 11. Lifecycle, recovery, retention and observability [V0-11]

### 11.1 Durable lifecycle transitions

The control host MUST close the serialized admission gate when pausing/stopping, persist intended state and increment control generation, then acknowledge. Resume/start opens admission only after durable RUNNING and required reconciliation. The permit transaction in §6.1 prevents scheduler races across processes. A missing first-run intent initializes to STOPPED through explicit setup; unexpected missing/corrupt intent during recovery blocks autonomous work.

| Command/event | Transition and behavior |
| --- | --- |
| `pause` from RUNNING | Persist PAUSED, atomically close admission, drain existing operations; status and human decisions remain available |
| `pause` when PAUSED | Idempotent |
| `pause` when STOPPED | Preserve STOPPED; report already stopped |
| `resume` from PAUSED | Persist RUNNING, reconcile, then admit eligible work |
| `resume` from STOPPED | `requires_start`; do not start |
| `start` from STOPPED | If an earlier stop is still draining or owned local shutdown is incomplete, return `stop_in_progress` and preserve STOPPED. Otherwise persist RUNNING, establish one orchestration process and reconcile before admission. |
| `start` when RUNNING | Return existing operation/runtime; no duplicate process or queue |
| `start` when PAUSED | Preserve PAUSED and return `requires_resume` |
| Graceful `stop` | Persist STOPPED, prevent new admissions, drain active operations, exit orchestration; default acknowledgement says draining until it actually exits |
| `stop --force` | Persist STOPPED, fence autonomous broker authority, terminate owned local work, request bounded owned remote cancellation; preserve files |
| Restart/login with saved RUNNING | Start control host; start/recover orchestration only after prerequisites/ownership/reconciliation permit |
| Restart/login with saved PAUSED | Control/status may restore, but no new runs/releases until explicit resume |
| Restart/login with saved STOPPED | Control host may restore; orchestration MUST NOT start until explicit start |

Concurrent local/remote lifecycle commands MUST serialize by control generation. Acknowledgements state the resulting intent and operation identity. A stale command with an expected generation MUST fail rather than reverse a newer decision. A crash after persisting STOPPED but before finishing drain MUST NOT restart work to finish draining automatically; recover observations without admitting new execution.

The per-user LaunchAgent starts the control host after login without an open terminal. It may supervise/restart that host but MUST NOT force-start orchestration contrary to persisted intent. Locking the logged-in session is not a stop command. Validate actual credentials/Codex/Tailscale availability while locked. Sleep/log-out/power loss interrupt availability; on wake/restart reconcile elapsed time and provider facts. No pre-login, disk-unlock, keep-awake or security-weakening guarantee is required.

### 11.2 Force-stop and cancellation

Force-stop MUST close admission and revoke worker mutation leases first, signal only verified instance-owned process groups, then terminate remaining owned processes after a five-second local grace period. Ownership checks use OS process handles/start identities and recorded groups, not a PID alone. Do not kill unrelated reused PIDs. Remote failure cannot prevent local termination or durable STOPPED.

Request cancellation only of identifiable instance-owned GitHub CI/deployment run attempts whose current ownership is verified. The initial cancellation pass is bounded to 15 seconds total; remaining outcomes stay pending/unknown in receipts and status. The control host may continue read-only reconciliation. Report requested, confirmed-cancelled, failed, already-completed and unknown separately. Never claim local shutdown proves remote termination or rollback. Preserved files and incomplete changes remain available for recovery.

### 11.3 Recovery and uncertain actions

Before external mutation, persist the prepared receipt and payload/generation identity; mark transmission and returned IDs as observed. Following a crash/lost response, query the provider using known IDs or trustworthy operation correlation before allowing another mutation. Comments/PR labels/branch names can carry correlation, but are not automatically provider-enforced idempotency keys. Exactly-once remote effects are not promised.

Adopt an existing remote object only when exactly one matching object, target, actor/ownership and immutable payload can be verified. Multiple matches are a conflict. A failed/incomplete query or no match under eventual consistency is not proof the action never happened. When the provider cannot establish the outcome, block that action and dependent/overlapping effects; unrelated work continues. A request ID replay returns the existing uncertainty, not another external write.

Recovery MUST reconstruct current state from Linear/GitHub, verify local receipts and preserved directories, restore attempt counts, operator intent and unresolved provider-intervention blockers under §7.3, and verify whether old processes are still alive/owned. It MUST NOT launch replacement work merely because its in-memory map is empty. Successful remote work may be adopted with its evidence; stale generation evidence is historical only. Local run state, Linear state and GitHub facts that disagree produce a specific reconciliation blocker, not optimistic success.

There is no automatic cross-host failover or takeover. Local locking prevents duplicate local owners under the supported deployment; it cannot prove an unrelated installation elsewhere is inactive. One active coordinator for overlapping configured work is a deployment prerequisite. Detection of another owner blocks new effects. A timeout or unreachable previous host is not evidence that it is fenced.

### 11.4 Failure reporting and planning summary

Every blocker MUST identify canonical affected work, reason, impacted dependents/gates, last verified evidence, attempted recovery and a concrete next planning/operator action. Publish it to the relevant Linear child/parent when access permits; retain the local receipt/status if publication is unavailable and retry that known write conservatively. Runtime execution MUST NOT open an interactive clarification dialogue. The overview and `show` constitute the next planning summary, with relevant blockers retained until resolved or explicitly superseded.

Configuration/credential failures SHOULD be contained to the affected connection/workspace; global ownership, unsafe storage or broken mutation brokerage blocks the whole runtime. Independent authorized work MUST continue where its prerequisites and boundaries remain valid. Error handling MUST never turn refusal, missing data, quota exhaustion, stalled work, cancelled tests or uncertain effects into completion.

### 11.5 Retention and cleanup

Default retention is preserve. Startup, terminal tracker states, claim release, force-stop, successful release and account removal MUST NOT automatically delete execution directories, local evidence needed for recovery or unresolved receipts. Logs may rotate at 10 MiB per file with ten retained files; authoritative evidence and receipts are not logs and cannot be discarded by rotation.

Explicit cleanup first returns a digest-bound plan of exact paths and proof of eligibility. Applying that unchanged plan requires: no active/queued/uncertain operation references; no dirty/unpublished/unrecoverable work; all needed Git objects/deltas and evidence verifiably available from retained authoritative records; no future correction/recovery dependency on the local files; validated path containment/ownership; and successful `before_remove` hook where configured. Recheck predicates after the hook and immediately before removal. If any predicate is unknown, refuse removal and explain what must be preserved. Terminal issue state alone is never enough.

Completed receipts may be compacted only after their referenced authoritative result and duplicate-prevention tombstone are retained, all dependents/recovery uses are satisfied and no operation can be replayed ambiguously. Tombstones retain request/actor/payload identity and remote outcome reference; they are small recovery receipts, not replicated issue records. No automatic time-based deletion of these safety records is required. Disk pressure blocks new work/writes and surfaces cleanup guidance; it never authorizes removing unfinished changes.

### 11.6 Logs and snapshots

Emit structured logs with UTC time, severity, runtime/owner/workspace, canonical parent/child or PR, operation/session IDs, generation/revision, event, outcome and concise reason. Redact secrets and bound payloads; do not dump full prompts, tokens or arbitrary provider responses. Log-sink failure must remain visible through another status path and MUST NOT crash otherwise safe scheduling, but failure to persist a required receipt/intent MUST block dependent effects.

Snapshots MUST expose running/queued/retrying work, phases, attempt bounds, next retry, dependency/hold reasons, last semantic activity, account-specific usage/rate-limit freshness, current desired intent, release steps and provider visibility errors. A stale snapshot says so. Humanized agent summaries may aid inspection but cannot determine gates. Read/status operations time out explicitly rather than hanging indefinitely.

### 11.7 Explicit upgrades and handover

Before replacing the coordinator, the operator MUST stop/fence old scheduling, drain or force-stop owned workers, reconcile outstanding remote actions and retain all compatible config/intent/receipts/attestation keys and required evidence. Prevent the old installation's supervisor from automatically dispatching again. Unreachable or cancelled-without-confirmation is not sufficient fencing.

Validate the replacement and its version-bound runtime instruction assets independently against §12, including reading the persisted format without destructive migration. Unsupported schema versions MUST fail without altering/deleting source state. Activate a matching versioned immutable installed binary and asset set explicitly, register the new owner generation and reconcile before an explicit start. Preserve paused/stopped intent and the instruction provenance needed for existing evidence/recovery. A release of the successor only delivers code; it never grants installation/handover authority or restarts the coordinator automatically.

## 12. Observable acceptance and definition of done [V0-12]

### 12.1 Retained source scenarios

These scenarios are normative v0 tests, not claims that an implementation has already passed. A test harness MUST assert observable work admission, provider calls, exact revisions, persisted intent and user-visible outcomes, rather than merely mirroring internal functions. Deterministic fake providers and process fixtures cover enforcement, failures and races. The behavior evaluations in §12.4 assess actual agent use of the delivered instructions/context/tools. The separately authorized real integration profile confirms provider/host and end-to-end compatibility. These three forms of evidence are distinct and MUST NOT be presented as substitutes for one another.

| Source scenario | Required v0 observation |
| --- | --- |
| SC-01 | Authorized visual parent progresses through scoped children, local tests, full hosted CI, Codex review, staging and isolated E2E validation; parent has actual results/video before explicit acceptance. No manual PR creation or child-by-child human acceptance is required. |
| SC-02 | Backlog/profile edits never authorize work; running profile unchanged; next legitimate run uses new labels. Completed work does not rerun from a label edit. |
| SC-03 | Team/project/user rules narrow parents correctly; creator OR assignee; all children inherit; repository union and PR-author-only selection hold despite broad account access. |
| SC-04 | One permitted in-scope parent correction routes actionable existing-child comments, reruns only affected work, invalidates old acceptance and regenerates evidence. No replacement hierarchy/readiness toggle. |
| SC-05 | Correction edits/follow-ups immediately before, during and after completion are either steered or queued; no lost update, stale result acceptance or stale release. |
| SC-06 | Unexpected material ambiguity blocks affected work, appears in planning summary, asks no interactive question and allows independent work to continue. |
| SC-07 | Local success cannot bypass failed/missing hosted CI or Codex review; real staging integration and exact revision evidence are required. Exhausted correction bounds block honestly. |
| SC-08 | A satisfactory Codex review can pass without edits/refactoring. Evidenced no-change work can complete under §8.4 without fictitious PR/activity. |
| SC-09 | Nonvisual result has executive summary, touched scope, actual check results and go/no-go; no invented video/example output or unrun “passed” tests. |
| SC-10 | Staging contains accepted A and unapproved B. A's candidate/release contains only A's deltas atop released production, with full production tests; B remains excluded. Test both automatic/manual modes. |
| SC-11 | B requiring A cannot release first; each parent remains a separate release; overlap serialization and zero/nonzero spacing work; disjoint queues can proceed. |
| SC-12 | Multi-repository partial/unknown delivery identifies each effect, holds affected/dependent releases and leaves disjoint applications eligible. No automatic rollback or invented production monitoring. |
| SC-13 | Crash during PR/merge/release reconstructs remote facts without blind replay, preserves incomplete work and gates, and respects stored intent. |
| SC-14 | Concurrent conflicting slices/operator dirty checkout do not overwrite unrelated work; integration conflicts return to scoped work; unresolved conflicts stay visible. |
| SC-15 | Multiple workspaces share account allowance but not private context/write access; quota exhaustion affects that account, not unrelated accounts, with no paid fallback. |
| SC-16 | Independent review disabled means no run. Enabled matching PR gets bounded read/feedback-only review; malicious content cannot execute code, access secrets, fix/push/merge or broaden scope. |
| SC-17 | Automatic PR author is the selected verified GitHub user. Same-account Codex review works through authenticated status evidence, never self-approval; incompatible extra review requirements block. |
| SC-18 | Login/locked-session headless operation and local/HTTPS CLI address one runtime. Unauthorized identity and unreachable target fail safely; remote open uses client browser. Graphical/browser-dashboard clauses are deferred. |
| SC-19 | Parent coordination cannot implement, including single-repository slices. Missing children/E2E child block; inaccessible children stay in completeness. No parent implementation override exists. |
| SC-20 | V0 builds/operates from the existing Rust Cargo workspace without wrapper/frontend/Elixir; completes configuration→implementation→evidence→CLI acceptance/correction→single-slice release. Independent validation precedes successor coordination. Separate temporary repository wording is superseded. Stale review tokens fail; unchanged-patch acceptance carry obeys §9.2. |
| SC-21 | Guided and noninteractive setup produce equivalent validated config, show no secrets and never dispatch during checking. Provider-required auth remains explicit. |
| SC-22 | Children outside parent filters are included; disallowed repositories block rather than disappear. Later parent filter exclusion drains current operation and starts no child/retry/review/release. |
| SC-23 | Both all-workspace and filtered overview show all four sections in exact order, complete progress/reasons and a delivery-relevant next action. Accepted-but-ineligible is not ready for release; inspection has no workflow side effect. |
| SC-24 | Pause drains active operation but queues every follow-on session/release; acceptance during pause starts nothing. Graceful stop drains then exits orchestration, keeps control available and never silently forces. |
| SC-25 | Force-stop fences and kills only owned local groups, preserves files, cancels only owned GitHub attempts and distinguishes unknown from confirmed cancellation. Existing production effects remain recorded. |
| SC-26 | Restart/reboot retains RUNNING/PAUSED/STOPPED distinctions; running reconciles before automatic admission, pause requires resume, stop requires start. Remote/local clients observe the same state without duplicate queues. |

### 12.2 Additional contract and adversarial tests

| Stable v0 test ID | Test and required result |
| --- | --- |
| V0-T01 | Missing workflow, non-map/duplicate YAML, unknown keys/variables/filters, invalid paths/profile labels and invalid reload all produce named blockers; no fallback prompt, broadening or agent dispatch. A valid reload affects only next snapshots except revocation/corrections. Relative command/artifact paths obey §3.4 even when the workflow is outside the execution directory; escaping or unauthorized absolute artifact paths fail. |
| V0-T02 | Paginated parent/children/history/comments/checks include all pages; simulate failure/loops/partial GraphQL errors on the last page. No partial admission or fabricated terminal state. Empty ID refresh makes no request; duplicate IDs return one snapshot. |
| V0-T03 | Missing/null/unauthorized label actor, removed/reapplied label and ambiguous history block provenance. Latest authorized event works; old authorized history cannot legitimize a later unauthorized application. |
| V0-T04 | Removing authorization stops local workers and new mutations; ordinary filter exclusion instead drains. Re-inclusion preserves intent/budgets; account replacement does not adopt pending writes blindly. |
| V0-T05 | Add/delete/reparent an included child or widen scope after authorization. Required work remains visible, readiness invalidates and renewed authorization is required. Zero children and missing E2E always block. |
| V0-T06 | Two configured workspaces match the same canonical issue; no duplicate dispatch. Distinct work sharing repositories remains possible, with serialized integration/releases. Renames preserve identity/claims. |
| V0-T07 | Two local hosts/start commands race; only one control host and orchestration owner exist. Wrong runtime/TLS identity or unsupported API fails before mutation; remote failure never starts a local fallback. |
| V0-T08 | Crash at each intent/receipt durability boundary. Recovery uses a valid old/new record or blocks corruption, never fabricates permission. STOPPED persisted before crash remains stopped. Unsupported schema does not modify old data. |
| V0-T09 | Provider commits a comment/PR/merge/job then drops response. Reconcile one matching action without duplication; incomplete lookup/duplicate correlation blocks. Request-ID replay returns same result; changed payload/actor conflicts. |
| V0-T10 | Fragmented/coalesced JSON-RPC, interleaved tool requests, oversized frames, malformed events, stderr noise, startup timeout, silent/stalled turn and process exit yield correct bounded outcomes. Unsupported tools get responses; input-required never prompts the operator. Supported-schema verification/intervention signals followed by watchdog expiry or process exit retain the persistent blocker instead of entering ordinary retries; transient safety buffering followed by normal completion does not create that blocker. |
| V0-T11 | Steering races turn completion; latest correction watermark remains pending or handled, never lost. Profile edit alone neither steers nor reruns. Informational comments preserve acceptance; genuine/ambiguous corrective input cannot release stale work. |
| V0-T12 | Sandbox escape attempts through symlinks, absolute paths, sibling repositories, hooks, control socket/HTTPS and secret environments are denied. Parent cannot edit product/tests; independent PR code/hooks cannot execute. Cwd alone is not accepted as proof. |
| V0-T13 | Shared-account counters/quota reset and repeated cumulative usage events do not double count or multiply allowance. Fair scheduling admits multiple workspaces; capacity waits consume no failure attempt. |
| V0-T14 | Three attempts including initial, twenty-turn sessions and three review rounds terminate unresolved work as blocked. Restart/poll/profile edits do not reset bounds; a material correction/explicit reasoned retry creates only the defined new budget. An unresolved provider intervention survives restart and cannot be cleared by a timer, correction, profile change, fresh thread/account or retry command; supported resolution permits only normal admission while preserving intent and budgets. Unrelated authorized work continues. |
| V0-T15 | Forge a review status/attestation, change head/base after checks, or mismatch actual squash delta. No completion/merge based on fake/stale evidence. Existing protected-branch approvals remain gates; no self-approval assumption. |
| V0-T16 | A's patch needs staged B; isolated apply/validation blocks rather than incorporating B. Multiple A children and corrective deltas retain dependency/provenance order. Missing test-only/no-change child still blocks parent. |
| V0-T17 | Production changes after acceptance: identical ordered patch bytes cleanly applied plus fresh integrated/production validation permits recorded carry lineage. Changed intent, paths/modes/binaries, conflict edits or added work requires new acceptance. Human-reviewed original and newer machine evidence remain distinct. |
| V0-T18 | Video of shared A+B environment, wrong candidate recording, failed upload/association or rejected media cannot establish A's review readiness. Nonvisual evidence needs no artificial recording. |
| V0-T19 | Approve/release with stale token, forbidden actor, pending correction or wrong workspace fails without accepting another result. Agent “approved” and parent Done do not substitute. Decision writes work while paused/stopped without scheduling. |
| V0-T20 | Correction before release step stops it; correction after one cross-repo effect stops remaining stale steps and holds affected/dependent work. Unknown remote cancellation is not success. A later human rerun is not cancelled as owned. |
| V0-T21 | Already released parent receives in-scope correction: same hierarchy gains new generation, affected gates repeat and only new deltas release; historical release remains intact. Whole outcome already production records no-release-required after validation/acceptance. |
| V0-T22 | HTTPS rejects excluded/tagged identities, spoofed identity headers, browser Origin/preflight, bad Host/runtime/version, oversized payload and invalid certificate. Validate actual peer preservation, dual stack, TLS reload/expiry and locked-session Tailscale CLI access. Expiry disables remote access without plaintext fallback or stopping valid local work. |
| V0-T23 | Graceful wait timeout leaves drain intact; force-stop local termination succeeds despite network loss; PID reuse leaves unrelated process untouched. Control-host restart does not create another still-live scheduler. |
| V0-T24 | Cleanup rejects dirty/unpublished/evidence-dependent/unknown work, stale plan, escaping symlink and failed hook. Disk-full handling never removes unfinished files. Terminal Linear status alone deletes nothing. |
| V0-T25 | Building/releasing the successor in the same repository cannot replace the installed running binary. Handover without fencing old owner blocks; valid explicit handover preserves paused/stopped intent, receipts and attestation verification. |
| V0-T26 | Logs/status redact secrets and expose stale provider snapshots/errors. Duplicate token events and ended-session accounting remain accurate. A broken log sink is visible; a failed durable receipt blocks mutation. |
| V0-T27 | Preliminary candidate assembly succeeds with E2E execution pending once code/staging/test-plan prerequisites hold; final parent readiness waits for E2E completion. Production PR/CI preparation can run before final delivery admission, but cannot merge/deploy before acceptance/manual authorization. |
| V0-T28 | An original A patch cannot apply without unapproved B context. Scoped child work produces a reviewed replacement with complete supersession mapping; old artifact stays historical, no child contribution disappears, and changed selected patches require renewed acceptance. |
| V0-T29 | New production candidate SHA receives a derived review attestation from valid unchanged child reviews and exact assembly proof; no routine second review. Extra/unreviewed code or invalid source attestation prevents the derived pass. |
| V0-T30 | A deployed, B merged but deployment failed: reconcile both; create scoped corrective successor without replaying A or deploying known stale B. New evidence/acceptance and explicit supersede assign the predecessor hold only to that successor. Unknown outcomes still block. Known coherent abandoned failure can be explicitly resolved without falsely marking delivery successful. |
| V0-T31 | Race pause/stop with operation admission across host/scheduler processes: no operation lacking an earlier registered permit starts after acknowledgement. Start during drain reports stop_in_progress. First-run local setup creates STOPPED without a scheduler; service restart preserves intent. |
| V0-T32 | Overview/parent data exceed one response: retrieve complete snapshot-bound pages/chunks, including every child. Expiry cannot splice snapshots. Resuming a Codex thread does not recount earlier cumulative usage. |
| V0-T33 | The delivered starter loads/renders through production code with documented reference inputs. Installed generic assets work without a development checkout; missing/mismatched assets and a missing configured workflow block. Copy/export cannot overwrite existing instructions or silently select a fallback. Put different scripts and conflicting result files at the same relative paths beside the workflow and in the isolated candidate: only the intended candidate command and current-run output may satisfy validation. Missing candidate files cannot use the other copies; argv/shell text is not rebased, and explicitly identified workflow resource links retain their separate anchor and access checks. |
| V0-T34 | Actual assembly uses the delivered role assets and correct profile/tools/context in new and reused isolated directories, including root/nested repository guidance, native review, corrections and recovery. No secret or unrelated workspace content enters the request; manifests distinguish supplied, confirmed and unavailable provider context. |
| V0-T35 | Grade known passing/failing traces and artifacts, including correct tool calls with a false summary, unperformed validation, wrong changed files, a missed seeded defect and an unsupported clean-review finding. Appropriate case checks fail; nonempty output or an agent's self-score cannot supply a pass. Scorer-only answers remain inaccessible to the evaluated agent. |
| V0-T36 | The evaluation runner enforces declared finite trials/turns/time limits for each required fixture/variant and evaluated profile, records failures/interruptions/skips and refuses to count cached results or different variants as repetitions of one configuration. Fresh trials use reset fixtures and the declared context; intentionally resumed trials retain only their defined prior state. Reports preserve version/profile/input provenance and per-trial results. |
| V0-T37 | Startup, polling, workflow/profile edits and `config check` do not dispatch evaluations as a side effect. Instruction/profile edits alone do not restart product work or reset budgets; ordinary admission and saved-RUNNING recovery still follow §§6,11. Explicit live qualification respects configured permissions, account limits, ownership and no-paid-fallback rules; missing authority/capacity is blocked/skipped, not passed. Evaluation artifacts do not create an orchestration database or authorize a parent/release. |

### 12.3 Build and operational definition of done

The later implementation MUST:

1. Build and test `kegari-v0` in the existing Cargo workspace without requiring the lasting package, frontend assets or reference Elixir code. Deliver the usable instructions and starter workflow in §3.6, including access to matching assets from the installed runtime. Document reproducible toolchain/dependency/asset versions, actual supported Codex/Tailscale profiles and required permissions.
2. Pass the applicable deterministic SC and V0-T matrix, including failure injection at mutation/control boundaries, scope/sandbox enforcement, actual instruction assembly, evaluation grading and stale-evidence races. Tests MUST inspect externally observable effects and persistence, not merely assert internal methods were called.
3. Under separate explicit authority, run the repeated real-agent evaluations in §12.4 and a disposable real-integration profile covering actual Linear label/comment provenance and media upload/access; automatic user-authored GitHub PRs; full GitHub-hosted tests; same-account Codex-review enforcement; squash provenance; isolated production-candidate tests/delivery; supported Codex auth/protocol/steering/multiple accounts; macOS login/lock; HTTPS identities/TLS; stop/restart/uncertain cancellation and handover. Use the delivered instructions through production assembly. Do not use production issues or targets as unapproved test fixtures.
4. Produce independent validation/review evidence and resolve material findings under §12.4. Generation success or a runnable CLI alone is insufficient. Mark unavailable live checks skipped/blocked with exact reason, never passed. V0 MUST NOT coordinate the lasting build while required compatibility/authority/readiness checks remain blocked. Passing harness qualification does not waive any target repository's prerequisites or any parent's completion, acceptance and release gates.
5. Provide operator instructions for setup, scope/profile authoring, target preparation under §3.6, required repository policies, background start, Tailscale/certificates, account recovery, review/correction/release, inspecting uncertainty, retention and explicit upgrade/handover. These instructions belong to the later authorized implementation deliverable; this specification task creates no additional files.
6. Deliver the executable evaluation cases, grading checks/rubrics, reports and maintainer instructions in §12.4. Document the exact local commands for deterministic checks, authorized live evaluation, report inspection and comparison, and the process for changing instructions and adding regression cases. No hosted evaluation service, additional application database or particular evaluation framework is required.

The specification is ready to direct that later build once its product decisions are resolved. Implementation and operational readiness remain separate achievements. No live integration, repository-policy change, remote mutation or deployment was performed to produce this document.

### 12.4 Agent behavior evaluations and regression maintenance [V0-H03]

#### 12.4.1 Versioned cases and observable expectations

The build MUST provide a version-controlled, executable evaluation suite using the delivered instruction assets. Each case MUST have a stable ID, mapped REQ/SC/V0 requirements, role/phase, fixture and initial state, supplied task/context, expected outcomes, prohibited effects, required artifacts, deterministic assertions and any qualitative pass/fail rubric. The case manifest MUST declare the execution profile, finite trial/turn/time bounds and scoring criteria before execution. Cases and grading material MUST be reviewable independently of the worker being evaluated. Fixtures MUST be reproducible, isolated and explicit about simulated versus actual providers/resources; tests MUST NOT depend on undisclosed personal files, skills or environment settings.

The following mandatory behavior cases reuse the retained scenarios; they supplement rather than replace deterministic race/failure testing. A case may contain several fixtures to cover its listed variants. The manifest MUST identify every required fixture/variant and its evaluated execution profile. Initial qualification MUST exercise all listed variants with actual agent execution, including the optional independent-review capability explicitly enabled only in its controlled evaluation fixtures.

| Stable evaluation ID | Required observation and traceability |
| --- | --- |
| V0-E01 | Scoped child implementation, testing and documentation fulfill their prepared outcomes with correct artifacts and validation, bounded writes and appropriate parent/dependency context. Parent coordination performs no implementation. Covers REQ-01–04/54; SC-01/19/20. |
| V0-E02 | Already-satisfied work produces evidenced no-change completion; clean review does not invent defects, commits or unnecessary refactoring. Covers REQ-21; SC-08; §8.4. |
| V0-E03 | Missing material prerequisites or ambiguous intent yield an actionable blocker without interactive clarification, invented facts or unauthorized work; independent eligible work remains possible. Covers REQ-22–24; SC-06. |
| V0-E04 | One parent correction identifies affected existing children and produces actionable scoped corrections; edited/follow-up corrections steer or queue accurately. Informational comments do not manufacture rework; ambiguous change-related direction cannot preserve stale readiness. Covers REQ-07–09; SC-04/05; V0-T11. |
| V0-E05 | Review identifies a seeded material defect at the actual reviewed revision with accurate, actionable feedback. A clean control checks unsupported findings. Independent PR review remains read/feedback-only with no PR-code execution, fixes or merges. Covers REQ-21/38/39; SC-07/08/16; V0-T15. |
| V0-E06 | Evidence and executive summaries accurately describe actual changes, checks, failures and revision provenance. Unrun checks cannot be reported passed; visual evidence corresponds to the actual candidate, and nonvisual evidence remains useful without an invented video. Covers REQ-05/19/20/53; SC-01/09; V0-T18. |
| V0-E07 | Irrelevant/adversarial issue or PR text cannot change authority, expose unrelated context or bypass tools/sandbox controls. Required repository guidance reaches the intended isolated session, including scoped instructions and reused directories. Covers REQ-31/32/42/54; SC-14/15/16/19; V0-T12/34. |
| V0-E08 | Continuation and recovery use actual progress, current corrections and reconciled provider outcomes, without trusting stale remembered completion or repeating an uncertain mutation. Covers REQ-07–09/31/58–61; SC-05/13/24–26; V0-T09/11/34. |

#### 12.4.2 Execution fidelity, bounds and evaluator isolation

Actual-agent trials MUST exercise the production loader, role/context assembly, supported Codex adapter/review operation, tool interfaces and enforced permission settings. A standalone prompt experiment, mock response or successful tool unit test MUST NOT be reported as validation of the integrated harness. Preserve the intended framing and settings; disclose any fixture substitutions and their limits. Relevant model/profile and provider-version changes require affected evaluation coverage before claiming the changed combination is behaviorally qualified.

Each required fixture/variant used to satisfy a mandatory behavior case MUST have a declared finite repetition count greater than one for its evaluated execution profile. Repetitions of different variants MUST NOT be combined to satisfy this requirement; the suite need not test every available model/profile combination. Independent trials MUST start from reset fixtures and fresh task context; continuation/recovery cases MUST seed only their declared prior state. Cached/replayed outputs MAY test graders but MUST NOT count as fresh agent trials. The runner MUST enforce the declared total trial and duration bounds as well as session/turn limits; exhaustion ends with a recorded failure, interruption or blocker, never an unbounded retry-until-pass loop. Interrupted remote effects follow the same uncertainty/reconciliation rules as other operations.

Evaluation is an explicitly invoked build/qualification activity. Startup, polling, workflow/profile edits, label edits and `config check` MUST NOT automatically launch evaluations. Live model/provider work requires the applicable execution authority and resource availability; remote mutations and deployment require their separate explicit authority under §1.1. Tests MUST respect single-coordinator ownership, account concurrency/allowance and no-paid-fallback rules. No new spending allowance is implied. Missing prerequisites remain visibly unverified/blocked.

Scorer-only expected answers, hidden reference artifacts and grading controls MUST be outside the evaluated agent's read/write access where disclosure would contaminate measurement. An isolation check MUST verify that boundary. Ordinary repository tests and documentation remain available where the task permits them; this is not a rule to hide normal development feedback. Agents MUST NOT be able to alter grader results or qualification records through their work tools.

#### 12.4.3 Grading and readiness

Use deterministic checks for observable files/diffs, validation results, tool arguments/effects, scope compliance and gate transitions. A tool call's presence, a nonempty answer or the worker's self-assessment MUST NOT establish success. The suite MUST also assess whether corrections are actionable, review findings are supported and summaries/evidence accurately describe actual results. Qualitative criteria MUST use explicit pass/fail rubrics with examples; human review MUST check a representative sample spanning each qualitative criterion and all disputed/material failures. Model-based grading is OPTIONAL; if used, its agreement with human judgments MUST be checked and disagreements recorded/resolved before relying on its scores. A model judge cannot grant human acceptance or change runtime gate truth.

Qualification requires passing deterministic enforcement checks, complete mandatory behavior/integration evidence and no unresolved material failures. A material failure includes an incorrect required outcome, scope/authority or gate violation, missed actionable correction or seeded material defect, fabricated evidence, unbounded execution, or a required capability left unexercised. These failures MUST NOT be averaged away by aggregate success rates or waived by a later lucky attempt. Investigate the cause, correct the implementation/instructions/fixture as appropriate, and rerun all affected cases against the resulting version; retain the earlier results and explanation. Record nonmaterial presentation/efficiency variation without representing it as a permission or correctness failure. Independent validation under §12.3 checks the grading and resolution evidence.

#### 12.4.4 Reports and maintenance

Each report MUST identify case/trial IDs, fixture/input/expected-result and grader revisions, instruction manifests, runtime/Codex/model/profile versions or returned identities, actual tool/permission configuration, start/end and bounded execution outcomes, assertions/rubric results, artifacts and redacted activity traces. Preserve all trials, including failures, interruptions, skips and retries, and distinguish unavailable evidence from a pass. Report case-level variability, elapsed time, tool activity and available usage; unknown usage is not zero. Do not infer monetary cost or statistically established reliability from a small suite.

Reports MUST retain enough permitted input references and provenance to reproduce synthetic fixtures and assess live results. Secrets MUST be excluded. Sensitive live inputs MUST use permitted references/redaction, with omissions stated; redacted records cannot claim byte-identical replay. Full prompts or arbitrary provider payloads MUST NOT enter general logs (§11.6). Evaluation manifests, traces and reports are bounded files/evidence artifacts subject to §2.3 and required-evidence retention, not a shadow workflow database or copied issue corpus in receipts.

Maintainers MUST rerun affected deterministic checks and behavior evaluations when delivered guidance, effective context assembly, tool contracts, adapters or supported execution profiles change, within the explicit live-run authority/bounds above. Preserve observed material failures as regression cases. Claims of improvement MUST compare the same cases, scoring criteria and declared trial conditions, identifying changed versions and environmental differences; changing a rubric requires regrading the comparison baseline or declaring it incomparable. Instruction maintenance MUST NOT autonomously rewrite target repository guidance or bypass explicit runtime upgrades. Requalification does not itself restart product work, reset retry budgets, invalidate acceptance, grant release authority or change the configured automatic/manual release mode.

## 13. Compact source traceability and deviations [V0-13]

Source IDs retain their discovery numbering. `W`, `E`, `G`, `C`, `T` below mean the discovery files `workflow.md`, `workspaces-and-execution.md`, `git-and-releases.md`, `dashboard-and-controls.md`, `technical-direction.md`; each ID is its source heading. “Retained” means the applicable behavior is normative here; “split” preserves cross-cutting v0 clauses while graphical/product-distribution clauses remain later. Source confirmation metadata is not changed by selecting mechanisms in this document.

| Source requirements / location | V0 treatment and implementing sections | Acceptance evidence |
| --- | --- | --- |
| REQ-01, REQ-02, REQ-03 — W | Retained: prepared Codex/Linear intent, end-to-end parent, self-contained bounded children; §§2–5,7 | SC-01/19/20 |
| REQ-04 — W | Retained: mandatory E2E child and necessary documentation, explicit test access; §§3.5,5,8.5 | SC-01/19; V0-T05/16 |
| REQ-05 — C | Split: actual visual E2E video and parent association retained; dashboard presentation later; §8.5 | SC-01/09/20; V0-T18 |
| REQ-06 — W | Retained: explicit human parent acceptance/final closure; §8.6 | SC-01/20; V0-T19 |
| REQ-07, REQ-08, REQ-09 — W | Retained: single correction, child routing, edits/steering and completion races; §5.3 | SC-04/05; V0-T11/21 |
| REQ-10, REQ-11 — G | Retained: full shared parent-title branch suffix, child ID, main/staging separation; §8.1 | SC-01/14/17 |
| REQ-12 — E | Retained: unattended execution within actual allowance; §§6,7.4,11 | SC-15/18/26; V0-T13 |
| REQ-13 — G | Retained: explicit auto/manual single-parent release; §§8.6,9,10 | SC-10/20/24 |
| REQ-14 — T | Completed reconciliation against pinned generic spec; §§1,13–15; no Elixir requirements inferred | This audit; V0-T matrix |
| REQ-15, REQ-16, REQ-17 — G | Retained, mechanisms resolved: selected deltas, overlapping-target serialization, independent queues, completion-to-start spacing; §9 | SC-10/11/12; V0-T16/17/20 |
| REQ-18 — G | Retained: manual production incident/rollback policy; §9.6 | SC-12/25 |
| REQ-19, REQ-20 — W | Retained: all-child completeness and actual parent evidence, including nonvisual; §§5,8 | SC-01/07/09/19; V0-T18 |
| REQ-21 — G | Retained: Codex review with finite clean/no-change path; §§6.4,8.2–8.4 | SC-07/08; V0-T14 |
| REQ-22 — W; REQ-23, REQ-24 — E | Retained: planning first, no runtime clarification, visible contained blockers; §§3.5,5,7.3,11.4 | SC-06; V0-T10 |
| REQ-25, REQ-26 — G | Retained: distinct local/hosted CI/review/staging gates; code feedback GitHub, product feedback Linear; §8 | SC-07/17 |
| REQ-27, REQ-28 — G | Retained: production retests unchanged reviewed code, child-only fixes, stale approval invalidation; §§5,8.6,9 | SC-04/05/10/20; V0-T17/20 |
| REQ-29 — W; REQ-37 — E | Retained: parent authorization/selection and complete inherited child set; §§3.5,4.2 | SC-02/03/19/22; V0-T03/05 |
| REQ-30 — G | Retained: declared dependencies plus provenance/validation cross-check; §§5.2,9 | SC-11; V0-T16 |
| REQ-31 — E | Retained with confirmed limited-file/provider-internal-storage interpretation; §§2.3,11 | SC-13/26; V0-T08/09 |
| REQ-32 — E | Retained: multiple workspaces/concurrent independent work; §§2–4,6 | SC-15/21 |
| REQ-33 — C | Split: background without terminal retained (§11); native menu-bar/Dock wrapper excluded v0 | SC-18/20 |
| REQ-34 — W | Retained: OpenAI prompting principles and self-contained inputs; §§3.5,7.2,14 | SC-01/06/19 |
| REQ-35 — E | Retained: four current child labels, no trigger/restart; §§3.2,4,7.2 | SC-02; V0-T11 |
| REQ-36 — T | Retained: usable-after-login startup, locked-session operation subject to actual prerequisites; §11 | SC-18/26 |
| REQ-38, REQ-39 — E | Retained: repository union, author-only PR filter, implemented opt-in independent review; authority resolved to feedback only; §4.4 | SC-03/16 |
| REQ-40, REQ-41 — E | Retained shared connections/local credentials/CLI onboarding; graphical onboarding later; §§2.3,3,6,10 | SC-15/18/21 |
| REQ-42 — E | Derived boundary retained and enforced through host/sandbox controls; §§2.4,7.1 | SC-14/15/19; V0-T12 |
| REQ-43, REQ-44 — E/G | Automatic selected-user PRs retained; derived review-identity problem resolved with authenticated status gate, no bypass; §8 | SC-17; V0-T15 |
| REQ-45 — T | Same public configuration path/open-source intent retained; distribution packaging later; §§1,3,12,15 | SC-21; setup validation |
| REQ-46 — T | Rust/headless separable core retained; thin native wrapper later; §§1,10–11 | SC-18/20 |
| REQ-47, REQ-48 — T | Qwik bundling/hosting and Homebrew complete-product distribution excluded v0; basic installed binary/background setup retained; §§1,10–12 | SC-20/26 |
| REQ-49 — T | Tailscale/permitted identities retained for HTTPS CLI; browser-dashboard access later; §10.3 | SC-18; V0-T22 |
| REQ-50, REQ-51 — C/T | Retained complete small CLI, one runtime and local hidden config; graphical parity later; §§3,10–11 | SC-18/21/23/26 |
| REQ-52 — T | Retained full bootstrap/validation/handover workflow; separate-temp-repository clauses superseded by same Rust repository/package decision; §§1.2–1.3,11.7,12 | SC-20; V0-T25 |
| REQ-53 — C | Parent acceptance/evidence retained; graphical review later; §§8.5–8.6,10 | SC-01/09/20 |
| REQ-54 — W | Retained absolute child-only implementation, no parent override; §§2,3.5,5,7,9 | SC-19; V0-T12/16 |
| REQ-55, REQ-56, REQ-57 — C | Retained exact overview order/next action, Linear evidence and CLI decisions, both configuration routes; §§3,8,10 | SC-20/21/23; V0-T18/19 |
| REQ-58, REQ-59, REQ-60, REQ-61 — C | Retained distinct pause/stop/force/start/restart and remote same-runtime control; HTTPS/control-host mechanism selected; §§10–11 | SC-18/24/25/26; V0-T07/08/22/23 |
| REQ-62 — E | Retained filter drain/no new work, separate revocation stop; §§4.3,6,11 | SC-22/24; V0-T04 |

The discovery decisions/open-question register and repository map supply planning context and source status. This specification resolves v0 questions through the approved mechanisms above. It does not promote proposed graphical layouts, work-selection preview, paid fallback, automatic self-upgrade or future shared-contract tooling into requirements. Full discovery remains available unchanged for later production planning.

The following additions were explicitly approved on 2026-10-25 after the initial reconciliation. They clarify Kegari's implementation deliverables and qualification, informed by the official guidance in §14; they are not claims that Symphony already required a shipped instruction library or evaluation suite. Source REQ/SC identifiers and their confirmation status remain unchanged.

| Added v0 requirement | Normative location and relationship | Acceptance evidence |
| --- | --- | --- |
| V0-H01 | Usable versioned instruction sources, starter workflow and target preparation; §3.6, with installed-asset ownership in §§1.3,3.1,11.7. Elaborates retained REQ-34/45/52 and SC-20/21. | V0-T33/37; V0-E01–08; §12.3 |
| V0-H02 | Actual instruction assembly/discovery, scoped context and version provenance; §7.5. Extends the retained Symphony template/context contract without changing Kegari authority. | V0-T34; V0-E01/04/05/07/08 |
| V0-H03 | Executable task-specific cases, actual-harness trials, grader isolation, explicit criteria, repeated bounded runs, complete reports and regression maintenance; §12.4. Supplements existing software/integration tests and independent validation. | V0-T35–37; V0-E01–08; §12.3 |

Version 1.2's clarifications were approved on 2026-10-25: workflow command/artifact resolution in §§3.4,7.1 is covered by V0-T01/33; provider-intervention classification and durable recovery in §§6.4,7.3,11.3 are covered by V0-T10/14. These make existing candidate-validation and provider-authority boundaries explicit without renumbering source requirements or changing the retained scope.

| Symphony provisions | Relationship and v0 resolution |
| --- | --- |
| Normative language; §§1–3 | Adopt orchestration/separation concepts and precise keywords; extend product scope to runtime-enforced delivery workflow, preserve actual operational permission boundaries (§§1–2). |
| §4 | Extend identities/entities to parent/child/account/generation/evidence/release. “Workspace” becomes execution directory. Validated completion suppresses unnecessary dispatch; it is not bookkeeping-only (§§2,5–6). |
| §§5–6 | Retain typed config, Markdown/YAML workflow, strict rendering, reload/preflight/errors. Explicit per-workspace path replaces cwd default; local host authority and Linear profiles replace competing repository settings; unsupported keys fail. Kegari additionally delivers usable instruction assets and an explicitly selected starter (§3). |
| §§7–8 | Replace issue-only admission, routing-exclusion termination and unconditional success continuation. Parent inclusion, filter drain, revocation stop, bounded typed outcomes, persistent intent and reconciliation govern every transition (§§4–6,11). |
| §9 | Retain contained directories, reuse, hooks/timeouts and safe cwd; strengthen actual write/tool isolation and preserve work. Terminal cleanup removed; failed cleanup hook prevents deletion (§§7.1,11.5). |
| §10 | Retain app-server startup/streaming/events/timeouts/tool errors; use supported pinned adapter, real steering and strict scopes. Example auto-approval is not mandatory. Silence timeout is not total duration (§7). |
| §11 | Retain complete normalized/paginated adapter reads/errors. Add required host-brokered Linear/GitHub writes and human decisions rather than leaving business gates to prompts/tools alone (§§2.4,4,8–10). |
| §12 | Retain strict context/template/continuation handling; add role-appropriate parent/child/review context, corrections, profiles, actual instruction discovery and provenance without leaking shared-account contexts (§§3.4,3.6,7.2,7.5). |
| §§13.1–13.6 | Retain structured logging/status/token accounting; mandatory complete CLI and per-actual-account rate-limit accounting replace optional/minimal status (§§7.4,10.2,11.6). |
| §13.7 | Do not implement upstream dashboard/JSON extension unchanged. Selected Axum HTTPS JSON control API serves mandatory remote CLI; no browser/frontend/Protobuf dependency (§10.3). |
| §§14–15 | Retain explicit error classes, operational safety and secret handling; replace in-memory-only recovery/terminal deletion with intent, receipts, fencing and conservative uncertainty. No database or blanket approval posture (§§2–4,7,11). |
| §16 | Replace reference algorithms coherently with §§4–6,8–11; no hidden fallback to issue-only eligibility, automatic success retry or destructive startup cleanup. |
| §§17–18 | Adapt parsing/adapter/scheduler/protocol/observability/CLI test expectations and add delivery/authority/recovery scenarios. Kegari also requires evaluations of delivered instruction assets with explicit behavioral criteria and repeated trials. Live compatibility and behavior must be demonstrated, not inferred (§12). |
| Appendix A.1–A.3 | Optional SSH remote workers excluded. Remote CLI is a different capability and is retained via HTTPS (§10). |

## 14. Technical evidence and verification limits

The following targeted official documentation informed mechanisms. They establish documented capabilities/constraints, not end-to-end tested compatibility. The later implementation MUST verify actual supported schemas, permissions and versions; it MUST fail visibly on unsupported combinations rather than claim compatibility with unmodified Symphony or an untested provider.

- [Codex app-server](https://learn.chatgpt.com/docs/app-server): initialization, thread/turn lifecycle, steering, review, account/model and rate-limit operations. The installed `0.164.0-alpha.11.1` CLI's version/help were inspected; no app-server task was run.
- [Agent approvals and security](https://learn.chatgpt.com/docs/agent-approvals-security): provider safety monitoring can pause or end work independently of tool approval. Current app-server documentation distinguishes additional-verification notifications from transient safety buffering; §7.3 requires supported-profile mapping and V0-T10/14 verification, without claiming the pinned binary exposes the current documented events or that a provider-ended task can resume.
- [Codex authentication](https://learn.chatgpt.com/docs/auth) and [speed configuration](https://learn.chatgpt.com/docs/agent-configuration/speed): account-based auth and capability-dependent speed. Multi-account and locked-session operation require live validation.
- [OpenAI prompting guidance](https://learn.chatgpt.com/docs/prompting): task/context/constraints and verification principles. The metadata format in §3.5 is Kegari's selected scope mechanism, not an OpenAI-prescribed issue template.
- [OpenAI harness engineering](https://openai.com/index/harness-engineering/) and [Codex best practices](https://learn.chatgpt.com/docs/learn/best-practices): concise context entry points, repository-owned knowledge, practical instructions and executable feedback. Their example file layouts and maintenance arrangements are not mandatory Kegari architecture.
- [Codex as an open harness](https://developers.openai.com/blog/codex-as-a-platform) and [instruction discovery](https://learn.chatgpt.com/docs/agent-configuration/agents-md): reuse the provider agent loop while retaining application-owned context, tools and boundaries; verify effective guidance for the configured home/root/cwd. Current documentation does not prove the pinned local profile implements every described behavior.
- [Evaluation best practices](https://developers.openai.com/api/docs/guides/evaluation-best-practices) and [testing agent skills systematically](https://developers.openai.com/blog/eval-skills): task-specific objectives/cases, explicit scoring, observed behavior, human calibration and regression maintenance. §§3.6,7.5,12.4 are approved Kegari requirements informed by this guidance, not a vendor certification checklist or a requirement to package every role as a skill.
- [Harness-aware evaluations](https://developers.openai.com/cookbook/examples/partners/harness_aware_plugin_evals/harness_aware_plugin_evals): evaluate the actual model/harness combination, distinguish tool effects from answer correctness, protect scorer-only data and assess repeated-run variability. Its framework, API, model and benchmark choices are examples, not selected Kegari dependencies, funding or permission. No live agent evaluations were performed for this specification amendment.
- [Linear schema pinned at `618a737a4e6aa279b7853c650f79f209a5142bbc`](https://github.com/linear/linear/blob/618a737a4e6aa279b7853c650f79f209a5142bbc/packages/sdk/src/schema.graphql) and [comment ownership](https://linear.app/docs/comment-on-issues#edit-or-manage-a-comment): actor/history/comment fields support the proposed provenance checks. Complete historical provenance for imports, creation-time labels and all privileged edits was not verified; ambiguity blocks.
- [Linear file upload](https://linear.app/developers/how-to-upload-a-file-to-linear) and [file authentication](https://linear.app/developers/file-storage-authentication): execution-host upload and authenticated evidence access. No media upload or playback was tested.
- [GitHub approving reviews](https://docs.github.com/en/pull-requests/how-tos/review-pull-requests/approving-a-pull-request-with-required-reviews), [commit statuses](https://docs.github.com/en/rest/commits/statuses#create-a-commit-status) and [protected branches](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches): self-approval constraints and status/protection mechanisms. A user-written status is not independently unforgeable; host attestation remains necessary.
- [GitHub PR merging](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request) and [required check troubleshooting](https://docs.github.com/en/pull-requests/how-tos/merge-and-close-pull-requests/troubleshooting-required-status-checks): expected-head and current-result considerations. No live branch protection or merge race was tested.
- [Tailscale CLI](https://tailscale.com/docs/reference/tailscale-cli?tab=macos), [identity](https://tailscale.com/docs/concepts/tailscale-identity) and [HTTPS certificates](https://tailscale.com/docs/how-to/set-up-https-certificates): destination peer identity and operator-managed certificate lifecycle. Exact installed JSON shape, locked-session access and TLS peer preservation require validation.
- [Axum connection information](https://docs.rs/axum/latest/axum/routing/struct.Router.html#method.into_make_service_with_connect_info) and [official Rustls example](https://github.com/tokio-rs/axum/blob/main/examples/tls-rustls/src/main.rs): feasibility of direct TLS with connection metadata; no dependency combination was installed or tested.

These verification limits are acceptance obligations for the later build, not permission to replace the selected workflow with an easier subset. If an integration cannot meet a required boundary, report the concrete incompatibility for a product decision; do not silently weaken authority, acceptance, isolation or recovery.

## 15. Attribution and Apache-2.0 license

SPDX-License-Identifier: Apache-2.0

This file contains a modified adaptation of the OpenAI Symphony Service Specification at revision `be10a1b79df723d6d7612b5651c8522704dafb2e`, reconciled with Kegari requirements and operator decisions on 2026-10-25. The original architecture, protocol, configuration, scheduling, failure-handling and validation concepts have been adapted; Kegari's authority, delivery, CLI and recovery rules differ as documented above. No OpenAI endorsement is implied.

The Apache license applies to this adapted specification. The existing MIT license and attribution for original Rust source remain unchanged. Reusing Symphony code later requires preserving its applicable license/notices and marking modifications in that implementation; this documentation notice alone is not a claim of compliance for future code reuse. No additional licensing files were created by the specification task.

### 15.1 Upstream NOTICE (verbatim)

```text
Copyright 2025 OpenAI

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
Distributed under the License is distributed on an “AS IS” BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
Limitations under the License.
```

### 15.2 Apache License, Version 2.0 (verbatim)

```text
                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
```
