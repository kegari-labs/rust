# Codex protocol and execution integration

The protocol client, session/history data, request gates, deadlines, protocol
recovery, observations and stream framing/I/O live in `kegari-codex`. Backend
`kegari_v0::codex_host` retains permission derivation, native launch and owned
process cleanup. Protected journals, actual receipt lineage, joined recovery and
resume admission stay in the backend. `ResumeCandidate` supplies consistent data;
it cannot construct `worker_run::RetainedResumeSession` or authorize execution.
Stream EOF is not owned process-exit or successful-work evidence.

**Migration status:** [SPEC 1.5](../SPEC.md) makes Codex app-server the repository
execution layer. The ordinary admitted child path uses native tools and the old
repository executor has been removed. Remaining integrations and focused checks
are tracked in [PROGRESS.md](../PROGRESS.md); live qualification is pending.
The pinned protocol client and protected workflow/session recovery remain useful.

The adapter targets **codex-cli 0.164.0-alpha.11.1**, profile
`kegari-codex-0.164.0-alpha.11.1-v1`. Its deterministic tests pass; live account,
agent, tool-discovery and OS-boundary qualification remain incomplete. This is
not permission to activate a coordinator.

## Provenance

On 2026-10-25 the installed executable reported the exact version above.
`codex app-server generate-json-schema --experimental --out target/codex-schema`
completed without creating a session or running a model. The compact schemas in
[`kegari-codex` protocol assets](../../kegari-codex/protocol/codex-0.164.0-alpha.11.1/)
retain generated structural constraints, omitting descriptive prose and titles.
`manifest.json` records both original generated-byte digests and compact-byte
digests. These files describe the installed protocol, not a hand-invented API.
They are mechanically extracted interface data from OpenAI Codex; no upstream
implementation source was copied into the Rust adapter. The checked-in generated
data retains upstream Apache-2.0 attribution and the complete public Codex notice;
see the adjacent `PROVENANCE.md`, `LICENSE-CODEX.txt` and `NOTICE-CODEX.txt`.
Each compact schema identifies the mechanical modifications in `$comment`.
The installed alpha source commit is not yet identified; current upstream
license provenance is distinguished from exact binary/source correspondence.

Current official references inspected:
[app-server](https://learn.chatgpt.com/docs/app-server) and
[permission profiles](https://learn.chatgpt.com/docs/permissions).
The installed schema takes precedence for the pinned profile. Documentation alone
does not establish account entitlement or an exercised sandbox boundary.

The pinned schema confirms `initialize`/`initialized`, `account/read`,
`account/rateLimits/read`, `model/list`, `thread/start`, `thread/resume`,
`turn/start`, `turn/steer` with required `expectedTurnId`, `turn/interrupt`,
`review/start` and dynamic tool requests/results. Native review uses an explicitly
bound thread and `target.type=custom` to receive the delivered review guidance,
requirements and exact revisions. It does not use the deprecated detached-review
fallback.

## Production path

The migrated path follows the existing operation and session boundaries:

1. Admit current work, source, role, account and finite attempt/turn limits.
   Configure the selected Codex profile's supported permissions; missing required
   capabilities block the affected role specifically. Independent PR review must
   not execute repository code; filesystem read-only access alone is insufficient.
2. Launch the verified pinned app-server with the authorized private Codex home.
   Verify the returned authentication home, stable ChatGPT account identity and
   complete model catalog. Model, reasoning and speed must match the selected
   supported profile; no paid provider fallback or silent version upgrade applies.
3. Verify effective supported permissions, assemble the role-scoped context and
   retain its manifest. Native Codex tools provide repository reads, edits and
   commands. Kegari exposes only needed workflow/provider tools. A thread may
   resume only with compatible account, work, instructions, scope and profile.
4. Persist admission before changing protocol requests. The bounded transport
   write uses the existing request gate; response waits and callbacks remain
   outside the control mutex. A lost response is uncertain, not permission to
   resend or obtain free turns.
5. Consume supported command/file and lifecycle events, retaining operation,
   thread, turn, item and source associations and available results. Unknown facts
   stay unknown. Kegari applies its required-check, review and acceptance gates;
   model prose and successful transport do not establish successful work.

Codex owns internal command launch, sandbox enforcement and supported terminal
management. Kegari must not recreate a shell/file executor, resource catalog,
per-command admission or process journal around those events. Its actual owned
app-server and deterministic Git/control processes retain their existing host
launch and recovery machinery. Old execution evidence keeps its original meaning;
it cannot silently become Codex-native result evidence.

The host must retain account/work intervention holds across *all* sessions. The
session state alone cannot enforce a host-wide ban on replacement work. It must
also recheck authority immediately before broker effects; transport success is
never a workflow gate.

The concrete worker factory in [`worker-run.md`](worker-run.md) binds a
`RequestGate` to every changing protocol request. Only its bounded pipe write
runs under the fresh serialized host gate; responses and tool callbacks run
outside that mutex. Native process creation and owned-handle registration use
the same fresh permit boundary. Bounded version probes and provider cancellation
explicitly reap the actual owned child. A cached confirmed exit remains visible
even when a concurrent cancellation request holds the child mutex. Deterministic
fixtures exercise these races without starting an installed Codex model.

## Intervention and outcome handling

The generated schema supplies `model/verification` (currently including
`trustedAccessForCyber`) and the error classifications `cyberPolicy`,
`misalignmentPolicyViolation` and `tooManyDenials`. These produce persistent
provider-intervention records with account/session/source digest and required
resolution. They survive process exit, watchdog expiry and state serialization.
The distinct `model/safetyBuffering/updated` notification is transient and does
not create that persistent hold.

Interactive input and escalation requests receive a structured refusal/empty
input response and a planning blocker, never an operator question. Unsupported
server tools receive a JSON-RPC error; unsupported dynamic tools receive a
structured failed result. No approval-rule amendment or broad auto-approval is
sent. A normal process exit does not imply successful work. Diagnostics are
drained independently and never parsed as protocol or copied into public status.

There is deliberately no “clear intervention” assertion API. The pinned schema
alone does not prove which supported verification/policy resolution permits
resumption of each intervention. A live qualification record must establish that
flow before operational use of a clearing transition. Source implementation may
proceed under the focused-validation policy without claiming that live resolution
is qualified. A new model, account, thread or retry must not bypass it. This missing qualification is an
outstanding §7.3 requirement, not a completed recovery capability.

## Local probes and limitations

A credential-free, newly owned `target/codex-profile-probe/home` was used with
`app-server --print-startup-config --strict-config`. The installed binary accepted
named filesystem rules, disabled network, `approval_policy="never"`, disabled
apps/web search and selected feature controls. It returned:

```json
{"appServerVersion":"0.164.0-alpha.11.1","configRequirements":{"supportsIndependentSpeedModes":true,"requirements":null}}
```

No model, account login, real provider mutation or persistent installation was
performed. This parse probe does not prove effective tool exclusion. Feature
listing showed `apply_patch_freeform` is **removed**; its setting is not used as
proof that patch tools are disabled.

The attempted local probe `codex sandbox macos --help` entered sandbox setup and
exited **71** with `sandbox-exec: sandbox_apply: Operation not permitted` under
this build conversation's outer sandbox. It did not run an evaluated agent.
There was no unsandboxed retry or host security change. Executing nested sandbox
canaries is therefore **blocked in the present environment**, while deterministic
path/permission generation and rejection tests remain available.

Before operational qualification, a separately authorized disposable local profile
must demonstrate source/scratch separation, forbidden home/controller/Unix socket
reads and writes, HTTPS/network denial, symlink/hardlink denial, no execution for
independent review, and required root/nested instruction discovery. Actual
ChatGPT identity, model entitlement, steering, native review, multiple-account
separation and locked-session behavior also remain not run. Record every denial
and failure; never convert these missing checks into passes.

## Deterministic evidence

During implementation use the smallest relevant existing check for changed
behavior. Comprehensive coverage and authorized actual-agent qualification belong
to milestone 0.7, after core flows work. These focused commands are references,
not a mandatory sequence for every increment:


```sh
CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo +stable test --offline --locked -p kegari-codex
CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo +stable test --offline --locked -p kegari-v0 --test codex_protocol
```

The local environment uses the preinstalled stable alias for the exact 1.99.1
compiler; the alias is not a permission to select another compiler version.

The deterministic protocol fixtures exercise bounded fragmented/coalesced JSON-RPC,
malformed/oversized frames, exact outgoing pinned schemas, input/tool refusal,
complete model pagination failure, native review, resume context, correction and
completion races, duplicate notifications, persistent policy/verification holds,
transient buffering, request timeout and no second mutation, account/profile
binding and cumulative usage. Retain applicable role/source-boundary checks
while removing tests that enforce the superseded repository executor.
They use controlled duplex transports and files; they do **not** count as actual
agent behavior trials, real provider validation or complete V0-T10/12/14/34
qualification.

## Metadata-only connection verification

`codex_host::probe_connection` implements explicit setup/reconnect verification using
an authorized private Codex home and the caller's narrow probe permission plan.
It runs the pinned version probe, initializes stdio app-server, reads the exact
ChatGPT account without token refresh, enumerates its model catalog, reads
allowance metadata and verifies effective permissions. It creates no thread or
turn and performs no login. The result binds account, canonical auth home,
permission digest and the actual native argument digest; injected transports
leave argument provenance absent. The probe has bounded request/overall duration
and terminates its owned process on success or failure. This implementation has
only been exercised with finite deterministic protocol responses in this build;
no native account/authentication probe was invoked.

The ordinary worker path now uses Codex-native repository tools; the competing
custom executor has been removed. The migrated path checks Codex version,
supported permissions and required capabilities directly. Remaining integration
and qualification work is recorded in [PROGRESS.md](../PROGRESS.md); unavailable
capabilities are specific refusals, not a reason to rebuild a second executor.
App-server exit or closed streams do not prove arbitrary descendants stopped;
use supported Codex lifecycle observations and preserve uncertainty.

## Session recovery

Before a recorded thread is resumed, `thread/read` with `includeTurns: true`
reobserves its exact account, thread identity, private cwd, provider and complete
ordered completed-turn inventory and full retained item/content fingerprints.
Summary/not-loaded views, missing item content and legacy journals without full
provenance cannot qualify. An active, failed, duplicate, missing or additional
turn blocks reuse. A policy or verification signal is durably reported through
the normal broker immediately; a later history mismatch cannot erase it. The
read is bounded by the ordinary frame limit and a 4,096-turn inventory limit.
It supplies a digest of actual provider history to reconciliation, without
starting another turn or treating history as a verified reason to continue.
The request/response schemas are pinned from the same installed CLI schema
output as the original protocol inventory. Deterministic duplex fixtures cover
history divergence and intervention retention; native resume qualification has
not been run.

An interruption preserves the known unresolved turn ID in the durable state;
restart cannot turn it into a clean idle session. An exact late terminal event
can resolve that identity but is not itself permission to retry or success of the
work. The usage ledger exposes a new-session constructor that retains only the
same account/thread cumulative baseline and unknown-history marker, leaving old
token totals and elapsed time on the original session. Production resumed
sessions consume the sealed recovery/readmission bridge described below; raw
prior-state JSON cannot select the resumed launch mode.

A separate narrow terminal-failure profile now produces
`VerifiedThreadRecovery`. Every actual `turn/start`, `turn/steer` and
`review/start` records its exact frame/input fingerprint before transmission and
an acknowledged native turn identity after the response. The recovery profile
accepts only a complete one-to-one inventory of acknowledged `turn/start` text
inputs and full user/agent/reasoning items, with completed earlier turns and one
final `serverOverloaded` or `internalServerError` turn. Steering, review starts,
unknown responses, effectful/unknown items, corrections, input requirements and
legacy missing provenance remain incompatible. Full native input bytes must match
the actual submitted input fingerprint; summaries or unchanged IDs are
insufficient.

Complete failed-turn observations also retain a minimal typed transient code for
bounded fresh-thread retry planning. Only `serverOverloaded` and
`internalServerError` qualify, with no policy/misalignment or input requirement.
The protected history links that code to the terminal and error digests; it does
not serialize raw error messages. Restore rejects inconsistent terminal links,
and legacy history gains no inferred classification. The getter is observational:
retry admission still needs the actual joined completion, exact receipt and
current authority; caller-created state is not a retry capability.

The guarded metadata wrapper uses a new owned transport and only
initialize/account/config/thread-read operations. It retains intervention
notifications even during initialization, refuses unsolicited server requests,
and never exposes a general RPC client. Deterministic duplex scenarios exercise
exact input/full-history match, changed input/history, nontransient failure,
policy retention before mismatch, summary views, unsolicited tools, wrong
account, and the entire metadata handshake. Separate actual-control/protected
journal fixtures exercise finite admission and fencing before any spawn. These
are component evidence, not native account access or completed provider resume.

`AdmittedSessionResume` binds the actual typed run admission, verified original
and current complete assemblies, metadata recovery and enrolled directory family.
The adapter rereads full history immediately before `thread/resume`, transmits
the actual current assembled prompt, and records that readback on the new
session. Its new turn counter, submitted inputs, history and usage totals remain
separate from the original operation. Cumulative usage arriving before the new
session's first turn advances its baseline without charging the new operation.
The same applies later when the notification's actual `turnId` is not an active,
completed or unresolved turn of the new session. Its timing remains explicitly
unknown; the normalized event preserves the actual turn identity.
The first monotone counter for a known new-session turn also establishes only an
unqualified boundary: retained terminal history does not authenticate the old
session's final token total. Its increase is left unattributed, with the observed
counter retained as `first_new_turn_boundary`; only subsequent known-turn
increments are charged. A lower counter cannot establish or lower this boundary.
Known increases are retained as `prior_unattributed_usage.known_increase`; an
absent original baseline remains explicitly marked unknown alongside any later
measurable increase. Thus new-operation totals can be incomplete; neither the
ambiguous first increase nor `tokenUsage.last` is guessed to belong to a turn.
Duplex tests cover the absence of a final old-turn update, delayed old-turn
updates before and after the new boundary, repeated/lower counters, and later
new-turn increments, as well as changed history
and policy signals arriving between metadata preparation and resume.

Repeated recovery uses a versioned ordered prefix with one immutable segment per
original operation: exact full-turn fingerprints, submitted text/frame digests,
original run/receipt/journal identities, verified transient boundary and original
usage ledger. Each full readback must match every prior segment and the current
session, including its actual submitted text. Earlier failed turns are allowed
only at those exact previously verified transient boundaries. RPC identifiers
are local to each operation; reusing a number on a new transport cannot replace
an earlier operation's input. Native gates recheck the actual typed resume receipt
chain as well as the immediate genuine joined worker and current source/family.
Missing legacy prefixes, changed/reordered/truncated history and unknown or
nontransient prior boundaries remain ineligible. Earlier usage and unattributed
summaries remain under their original operations. A successor carries the last
cumulative baseline and sticky unknown-attribution marker, then establishes its
own unqualified first-turn boundary without charging inherited totals again.
The same phase history and minimum retained caps govern every successor.
Duplex component coverage exercises two consecutive resumes, repeated RPC IDs,
changed prefixes and new-session usage deltas; it does not manufacture a native
joined cache or qualify a live provider. This profile still excludes corrected,
cold, active, effectful and review histories.
The sealed origin retains the bounded actual historical turn-ID inventory.
Replayed historical IDs in a new start acknowledgement, start notification or
completion are rejected without copying old results into the new session;
same-thread policy observations are retained before that rejection.
Matched turn acknowledgements persist policy signals before owner/fence
bookkeeping can fail. The original request thread and a nonempty returned turn
ID establish identity even when ordinary turn fields are malformed; inline
review additionally requires its exact returned review-thread ID. Foreign
review replies and replies lacking enough identity cannot create a hold on the
original session. Duplex regressions verify this ordering with deliberately
failing acknowledgement bookkeeping; they do not qualify native ownership.
