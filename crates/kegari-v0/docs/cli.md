# CLI and client transport

The `kegari-cli` package owns the `cli`, `client`, `local_cli` modules and the
`kegari-v0` executable. Build or run it with `-p kegari-cli`. Its temporary backend
dependency supplies shared contracts and native operations without a reverse
dependency. Both packages remain workspace default members; use `--workspace`
for complete validation. The evaluation-inspection tool stays in `kegari-v0`.

The executable routes public commands through the typed local/HTTPS client and
dispatches exact hidden build/supervisor/host entrypoints. `local_cli` implements client-only preferences,
bounded terminal TOML guidance, explicit local bootstrap, browser opening and
native service actions. The executable supplies `NativeRuntimeFactory` through `host`;
provider/Keychain/native ownership qualification remains incomplete. No real host
bootstrap, LaunchAgent installation/start/stop, browser launch, credential import
or remote mutation was executed during deterministic tests. Service installation
fixtures use synthetic metadata executables and disposable account directories;
they never invoke launchctl or modify the actual OS account home.

`kegari-v0 [--target NAME] [--json] COMMAND` defaults to `overview`. An omitted
target selects only the registered local Unix endpoint. An explicit target
selects one named HTTPS origin/runtime pin. Missing/unreachable targets fail;
there is no fallback or automatic repinning. The public grammar follows every
command in SPEC §10.1. Hidden supervisor/build-information modes belong to the
entrypoint, not remote commands.

## First install and local status

With no command, `./kegari-v0` means `overview`. Before initial setup alpha.4 reports
`target_unavailable: no local runtime is configured` and directs the operator to
`kegari-v0 setup --help`. Alpha.2 used the earlier message
`no registered local runtime; use explicit local setup` for the same condition.
This is an absent registration, not an invitation to launch a hidden entrypoint.
`--help`, `--version` and `setup --help` do not require a registered runtime.

Alpha.4 can be inspected as a development artifact, but is not qualified or ready
for activation. The sequence below describes the intended authorized installation
after the supported profile and required native/provider prerequisites are
established; it is not a recommendation to activate this prerelease now.

For that installation, first copy the binary to its final protected, versioned
path. Interactive initial local `setup` then collects a non-secret operator label,
your Linear user UUID, an explicit positive concurrency limit, execution root,
and local operator roles. It obtains the host identity and OS user mapping from
the invoking host and generates a new runtime UUID. A supplied Linear UUID is
operator-provided until a configured connection verifies the provider identity.
There is no invented account or provider discovery.

The displayed role defaults are `inspect`, `configure`, `lifecycle`, `correct`,
`accept`, `release` and `recover`; they apply to the setup operator. You can choose
an explicit subset containing `configure`, or explicitly add `evaluate`. The
initial configuration has no connections, workspaces, remote control, repository
selection or release scope. The guide displays the complete validated non-secret
TOML and what remains to configure. Only typing `create` submits it. `/cancel`,
EOF, an invalid answer, or a failed review output submits nothing. Answers are
bounded to 4 KiB each. Credentials are never requested by this guide.

You can instead supply a complete TOML document using the
[configuration contract](configuration.md) and `setup --from PATH --noninteractive`.
That route keeps the same validation and actual-host comparison; it does not
prompt or fill missing identities. A standalone host-identity export command and
universal provider/workspace configuration template are not provided.

The explicit sequence is below; replace the example absolute binary path with the
actual installed binary. Each step must succeed before
continuing. These commands perform setup and service actions, not just inspection.

```sh
/absolute/versioned/kegari-v0 setup
/absolute/versioned/kegari-v0 service install --binary /absolute/versioned/kegari-v0
/absolute/versioned/kegari-v0 service start
/absolute/versioned/kegari-v0 status
/absolute/versioned/kegari-v0 config check --noninteractive
```

Initial setup records STOPPED intent. Service installation writes the LaunchAgent
without loading it; `service start` starts the control host and preserves that
intent. Next use `connection add` with supported authentication and `workspace add`
with real IDs, exact selectors, profiles, repository/check contracts and an
explicit release policy. The empty guided configuration grants none of that scope.
Check actual readiness with `status` and `config check`; metadata checks
do not establish operational qualification. Only after qualification and current
execution prerequisites are satisfied, and the operator explicitly intends to
admit work, the separate orchestration command is:

```sh
/absolute/versioned/kegari-v0 start
```

The earlier setup/service sequence does not grant qualification or start product
work. Do not use `start` to bypass a reported blocker.

Guided setup for an existing or explicit remote target uses the bounded
whole-document TOML editor described below, never the client host's generated
initial configuration. A registered but unreachable runtime requires
service/ownership diagnosis, and partial setup
requires reconciliation; neither condition authorizes deleting registration or
rerunning setup as a replacement installation.

## Runtime commands

```sh
kegari-v0 --json overview
kegari-v0 --target studio overview --workspace WORKSPACE-ID
kegari-v0 show WORKSPACE/ABC-123
kegari-v0 open WORKSPACE/ABC-123 --url-only
kegari-v0 approve WORKSPACE/ABC-123 --result EXACT-EVIDENCE-TOKEN
kegari-v0 correct WORKSPACE/ABC-123 --message-file ./correction.txt
kegari-v0 release WORKSPACE/ABC-123 --result EXACT-ACCEPTED-TOKEN
kegari-v0 pause --expected-control 12
kegari-v0 stop --force --wait --timeout 30
kegari-v0 operation show OPERATION-UUID
kegari-v0 operation reconcile OPERATION-UUID --request-id REQUEST-UUID
kegari-v0 release recover WORKSPACE/ABC-123 --operation OPERATION-UUID --action resume --reason 'provider outcome reconciled'
kegari-v0 cleanup --operation OPERATION-UUID
kegari-v0 cleanup --operation OPERATION-UUID --apply --plan EXACT-PLAN-TOKEN
```

Work references are passed to the selected runtime for canonical resolution before
its authorization/receipt transaction. Ambiguity must return candidates without
mutation. Lifecycle commands reject `--workspace` and affect the whole runtime.
If `--expected-control` is omitted, the client reads current generation once and
submits that exact value. A stale conflict is returned without refetch/resend.
`--wait` defaults to 30 seconds. A waiting timeout never forces, stops, repeats or
restarts the operation; operation identity and pending observation remain visible.
Each successful observation must identify that same operation; another operation's
completion or an unidentified response cannot satisfy the wait.

Every mutation has one request UUID, explicitly supplied or generated once during
preparation. The final transport submits only once. A lost acknowledgement keeps
that UUID and reports uncertainty, without inventing an operation ID. Use
operation/status reconciliation; do not treat a network error as permission to
create another request and repeat a provider effect. The server remains responsible
for actor/payload-bound receipt idempotency and conservative provider recovery.

## Input and configuration

Corrections take exactly one of `--message`, `--message-file` or `--stdin`, with a
1 MiB UTF-8/nonempty bound. Files are read on the CLI device. Setup, connection
and workspace input is typed TOML, limited to 1 MiB; `--from -` explicitly reads
configuration from stdin. Noninteractive missing fields fail without prompting.
Except for the initial local setup guide above, interactive guidance is a bounded
TOML document editor: it describes the required
fields, accepts lines until a line containing only a period, and validates the
whole document before submitting it through the same transaction as `--from`.
EOF before the terminator cancels without submission. The document editor does
not echo its input; the initial setup guide deliberately displays its generated
non-secret configuration for review. Credentials belong only in the distinct secret
import path. Guidance cannot bypass validation. Configuration edits still require the server's
complete cross-connection/workspace validation and serialized apply.

```sh
kegari-v0 setup --from ./runtime.toml --noninteractive
kegari-v0 connection add github --kind github --from ./github.toml --secret-stdin --noninteractive
kegari-v0 connection reconnect github --noninteractive
kegari-v0 workspace add product --from ./workspace.toml --noninteractive
kegari-v0 workspace configure WORKSPACE-ID --from ./workspace.toml --expected-config EXACT-REVISION --noninteractive
kegari-v0 config check --workspace WORKSPACE-ID --noninteractive
```

`--secret-stdin` is distinct from configuration stdin; they cannot be combined.
It requires non-terminal input and refuses echoing interactive terminal reads.
The credential bound is 64 KiB. One terminal newline (and its optional preceding
CR) is removed; other bytes are preserved and UTF-8 validated. The value remains
in nonserializable `Secret`, omitted from ordinary command/debug/config/receipt
objects. Only the final verified Unix/TLS body receives `args.secret_input` for
connection addition. The server strips it into its separate secret argument
before dispatcher/config/receipt processing and imports it only after supported
provider identity verification. A client Keychain reference is never reinterpreted
as a remote credential. Reflected credential material causes the entire unsafe
response to be omitted, rather than displaying a partly redacted secret.

Initial setup reaches `LocalActions::bootstrap` only when no explicit remote
was selected and the host verifies that no registration/control configuration
exists. Partial setup must produce a reconciliation error. Existing/stale local
registration uses that host or fails; it cannot trigger a replacement bootstrap.
Remote setup never falls back to local setup. Bootstrap must acquire `control.lock`,
validate config, establish STOPPED intent and not dispatch agents.

## Client-only actions

```sh
kegari-v0 target add studio --url https://host.example.ts.net --runtime RUNTIME-UUID
kegari-v0 target list
kegari-v0 service install --binary /absolute/versioned/kegari-v0
kegari-v0 service start
kegari-v0 service stop
```

Targets are stored in the actual OS account's `~/.kegari/clients.toml`, with
owner-only files and a serialized client preference lock. The path comes from
OS account lookup, never a client-supplied HOME. Remote commands do not inspect
local owner/config/intent records, query native boot identity, initialize Keychain,
or create scheduler/operation state. Only client preferences may be created. `target add` verifies the supplied expected runtime over normal
TLS before saving it. A conflicting existing name is refused; no observed UUID
can replace the explicit pin. Target preferences and service actions reject
`--target`; they do not invoke a remote shell. Local service actions use the platform implementation and preserve durable
orchestration intent. An installed immutable executable must return matching
API/schema/package/source-build identity from `__build-info`; service installation
refuses a different registered binary or existing unrelated plist. Copy/install
the binary into its final versioned protected path before invoking initial setup
from that binary. Upgrades require explicit reconciled handover; replacing a
registered executable or rerunning setup is not a migration mechanism.

Installation writes the per-user LaunchAgent and its protected registration; it
does not load the service. `service start` checks the immutable binary and current
owner, never force-restarts a live host, and reports `start_requested` until status
confirms readiness. It clears an explicit shutdown marker only under the control
lock after the prior host has exited. `service stop` first sends the closed local
API command: the host atomically verifies STOPPED/no active owned work and reserves
durable shutdown. Only after that acknowledgment and process exit does the client
acquire the registration lock, recheck the marker and remove the launchd job.
A concurrent restart changes the marker and blocks removal. A timeout keeps the
original request/operation identity and uncertainty; it never resends a mutation.
The marker makes supervised restart fail closed until explicit service start;
`__control-host` exits successfully for this intentional stopped condition.
No local service action is available through a remote target.

After recovery, unfinished local admissions or suspended-owner records prevent
service-shutdown reservation even if live process maps are empty. A latched
control-persistence failure also requires recovery before reservation. Replaying
an unfinished shutdown reports `operation_uncertain` and does not repeat it.
Once local ownership has retired, remote-only uncertainty may remain recorded
without preventing service stop; handover still has its own reconciliation gates.

`open` obtains the current Linear URL/token through a read operation. The browser
callback runs only on the reviewing client, and only for an authoritative HTTPS
`linear.app` URL. `--url-only` never invokes it. Browser failure returns the URL
and a client failure without changing workflow. The native launcher uses argument
passing, no shell, and a finite timeout.

## Transport, pagination and output

The production `ControlClient` uses reqwest's pinned version with normal rustls
certificate-chain and hostname validation. No insecure mode, proxy environment,
redirect, HTTP/2 replay or retry policy is enabled. The local transport connects
only to the registered owner-protected Unix socket and validates its private
ownership/path before each request. HTTPS origins reject userinfo, path, query
and fragment. Every request sends the expected authority, runtime and protocol;
responses must match those pins and the request identity. Pre-body auth/input
rejections retain the client's original request ID without pretending to be a
mutation acknowledgement.

All requests use the shared closed `POST /v1/commands` contract, including
read-only commands; the server must preserve their read-only semantics. The
server also exposes the normative GET routes. Each body is bounded to 8 MiB,
headers to 32 KiB, and each request to 10 seconds. Collection/detail responses
must be `snapshot::Page`, even for one page; small status/open results may be
direct values. The client reads every page and verifies immutable identity,
config revision, ordered cursors, UTF-8 chunks and complete projection digest
before producing output. Snapshot expiry discards all pages and restarts the
read once. No mixed or truncated view is returned. The explicit client resource
limits are five minutes, 8192 pages and 128 MiB aggregate page data; exhaustion
is an incomplete-snapshot error, not a partial success.

JSON output is one complete UTF-8 response envelope plus a newline; diagnostics
belong on stderr. Client-only operations without a runtime use `runtime_id:null`,
not an invented UUID. Human output retains target/runtime/request/operation/state,
result/reason, full detail and next action. Overview always renders Ready for
release, Ready for review, Blocked and Running in that order, including empty
sections; supplementary history/health/independent jobs remain separate. Root
blocker priority and canonical resolution are authoritative server projections,
not guesses made by the client. Exit codes follow SPEC §10.1; a durably accepted
pending/draining mutation is distinct from a completed/failed/unknown operation.

Read-only `overview`, `show` and `open` can resolve authenticated retained release
and no-release-required history. A removed workspace is selectable by its UUID; a retained display name
is not a new configuration alias. Parent references are deduplicated by runtime
workspace and complete native identity, with ambiguity reported explicitly.
`show` retains historical attempts and exact completion/hold diagnostics when a
current parent projection is absent. Separate `no_release_history` rows preserve
the original generation, completion time and observed publication identity without
inventing a release attempt. Missing required references remove the completion
marker while retaining an authenticated diagnostic row. `open` requires an actual observed,
credential-free HTTPS parent URL; missing or conflicting retained URLs return an
error. Historical identity does not make a target eligible for mutation, recovery,
acceptance or release.

## Deterministic validation

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --offline --locked -p kegari-cli --test cli --test local_cli --test main
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --offline --locked -p kegari-v0 --test platform
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo clippy --offline --locked --workspace --all-targets -- -D warnings
```

Fixtures exercise real Unix HTTP transport, TLS rejection, parser/input rules,
selected-target pins, generation conflicts, lost-response identity, credential
separation/reflection refusal, complete/expired/mixed snapshots, client-only
browser behavior, bootstrap refusal and stdout/exit contracts. Public fixture TLS
keys are synthetic test material. They do not prove real Tailscale identity,
actual launchd/bootstrap behavior, a live account flow or production TLS
provisioning; those require separately authorized integration qualification.
