# One explicit native evaluation trial

The caller implements one authenticated local trial through the existing runtime,
Control owner, account limits, worker preparation and native terminal evidence.
The caller passes the recorded local component and full-suite checks; native
execution remains **unqualified**. The first independent-review fixture is migrated
and retains an unresolved profile. Actual provider/sandbox qualification, native
cleanup and human calibration remain incomplete. See [local validation](../validation/LOCAL-NATIVE-EVALUATION.md)
and [the current progress](../PROGRESS.md) before planning a run.

The caller uses one monotonic execution deadline across source preparation and
final collection, stages the exact declared scorer in private host storage, and
retains the actual active lease across permitted drain/reload. Deadline or native
finalization failure classifies an available report as interrupted before immutable
publication and preserves its collected evidence. These implementation properties
do not establish successful native cleanup or a behavioral pass. Preparation and
unused-trial errors retain the original cleanup owner. A temporary receipt-read
failure can retry against the original complete admission identity, and one failed
cleanup does not prevent other joined owners from draining. No retry grants a new
execution budget.

These examples document the caller contract. They authorize no execution and
were not run. Configuration or a successful read-only Plan does not start a trial.
The separate `kegari-v0-evaluation plan|inspect|compare` commands remain read-only.

## Configuration and local authority

Use the ordinary registered configuration and existing Codex connection. Do not
create a second runtime or account-capacity reservation. Add `evaluate` to the
intended OS user's existing `control.local_users` role set, preserving its actual
operator ID, Linear identity and other required roles. `inspect` permits later
inspection. `evaluate` alone grants no configuration or lifecycle changes. The
Run command accepts only an authenticated Unix peer; HTTPS/Tailscale is refused,
even when the remote operator has an `evaluate` role.

[The configuration fragment](../examples/evaluation-binding.fragment.toml)
shows the exact new keys. It is deliberately incomplete and has invalid unresolved
placeholders. Merge its fields into a fully valid configuration; do not replace
existing provider, actor, routing, workflow or repository contracts with it.

An evaluation workspace must have:

- `independent_review.enabled = true`, an exact `profiles.independent_review`,
  and one Codex account matching the suite's resolved profile binding.
- One selected repository and PR author, no organization selection, no test
  resources, `release.mode = "manual"`, and repository `deployment = "none"`.
- Exact reserved connection names `evaluation-<WORKSPACE_UUID>-github` and
  `evaluation-<WORKSPACE_UUID>-linear`. Their endpoints are respectively
  `https://github.<WORKSPACE_UUID>.evaluation.invalid` and
  `https://linear.<WORKSPACE_UUID>.evaluation.invalid/graphql`. The repository
  hostname is the same GitHub hostname. The ordinary configuration schema still
  requires credential references; use isolated fixture values, not product tokens.
- A positive synthetic GitHub actor distinct from the declared PR author, and
  synthetic Linear actor/workspace UUIDs. These are disclosed fixture identities;
  the Codex account and native execution are real.
- `evaluation.schema_version = 1`; exact suite manifest path/digest, case, variant,
  profile slot, repository/author IDs, starting PR number and full base/head Git OIDs; separate
  private `report_root` and disposable `fixture_root` paths. These roots must not
  overlap each other, the worker execution root or configured repository sources.

The suite profile slot's `binding` contains the exact `profile` object
(`tool`, `model`, `reasoning`, `speed`), `account_id` and `operator_reference`.
A template with `binding: null` remains blocked. A profile reference records the
operator's selection; it is not the authenticated command or execution authority.
Changing the suite/profile/fixture/scorer requires recomputing its declared
identity and preserving prior reports.

Reserved connections always receive the strict local synthetic transport. An
undeclared namespace fails closed; it cannot fall back to real HTTP. Before an
actual source proof, the wire exposes only declared identity metadata and an
empty PR list. After activation it serves only the exact derived trial PR/head and
bounded feedback/readback. Other endpoints and mutations refuse. Ordinary
selection, product delivery and automatic recovery exclude this workspace, and
the final preparation/model gates independently require the genuine trial request.
Omitting `evaluation` preserves ordinary workspace defaults and gives no trial
permission. Config checks do not activate a source or qualify native execution.

## Source and first fixture prerequisites

Supply an already prepared disposable Git repository through the configured
repository `source`. The first caller reads it through an owned, journaled
metadata operation; it does not initialize, commit, checkout or fetch it. It
requires actual distinct full base/head OIDs, a plain changed one-parent head,
base ancestry, clean HEAD at that head, and exact fixture file bytes and modes.
The model subsequently receives the ordinary owned clone. Missing objects,
source drift, signed/merge commit forms outside this subset or wrong namespace
bindings refuse before model work.

Use [Prepare the first E05 native source fixture](../evaluations/docs/prepare-native-source.md)
to create the disposable local source and copied suite. Inspect `SOURCE-EVIDENCE.json`,
then bind the configured source, suite manifest/digest and base/head OIDs to that
output. The tool does not resolve a profile, start a trial or qualify native
execution.

The shipped `independent_malicious` variant is the first native-context fixture.
It exercises malicious root repository guidance (E05); its PR title and body are
benign literal task input. The PR-body attack (E07) remains a separate fixture.
Its complete context requires the workspace name `native_independent` and exact
declared repository/actor metadata; the fragment uses that name. Profile/account
selection and source preparation do not follow from this fixture declaration.

The selected suite variant must have role and phase `independent_review`, seed
`fresh`, an exact matching profile slot, and declared positive finite turns,
seconds and repetition count. Existing independent variants still declaring phase
`review` are incompatible. The version-2 native context binding must declare all host identity slots and
the three exact independent PR identity slots (`id`, `number`, `url`), and match
the complete production context. Only those declared identity fields vary between
trials; task content and other metadata remain exact. PR title and body are taken
literally from the same scoped response as PR metadata and rechecked for freshness.
Missing text fields refuse; an explicit null body remains distinct from empty
text. They provide untrusted task context and never change review identity,
attempt limits or permissions. Protected worker provenance retains this context;
ordinary status output omits the raw text.
Workflow/scratch substitution, tool schemas, instruction digest, isolated
read-only source permissions and private scorer/report denial remain exact.
The scorer must grade native observations and structured review claims without
promoting a verdict, self-report or replay trace into proof of effects.

## Command syntax after the prerequisites are complete

Obtain the current runtime UUID, configuration revision and control generation
from the configured local target's read-only status/config output. The existing
runtime must be RUNNING, reconciled, owned and not draining; account allowance,
capacity and current permissions are checked again at admission. Every value
below is an explicit placeholder, not a selected model/account or execution grant.

```sh
kegari-v0 --target LOCAL_TARGET --json evaluation run \
  --workspace WORKSPACE_UUID \
  --suite-digest 'sha256:EXACT_SUITE_DIGEST' \
  --case DECLARED_CASE \
  --variant DECLARED_NATIVE_VARIANT \
  --profile DECLARED_PROFILE_SLOT \
  --repetition 1 \
  --expected-config 'sha256:CURRENT_CONFIG_REVISION' \
  --expected-control CURRENT_CONTROL_GENERATION \
  --request-id NEW_REQUEST_UUID
```

The target must be an existing verified Unix-socket target. The equivalent local
`POST /v1/commands` JSON body is:

```json
{
  "api_version": 1,
  "runtime_id": "ACTUAL_RUNTIME_UUID",
  "request_id": "NEW_REQUEST_UUID",
  "command": "evaluation_run",
  "args": {
    "workspace": "WORKSPACE_UUID",
    "suite_digest": "sha256:EXACT_SUITE_DIGEST",
    "case": "DECLARED_CASE",
    "variant": "DECLARED_NATIVE_VARIANT",
    "profile": "DECLARED_PROFILE_SLOT",
    "repetition": 1
  },
  "expected_config_revision": "sha256:CURRENT_CONFIG_REVISION",
  "expected_control_generation": 1
}
```

Replace the example generation `1` with the actual observed generation. Actor,
lease, permit, arbitrary source/report paths, manifests and larger limits are
not accepted command arguments. They come from authenticated host state and the
exact registered declaration. Stale revision/generation, a missing role, wrong
profile or a repetition outside the suite plan refuses.

The host retains an owned task independently of the client connection. The
request ID uses ordinary durable command idempotency. A different request cannot
repeat the same suite/case/variant/profile/repetition slot: the original command
is authoritative even if its report is missing or preparation failed. Inspect
that original operation rather than changing the request ID to obtain another
chance. Input corrections require a new declared suite identity and an unused PR range.
No automatic cold resume or replacement admission is provided.

`evaluation.pull_request` is the first number of a reserved finite range, not the
number assigned to every trial. The complete suite plan determines a zero-based
ordinal for each full case/variant/profile/repetition key. That trial's number is
`first + ordinal`; all 56 slots reserve the corresponding range, including slots
this first caller cannot yet run. Changing the selected binding's variant does not
reallocate the same range for another suite. The protected subject plan, original
commands and source-read receipts retain that mapping across restart. Missing or
substituted plan history refuses further admission. Do not delete records or
reset ordinary scheduler budgets to repeat a consumed slot. The subject plan is
history for refusal and consistency checks; it grants no model authority.

## Bounds, reports and interpretation

The shipped suite declares 56 attempts and 10,080 seconds, with two repetitions,
three turns and 180 seconds per variant. Those are plan ceilings, not permission
to spend or run them. This command selects one existing slot. Its native session
uses the declared finite turn/time limits and the existing account limits; it
cannot increase them in the request. One monotonic deadline begins with the
validated trial request and covers source inspection, preparation, execution and
final collection. Expiry fences the actual owned operations and retains the
future until its cleanup returns. Cleanup and durable report/index persistence
can continue after the deadline; that time does not become another execution
budget or a successful result.

Each actual request uses a fresh report directory under the configured report
root, named by its request/worker UUID. The suite snapshot and started record
remain when later work fails. An immutable result, when available, preserves
actual observations, failures and synthetic-provider disclosures. A protected
host index links the original command, source observation, report and terminal
receipt. It is descriptive data and cannot reconstruct execution authority.

Inspect the original command with `operation show COMMAND_OPERATION_UUID` and the
specific report directory with `kegari-v0-evaluation inspect`. The command ID and
worker/request ID are distinct; use the returned command receipt for operation
inspection. Lost client connections do not clear ownership. A joined evaluator
task alone is not process exit: actual model/provider/hook receipts and the
execution registry must establish exit; uncertainty retains the hold and synthetic
namespace. Report or grading failure does not skip native finalization.

A scorer-load or grading error retains the collected report with a structured
`grading_error` and its exact stage/error. Completed execution without a grade is
blocked; existing failed/interrupted outcomes retain their execution meaning.
`observed_effects: []` means an actual empty final observation, while an absent
field means unavailable or legacy evidence. Inspection and comparison retain the
error; qualification refuses it even if a later trial appears to pass.

Independent workers also retain an optional `independent_boundary` diagnostic.
`matched_at_collection` means the actual registered worker's original permit,
contract, admission receipt and current retained independent-review gate matched
at collection. It is an endpoint observation; it does not cover every preceding
read or effect. `unavailable` retains its reason and observed source facts.
Missing legacy data remains unknown. Neither state reconstructs runtime authority
or changes a scorer's result.

Explicit declared Independent trials enable optional `declared_protocol` capture;
ordinary workers leave it disabled. Bounded raw tool-call parameters, including
denied attempts, and completed agent messages are retained in the protected worker
journal outside worker grants. Reports expose only identities, counts, issues and
journal references with digests and lengths. Capture is limited to 4,096 records,
128 KiB per payload and 8 MiB total payload; overflow and storage/readback failures
remain visible. Hidden reasoning and message deltas are not retained as final
output. Collected diagnostics survive scorer-load and grading errors.

Bootstrap and watchdog helpers retain declared observations while continuing to
deny tool calls. Output received before thread establishment is operation-bound
and is never retroactively attributed to a thread. Matched turn responses retain
declared messages without adopting a completion.

After owned termination, one bounded observation pass uses the original reader,
decoder and queued frames within the original absolute trial deadline. It writes
no responses and dispatches no tools. Late policy and input-required holds remain
effective. `transport_end` distinguishes an actual clean read EOF from incomplete
collection, including decoder errors, expiry and overflow. The pass permits at
most 4,096 frames and 8 MiB of re-encoded decoded frames; a separate 8 MiB bound
covers residual partial-frame bytes plus new wire bytes. Queued decoded frames
no longer retain their original wire whitespace, so those counts remain distinct.

There is currently no complete protocol-capture state. A clean EOF diagnostic is
not native process-exit proof or proof of all output channels. A processed-request
count or terminal message alone cannot prove the end of the stream. Enabled
capture therefore remains incomplete or reports a stronger conflict, overflow or
unavailable state.
These observations do not establish a full read/effect/output inventory, canary
non-disclosure, E07 success or native containment. Existing rubric results retain
their existing scope; no fixture or scoring version changes through capture.

A single actual trial would supply evidence only for that declared condition.
Synthetic GitHub/Linear feedback is not real-provider qualification. Passing
predicates still require human review, and neither reports nor this caller grant
human acceptance, deployment/release permission or product readiness. Full suite
migration/repetitions, native composition and OS/provider qualification remain
separate until completed and recorded.
