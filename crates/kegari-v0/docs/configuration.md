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

SPEC 1.4 requires agent-led discovery without mandatory command/environment
specifications. Runtime-owned generic guidance is the default when no repository
workflow is selected; an explicitly selected invalid/missing workflow remains an
error. Discovery does not change configured account, repository/write, resource,
network or release authority. Applicable known validation obligations remain
required even when a workflow supplies no commands.

Workspace configuration no longer accepts `managed_execution` or any nested
executable, digest, read-root, executable-mapping, generated-executable or
qualification catalog. This includes an empty table or a JSON null field: old
configuration is rejected as unknown input rather than silently ignored or
converted into execution authority. Remove the obsolete field through an explicit
configuration change; this does not approve native execution or migrate retained
execution evidence.

Generic `test_resources` retain their actual paths and read/write/disposable
permissions. Account, actor, repository, credential, role and release boundaries
also remain configured and enforced. Fixed Git/Codex integration configuration is
separate from target technology discovery.

Preparation now produces a managed schema-5 command domain from fixed OS backend
resources, the actual owned repository/scratch directories and already-admitted
resource grants. No executable names, expected hashes or executable-mapping list
are configured. Ordered command lookup uses fixed OS paths followed by those
owned/admitted directories; explicit paths may select deeper programs. Canonical
target checks and the rendered sandbox retain the same role, source-write,
resource, forbidden-path and network boundaries. Finding a tool outside this
domain does not admit its directory or expose the user's home or credentials:
external host directories still need actual generic resource authority.

The preparation receipt binds the backend policy, and the retained isolation
binds its actual paths, root identities and bounded observations of immutable
read resources. Full resource capture/revalidation occurs outside Control;
serialized gates validate the retained binding. Symlink text is observed without
following it or claiming target-content closure. A broad read grant containing
a narrower writable resource or owned repository/scratch subtree currently
refuses: use disjoint grants so read-only siblings are still observed. Reads
wholly inside owned or admitted writable scopes retain their mutable semantics.
Actual invocation evidence binds
the selected executable, argv, cwd, rendered environment and isolation. These
observations are not a user inventory or a `qualified` bit. Workers can select
literal commands through `command_run` and enroll revision-bound obligations
through `validation_propose` before `validation_run`. Named workflow checks remain
optional inputs. Independent review assesses validation adequacy; a successful
command alone never passes a delivery gate. Production repository commands still
refuse missing native backend qualification.

`context_read` with key `execution_environment` reports the effective command
environment and host-issued handles for existing read resources. `resource_list`
and `resource_read` inspect those handles without running code or requiring
project-specific configuration. They accept a relative path, never a new host
root. Exact-file grants stay exact; links, Git administration, unavailable access,
changed observations and oversized results refuse explicitly. An empty listing
means there were no visible non-Git children in its declared inspection scope,
not that an inspection failed or the repository has no Git history.
These observations cannot add tools or resources outside the admitted boundary.

The unpublished `offline_cargo` configuration and schema-4 launch/check adapter
are removed. Unknown obsolete fields/records are rejected without migration or
source deletion; they are not silently accepted as generic proof. Prior native
approvals and test results remain bound to their actual earlier implementations.
Native generic-backend qualification remains incomplete.

A local or integrated workflow check may have an optional `services` array.
Each service has a unique stable `name`, literal `argv`, a `readiness` object
with literal `argv` and `timeout_ms`, and `max_lifetime_ms`. At most eight services
are allowed; each requires one finite readiness invocation before the main
check. Readiness may poll within its configured bound. Services share the check's
cwd, declared grants, aggregate deadline and captured-output budget. This does
not grant environment overrides, sockets or network access; endpoint services
remain unsupported until their scoped network boundary is implemented and qualified.

Control retains the original validation effect throughout service start, readiness,
check and shutdown. Each attempted member has an immutable order and distinct
process identity. Service leaders must still be running after readiness and at
check completion. Shutdown requests TERM, escalates after five seconds if needed,
and joins every member before source and artifact snapshots. A nonzero service
exit, timeout, truncation or KILL escalation prevents check success. Graceful
services must handle TERM and exit zero. A service failure does not trigger code
repair; a genuine nonzero main-check result may do so after clean service closure.
These are finite check-owned services, not reusable daemons or independently
installed host services. Supported native service qualification remains incomplete.

The current automatic domain uses managed schema 5 for foreground and bundled
checks; retained catalog schemas 1–4 and removed isolation fields are refused.
Schema-2 resource observations are descriptive and retain their original identity.
Service-member process rows use schema 3, while ordinary process rows remain
schema 2 and legacy process rows remain readable without promotion. Service-bearing
check records use schema 3; checks without services retain schemas 1/2. Handover
metadata is version 5 and requires process schema 3, managed schema 5, workflow-source
schema 1 and repository-policy schema 1. The latter requires readers to preserve
unresolved delivery declarations without converting them to permission or
deployment-free evidence. Complete configurations retain their existing serialized
shape and config schema 1; older replacement readers are refused.

The schema-5 policy digest includes the exact build identity. A retained command
domain from a different build is refused even if its schema number matches;
schema compatibility alone does not authorize a retained cleanup hook. There is
no automatic migration, evidence rebinding or destructive cleanup fallback.
Existing account, repository and generic resource permission changes retain
their configuration fencing and drained-update requirements. Removing the old
catalog does not change those requirements or resolve outstanding effects.

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

Run from the workspace root with the installed, verified toolchain:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --test configuration
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
