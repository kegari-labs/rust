# Configuration transactions

`configuration::ConfigManager` implements the host configuration interface used by
setup, connection add/reconnect, and workspace add/configure. It consumes the same
strict typed TOML as startup. Unknown or duplicate TOML fields are errors; command
arguments are validated through the versioned API before preparation.

Preparation resolves host paths, loads every configured workflow and verifies
affected provider identities. Linear verification binds the actual user and
organization to the configured workspace. GitHub verification requires the
configured numeric user ID and a `/user` response of type `User`. An absent scope
header remains unknown. Codex verification uses only the supported app-server
metadata methods in a private empty context: account without refresh, model
catalog, rate limits and configuration. It verifies the selected account, pinned
protocol/version and every configured model/reasoning/speed combination. It does
not log in, open a thread, start a model turn, run hooks or execute target tests.

An explicit stdin secret or declared `$VARIABLE` credential is observed before
import. Environment inputs are frozen for the transaction; an existing Keychain
reference is read as declared. The verified credential capability binds its
identity to the exact input connection. Only the serialized commit gate may save
it, using a deterministic operation-specific Keychain reference. Same-account
reconnect reads the existing reference and does not silently switch accounts or
overwrite the existing secret. The transaction receipt retains only the final
payload digest, safe identity observations and configuration revisions. Neither
plaintext credentials nor standalone secret fingerprints appear in it.

The commit shares `ControlHost`'s lifecycle and mutation mutex. It rechecks the
authenticated OS UID or stable Tailscale user ID against the current role mapping,
including the operator ID, then compares the prepared and client configuration
revisions and the requested control generation. Identity, account, permission, repository-path and delivery-policy changes
require PAUSED or STOPPED, drained work and no unresolved effects. Ordinary
selection, Codex profile, workflow, budget and capacity changes apply to future
admissions; active operations retain their exact admitted settings while current
provider permission, correction and dependency checks continue. Same-account
credential replacement is identity-verified before import.

After verification and admission, the transaction imports credentials, commits
the typed file through `Registration::commit_configuration`, replaces transport
role mappings, then publishes the in-memory configuration. Disk revision and
registration mismatch fails closed on recovery. A failure after the sent receipt
requires explicit reconciliation; it never automatically repeats a Keychain or
configuration write. A newly imported secret may remain under its unique private
reference after a later disk failure. It must be reconciled before cleanup, not
deleted or reused by an automatic retry.

The HTTP request receives an owned background-command receipt before potentially
slow verification. Its derived configuration effect carries both the original
command ID and original payload digest. The manager verifies those against the
actual durable parent receipt, exact actor/principal, runtime, command, arguments,
secret input and derived UUID. It requires that command to remain owned by the
host queue at the final commit gate. A caller-selected matching UUID alone is not
lineage. Identical request replay returns the recorded receipt without rechecking
providers or repeating effects; changed input under the same ID conflicts.

`prepare_initial` supports read-only local bootstrap preflight before registration.
It requires the actual host ID and a configuring local operator mapping. Metadata
probing may create private empty probe directories, but this helper does not save
credentials, configuration or registration. The concrete bootstrap path may use
the same manager after taking the canonical control lock and remains STOPPED.

## Generic execution migration

[SPEC 1.5](../SPEC.md) delegates repository discovery, editing and execution to
Codex's native tools. Users do not need command/environment specifications,
executable catalogs or target-toolset adapters. Runtime-owned generic guidance
is the default when no repository workflow is selected; an explicitly selected
invalid or missing workflow remains an error.

Configuration continues to define accounts, repository/write scope, operator
roles, supported provider permissions and release policy. Discovery cannot grant
new authority, expose ambient credentials or change host configuration. Fixed
Git/Codex integration configuration remains distinct from target-project commands.
Required checks and source-bound result acceptance remain workflow responsibilities.

The custom managed-domain/resource consumers have been removed. Retained service
declarations express workflow needs; unsupported lifecycle behavior remains an
explicit limitation rather than a prerequisite to generic native Codex discovery. Remove obsolete
fields and consumers with their production callers; do not preserve the custom
executor behind compatibility options. Runtime instruction assets must migrate
with those callers and their explicit asset identities.

Preserve old records and reject unsupported recovery without deleting source data
or reinterpreting old execution proofs. Changed persisted/asset contracts require
explicit identities. Actual configuration schema/revision changes occur with the
implementation, not this documentation correction. Permission reductions retain
their fencing and drained-update requirements, and source-bound uncertainty cannot
be cleared by changing configuration.

### Managed repository execution policy

This historical link target is retained for older evidence references. The former
Kegari-managed repository policy is superseded by Codex-owned execution under
SPEC 1.5; it is not a current configuration or qualification recipe.

## Repository delivery declarations

Repository `ci`, `production_ci`, `no_change_validation` and `deployment` may be
omitted while agents discover the repository and its requirements. Omission is
unresolved policy; explicit JSON null, empty CI arrays and malformed supplied
policies are rejected. Only an explicit `deployment = "none"` establishes a
configured deployment-free choice. Existing supplied values retain their exact
serialization and configuration digest. Evaluation repositories still require an
explicit deployment-free policy.

These declarations describe provider delivery policy, not a prerequisite command
inventory. Source identity, scoped acquisition, authorization and worker planning
can proceed without them. Hosted CI, no-change validation, merge, production and
deployment consumers require the policy for their actual phase before dispatch,
cached proof reuse or final approval. Missing policy cannot become a passing empty
suite, an empty release graph or an automatic deployment-free baseline. Changing
or removing a supplied policy fences the affected workspace even if another field
makes the proposed update invalid; committing a policy change requires drained,
paused/stopped ownership.

`config check` returns a top-level `status` of `blocked` when identity, workflow,
read-access or delivery prerequisite metadata is blocked. Its separate
`configuration_admission` field records whether configuration identity, selected
workflow and repository read checks passed. A verified admission field does not
qualify command execution or delivery. `prepare_initial` uses that narrower
field for its read-only preflight result; the production bootstrap uses the
configuration transaction directly, which validates structure, selected workflow
and affected connection identities. Neither route grants worker or delivery
authority. A workspace-limited check retains the full registered configuration
revision in its report.

Repository checks enumerate the complete permitted repository set and require
observed read permission and active repositories. Missing repositories or read
access block admission. An unresolved delivery policy reports its missing fields
without probing dependent workflow/branch metadata. With complete policy, checks
inspect write permission, active workflow IDs and exact caller paths, pinned
caller commits, main/staging references, branch protection and rulesets. They use
the delivery module's shared policy validator for strict current-base checks,
required CI/review contexts, allowed squash merges and incompatible formal or
code-owner approval requirements. Failures remain visible delivery blockers while
scoped discovery remains available.

These reads issue no delivery proof and change no provider policy. Reports keep
`operational_qualification: unverified`; trusted reusable workflow execution,
exact-source artifacts, isolation and locked-session checks remain separately
unverified. Identity and workflow metadata do not qualify live Codex isolation,
GitHub-hosted CI, deployment, Tailscale, Keychain locked-session behavior or human
agent evaluations. These checks have not been executed against live accounts in
this builder conversation.

## Deterministic evidence

Use focused configuration checks when changing this boundary; broader testing and
live qualification follow the milestone policy. From the workspace root with the
pinned toolchain/cache:


```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --offline --locked -p kegari-v0 --test configuration
```

The tests use real owned private storage and the production `ControlHost` and
registration transaction. Clearly labeled injected provider, process-identity and
Keychain fixtures cover identity mismatch, stale revisions, revoked principal
roles during an awaited verification, running-state rejection, import failure,
exact replay, changed secret/environment input, command lineage and same-account
reconnect. They never call the native Keychain, launch Codex or contact a provider.

Authenticated proposals are also parsed as strict TOML data before full typed
validation. Reliably identified role, actor, account, repository or resource
permission reductions create a separate monotone operational fence even when an
unrelated field is invalid or an unknown external effect holds the runtime. This
local restriction cannot send provider effects or admit work. Current actor and
workspace gates consult it immediately. A successful explicit update resolves
only its affected scope; unrelated updates cannot restore denied authority.

`permission-fences.json` stores scope identities and proposal digests, with durable
receipt provenance. No issue bodies or credentials are copied. Restart refuses
a missing restriction record that should represent a sent/acknowledged proposal.
This requires explicit local reconciliation instead of reviving old permissions.
Malformed TOML with no reliably parsed reduction grants nothing new. All changes
use authenticated configuration commands; this manager does not interpret an
out-of-band replacement of its registered file as an authorized update.
