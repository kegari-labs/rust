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
revisions and the requested control generation. Identity, account, permission, repository-path and deployment-authority changes
require PAUSED or STOPPED, drained work and no unresolved effects. Ordinary
selection, execution profile, workflow, budget and capacity changes apply to future
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

`config check` returns explicit verified/blocked metadata checks and an
`operational_qualification: unverified` field. A workspace-limited check keeps the
full registered configuration revision in its report. Repository checks enumerate
the complete permitted repository set, require actual read/write permission and
active repositories, enumerate configured active workflow IDs and exact caller
paths, read the pinned caller commits, and read both main/staging references,
branch protection and rulesets. They use the delivery module's shared policy
validator for strict current-base checks, required CI/review contexts, allowed
squash merges and incompatible formal/code-owner approval requirements. These
reads do not issue a delivery proof or mutate provider policy. Trusted reusable
workflow execution, exact-source artifacts, isolation and locked-session checks
remain separate explicitly unverified report entries. Identity and workflow
metadata do not qualify live Codex isolation, GitHub-hosted CI, deployment,
Tailscale, Keychain locked-session behavior or the required human-reviewed agent
evaluations. These checks have not been executed against live accounts in this
builder conversation.

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
